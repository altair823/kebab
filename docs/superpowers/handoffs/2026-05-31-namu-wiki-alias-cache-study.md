# 나무위키 대규모 측정 — doc-side expansion 별칭 효과 + 파생물 캐시

> 2026-05-31. Phase 2 doc-side expansion(별칭) 의 효과를 실사용 규모(한국어 나무위키
> corpus)로 검증하고, 그 과정에서 드러난 별칭 생성 비용 문제를 "내용 해시 기반 파생물
> 캐시"로 해결한 기록. 선행: `2026-05-30-phase2-doc-expansion-kickoff.md`,
> 설계: `../specs/2026-05-30-dense-alias-vectors-design.md`,
> `../specs/2026-05-31-derivation-cache-design.md`.

## 1. 출발 질문 (사용자 제기)

측정을 진행하며 사용자가 던진 질문들이 설계를 단계적으로 교정했다:

1. **"테스트 모수가 너무 적지 않나? 더 넓게(대규모, 영+한 혼합) 테스트하자."**
   → 기존 8~32개 golden 으로는 "변형 일관성 개선"이 우연인지 실재인지 판단 불가.
2. **"실사용은 약 2천 개 한국어 위키 문서다."** + 기존 크롤링한 나무위키 parquet
   (`/build/cache/namu-crawler/pages.parquet`, 119만 문서) 제공.
   → 측정 corpus 를 실사용에 맞춤. 노이즈는 크게, 별칭은 정답 문서에만(비용).
3. **"정답과 주제가 완전히 다르면(야구·게임) 검색이 너무 쉬워 별칭 효과가 과소평가된다.
   실사용은 한 개발조직 위키 = 유사 주제 밀집이다."**
   → 노이즈를 정답과 같은 분야(CS/IT)로 교체. 진짜 어려운 "유사 경쟁" 환경 구성.
4. **"대조군(정답 없는 질문)도 측정하자."** → false-positive(별칭이 노이즈를 grounded
   answer 로 끌어오는지) 검증.
5. **"별칭 벡터 생성이 너무 오래 걸린다(18문서 2.5시간). 캐싱이 절실하다 — 별칭뿐 아니라
   비용 큰 모든 데이터에."** → 내용 해시 기반 파생물 캐시 설계·구현.
6. **"비싼 계산을 외부 CPU ollama 서버에서 하고 결과 DB 파일만 가져오고 싶다. 가능한가?"**
   → KB 이식성 검증.

## 2. corpus 구축

- 소스: 나무위키 덤프 119만 문서(`pages.parquet`, redirect 제외 완료).
- **노이즈 979개**: 본문 3k~30k자 + "분류" 헤더에 CS 키워드(컴퓨터공학·프로그래밍·알고리즘
  …)가 있는 문서 ~70% 정밀도로 필터 → 무작위 샘플(CCleaner·LLaMA·SQL·멀티스레딩 등).
  정답과 같은 임베딩 공간(유사 주제 밀집)이라 현실적 난이도.
- **정답 18개**: 명확한 CS 개념(경사하강법·TCP·정렬·이진탐색·뮤텍스·정규표현식 …),
  전부 한국어 문서 → 영어 변형은 자동으로 cross-lingual(영→한) 시나리오.
- **변환 핵심 교훈**: nawiki `text_extracted` 는 **개행 0**인 한 덩어리라 md 청커(단락
  경계 분할)가 거대 청크(4000+토큰)를 만들어 e5 512토큰 한계에서 잘렸다. → `html`
  컬럼을 pandoc(`-f html -t markdown_strict-raw_html`)으로 변환 + base64/링크 정제 →
  헤딩·단락 구조 복원 → 청크 중앙값 272토큰으로 정상화.
- golden: 변형 18그룹 × 4변형(한국어 용어 / 영어 용어 / 동의어·약어 / 설명형) + 대조군 10
  (`/build/dogfood/namu_golden.yaml`).

## 3. 측정 결과

### 3.1 변형 일관성 (search run, hybrid k=50)

| 구성 | fully_consistent | A(MisRanked) | B(Missing) | mean_spread@10 |
|------|------------------|--------------|------------|----------------|
| baseline (별칭 off) | 14/18 | 2 | 2 | 0.222 |
| 별도-벡터 (별칭 묶음 1벡터) | 13/18 | 2 | 3 | 0.278 (악화) |
| **개선 (별칭 개별 벡터 + boilerplate skip)** | **16/18** | 1 | 1 | **0.111** |

- baseline 약점은 **전부 "설명형" 변형**(용어·약어·영어는 18그룹 전부 완벽). 자연어 설명이
  문서 전문용어와 어휘가 멀어 벡터 검색이 못 잡음 = "어휘 격차".
- **별도-벡터(묶음)가 오히려 악화**한 원인 진단: ① 청크당 별칭 8개를 줄바꿈으로 묶어 한
  벡터로 임베딩 → 평균화로 특정 표현 **희석** ② 나무위키 메뉴(boilerplate) 청크에도 별칭
  생성 → 18문서 공통 노이즈.
- **개선판**: 별칭을 줄별 **개별 sentinel 벡터**(`{orig}#alias#N`) + boilerplate 청크 skip.
  → linked_list·sorting 회복, tcp 회귀 복구. 남은 약점은 stack·svm 설명형 2개.

### 3.2 대조군 (RAG run, refusal_correctness)

- refusal 0.6 (대조군 10개 중 6개 정상 거부, 4개 grounded).
- **false-positive 4개(graphql·oauth·react·grpc)의 인용 출처는 전부 노이즈 본문**
  (GitHub_Mobile·API·Svelte), **별칭 sentinel 인용 0** → 별칭이 false-positive 를
  유발하지 않음(별칭 무죄). 게다가 answer 는 "근거에서 찾을 수 없다"고 정직히 거부했는데
  grounded 판정이 "부분 언급 인용 있음"을 grounded 로 오분류 → 실제 refusal 은 0.6 보다 높음.
  (kebab grounded/refusal 판정의 별도 개선 여지 — HOTFIXES 후보.)

### 3.3 정답 RAG

- 변형 72개 중 대부분 grounded=True + 정답 문서 다수 인용(sort 28·linked_list 23 등). 양호.

## 4. 파생물 캐시 (V012)

별칭 18문서 재생성 2.5시간이 근본 병목. `chunk_id` 가 `ordinal+span`(위치) 기반이라
chunk_id 캐싱은 중간 수정 시 무력 → **청크 text 내용 해시**를 키로 한 범용 캐시 설계.

- `derivation_cache(cache_key, kind, payload, created_at, last_used_at)` (SQLite, V012).
- `cache_key = blake3(kind ‖ text_blake3 ‖ version_key)`. version_key 에 model/prompt/
  dimensions 포함 → §9 cascade 와 정합(버전 bump 시 자동 miss).
- 적용: embedding(본문 + 별칭 벡터 양쪽) + 별칭 LLM. korean_tokens 는 우선순위 낮아 보류.
- **측정: 정답 3개 cold 1879초(31분) → warm 13초 ≈ 145배.** 18문서 환산 시 2.5h → ~80s.
  derivation_cache 1237 엔트리(alias 140 + embedding 1097).

## 5. KB 이식성 (외부 계산 워크플로)

- `storage_path`(asset 절대경로)는 search/ask 경로에서 **사용처 0** — 저장·재처리에서만.
- **search/ask 는 `kebab.sqlite` + `lancedb` 만으로 동작**(asset 불필요).
- 실증: 원본 KB 와 다른 경로로 복사한 portable KB(asset 제외)의 search 결과가 score·순서·
  문서까지 **완전 동일**.
- 결론 워크플로:
  ```
  [외부 CPU ollama 서버]  같은 corpus + 같은 e5 모델/버전 + 같은 parser/chunker/embedding 버전
      kebab ingest → 별칭 LLM + embedding (비싼 계산, 캐시 워밍)
          ↓  kebab.sqlite(+derivation_cache) + lancedb/ 만 복사
  [로컬]  kebab search/ask → 계산 0. 증분 수정 시 외부 캐시가 머신 독립적으로 히트.
  ```

## 6. 결정 / 후속

- **채택**: 별칭 개별 sentinel 벡터 + boilerplate skip(효과·안전 입증) + 파생물 캐시(V012).
- **보류**: stack·svm 설명형 2그룹 추가 개선, korean_tokens 캐시, 이식용 캐시 export/import
  명령, 별칭 default-on 여부(현재 off-by-default, 실사용 관찰 후 재결정).
- **별도 이슈**: grounded/refusal 판정이 부분 인용을 grounded 로 오분류 — 정직한 거부가
  false-positive 로 집계됨.
- 측정 데이터: corpus `/build/dogfood/corpus/markdown/namu-wiki/`,
  golden `/build/dogfood/namu_golden.yaml`, 로그 `/build/dogfood/logs/`.
