---
title: "md-heading-v2 — 예산 초과 청크 일반 분할 (oversize-chunk split)"
created: 2026-06-24
status: implemented
extends: tasks/p1/p1-5-chunk.md (md-heading-v1, frozen)
contract_sections: [§3.5 Chunk, §4.2 chunk_id recipe, §7.2 Chunker, §9 versioning]
design_doc_change: none  # 설계 §9 가 라벨 bump 를 변경 메커니즘으로 이미 명시
---

# md-heading-v2 — 예산 초과 청크 일반 분할

## 문제

`md-heading-v1`(p1-5)은 규칙 2로 "코드/테이블 블록은 `target_tokens`를 넘어도
절대 분할하지 않는다"를 둔다. 실제로는 **모든** atomic/single 블록(코드뿐 아니라
`list`·`table`·거대 `paragraph`)이 한 청크가 될 수 있고, 그 청크가 임베더
컨텍스트를 초과하면 임베딩이 실패한다.

도그푸딩에서 jira 이슈 일부(예: `SERVER-22906`)가 임베드에 실패했다. 긴 MongoDB
로그/스택트레이스가 md 변환 시 **하나의 거대 `list` 블록**(76189 byte/3 토큰,
소스 60–1303 줄)으로 렌더된 것이 원인이었다.

- 기존 ollama(`/api/embed`)는 이런 입력을 서버측 8192로 **조용히 truncate** →
  0 errors 였지만 사실상 잘려 색인됨.
- AMD Lemonade 같은 strict 백엔드는
  `500 "input (N tokens) is too large ... increase the physical batch size"` 로
  **거부**한다.

임베더-무관하게 견고하려면 청커가 애초에 예산 초과 청크를 만들지 않아야 한다.
임베더 측 전역 truncate(silently 잘림)는 reject.

## 결정 — 왜 v2(새 라벨)인가, 왜 일반(generic) 분할인가

- **새 변종 `md-heading-v2`, frozen doc 미변경.** 설계 doc은 "코드 블록 미분할"을
  계약 불변식으로 못박지 않는다(예제 출력 텍스트일 뿐). 규칙은 frozen task spec
  p1-5 소유. 설계 §9가 `chunk boundary/policy 변화 → 라벨(md-heading-v2)`을 변경
  메커니즘으로 **명시**한다. → 설계 §1/§3.5/§7.2/§9 byte-identical, p1-5 frozen
  유지. 선례: pdf-page-v1 → pdf-page-v1.1(HOTFIXES, frozen doc 미변경).
- **코드 한정이 아닌 일반 분할.** 실패의 실제 원인은 코드가 아니라 list 블록.
  블록 종류별 특수 로직 대신 "예산 초과 청크"를 일반적으로 분할하면 list·code·
  table·paragraph를 균일하게 덮고, fenced-code의 span 비대칭 문제(아래)도 피한다.

## 설계

`md-heading-v2`의 `chunk()`는 v1과 **출력이 동일**하다(같은 블록 처리, 같은
soft-split). 마지막에 후처리 패스를 둔다:

```
let chunks = <v1-equivalent chunking>;
chunks.into_iter().flat_map(|c| {
    // 판정 기준 = 실제 임베드 text 크기 (token_estimate 아님)
    let embed_tokens = c.text.len().div_ceil(BYTES_PER_TOKEN);
    if embed_tokens <= max_chunk_tokens { vec![c] }           // v1 parity
    else { split_oversize_chunk(c, max_chunk_tokens) }        // 분할
})
```

**왜 `token_estimate` 가 아니라 `text.len()` 인가.** text 청크는 둘이 같다
(`token_estimate == text.len()/BYTES_PER_TOKEN`) → 비이미지 출력 불변. 하지만
`ImageRef`/`AudioRef` 청크는 `build_chunk` 가 image-only 규약으로
`token_estimate=0` 을 박는데, 그 `text`(alt+OCR+caption)는 임의로 클 수 있다
(빽빽한 스크린샷이 수십 KB 로 OCR 되는 경우). 임베더가 실제로 받는 건 `text` 이므로
거기에 맞춰 판정해야 oversize 이미지 OCR 도 분할된다. 이 구멍은 markdown-only
검증에선 안 보였고, **사용자 실 config(이미지 OCR ON) 재현 도그푸딩에서 발견**됐다
(회귀 테스트 `oversize_image_ocr_chunk_splits`).

`split_oversize_chunk`:

1. `chunk.text`를 줄(`\n`) 경계로 그리디 누적 — 다음 줄을 더하면 예산(byte/3)을
   넘을 때 조각을 닫는다.
2. **단일 줄이 홀로 예산을 넘으면**(거대 paragraph는 개행 없는 한 줄로 렌더)
   그 줄을 **UTF-8 char 경계**(`char_indices`)로 ≤ `budget * 3` 바이트씩 분할 —
   codepoint 중간을 자르지 않는다. → 예산 상한이 모든 입력에 대해 **하드 보장**.
3. 각 조각 i → `Chunk`: 동일 `doc_id`/`block_ids`/`heading_path`/`chunker_version`,
   `text`=조각, `token_estimate`=조각 byte/3, 저장 `policy_hash`=**bare** base 해시,
   `chunk_id = id_for_chunk(doc_id, version, block_ids, "{base}#seg{i}")`.

### chunk_id 충돌 회피

분할 조각은 동일 `block_ids`를 공유하므로 `id_for_chunk`의 기본 recipe로는
충돌한다. id-input 해시에만 `#seg{i}`(i = 0-based 조각 인덱스, 단조증가) 접미사를
붙여 disambiguate하고, 저장 `Chunk.policy_hash`에는 bare base를 남긴다 —
pdf-page-v1의 `#L` recipe(HOTFIXES 2026-05-02 P7-2)와 동형.

### `max_chunk_tokens`를 policy_hash에 fold

신규 config `[ingest.chunking] max_chunk_tokens`(byte/3, default 4000)를 공유
`ChunkPolicy`에 넣으면 **모든** 청커(코드·PDF 포함)의 policy_hash가 바뀌어
cascade가 번진다. 대신 v2의 `policy_hash()`만 override해 canonical `ChunkPolicy`
바이트 뒤에 `max_chunk_tokens.to_le_bytes()`를 이어 blake3에 먹인다 → 값을 바꾸면
markdown만 재청크, v1·공유 ChunkPolicy 무영향.

### citation 정밀도 (known limitation)

분할 조각은 원 청크의 `source_spans`(원 블록 전체 범위)를 **그대로** 보존한다 →
거대 블록을 쪼갠 조각의 citation은 **블록 단위**(sub-line 정밀 아님). fenced
코드의 `SourceSpan::Line`은 fence 줄을 포함하는데 `code`는 content만 담아
(`kebab-parse-md/src/blocks.rs` `span_for(full range)`), 조각별 줄 범위를 정확히
좁히는 건 (span, code)만으로 일반적으로 불가능 → "절대 틀리지 않되 블록 단위"를
택했다. 미분할 청크는 v1과 byte-identical이라 영향 없음.

## cascade / 업그레이드

- 마크다운 청커 dispatch는 하드코딩(`kebab-app`) — config `chunker_version`
  문자열은 impl 선택에 쓰이지 않는다. v2는 type swap으로 기본값 승격.
- `chunker_version` 라벨 `md-heading-v1` → `md-heading-v2` → 다음 plain
  `kebab ingest`에서 markdown 자산 1회 자동 재청크(`--force-reingest` 불필요,
  skip 비교 mismatch). 코드/PDF는 chunker_version unchanged → 무영향. 마이그레이션
  불필요(chunks 스키마 V001부터 동일, chunk_id 키 재계산).
- **wrinkle**: 기존 config가 `chunker_version = "md-heading-v1"`로 핀돼 있어도
  실제로는 v2가 돈다(문자열은 정보성). `config migrate`/새 default config는
  `max_chunk_tokens`를 additive 주입.

## 검증 (도그푸딩)

실험 KB `/home/user/large_data/out/kebab-ab/xdg_sources`, arctic-embed-l-v2 @
Lemonade `.243`. v2 전: 620 중 2 doc(`SERVER-22906`/`SERVER-23097`) 임베드 실패.
v2 후: **620/620, errors=0**, 7114 청크 전부 ≤ 4000(초과 0). `SERVER-22906` →
14 청크(max 3897), `SERVER-23097` → 5 청크(max 3850). 검색 payoff: "WiredTiger
excessive memory cache size" 질의에 `SERVER-22906`가 **1위(0.977)** — "임베드
불가"에서 "최상위 검색 결과"로. `--trust-min primary` 등 출처 필터 정상.

**일치 재테스트 (사용자 실 config 재현)**. 사용자 실 config 가 이미지 OCR + PDF
OCR 를 paddle-onnx 로 ON 함을 반영해, 미디어(생성 이미지 2 + repo scanned PDF 2)를
같은 paddle-onnx + arctic@Lemonade 로 재인덱싱(624 자산 errors=0). PDF OCR →
청크(`pdf-page-v1.1`) → arctic 임베드 정상. **여기서 image-OCR 구멍 발견·수정**
(위 "왜 token_estimate 가 아닌가" 참조). **known limitation**: PDF 는 별도 청커
`pdf-page-v1.1` 이라 v2 의 oversize-split 미적용 — 초고밀도 scanned page 가 한
청크로 budget 초과 시 잔존. 후속 후보: pdf-page 청커에도 동일 oversize-split.

단위 테스트(`crates/kebab-chunk/src/md_heading_v2.rs`): `oversize_list_block_splits`,
`oversize_paragraph_single_line_char_splits`(다국어 UTF-8 경계), `oversize_code_block_still_splits`,
`oversize_image_ocr_chunk_splits`(token_estimate=0 이미지 OCR), `non_oversize_identical_to_v1`(v1 parity),
`split_pieces_unique_deterministic_ids`(1000-iter 결정성), `budget_in_policy_hash`.
kebab-chunk / kebab-config / kebab-app 전체 pass, clippy `-D warnings` clean.

## 버전

`Cargo.toml` 0.29.0 → **0.30.0**. 신규 config 키 + 청커 동작 변경(검색 hit 분할)
= pre-1.0 minor + 도그푸딩 트리거(CLAUDE.md §Versioning/§Dogfood).
