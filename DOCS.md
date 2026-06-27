# 📍 kebab 문서 지도 (Documentation Map / Source-of-Truth Index)

**현재 코드베이스의 진실 = 코드 + 아래 *living* 문서.** *계약* 은 "무엇을 의도했나"의 동결된 baseline, *증거* 는 과거 검증 기록이다. 그 외 historical 문서(옛 실행계획·task spec·feature spec·handoff)는 **git history 에만** 있다 — 2026-06-27 doc-reorg 에서 압축·삭제했고, durable 한 내용은 전부 아래 living 문서로 흡수했다.

> 처음 보는 사람은 이 표 하나로 "어디를 봐야 현재 진실인지" 알 수 있다. 무엇이 최신인지 헷갈리면 여기부터.

## 알고 싶은 것 → 어느 문서 (Source of Truth)

| 알고 싶은 것 | 봐야 할 문서 | 구역 |
|---|---|---|
| 어떻게 설치·사용하나 (명령·플래그·config) | [README.md](README.md) | living |
| 내부 구조·crate 그래프·기술 결정·구현 불변식 | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | living |
| 지금 어디까지 됐나 (phase 진척·다음 후보) | [HANDOFF.md](HANDOFF.md) + [tasks/INDEX.md](tasks/INDEX.md) | living |
| **동작이 설계와 다를 때 뭐가 맞나** (머지 후 deviation) | [tasks/HOTFIXES.md](tasks/HOTFIXES.md) | living · **최우선 진실** |
| 버전별 변경 이력 | [CHANGELOG.md](CHANGELOG.md) + [docs/release-notes/](docs/release-notes/) | living |
| 컴포넌트(crate 그룹) 상세 구조 | [docs/components/](docs/components/)`<group>/README.md` | living |
| `--json` wire 계약 (외부 통합) | [docs/wire-schema/v1/](docs/wire-schema/v1/) | living (계약) |
| 에이전트/MCP 통합 | [docs/mcp-usage.md](docs/mcp-usage.md) | living |
| 격리 KB 로 직접 돌려보기 (smoke) | [docs/SMOKE.md](docs/SMOKE.md) | living |
| 기능별 도그푸딩 시나리오 | [docs/DOGFOOD.md](docs/DOGFOOD.md) | living |
| AI 코딩 에이전트 작업 규칙 | [CLAUDE.md](CLAUDE.md) | living |
| 설계 원안 (12 섹션 계약) | [docs/superpowers/specs/2026-04-27-kebab-final-form-design.md](docs/superpowers/specs/2026-04-27-kebab-final-form-design.md) | **계약 (frozen)** |
| 설계 기원·근거 (최초 보고서) | [kebab_local_rust_report.md](kebab_local_rust_report.md) | 증거 |
| v0.18.0 NLI 검증 도그푸딩 증거 | [docs/dogfood/v0.18.0/](docs/dogfood/v0.18.0/) | 증거 |

## 구역 (zones)

- **living** — 현재 코드베이스의 진실. 기능/표면 변경 시 동기화 필수 (CLAUDE.md §User-facing docs 가 강제).
- **계약 (frozen)** — `docs/superpowers/specs/2026-04-27-…-design.md` 단 하나. "무엇을 의도했나"의 동결 baseline. 편집 금지. 실제 동작이 이와 다르면 **HOTFIXES.md 가 진실**.
- **증거** — 과거 검증/설계 기록 (rust_report, dogfood/v0.18.0). 갱신 안 함.
- **git history** — 삭제된 historical 문서(plans·handoffs·task specs·feature specs)는 git 에만. `git log --all -- <path>` 로 복구 가능. durable 내용은 이미 living 으로 흡수됨.

## 원칙 (재발 방지)

새 결정·deviation 은 **HOTFIXES.md** 에, 구조·기술결정은 **ARCHITECTURE.md** 에 직접 기록한다. 별도 "실행계획/handoff" 문서를 새로 쌓지 않는다 — 그게 268개로 불어난 원인이었다 (CLAUDE.md §User-facing docs).
