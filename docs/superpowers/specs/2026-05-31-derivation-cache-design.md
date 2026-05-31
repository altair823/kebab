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

`chunk_id` 는 `id_for_block` 의 `ordinal + span`(ids.rs) 때문에 **위치 기반**이라,
chunk_id 를 캐시 키로 쓰면 중간 수정 시 뒤 청크가 전부 무효화된다 → 캐시 키는
**청크 text 의 내용 해시**여야 위치와 무관하게 재사용된다.

> **`chunk_id` vs `cache_key` — 둘은 완전히 별개다(가장 혼동하는 지점).**
> - **`chunk_id`** 는 LanceDB 벡터 / SQLite chunk row 의 **식별자**다. `id_for_block`
>   이 `ordinal + source_span`(ids.rs) 을 canonical-JSON+blake3 한 **위치 기반** 해시라,
>   문서 중간이 밀리면 뒤 청크의 chunk_id 가 바뀐다. 이 작업은 **chunk_id 생성 방식을
>   전혀 바꾸지 않는다**(frozen 동작 — §2 비목표).
> - **`cache_key`** 는 `derivation_cache` 테이블의 **조회 키**다. `chunk.text` 의 NFC
>   정규화 **내용 해시** + kind + version_key 로만 만든다(위치·chunk_id·문서 무관).
> - 즉 위치가 밀려 chunk_id 가 바뀌어도, 내용이 같은 청크는 같은 cache_key 로 캐시
>   히트한다. chunk_id 는 "이 벡터가 어디에 속하나", cache_key 는 "이 내용을 전에
>   계산했나" — 묻는 질문이 다르다. 별칭 sentinel chunk_id(`{orig}#alias#N`) 역시
>   벡터 식별자일 뿐 cache_key 와 무관하며, 별칭 dense 벡터의 cache_key 는 **별칭
>   문자열 자체**의 embedding 내용 해시다(§3.4).

구체 예: 문서 중간에 헤딩/내용이 삽입되면 뒤 청크들의 ordinal/span 이 밀려
chunk_id 가 바뀌고 `put_chunks` 가 그 문서의 row 를 **전부 재작성**한다(싼 DB
write — chunk row + LanceDB 벡터 재기록). 그러나 내용이 변하지 않은 청크는
내용 해시 cache_key 가 동일하므로 embedding·별칭 캐시가 **히트**한다 → 비싼
재계산(e5 forward / LLM)은 **0**, 새로 삽입된 청크만 실제로 계산된다. 즉
"row 재작성(싸다)"과 "compute 재실행(비싸다)"을 분리해, 위치가 밀려도 compute
는 변경분에만 든다. 이것이 chunk_id 를 위치 기반으로 두면서도(diff 불필요)
재색인 비용을 없애는 핵심이다.

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
- `version_key` (kind 별, 버전 변경 시 캐시 무효화) — **구현 기준(e9b5202, lib.rs)**:
  - embedding: `doc|{model_id}|{model_version}|{dimensions}` — 맨 앞의 **kind 토큰
    `doc`** 은 PR #195 리뷰 반영. 임베더는 호출 kind 별 프리픽스(Document=`passage:`,
    Query=`query:`)를 붙여 *같은 text* 라도 다른 벡터를 만든다. 현재 ingest 는 Document
    고정이라 live 버그는 없지만, 미래에 query 임베딩이 같은 캐시를 타도 충돌하지 않도록
    방어적으로 분리한다(현재 토큰은 `doc` 상수).
  - alias: `{prompt_version}|{max_aliases_per_chunk}|{model}`  (model="" 면 LLM 기본).
    구현은 `expansion::PROMPT_VERSION`(현재 `"expansion-v1"`) + `max_aliases_per_chunk`
    + `exp.model` 을 `|` 로 join.
  - korean_tokens: `{tokenizer_version}` (현재 lindera 고정 → 상수 "lindera-v1";
    추후 토크나이저 교체 시 bump). **미구현(보류)** — embedding/LLM 이 주 비용이라 미적용.

text 내용이 같고 버전이 같으면 문서·위치·chunk_id 와 무관하게 동일 cache_key.
실제 키 함수는 `kebab-core::derivation_cache_key(kind, text, version_key)`
(derivation.rs): `blake3(kind ‖ 0x00 ‖ blake3(NFC(text)) ‖ 0x00 ‖ version_key)` 의
hex 앞 32자. `0x00` 구분자는 hex 다이제스트에 못 나오므로 kind/version 경계가 절대
섞이지 않는다.

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
  `derivation_payload::{encode,decode}_embedding`(kebab-app). 디코드는 길이가 4의
  배수가 아니면(손상) `None` → 미스 강등.
- alias: 별칭 **묶음** 문자열의 UTF-8 (현행 `chunk.aliases` 와 동일 형식 — 줄바꿈 join).
  즉 캐시 payload 는 LLM 이 청크당 생성한 별칭 *전체 묶음*이다. 이후 임베딩 단계에서
  이 묶음을 줄 단위로 쪼개 개별 벡터로 색인하는 것(§3.4)과는 별개 — alias kind 캐시는
  "이 청크 text 의 별칭 묶음을 LLM 으로 이미 뽑았나"만 기억한다.
- korean_tokens: 토큰 문자열 UTF-8. (미구현 — §3.1 참고.)

### 3.4 ingest 흐름 변경 (kebab-app lib.rs)

각 파생물 생성 직전에 캐시를 조회한다. 의사코드(e9b5202 lib.rs 기준):
```rust
// --- 별칭 (lib.rs ~1346) ---
if expansion.enabled {
    for chunk in &mut chunks {
        let key = cache_key("alias", &chunk.text, &alias_version_key);
        if let Some(p) = cache.get(&key)? {       // 히트 (비-UTF8 이면 None → 미스 강등)
            chunk.aliases = Some(String::from_utf8(p)?);
        } else if is_nav_boilerplate(chunk) {     // (기존 skip 규칙 유지)
            chunk.aliases = None;                  // 캐시에 넣지 않음(None 표현 불가)
        } else {                                   // 미스 → LLM
            chunk.aliases = generator.generate(chunk);
            if let Some(a) = &chunk.aliases { cache.put(&key, "alias", a.as_bytes())?; }
        }
    }
}

// --- embedding (lib.rs ~1434, fn embed_with_cache) ---
// 1) 각 청크 cache_key 계산 → 히트/미스 분리 (out: Vec<Option<Vec<f32>>>, 입력당 1슬롯)
// 2) 미스 청크만 emb.embed(&miss_inputs) (배치 축소)
// 3) 미스 결과를 캐시에 put
// 4) 히트 vector(슬롯)와 미스 vector(miss_indices 의 슬롯)를 각자 제자리에 채운 뒤,
//    슬롯 순서대로 collect → **입력 texts 순서와 1:1 보존**(off-by-one 없음).
//    이후 chunks.iter().zip(vectors) 로 VectorRecord 를 만들므로 순서 보존이
//    정확성에 직결된다.
```

순서 보존(§3.4 핵심 불변): `embed_with_cache` 는 히트/미스를 분리 계산하되 결과를
입력 인덱스 슬롯(`out[i]`)에 되돌려 채우고 그 순서대로 반환한다. 따라서 히트·미스가
섞여도 반환 벡터의 i번째는 항상 입력 text 의 i번째에 대응한다 — 호출부의
`chunks.iter().zip(vectors)` 가 잘못된 청크에 벡터를 붙이는 off-by-one 이 발생하지 않는다.

핵심: **embedding 캐시는 청크 본문 + 별칭 문자열 양쪽에 적용**된다(같은 `embed_with_cache`
+ 같은 `emb_version_key` 재사용). 같은 text 면 본문이든 별칭이든 같은 cache_key 로 적중하므로,
별칭과 동일한 문자열이 본문에도 있으면 한쪽 계산이 다른 쪽을 워밍한다(별칭 LLM 캐시 +
별칭 임베딩 캐시 2중 절감).

별칭은 **묶음 1벡터가 아니라 줄별 개별 sentinel 벡터**로 색인한다(`{orig}#alias#0`,
`#alias#1`, …). 근거: 측정(handoff §3.1)에서 청크당 별칭 8개를 줄바꿈으로 묶어 한 벡터로
임베딩하면 평균화로 특정 표현이 **희석**되어 오히려 변형 일관성이 악화했다(13/18). 줄별
개별 벡터로 바꾸자 16/18 로 회복. 구현은 `chunk.aliases`(묶음)를 `\n` 으로 split·trim 한
뒤 빈 줄을 거르고, 각 줄을 같은 청크 안에서 0부터 인덱싱해 `{chunk_id}#alias#{i}` 의
VectorRecord 를 만든다. 별칭 dense 벡터의 cache_key 는 **별칭 줄 문자열 자체**의 embedding
내용 해시이므로(본문 chunk text 가 아님), 같은 별칭 문자열이 재등장하면 캐시 히트한다.

// korean_tokens: tokenize 직전 cache 조회 + 미스만 lindera 호출 — **미구현(보류)**.

### 3.5 무효화 / 정리
- **버전 무효화**: version_key 가 cache_key 에 포함 → model/prompt/tokenizer 버전이 bump
  되면 새 키가 되어 자동 miss(옛 엔트리는 고아). §9 cascade 와 자동 정합.
- **캐시 엔트리 고아 정리(GC)**: `derivation_cache_gc(ttl_days)` 가 `last_used_at` 이
  N일(설계 기본 30) 지난 엔트리를 삭제한다(`ttl_days <= 0` 은 통째 wipe 방지 no-op).
  히트 키는 `derivation_cache_touch` 로 `last_used_at` 을 갱신해 GC 가 live 청크를 유지.
  **구현 상태(e9b5202)**: `touch` 는 ingest 종료 시 호출되어 wired 되어 있으나, `gc` 는
  store 메서드로 **존재만 하고 아직 어느 호출부(ingest/doctor)에도 연결되지 않았다**.
  즉 현재 캐시는 무한 누적이며, TTL/LRU 자동 정리는 후속 작업이다. 행수 임계(기본 50만)
  LRU 삭제도 미구현. 당장은 `kebab reset`(같은 sqlite 라 같이 비워짐)이 유일한 정리 경로.
- **stale 별칭 sentinel cleanup**(별개 — 캐시 GC 아니라 *벡터 스토어* 정리, PR #195 MAJOR):
  별칭 dense 벡터는 본문 청크가 아니라 줄별 sentinel `{orig}#alias#N` 로 LanceDB·
  embedding_records 에 색인된다. 이 sentinel chunk_id 는 SQLite `chunks` 에 **존재하지
  않아** 재색인/문서삭제 시 stale-set SELECT 에 안 잡힌다. 정리 안 하면 옛 별칭 벡터가
  남아 검색에 hit 하는 누수(리뷰 MAJOR). 따라서 재색인·삭제 경로가 본문 chunk_id 와 함께
  별칭 sentinel 을 양쪽에서 명시 삭제한다:
  - **LanceDB**: `alias_sentinel_ids_to_delete(body_ids, max_aliases_per_chunk)`
    (lib.rs) 가 본문 id + legacy `{orig}#alias` + `{orig}#alias#0..max-1` 를 모두
    생성해 `delete_by_chunk_ids` 의 exact-match `IN (...)` 로 삭제. `max` 는
    `expansion.max_aliases_per_chunk`(parse_aliases 가 강제하는 상한)라 index ≥ max 는
    절대 안 나오고, 안 쓰인 index 는 무해한 no-op.
  - **SQLite** `embedding_records`: `chunk_id LIKE chunks.chunk_id || '#alias%'`
    프리픽스 매칭(store.rs / documents.rs)으로 본문 chunk_id 의 모든 별칭 sentinel 행을
    함께 정리. 정확 일치 `|| '#alias'` 는 per-line sentinel 을 놓치므로 `%` 프리픽스 필수.

  이 두 정리는 **별칭 expansion 을 켰던 KB** 에만 해당하고, derivation_cache GC 와는
  독립적이다(캐시는 계산 결과 보관, sentinel 정리는 벡터 식별자 누수 방지).
- 캐시는 **순수 성능 레이어** — 손상/삭제되어도 정확성 영향 없음(miss → 재계산).
  `embed_with_cache` 는 길이 misalign payload 를, 별칭 경로는 비-UTF8 payload 를
  **미스로 강등**해 재계산한다(잘못된 결과 대신 재계산, §3.6 정확성 우선).
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
- `kebab-store-sqlite` — `SqliteStore` 의 inherent 메서드(derivation_cache.rs):
  `derivation_cache_get(key) -> Option<Vec<u8>>`, `derivation_cache_put(key, kind,
  payload)`(INSERT OR REPLACE), `derivation_cache_touch(keys)`(last_used 갱신, 1tx),
  `derivation_cache_gc(ttl_days)`(존재하나 미 wiring — §3.5). 별도 trait 안 만들고
  store 에 직접 단다.
- `kebab-app` — `embed_with_cache`(lib.rs, 히트/미스 분리 + 순서 보존 §3.4) +
  `derivation_payload`(embedding f32↔LE bytes encode/decode) + ingest hook(별칭/embedding
  캐시 조회·저장, hit/miss 카운트 로깅, touch 호출).
- `kebab-chunk` — korean_tokens 캐시(선택, 우선순위 낮음 — embedding/LLM 이 주 비용).
  **미구현(보류)**.

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

## 7. 호환성 / 마이그레이션 (기존 KB 영향)

이 작업이 기존 KB 를 어떻게 건드리는지 — 무엇이 재색인 필요하고 무엇이 그대로인지.

- **본문 청크 재색인 불필요.** chunk_id 생성 방식(위치 기반 `id_for_block`)을 안 바꿨고
  본문 dense 벡터 색인 경로도 안 바꿨다. 같은 corpus 를 같은 parser/chunker/embedding
  버전으로 다시 ingest 하면 본문 chunk_id·벡터가 그대로다. 캐시는 *계산*만 절감할 뿐
  결과(벡터 값)는 동일하므로 기존 본문 데이터는 손대지 않아도 된다.
- **V012 는 순수 가산 — 자동 적용, 기존 데이터 불변.** 새 테이블 `derivation_cache` 만
  추가하고 `corpus_revision` 을 bump 하지 않는다(§3.2). 기존 SQLite 를 새 binary 로 열면
  refinery 가 V012 를 자동 적용하며 기존 행은 건드리지 않는다. **단 binary 교체는 필수**:
  V012 가 적용된 DB 를 **이전 release binary** 로 열면 refinery 마이그레이션 상태가
  mismatch 한다(이전 binary 는 V012 를 모름) → 새 binary 로만 열 것. 이 schema 변경은
  CLAUDE.md §Versioning 의 release bump 트리거다.
- **별칭 dense 벡터 — expansion 을 켰던 KB 만 해당.** 별칭 색인 단위가 묶음 단일 sentinel
  `{orig}#alias`(1벡터) → 줄별 개별 sentinel `{orig}#alias#N`(N벡터)로 바뀌었다.
  - expansion 을 한 번도 안 켠 KB: 별칭 sentinel 자체가 없으므로 영향 0.
  - 기존 단일 sentinel 이 남아 있어도 **검색은 그대로 동작**한다: candidate strip 이
    `strip_alias_suffix`(ids.rs)의 `find("#alias")` 기반이라 legacy `{orig}#alias` 와
    신형 `{orig}#alias#N` 를 똑같이 원본 chunk_id 로 환원한다.
  - 개별 벡터의 검색 품질 이점(희석 회피, §3.4)을 원하면 **별칭만 재생성**하면 된다
    (본문은 그대로). 강제 사항은 아니다.
  - stale 별칭 sentinel 누수 방지는 §3.5 의 cleanup(LanceDB exact-match + SQLite
    `#alias%` LIKE)이 재색인·삭제 시 자동 처리한다.
- **KB 이식성(외부 계산 워크플로).** `derivation_cache` 는 SQLite 안에 있고 cache_key 가
  머신 독립적인 내용 해시라, 외부 서버에서 워밍한 `kebab.sqlite`(+`lancedb/`)를 그대로
  복사해 오면 로컬 증분 수정 시에도 캐시가 히트한다(측정: handoff §5).

## 8. Risks / notes
- LLM 별칭의 미세한 비결정성: 캐시가 첫 결과를 고정하므로 재현성은 오히려 향상.
  단 "더 나은 별칭" 재생성을 원하면 prompt_version bump 로 무효화.
- payload BLOB 크기: embedding 4KB/청크 × 캐시 엔트리. 50만 엔트리 ≈ 2GB. TTL/LRU 로 관리.
- V012 는 schema migration → release version bump 트리거(CLAUDE.md §Versioning).
- 본 설계는 frozen design contract(§9 versioning)의 *의미*를 바꾸지 않는다(캐시는 그
  위의 성능 레이어). design 문서 수정 불필요; cascade 안전성만 version_key 로 보장.
