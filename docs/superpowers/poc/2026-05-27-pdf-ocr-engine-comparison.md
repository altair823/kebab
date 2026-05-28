# PoC: 한국어 OCR engine comparison (2026-05-27)

## Goal

v0.20.0 sub-item 1 (PDF scanned OCR) 의 risk-aware 검증 — 한국어 OCR engine
선택을 결정하기 위한 PoC. single binary 원칙 (kebab CLAUDE.md) + Ollama 통합
정책 (user `project_llm_default`) + P9 책+PDF use case 정합 위해 vision LLM
경로 채택 결정.

## Setup

- Fixture A: `page1` — 일반 한국어 + 한자 + 영문 + 숫자 mix (803 char, 8 sections)
- Fixture B: `page2-batchim` — 받침 intensive (1724 char, 5 sections: 단순 받침 +
  겹받침 + 한자 혼용 + 의미 변화 + 외래어)
- Rendering: Pillow + Noto Sans CJK KR, 11pt, 300 DPI, A4 → 2480x3507 PNG.
- Comparison: python-Levenshtein. raw / nows / alnum 세 지표.
- LLM access: remote Ollama at 192.168.0.47:11434 (user default LLM host).

## Final comparison (PoC step 3, 모든 engine)

### Fixture A — page1 (mixed content, 803 char)

| engine | raw | nows | alnum | latency |
|---|---:|---:|---:|---:|
| Tesseract (best LSTM + PSM 6 + kor+eng) | 86.80% | 86.83% | 86.96% | ~1-2s |
| EasyOCR (ko + en) | 87.80% | 87.80% | 89.76% | ~10s |
| PaddleOCR (v3.5) | — bug — | — | — | — |
| gemma4:e4b vision (8B) | 79.45% | 78.86% | 77.09% | 36s |
| **qwen2.5vl:3b vision (3.8B)** | **95.64%** | **95.12%** | **94.79%** | 45.6s |

### Fixture B — page2 batchim (받침 intensive, 1724 char)

| engine | raw | nows | alnum | latency |
|---|---:|---:|---:|---:|
| Tesseract (best LSTM + PSM 6 + kor+eng) | 75.23% | 72.12% | 66.77% | ~1-2s |
| EasyOCR (ko + en) | 75.12% | 73.23% | 74.06% | ~10s |
| gemma4:e4b vision | 45.77% | 37.41% | 27.01% | 99.1s |
| **qwen2.5vl:3b vision** | **85.96%** | **84.03%** | **81.56%** | 105.2s |

## 핵심 발견

1. **Tesseract 의 PSM 가 main quality driver**: default PSM 3 (auto) 가 처참
   (20%), **PSM 6 (single block) 강제** 시 67-87% 까지 회복. spec 에 명시 필수.
2. **PaddleOCR v3.5 + PaddlePaddle 3.0+ PIR/oneDNN runtime bug**: env 변수
   우회 불가. paddlepaddle 2.6 downgrade 필요. production 의존성 churn risk.
3. **gemma4:e4b vision = transcription 불가**: paraphrase / hallucination /
   단락 누락. 받침 fixture 27% — Tesseract 67% 대비 -40%p.
4. **gemma4 family text-post-process = 무효 또는 악화**: e4b 로 Tesseract OCR
   결과 후처리 시 char accuracy 유지 (받침 fixture) 또는 한자/영문 부분 망가짐
   (page1). LLM 후처리 path 폐기.
5. **qwen2.5vl:3b vision = 최고 quality**: page1 94.79% / 받침 81.56% alnum.
   Tesseract 대비 받침에서 **+14.79%p**. paraphrase 위주가 아닌 transcription.
6. **qwen2.5vl latency** = Tesseract 의 **40-50x slower** — 800 page 책
   indexing ≈ 10 hours.

## Single binary 친화도 (CLAUDE.md core principle)

| engine | runtime dep | distribution path | kebab binary 영향 |
|---|---|---|---|
| Tesseract | libtesseract + libleptonica (~10MB C lib) | apt / brew / MSI | leptess Rust binding — native dep |
| EasyOCR | PyTorch CPU (~700MB) + easyocr | Python venv | sidecar IPC architecture |
| pdfium-render | PDFium native shared lib (~10-20MB) | bblanchon/pdfium-binaries github | static link or runtime download |
| **qwen2.5vl via Ollama** | **Ollama (이미 사용)** | **이미 user 환경** | **HTTP API 호출만 — 0 native dep** |

→ qwen2.5vl 가 single binary 원칙 + user Ollama 통합 정책 양쪽 만족.

## Recommended architecture (v0.20.0 sub-item 1)

```
PDF asset
  │
  ▼
[lopdf::Document::load_mem]
  │
  ▼
per-page loop:
  │
  ├── lopdf text extract  ──── text >= threshold (e.g. 50 char) ──► text block
  │                                     │
  │                                     │ (text < threshold = scanned)
  │                                     ▼
  ├── lopdf image stream / page-rasterize (pure Rust) ─► PNG bytes
  │                                     │
  │                                     ▼
  └── Ollama POST /api/generate
      model = config.pdf.ocr.model (default: qwen2.5vl:3b)
      images = [base64(page_png)]
      prompt = config.pdf.ocr.prompt_template (transcription-only)
      │
      ▼
      OCR text block (+ provenance: which method, latency)
```

핵심 결정:
- **OCR engine**: qwen2.5vl:3b (default). config 로 다른 vision model 선택 가능.
- **Architecture**: text-detect first + vision LLM fallback (사용자 always-on
  결정 reverse). 책 PDF 의 일부 (text PDF 페이지) 가 vision 호출 skip → 평균 cost ↓.
- **OcrEngine trait**: 기존 OllamaVisionOcr (image 용) 와 동일 trait, vision model
  config 만 다름. PDF + image 동일 path.
- **PDF rendering**: lopdf (pure Rust, 이미 dep) 의 page 추출 + image stream. pdfium-render
  도입 보류 (additional native lib avoid).
- **always-on config option**: `pdf.ocr.always_on = false` (default), `true` 시 text 있는 페이지도 vision 호출 (사용자 P9 의 책 PDF 최대 recall 시).

## 미해결 risk

- **real-world scanned book PDF baseline 미측정** — 합성 fixture 의 95% / 82% 는
  ceiling. 실제 책 scan 의 noise / skew / column / 한글 polyphony 에 qwen2.5vl 의
  robustness 미검증.
- **lopdf 의 page-rasterize capability 미검증** — lopdf 는 PDF parsing 만 제공.
  scanned PDF (page = embedded image) 의 image stream 추출은 가능하지만, vector
  PDF (text + image overlay) 의 page rasterize 는 lopdf 만으로 부족할 수 있음.
  대안: 처음에는 image stream 추출만 지원, vector PDF rasterize 는 future.
- **latency mitigation**: 800-page 책 ≈ 10 hours. async indexing background queue
  (user 가 책 추가 후 즉시 search 불가) + status reporting + cancellation 필요.
  v0.20 의 wire schema (ingest_progress.v1) 가 OCR latency 별 reporting 갱신 필요.
- **qwen2.5vl 의 한자→한글 변환 미세 paraphrase** ("大韓民國" → "대한민국"):
  검색 use case 에서는 문제 없음. citation 정확성 측면에서는 약점 — provenance
  에 "vision-ocr" 명시로 사용자가 인지 가능.
- **GPU 가속 미검증**: remote (192.168.0.47) 가 GPU 있다면 latency 3-5x 향상 가능.
  CPU 환경에서는 본 측정 latency 가 그대로.

## lopdf probe (2026-05-27, B1 deliverable)

B2 fixture 합성 후 actual `/Filter` 측정 (shell grep + Python re 기반 probe):

- F1 (`scanned_page1.pdf`): `/Filter [ /DCTDecode ]` — 466897 bytes, JPEG magic `ffd8ffe0` ✅ confirmed.
  - reportlab drawImage + useA85=0 → DCTDecode stream 직접 (ASCII85 래퍼 없음).
- F2 (`scanned_page2.pdf`): `/Filter [ /DCTDecode ]` — 773781 bytes, JPEG magic `ffd8ffe0` ✅ confirmed.
- F4 (`mojibake.pdf`): DejaVu Sans TTF 기반 (Noto CJK TTC PostScript outlines 미지원 → fallback).
  ToUnicode CMap absent 확인 (strip via byte-level regex, count=0) ✅. PDF header `%PDF-1.3`.
- F6 (`flate_raw.pdf`): `/Filter /FlateDecode` — 872 bytes, DCTDecode 부재 ✅.
  Pillow RGB→PDF 가 DCTDecode 를 사용하는 것 확인 → 수동 PDF 작성(zlib.compress) 으로 생성.
- F7 (`ccitt.pdf`): `/Filter [ /CCITTFaxDecode ]` — 2060 bytes, DCTDecode 부재 ✅.
  Pillow bilevel('1') + TIFF group4 → ghostscript `pdfwrite -dMonoImageFilter=/CCITTFaxEncode -dEncodeMonoImages=true` 경로.

Step 3 의 `extract_dctdecode_page_image` 의 baseline.

주요 관찰:
- reportlab 기본값 (`useA85=1`) 은 `/Filter [ /ASCII85Decode /DCTDecode ]` chain → `useA85=0` 으로 순수 DCTDecode 획득.
- Pillow `Image.new('RGB').save('.pdf','PDF')` 는 DCTDecode (JPEG) 로 저장 — FlateDecode raw pixel 이 필요한 F6 는 수동 PDF 작성 필요.
- ghostscript pdfwrite default 가 TIFF 입력 시 `/undefined in II*` 로 실패 — `-dMonoImageFilter=/CCITTFaxEncode -dEncodeMonoImages=true` flag 로 우회 (ImageMagick fallback 불필요).
