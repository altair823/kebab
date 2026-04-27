---
phase: P9
title: "TUI + desktop app"
status: planned
depends_on: [P5]
source: kb_local_rust_report.md §16, §17 Phase 9
---

# P9 — TUI + desktop app

## 목표

CLI 위에 사용성 레이어 추가. domain/검색/RAG 가 안정된 뒤 마지막에 붙임.

## 순서

```text
kb-tui (먼저)  →  kb-desktop (나중)
```

이유: TUI 는 domain 변화에 빠르게 적응 가능. desktop 은 packaging/배포 비용 큼.

---

## P9.A — kb-tui (Ratatui)

### 산출 crate

- `kb-tui` — Ratatui + crossterm 기반 terminal UI.

### 화면 구성

| 화면 | 내용 |
|------|------|
| Library | document 목록, tag/lang 필터, indexing 상태 |
| Search | 검색창 + 결과 list + preview pane (citation 포함) |
| Ask | RAG 질문창 + 답변 + citation 토글 |
| Inspect | document/chunk 상세 (heading path, source span, provenance) |
| Jobs | indexing/embedding/transcription 진행 |

### 키바인딩 1차

```text
Tab          : 화면 전환
/            : 검색 모드
?            : ask 모드
Enter        : 결과 열기
g            : citation 으로 점프 (외부 editor: $EDITOR +line file)
q            : 종료
```

### 의존성 경계

- `kb-tui` → `kb-app` 만. parser/store/LLM 직접 호출 금지.
- 비동기 I/O (검색/ask) 는 `kb-app` 비동기 wrapper 또는 thread + channel.

### 완료 조건

- [ ] document list / search / ask / inspect 4개 화면 동작
- [ ] 검색 결과 → editor 점프 (citation line 정확히)
- [ ] indexing job 진행률 표시
- [ ] CLI 와 동일 facade 호출 (기능 누락 0)

---

## P9.B — kb-desktop

### 후보 비교

| 후보 | 장점 | 단점 |
|------|------|------|
| Tauri | Rust backend + web frontend. native webview, 작은 binary | web frontend 별도 stack (TS/JS) |
| egui/eframe | 순수 Rust, immediate-mode | 디자인 자유도/접근성 한계 |

추천: Tauri 1차. 기존 `kb-app` facade 그대로 backend 로 노출. frontend 는 가볍게 (svelte/solid/vanilla).

### 산출 crate / 구조

- `kb-desktop` (Tauri app crate)
- `kb-desktop-frontend/` (web 자산)

Tauri command 는 `kb-app` 함수 1:1 wrap. 신규 비즈니스 로직 추가 금지.

### 화면 구성 (1차)

| 패널 | 내용 |
|------|------|
| Library | document grid, multimodal 썸네일 (이미지/PDF/audio waveform) |
| Search | hybrid search + filter + citation preview |
| Ask | RAG chat. citation 클릭 시 source pane 동기화 |
| Source viewer | Markdown 렌더, PDF page viewer, image viewer (region overlay), audio player (segment seek) |
| Settings | model 선택, indexing 옵션, 경로 |

### Citation 클릭 동작

- Markdown: 내장 viewer 의 해당 line range scroll + highlight.
- PDF: page jump + (선택) span highlight.
- Image: region bounding box overlay.
- Audio: segment 시작 시각으로 seek + 재생.

### 의존성 경계

- frontend 는 Tauri command (= `kb-app` wrapper) 만 호출. SQLite/LanceDB 직접 접근 금지.
- 모델 다운로드/실행은 backend 책임.

### 완료 조건

- [ ] document, image, PDF, audio citation 모두 viewer 에서 점프 동작
- [ ] hybrid search + RAG chat 동작
- [ ] indexing/embedding/transcription job UI 표시
- [ ] macOS dmg 배포 가능 (M4 기준 동작 확인)

---

## 공통 리스크 / 주의

- UI 부터 만들면 domain 흔들릴 때 비용 폭주. P5 까지 안정시킨 뒤 진입 (§16.3).
- TUI 와 desktop 모두 facade 만 호출. UI 안에 비즈니스 로직 들어가면 P10 같은 신규 phase 마다 양쪽 다시 손봐야 함.
- desktop packaging (코드 서명, notarization) 은 별도 작업. 1차 릴리스는 unsigned dev build OK.
- Tauri 채택 시 web stack 이 "최소"여야 함. 프레임워크 선택은 1주일 안에 결론.
