---
phase: P7
title: "PDF text extraction + page citation"
status: planned
depends_on: [P5]
source: kebab_local_rust_report.md §9.2, §17 Phase 7
---

# P7 — PDF ingestion

## 목표

text PDF 추출 → page-aware chunking → citation `paper.pdf:p13`. scanned PDF OCR 는 후속 단계.

## 산출 crate

- `kebab-parse-pdf` — `Extractor` 구현.

## 단계 분리 (§9.2)

| 단계 | 범위 | 우선순위 |
|------|------|---------|
| 1 | text PDF 추출 (page + text span) | P7 본체 |
| 2 | scanned PDF OCR | 후속, image OCR 인프라 재사용 |

처음부터 layout reconstruction 욕심 금지. **page number + text span 보존**이 1차 목표.

## 라이브러리 선택

- 1차: `pdf-extract` (단순 텍스트 추출).
- 보조: `lopdf` (페이지 단위 접근, metadata).
- text 추출 실패 / 빈 페이지 → scanned 의심 표시 → 2단계 OCR 후보로 큐잉.

## CanonicalDocument 매핑

PDF 1개 = 1 document. 페이지 단위 block:

```rust
pub struct PdfPageBlock {
    pub page_number: u32,
    pub text: String,
    pub source_span: SourceSpan, // byte range or char range within page
    pub section_hint: Option<String>, // 휴리스틱 추출, optional
}
```

heading 검출: PDF 자체엔 heading 의미 없음. 휴리스틱 (font size, bold, ALL CAPS) 1차에서는 생략. section 은 best-effort.

## Chunking

- page-respect: chunk 가 page 경계 넘지 않음 (citation 단순화).
- 긴 page → paragraph 단위로 sub-chunk.
- chunker version: `pdf-page-v1`.

## Citation 형식

```text
paper.pdf:p13
paper.pdf:p13:section=Experiment Setup
paper.pdf:p13:span=0-1240         # char range within page
```

## CLI

```text
kebab ingest ./paper.pdf
kebab ingest ./papers/
kebab search "PDF 안의 특정 개념"
kebab inspect doc <pdf_doc_id>
```

## 테스트

- fixture: 한글 PDF (논문/문서), 영문 PDF, 다단 layout, 표 포함, 빈 페이지 포함.
- page number 정확도 (1-based, 1페이지 PDF 도 OK).
- citation round-trip: `paper.pdf:p13` 으로 다시 page 텍스트 회수 가능.
- 추출 실패 페이지는 reject 하지 않고 provenance warning + scanned 후보 표시.
- 동일 PDF 재수집 idempotent.

## 의존성 경계

- `kebab-parse-pdf` 는 `kebab-core` + `pdf-extract` / `lopdf` 만.
- OCR 호출은 별도 adapter 통해 (P6 OCR 인프라 재사용).

## 완료 조건

- [ ] `kebab ingest <pdf>` 동작
- [ ] page-level chunk + citation
- [ ] 검색 결과에 `paper.pdf:p<n>` 포함
- [ ] 추출 실패 페이지에 대한 provenance warning
- [ ] 동일 PDF 재수집 idempotent

## 리스크 / 주의

- text 추출 품질은 PDF 생성 도구에 크게 좌우. 깨진 한글 (CID 미매핑) 흔함.
- 다단/표 layout 은 reading order 깨짐 → 검색 noise. 1차에선 감수.
- OCR 단계 들어가면 비용/시간 급증. 별도 background job 으로.
- 큰 PDF (>1000p) memory streaming 처리 필요.
