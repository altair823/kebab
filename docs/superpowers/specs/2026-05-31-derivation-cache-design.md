# 내용 해시 기반 파생물 캐시 (Derivation Cache)

> 작성 2026-05-31. 비용 큰 ingest 파생물(embedding 벡터 / LLM 별칭 / 한국어 형태소)을
> 청크 **내용 해시** 키로 캐싱해, 문서 갱신·재색인 시 변경되지 않은 청크의 재계산을 없앤다.

## 1. 문제

현재 kebab ingest 는 **doc 단위 skip**(`try_skip_unchanged`, lib.rs:894)만 한다. 변경된
문서는 모든 청크를 재파싱·재청킹·재임베딩·재별칭한다(`put_chunks` 가 doc 의 청크를
통째 DELETE 후 재INSERT — documents.rs:113, embedding/alias/tokens 무조건 재계산).

측정 증거: 정답 18개 문서의 별칭 재생성에 **2.5시간**(gemma LLM, doc 당 ~39청크).
embedding 도 전체 재계산. 문서 한 줄만 고쳐도 동일 비용이 든다. 실사용(나무위키
~2천 문서) 시 재색인이 비현실적으로 느리다.

`chunk_id` 는 `id_for_block` 의 `ordinal + span`(ids.rs:160) 때문에 **위치 기반**이라,
chunk_id 를 캐시 키로 쓰면 중간 수정 시 뒤 청크가 전부 무효화된다 → 캐시 키는
**청크 text 의 내용 해시**여야 위치와 무관하게 재사용된다.

## 2. 목표 / 비목표

**목표**
- ingest 시 청크별로 (embedding, alias, korean_tokens) 를 내용 해시로 캐싱.
- 캐시 히트 시 비싼 계산(embedder.embed / LLM.generate / lindera tokenize)을 건너뜀.
- 모델/프롬프트/토크나이저 버전을 캐시 키에 포함 → §9 version cascade 와 정합
  (버전 변경 시 자동 cache miss → 재계산).
- 별칭뿐 아니라 비용 큰 파생물 전반에 동일 메커니즘.

**비목표**
- 청크 단위 diff (put_chunks 의 전체 DELETE/INSERT 는 그대로 둔다 — chunks 행 재생성은
  싸다). 캐시는 *계산*만 절감한다.
- chunk_id 생성 방식 변경 (위치 기반 유지 — frozen 동작).
- doc 단위 skip(`try_skip_unchanged`) 변경 (그대로, 캐시와 독립).

## 3. 설계

### 3.1 캐시 키

```
cache_key = blake3_hex( kind || 0x00 || text_blake3 || 0x00 || version_key )[:32]
```
- `text_blake3` = blake3(chunk.text 의 NFC 정규화 UTF-8 bytes).
- `kind` ∈ { "embedding", "alias", "korean_tokens" }.
- `version_key` (kind 별, 버전 변경 시 캐시 무효화):
  - embedding: `{model_id}|{model_version}|{dimensions}`
  - alias: `{prompt_version}|{max_aliases_per_chunk}|{model}`  (model="" 면 LLM 기본)
  - korean_tokens: `{tokenizer_version}` (현재 lindera 고정 → 상수 "lindera-v1";
    추후 토크나이저 교체 시 bump)

text 내용이 같고 버전이 같으면 문서·위치·chunk_id 와 무관하게 동일 cache_key.

### 3.2 저장소 — SQLite `derivation_cache` 테이블

신규 마이그레이션 `V012__derivation_cache.sql`:
```sql
CREATE TABLE derivation_cache (
  cache_key    TEXT PRIMARY KEY,   -- §3.1
  kind         TEXT NOT NULL,      -- 'embedding' | 'alias' | 'korean_tokens'
  payload      BLOB NOT NULL,      -- kind 별 인코딩 (§3.3)
  created_at   TEXT NOT NULL,
  last_used_at TEXT NOT NULL       -- LRU 정리용
);
CREATE INDEX idx_dcache_kind     ON derivation_cache(kind);
CREATE INDEX idx_dcache_last_used ON derivation_cache(last_used_at);
```
- `corpus_revision` 은 bump 하지 않는다 — 캐시 테이블 추가는 기존 데이터 무효화가
  아니다(순수 가산). 단 V012 자체는 schema migration 이라 release bump 트리거(§Versioning).

### 3.3 payload 인코딩
- embedding: `dimensions × f32` little-endian 바이트열 (1024×4 = 4096 B/청크).
- alias: 별칭 묶음 문자열의 UTF-8 (현행 `chunk.aliases` 와 동일 형식 — 줄바꿈 join).
- korean_tokens: 토큰 문자열 UTF-8.

### 3.4 ingest 흐름 변경 (kebab-app lib.rs)

각 파생물 생성 직전에 캐시를 조회한다. 의사코드:
```rust
// --- 별칭 (lib.rs ~1259) ---
if expansion.enabled {
    for chunk in &mut chunks {
        let key = cache_key("alias", &chunk.text, &alias_version_key);
        if let Some(p) = cache.get(&key)? {       // 히트
            chunk.aliases = Some(String::from_utf8(p)?);
        } else if is_nav_boilerplate(chunk) {     // (기존 skip 규칙 유지)
            chunk.aliases = None;
        } else {                                   // 미스 → LLM
            chunk.aliases = generator.generate(chunk);
            if let Some(a) = &chunk.aliases { cache.put(&key, "alias", a.as_bytes())?; }
        }
    }
}

// --- embedding (lib.rs ~1309) ---
// 1) 각 청크 cache_key 계산 → 히트/미스 분리
// 2) 미스 청크만 emb.embed(&miss_inputs) (배치 축소)
// 3) 미스 결과를 캐시에 put
// 4) 히트 vector + 미스 vector 를 합쳐 VectorRecord 생성 → lance upsert
// (별칭 dense 벡터도 동일하게 alias text 의 embedding 을 캐시; 별칭 개별 벡터는
//  각 별칭 문자열 text 로 embedding cache_key 재사용 → 별칭 임베딩도 캐시 적중)

// --- korean_tokens (chunker 내부 또는 호출부) ---
// tokenize 직전 cache 조회, 미스만 lindera 호출.
```

핵심: **embedding 캐시는 청크 본문 + 별칭 문자열 양쪽에 적용**된다. 별칭 dense 벡터도
"같은 별칭 문자열"이면 재사용된다(별칭 LLM 캐시 + 별칭 임베딩 캐시 2중 절감).

### 3.5 무효화 / 정리
- **버전 무효화**: version_key 가 cache_key 에 포함 → model/prompt/tokenizer 버전이 bump
  되면 새 키가 되어 자동 miss(옛 엔트리는 고아). §9 cascade 와 자동 정합.
- **고아 정리**: `kebab doctor` 또는 ingest 종료 시, `last_used_at` 이 N일(기본 30) 지난
  엔트리를 삭제하는 경량 GC. 또는 테이블 행수가 임계(기본 50만) 초과 시 LRU 삭제.
  (정리 정책은 plan 에서 상수화; 초기엔 30일 TTL 만.)
- 캐시는 **순수 성능 레이어** — 손상/삭제되어도 정확성 영향 없음(miss → 재계산).
  `kebab reset` 시 함께 비워진다(같은 sqlite).

### 3.6 정확성 보장
- 캐시 히트가 재계산과 **동일 결과**임을 보장하는 근거: embedding/LLM/tokenize 는 같은
  입력(text) + 같은 버전에서 결정적이어야 한다. embedding(e5, temperature 무관) ✓.
  LLM 별칭은 `temperature=0.0, seed=0`(config) 라 사실상 결정적 — 단 LLM 비결정성은
  "캐시가 첫 생성 결과를 고정"하는 것이라 오히려 일관성↑(허용).
- 버전 키 누락이 가장 위험한 실패 모드(옛 모델 벡터 재사용). version_key 에 모든
  cascade 인자를 넣고, 테스트로 "버전 변경 → cache miss" 를 고정한다.

## 4. 컴포넌트 / 파일

- `migrations/V012__derivation_cache.sql` — 신규 테이블.
- `kebab-core` — `derivation_cache_key(kind, text, version_key) -> String` 순수 함수
  (도메인, 다른 crate 의존 없음). text NFC 정규화 + blake3.
- `kebab-store-sqlite` — `DerivationCache` 저장소: `get(key) -> Option<Vec<u8>>`,
  `put(key, kind, payload)`, `touch(keys)`(last_used 갱신), `gc(ttl_days)`.
  `DocumentStore` 또는 별도 trait.
- `kebab-app` lib.rs ingest hook — 별칭/embedding 캐시 조회·저장 통합. embedding 미스
  배치 분리 로직.
- `kebab-chunk` — korean_tokens 캐시(선택, 우선순위 낮음 — embedding/LLM 이 주 비용).

## 5. Allowed / forbidden deps
- `kebab-core` 의 키 함수는 순수(blake3 + unicode-normalization 만). 다른 kebab-* 금지.
- 캐시 저장소는 `kebab-store-sqlite`. UI crate 직접 접근 금지(facade 경유).
- `kebab-app` 만 캐시를 오케스트레이션(ingest 경로).

## 6. 측정 / 검증
- 동일 corpus 2회 ingest: 1회차(cold) vs 2회차(warm, 전부 캐시 히트) 시간 비교.
  warm 재색인이 별칭 LLM 0회·embedding 0회여야(로그로 hit/miss 카운트 노출).
- 정답 18 문서 별칭: cold 2.5h → warm ~수십초(캐시 히트) 목표.
- golden eval: warm 재색인 후 variant 16/18 + refusal 동일(결과 불변 = 캐시 정확성).
- 버전 bump 시뮬: prompt_version 변경 → 별칭 전부 miss(재계산) 확인.

## 7. Risks / notes
- LLM 별칭의 미세한 비결정성: 캐시가 첫 결과를 고정하므로 재현성은 오히려 향상.
  단 "더 나은 별칭" 재생성을 원하면 prompt_version bump 로 무효화.
- payload BLOB 크기: embedding 4KB/청크 × 캐시 엔트리. 50만 엔트리 ≈ 2GB. TTL/LRU 로 관리.
- V012 는 schema migration → release version bump 트리거(CLAUDE.md §Versioning).
- 본 설계는 frozen design contract(§9 versioning)의 *의미*를 바꾸지 않는다(캐시는 그
  위의 성능 레이어). design 문서 수정 불필요; cascade 안전성만 version_key 로 보장.
