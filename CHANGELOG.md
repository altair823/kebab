# Changelog

릴리스 변경 이력 인덱스 (역순). 각 항목의 상세 사용자-영향 설명은 [Gitea release](https://gitea.altair823.xyz/altair823-org/kebab/releases) 와 [docs/release-notes/](docs/release-notes/) 의 per-version 노트에 있다. 버전 bump 규칙은 [CLAUDE.md](CLAUDE.md) §Release.

| 버전 | 요약 | 노트 |
|---|---|---|
| **v0.32.0** | ponytail-audit over-engineering 정리 arc — 죽은 search-cache scaffold 제거, 9 AST chunker→1 통합(byte-identical), shim crate 흡수(crate 22→20), FusionPolicy/YAML/NLI. | [release](https://gitea.altair823.xyz/altair823-org/kebab/releases/tag/v0.32.0) · [notes](docs/release-notes/v0.32.0-draft.md) |
| **v0.31.0** | 척추 단순화 + 캐시 전면화 — tui·multi-turn 세션·candle 제거, config v4→v5, image/pdf/code 임베딩 캐시, OCR/caption derivation 캐시. | [release](https://gitea.altair823.xyz/altair823-org/kebab/releases/tag/v0.31.0) · [notes](docs/release-notes/v0.31.0-draft.md) |
| **v0.30.1** | pdf-page-v1.2 oversize 분할 / rag-v4 provenance 라벨 / config budget floor 검증. | [release](https://gitea.altair823.xyz/altair823-org/kebab/releases/tag/v0.30.1) · [notes](docs/release-notes/v0.30.1-draft.md) |
| **v0.30.0** | md-heading-v2 — 거대 청크가 임베딩 컨텍스트를 깨지 않도록 oversize 후처리 분할. | [release](https://gitea.altair823.xyz/altair823-org/kebab/releases/tag/v0.30.0) · [notes](docs/release-notes/v0.30.0-draft.md) |
| **v0.29.0** | provenance 출처 필터 — `[[workspace.sources]]` 멀티소스 + `--source` / `--source-type`. | [release](https://gitea.altair823.xyz/altair823-org/kebab/releases/tag/v0.29.0) · [notes](docs/release-notes/v0.29.0-draft.md) |
| **v0.28.0** | config 스키마 v2→v3 — 미디어 ingest 설정을 `[ingest]` 우산으로 통합. | [release](https://gitea.altair823.xyz/altair823-org/kebab/releases/tag/v0.28.0) · [notes](docs/release-notes/v0.28.0-draft.md) |
| **v0.22.0** | candle 임베딩 provider (NUMA-안전, opt-in). *(v0.31.0 에서 candle 제거됨.)* | [release](https://gitea.altair823.xyz/altair823-org/kebab/releases/tag/v0.22.0) · [notes](docs/release-notes/v0.22.0-draft.md) |
| **v0.20.2** | Ask 응답언어 자동 매칭 + 검색 품질 eval 인프라. | [release](https://gitea.altair823.xyz/altair823-org/kebab/releases/tag/v0.20.2) · [notes](docs/release-notes/v0.20.2-draft.md) |
| **v0.20.1** | 한국어 형태소 검색(V009) + 영어 substring 회귀 수정. | [notes](docs/release-notes/v0.20.1-draft.md) |

> **v0.1.0 ~ v0.19.0, v0.21.x, v0.23.0 ~ v0.27.x** 등 per-version 노트가 없는 릴리스는 [Gitea releases](https://gitea.altair823.xyz/altair823-org/kebab/releases) 의 태그 목록을 참조. (kebab 은 v0.1.0 부터 태그가 있으나 상세 노트는 v0.20.1 부터 작성.)

새 릴리스 컷 시: `docs/release-notes/vX.Y.Z-draft.md` 작성(gitea 가 긴 한국어 body 를 손상시켜 release 는 짧은 body + 이 파일 링크) → 이 표에 한 줄 추가.
