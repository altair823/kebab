# Spec: ingest 출력에 영향 주는 모든 설정 변경 시 자동 재색인 (skip 무효화 일반화)

**날짜**: 2026-06-03
**유형**: bug fix (patch)
**근거**: `[image.ocr]`/`[image.caption]` 를 off→색인→on 으로 바꿔도 증분 skip 이 이미지를 "Unchanged" 로 건너뛴다. 더 일반적으로, `try_skip_unchanged` 가 자산 내용(blake3)+`parser_version`+`chunker_version`+`embedding_version` 만 비교하는데, **ingest 산출물을 바꾸는 다른 설정들**(청킹 파라미터, OCR/caption, pdf.ocr, 코드 ingest 옵션)이 이 셋 중 어디에도 반영되지 않아 변경해도 재색인이 안 된다. 사용자 요구: **OCR/caption 뿐 아니라 ingest 출력에 영향 주는 모든 설정**이 같은 방식으로 동작(변경→영향 자산 자동 재색인). 결과 포맷·인터페이스·새 플래그 변화 없음(내부 skip 판정 정정) → **patch**.

## 동작 사실 (코드 근거)
- `try_skip_unchanged`(lib.rs:866)는 `get_document_by_workspace_path` 로 기존 doc 조회 후 `existing_doc.parser_version != current_parser_version`(line 959) 면 재색인(cascade). **조회는 workspace_path** 이므로 doc_id 파생과 무관 — 비교는 저장된 `parser_version` 필드 대 현재값.
- 각 경로가 상수 parser_version 을 넘김: md `md-heading-v1`(351), image `image-meta-v1`(1532), pdf `pdf-text-v1`(2109), code 등. 청킹 파라미터(`target_tokens`/`overlap_tokens`/`respect_markdown_headings`)는 `chunker_version` 상수에 안 들어가 변경해도 재청킹 안 됨(동일 갭).

## 설계: per-asset-type "ingest config signature" 를 effective parser_version 에 폴딩

`try_skip_unchanged` 에 넘기는 `current_parser_version` 과 **persist 되는 doc 의 `parser_version` 필드**를, 그 자산 타입의 **ingest 산출물에 영향을 주는 설정 전체의 결정적 서명**을 포함한 composite 로 만든다. 두 값이 같은 함수에서 나오므로, 관련 설정이 바뀌면 다음 run 비교가 mismatch → **영향 받는 자산만** 자동 재색인. doc_id 는 path 조회라 기존대로(안정, orphan churn 회피).

### 어떤 설정이 어느 자산에 영향 (서명 구성)
공통 헬퍼 `ingest_config_signature(config, media_type) -> String`. **ingest 산출물에 영향 주는 것만** 포함(아래 외 search/rag/nli/ui/logging/storage/workspace 는 **제외** — 바뀌어도 재색인 안 함):

- **공통(모든 타입)**: `[chunking]` target_tokens, overlap_tokens, respect_markdown_headings, chunker_version. (embedding model/dim 은 이미 `embedding_version` cascade 가 담당 — 서명에 중복 포함 불필요, 단 일관성 위해 포함해도 무방.)
- **image**: + `[image.ocr]` enabled (+enabled 면 model), `[image.caption]` enabled (+enabled 면 prompt_template_version).
- **pdf**: + `[pdf.ocr]` enabled (+enabled 면 model, always_on).
- **code**: + `[ingest.code]` skip_generated_header, max_file_bytes, max_file_lines, extra_skip_globs, ast_chunk_max_lines, fallback_lines_per_chunk, fallback_lines_overlap.
- **markdown**: 공통만.

서명 형식: 결정적 문자열 또는 그 blake3-12. 예 `image-meta-v1|chunk:500:80:true|ocr:1:qwen2.5vl:3b|cap:1:caption-v1`. off/미적용 항목은 안정적 표현(빈값)으로 — 동일 설정 재실행은 서명 동일 → **불필요 재색인 0**.

## 작업 (kebab-app)
1. `ingest_config_signature(config, media_type)` 헬퍼 추가(위 매핑). 출력 결정적(필드 순서 고정, Vec 는 join).
2. 각 ingest 경로에서 effective parser_version = `format!("{base}|{signature}")` 또는 base 를 서명으로 감싼 값으로:
   - md(351), image(1532), pdf(2109), code 경로의 `*_parser_version` 계산을 composite 로.
   - **persist 전 `canonical.parser_version` 을 동일 composite 로 override**(extractor 가 박은 상수 대신). skip-check 와 저장값이 같아야 함.
3. doc_id: 변경 불필요(workspace_path 조회). composite 는 비교 필드에만.

## 동작 / 호환
- ingest 영향 설정(청킹/OCR/caption/pdf.ocr/code) 변경 또는 모델·prompt 변경 → effective parser_version 변화 → **영향 자산만** `--force-reingest` 없이 자동 재색인(+UPSERT/purge). 비영향 설정(search/rag/ui/log) 변경 → 재색인 0.
- **업그레이드 1회 효과**: 기존 doc 의 저장 parser_version(상수)이 새 composite 와 달라 → 업그레이드 후 첫 ingest 에서 전 자산 1회 재색인(현재 설정대로). 마크다운/코드도 1회 재청킹되나 embedding 은 V012 캐시 히트라 재임베딩 비용 작음. (HOTFIXES/release notes 에 1회 재색인 명시.)
- `--force-reingest` 는 전체 강제용으로 그대로 유지.

## 검증 기준
- clippy 0. `cargo test -p kebab-app -p kebab-parse-image -p kebab-parse-pdf -p kebab-parse-code -p kebab-chunk -j 8` 통과 (**전체 워크스페이스 `-j 1` 금지 — `-j 8`**).
- 신규 테스트(자산 타입별):
  - image.ocr off→on / caption off→on → 해당 이미지 재색인(skip 아님). off→off, on→on(동일) → skip 유지.
  - pdf.ocr off→on → PDF 재색인. 동일 설정 → skip.
  - chunking target_tokens 변경 → md/code/image/pdf 전부 재색인. 변경 없으면 skip.
  - ingest.code 옵션 변경 → 코드 자산 재색인, 이미지/md 는 영향 받되 **공통(chunking) 변경 아니면 코드만** (code 전용 설정은 code 서명에만).
  - search/rag/ui 설정 변경 → 재색인 0 (회귀 가드, 중요).
  - 동일 config 재실행 → 전 자산 skip(불필요 재색인 0) — 회귀 가드.
- 스모크: 이미지 ocr off 색인 → config ocr on → `kebab ingest`(force 없이) → 그 이미지만 재색인 확인.

## 비범위
- 새 config 키/CLI 플래그/wire(없음).
- 서명에 max_pixels/languages/timeout 같은 *런타임 비-산출* 파라미터는 **제외**(산출물 불변 → 과도 무효화 회피). 포함 기준 = "그 값이 바뀌면 색인되는 chunk/embedding 내용이 달라지는가".
- search/rag/nli/ui/logging/storage/workspace 설정(ingest 산출 무관) 제외.

## 문서/버전
- tasks/HOTFIXES dated entry(일반화 + 1회 재색인 안내). Cargo.toml **patch bump (0.26.1 → 0.26.2)**(+Cargo.lock). README/wire 변화 없음. HANDOFF 1줄(선택).
