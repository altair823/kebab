---
phase: P9
component: kebab-parse-md + kebab-normalize
task_id: p9-fb-07
title: "Markdown title fallback chain (frontmatter → H1 → H2 → first paragraph → filename)"
status: planned
depends_on: []
unblocks: []
contract_source: ../../docs/superpowers/specs/2026-04-27-kebab-final-form-design.md
contract_sections: [§3.5 ParsedDoc, §5.5 CanonicalDocument]
source_feedback: p9-dogfooding-feedback.md item 5
---

# p9-fb-07 — Title fallback

## Goal

frontmatter `title` / 첫 H1 둘 다 없는 markdown 도 의미 있는 title 표시. fallback chain 명시 + parser_version cascade.

## Allowed dependencies

- 기존 kebab-parse-md / kebab-normalize. 신규 X.

## Public surface

`kebab-normalize::derive_title(parsed: &ParsedDoc, file_stem: &str) -> String` 가 chain 구현. CanonicalDocument.title 채움.

## Behavior contract

fallback 우선순위:
1. frontmatter `title` (기존)
2. 첫 H1 텍스트 (기존)
3. 첫 H2 텍스트 (신규)
4. 첫 non-empty paragraph 의 첫 80 자 (인용 / list 제외)
5. 파일명 (확장자 제외, kebab-case 유지)

빈 결과 / whitespace 만이면 다음 단계로 진행. 모든 단계 실패 시 (frontmatter only no body file) 파일명. 빈 문자열 반환 금지.

`parser_version` (현재 `md-frontmatter-v1`) → `md-frontmatter-v2` bump. 기존 doc 은 next ingest 시 재처리 (same `doc_id` recipe → upsert).

## Test plan

| kind | description |
|------|-------------|
| unit | frontmatter title only → 1단계 |
| unit | H2 부터 시작 → 3단계 |
| unit | 표만 있는 doc → 4단계 (paragraph 없음 → 5단계 filename) |
| unit | 한글 H1 → NFC 정규화된 title |
| snapshot | corpus 의 python-360-... → "Annotation issues at runtime" |

## DoD

- [ ] `cargo test -p kebab-parse-md -p kebab-normalize` 통과
- [ ] parser_version 갱신 (`md-frontmatter-v2`)
- [ ] HOTFIXES X — bump 은 정상 cascade 동작
- [ ] 사용자에게 "title 비어 있던 doc 은 `kebab ingest` 다시 돌리면 채워짐" 안내 (README 또는 changelog)

## Out of scope

- PDF / 이미지 doc 의 title fallback (별도 task — 다른 parser)
- AI 로 title 추출 (P+)
