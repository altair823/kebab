---
phase: P6
title: "이미지 ingestion (OCR + caption)"
status: planned
depends_on: [P5]
source: kb_local_rust_report.md §9.1, §17 Phase 6
---

# P6 — 이미지 ingestion

## 목표

이미지 파일을 `CanonicalDocument` 로 변환. 동일 검색/RAG 파이프라인에 합류. citation 은 파일 + region.

## 산출 crate

- `kb-parse-image` — `Extractor` 구현. 이미지 → CanonicalDocument.
- (선택) `kb-ocr` / `kb-vlm` 어댑터 (외부 모델 분리 시).

## 추출 정보 3종 (§9.1)

| 종류 | provenance.kind | 신뢰도 |
|------|-----------------|--------|
| 파일 metadata (경로, EXIF, 크기, mtime) | `metadata` | 높음 (관찰값) |
| OCR text + bounding box | `observed_text` | 높음 (관찰값) |
| AI caption / VLM 설명 | `model_caption` | 낮음 (생성값) |
| visual embedding | `visual_embedding` | 검색용 (의미값) |

핵심 규칙: **OCR 과 caption 을 같은 신뢰도로 취급 금지**. provenance 분리.

## CanonicalDocument 매핑

이미지 1장 → 1 document. blocks:

```rust
Block::ImageRef(ImageRefBlock {
    asset_id,
    caption: Option<String>,        // model 생성, 신뢰도 낮음 표시
    ocr_text: Option<OcrText>,      // 관찰값
    exif: Option<ExifMetadata>,
})

pub struct OcrText {
    pub regions: Vec<OcrRegion>,    // bounding box + text + confidence
    pub joined: String,             // 검색용 단일 문자열
    pub engine: String,             // "apple-vision" | "tesseract" | ...
    pub engine_version: String,
}
```

## OCR 엔진 선택

- macOS 1차: Apple Vision text recognition (sidecar Swift 또는 Tauri command 통해 호출).
- cross-platform fallback: tesseract binding 또는 PaddleOCR sidecar.
- 1차 구현: 1개 엔진만. abstract trait `OcrEngine` 으로 교체 가능하게.

## VLM caption (선택, 후순위)

- local VLM (예: llava, qwen-vl) 통해 caption.
- caption 은 chunk text 에 포함하되 prefix 표시 (`[caption(model=...): ...]`).
- 검색 시 caption-only hit 는 별도 `retrieval_method = "vlm-caption"` 로 표기.

## Visual embedding (선택)

- CLIP 계열 image encoder.
- text embedding 과 차원/모델 다름 → 별도 LanceDB table (`image_embeddings`).
- text query → image 검색 = CLIP joint space 필요. 1차 구현은 OCR/caption text embedding 으로 충분.

## Chunking

- region-aware: OCR region 1개 또는 인접 region 묶음 = 1 chunk.
- caption 1개 = 별도 chunk (provenance 표시).
- chunker version: `image-region-v1`.

## Citation 형식

```text
photos/diagram-2026.png
photos/diagram-2026.png#region=120,40,520,180   # x,y,w,h
photos/diagram-2026.png#caption                 # caption chunk
```

## CLI

```text
kb ingest ./assets/diagram.png
kb ingest ./assets/   # 폴더 안 이미지 자동 인식
kb search "이미지 안의 OCR 텍스트"
kb inspect doc <image_doc_id>   # OCR/caption/EXIF 모두 표시
```

## 테스트

- fixture: 한글 텍스트 이미지 + 영문 텍스트 이미지 + 텍스트 없는 사진.
- OCR region → CanonicalDocument round-trip.
- caption 이 chunk text 에 prefix 와 함께 들어가는지.
- 검색 결과에서 OCR hit 와 caption hit 구분 표기.
- 동일 이미지 재수집 시 idempotent (asset_id = blake3 동일).

## 의존성 경계

- `kb-parse-image` 는 `kb-core` + 이미지 디코딩 (`image` crate) + OCR adapter 만.
- LLM/embedding 호출 금지 (caption 은 별도 adapter 통해).
- VLM caption 은 background job. ingest blocking 금지.

## 완료 조건

- [ ] `kb ingest <image>` 동작
- [ ] OCR text 검색 가능
- [ ] OCR region citation 출력
- [ ] caption 과 observed text provenance 분리
- [ ] EXIF 보존
- [ ] 같은 이미지 재수집 idempotent

## 리스크 / 주의

- OCR confidence 낮은 region 을 chunk 로 색인하면 noise. threshold 적용.
- caption hallucination = noise + 잘못된 RAG 인용 위험. citation 표기에서 caption 임을 항상 노출.
- Apple Vision sidecar 는 macOS 종속. linux 빌드는 다른 OCR 로 fallback.
- 대량 이미지 폴더 ingest 시 메모리/디스크 사용량 monitoring.
