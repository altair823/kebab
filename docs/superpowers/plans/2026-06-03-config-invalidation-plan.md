# Plan: ingest 설정 변경 자동 재색인 구현

spec: `docs/superpowers/specs/2026-06-03-ocr-toggle-invalidation-spec.md`. 브랜치 `fix/ingest-config-invalidation`. 빌드 `CARGO_TARGET_DIR=/build/out/cargo-target`, **테스트 `-j 8`**(절대 `-j 1` 금지), cli 통합테스트용 `target` 심링크 후 정리.

## Task 1 — ingest_config_signature 헬퍼 (kebab-app)
- `fn ingest_config_signature(config: &Config, media: &MediaType) -> String` 추가.
- 공통: `[chunking]` target_tokens, overlap_tokens, respect_markdown_headings, chunker_version.
- image: + image.ocr.enabled (+model if enabled) + image.caption.enabled (+prompt_template_version if enabled).
- pdf: + pdf.ocr.enabled (+model, always_on if enabled).
- code: + ingest.code.{skip_generated_header, max_file_bytes, max_file_lines, extra_skip_globs(join), ast_chunk_max_lines, fallback_lines_per_chunk, fallback_lines_overlap}.
- markdown: 공통만.
- 결정적(필드 순서 고정). 단위테스트: 같은 config→같은 서명, 관련 필드 변경→서명 변경, 무관 필드(search 등)→불변.

## Task 2 — 4개 ingest 경로에 composite parser_version 적용 (kebab-app/lib.rs)
- md(~351), image(~1532), pdf(~2109), code 경로: `*_parser_version` = `ParserVersion(format!("{base}|{}", ingest_config_signature(config, media)))` (base = 각 extractor PARSER_VERSION).
- 이 composite 를 (1) `try_skip_unchanged` 의 `current_parser_version` 으로 전달, (2) **persist 전 `canonical.parser_version` override** 로 저장. 두 곳 동일 보장.
- doc_id 파생은 손대지 않음(workspace_path 조회).
- markdown/code/image/pdf 각 경로에서 동일 패턴 적용 — 누락 없게.

## Task 3 — 테스트
- image.ocr off→on, caption off→on: 재색인(skip 아님). off→off / 동일 설정: skip 유지.
- pdf.ocr off→on: 재색인. 동일: skip.
- chunking target_tokens 변경: 전 타입 재색인. 무변경: skip.
- ingest.code 변경: 코드 자산만 재색인.
- **search/rag/ui 변경: 재색인 0** (회귀 가드).
- 동일 config 재실행: 전 자산 skip (불필요 재색인 0).
- 기존 skip 테스트(markdown unchanged 등) 회귀 0.

## Task 4 — 검증 + 문서
- `cargo clippy --workspace --all-targets -j 8 -- -D warnings` 0.
- `cargo test -p kebab-app -p kebab-parse-image -p kebab-parse-pdf -p kebab-parse-code -p kebab-chunk -j 8` 통과(touched 크레이트 타깃; 전체 -j1 금지).
- 스모크: 이미지 ocr off 색인 → config ocr on → `kebab ingest`(force 없이) → 그 이미지 재색인 확인.
- tasks/HOTFIXES dated entry(일반화 + 업그레이드 1회 재색인 안내), Cargo.toml version **0.26.1 → 0.26.2**(+Cargo.lock), HANDOFF 1줄. README/wire 변화 없음.
- 결과 요약 `/tmp/cfginval-result.md`(게이트 + 스모크 캡처).

## 리뷰 루프
완료 → 리더 clippy/타깃테스트(-j8) 독립 재확인 + 토글 스모크 → `gitea-pr`(title `fix(ingest): ingest 설정 변경 시 영향 자산 자동 재색인`) → 리뷰 루프 → 사용자 머지.
