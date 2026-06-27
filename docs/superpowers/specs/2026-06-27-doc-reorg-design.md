---
title: Documentation reorganization — max-compress to living-SoT + single design contract
date: 2026-06-27
status: design
supersedes_decision: 초안의 "archive 이동" 안은 폐기 — frozen 문서끼리 결합도가 높아 이동이 frozen 무결성을 깸. 사용자 지침으로 "압축/감축(destructive 허용)" 으로 전환.
---

# 문서 재정리 설계 (doc-reorg, 최종 = max-compress)

## 1. 목표 (사용자 지침 반영)

여러 세션 누적으로 ~268개로 불어난 문서에서 **stale·outdated·불필요 정보를 최대한 줄인다.** 핵심:

- **현재 코드베이스의 진실(SoT)이 뭔지, 어느 문서가 최신인지**를 단일 지도(`DOCS.md`)로 즉시 알 수 있게.
- **현재 코드와 달라진 과거 문서는 destructive(삭제) 허용** — frozen 규칙 완화. 단 **durable(아직 living 에 없는, 현재도 유효한) 정보는 삭제 전에 living 문서로 흡수**(안전망). git history 가 원본을 보존.
- 결과: 트리에 **living(현재 진실) + 설계 계약 1개 + 증거 소수**만 남긴다.

## 2. 근거 (5-에이전트 전수 분석)

| 클러스터 | 수/크기 | 판정 | 근거 |
|---|---|---|---|
| `docs/superpowers/plans/` | 64 / 2.4MB | **삭제** | 머지 완료 작업의 worker 실행 스캐폴딩. durable **0건**(코드+task spec+HOTFIXES 가 완전 대체). inbound 링크는 전부 provenance breadcrumb. |
| `docs/superpowers/handoffs/` | 8 / 132KB | **삭제** | 측정 기록. durable 측정값이 **이미 HOTFIXES 에 요약돼 있음**(in_living=True). |
| `tasks/p0../p10../` | 86 / 708KB | **압축→삭제** | 작업별 frozen 계약. 코드+HOTFIXES+ARCHITECTURE 가 대체. 단 **아직 living 에 없는 durable invariant 4건**은 추출 필요(§4). |
| `docs/superpowers/specs/` (feature specs) | 55 / 1.2MB | **삭제** | feature 설계 문서. 코드+ARCHITECTURE+HOTFIXES 가 대체. HOTFIXES 가 deviation 을 self-contained 로 기술(spec 참조는 provenance). |
| `docs/spec/` 스텁 | 7 / 32KB | **삭제** | 설계 계약으로 향하는 240–534B 오펀 포인터, inbound 0. |
| `docs/superpowers/specs/2026-04-27-…-design.md` | 1 / 71KB | **유지(frozen 계약)** | 워크스페이스 유일 설계 계약, 12 섹션. "무엇을 의도했나"의 baseline. HOTFIXES deviation 이 이걸 참조. |
| `docs/release-notes/` | 9 / 84KB | **유지(living)** | gitea 가 긴 한국어 body 를 손상시켜 release 가 이 커밋 파일을 링크 + release 프로세스가 계속 새로 생성 → load-bearing. |
| `docs/dogfood/v0.18.0/`, `kebab_local_rust_report.md` | 2 | **유지(증거)** | v0.18.0 NLI 검증 증거 + 설계 기원 보고서. 작고 참조됨. |
| living (README/ARCHITECTURE/HANDOFF/HOTFIXES/INDEX/CLAUDE/components/wire-schema/DOGFOOD/SMOKE/mcp-usage) | — | **유지** | 현재 진실. SoT territory 별로 명확. |

## 3. 최종 트리 (after)

```
kebab/
├── README.md  DOCS.md(NEW)  HANDOFF.md(슬림)  CHANGELOG.md(NEW)  CLAUDE.md(갱신)
├── kebab_local_rust_report.md            (증거·기원)
├── docs/
│   ├── ARCHITECTURE.md                   (+durable 4건 흡수)
│   ├── DOGFOOD.md  SMOKE.md  mcp-usage.md
│   ├── components/ (13)                  (task-spec 링크 정리)
│   ├── release-notes/ (9)                (living, CHANGELOG 가 인덱스)
│   ├── wire-schema/v1/
│   ├── dogfood/v0.18.0/                  (증거)
│   └── superpowers/specs/                (2개만: 2026-04-27 계약 + 2026-06-27 본 spec)
└── tasks/
    ├── INDEX.md(슬림)  HOTFIXES.md(링크정리)  _template.md  phase-*.md(링크정리)
    └── (p0../p10../ 삭제됨)
```

삭제 디렉토리: `docs/superpowers/plans/`, `docs/superpowers/handoffs/`, `docs/spec/`, `tasks/p0..p10/`. specs/ 의 feature spec 55개 삭제(계약 + 본 spec 만 잔존).

## 4. durable 추출 (삭제 전 — 안전망)

분석이 "아직 living 에 없다"고 확인한 항목만. 추출 후 원본 삭제.

→ **`docs/ARCHITECTURE.md`** 에 추가:
1. **LanceDB upsert 순서/원자성** (p3-3): SQLite-first(`INSERT OR REPLACE` + 3-state status marker `pending`/`committed`) → Lance-second(`MergeInsert`). 검색은 `status='committed'` 만 → partial-write orphan 회피. 크래시 시 reconcile. → §Persistence 새 소절.
2. **RAG score-gate + context-budget** (p4-3): `hits[0].fusion_score < rag.score_gate` 면 거절(한국어 메시지 + top-3 후보). budget = `max_context_tokens`(기본 8000) ∩ `llm.context_tokens() − (prompt+query+256 reserve)`. entry-by-entry packing 포맷. → §RAG.
3. **PDF chunk_id 충돌 회피** (p7-2): pdf-page-v1 이 한 page-block 을 byte 예산 초과 시 다중 chunk 로 분할 → 동일 block_ids 충돌을 `policy_hash#c{char_start}` variant 로 회피. → §핵심 결정 각주.
4. **heading-aware 청커 우선순위** (p1-5): ① heading 경계 우선 ② code block 절대 미분할 ③ table 가능하면 single-chunk ④ 긴 섹션 paragraph 분할 ⑤ heading_path 전파. → §청킹.

→ **`CLAUDE.md`** 에 추가:
5. **Allowed/Forbidden 의존성 경계표** (86 spec 에 흩어진 §deps 통합): crate별 1행 (예: `kebab-core` MUST NOT depend on `kebab-*`; `kebab-eval` metrics/compare MUST NOT import retrieval/embed/llm; UI crate 는 `kebab-app` 만). 기존 §Allowed/forbidden deps 확장.

(version cascade·facade rule·release bump 규칙은 이미 CLAUDE.md 에 있음 — 확인만.)

handoffs/feature-spec 의 측정·deviation durable 은 **이미 HOTFIXES 에 있음**(분석 확인) → 추출 불필요, 삭제 전 grep 확인만.

## 5. 신규 SoT 문서

**`DOCS.md`** (root): "알고 싶은 것 → SoT" 표 + 구역(living / frozen 계약 / 증거) 맵 + 한 줄 선언("현재 진실 = 코드 + living 문서; 계약 = 설계 의도 baseline; 삭제된 historical 은 git history"). 표는 §초안과 동일하되 archive 행 제거, "이력은 git" 로.

**`CHANGELOG.md`** (root): release-notes 9개 역순 인덱스(버전·핵심 1–3줄·gitea 링크). 미문서화 구간(v0.21–v0.27)은 "Gitea releases 참조" 명시.

## 6. living 문서 슬림화 + 링크 정리 (~94 inbound)

삭제 대상을 링크하던 kept 문서들 — 전부 living 이라 편집 가능:
- **`HANDOFF.md`** (52KB): p9-fb-* task spec 링크 + per-task 상세 → **phase-level 요약**으로 슬림(HANDOFF 본래 역할). 삭제된 spec/plan 링크 제거(내용은 이미 HOTFIXES/요약에).
- **`tasks/INDEX.md`**: 86 task spec 링크 dashboard → **phase 진척 dashboard**(spec 링크 없이 phase 상태만). source 헤더의 rust_report 참조 유지.
- **`tasks/HOTFIXES.md`**: 삭제된 plans/specs 로의 ~8 링크 → drop(텍스트 self-contained) 또는 inline. **HOTFIXES 본문은 보존**(live 진실).
- **`docs/components/*/README.md`**: "task spec:" 링크(36) → 제거 또는 "(설계 baseline: design contract §X)" 한 줄로.
- **`tasks/phase-*.md`**: 삭제된 p*/ 링크 정리(phase epic 요약만 유지).
- **`docs/superpowers/specs/2026-04-27 계약`**: **frozen — 편집 금지.** 이 계약이 삭제 대상(handoffs/plans)을 링크하면 그 링크만 깨짐 → 허용(계약은 의도 기록, 깨진 historical 링크는 저위험). 단 frontmatter 의 parent 링크는 본 spec 이 정리 가능하면 정리.

## 7. CLAUDE.md 갱신

- **§Spec contract / Allowed-forbidden deps / User-facing docs** 재작성: task spec 층이 사라졌으므로 "task specs frozen" 규칙 → "**유일 frozen 계약 = 2026-04-27 design doc**; 구현 진실 = 코드 + HOTFIXES + ARCHITECTURE" 로. 의존성 경계표(§4-5) 흡수.
- **DOCS.md = SoT 지도** 명시 + 3구역(living/계약/증거) 정책 + "신규 historical 산출물(plan/handoff 류)은 만들지 않는다 — 결정·deviation 은 HOTFIXES, 구조는 ARCHITECTURE 에 직접" 명문화(중구난방 재발 방지).
- **CHANGELOG.md** = release 변경 이력 living.

## 8. 실행 순서 (안전: 추출 → 슬림 → 삭제 → 검증)

1. **추출**: durable 4건 → ARCHITECTURE, 경계표 → CLAUDE.md. (삭제 전 필수)
2. **신규**: DOCS.md, CHANGELOG.md.
3. **stale 수정**: normalize-chunk·foundation README 2건.
4. **슬림+링크정리**: HANDOFF, INDEX, HOTFIXES, components, phase-*, CLAUDE §docs.
5. **삭제(`git rm`)**: plans(64)·handoffs(8)·docs/spec(7)·tasks/p*(86)·specs feature(55).
6. **검증**: kept 문서(README·DOCS·ARCHITECTURE·HANDOFF·INDEX·HOTFIXES·CLAUDE·components·CHANGELOG) **깨진 상대 링크 0** (markdown 링크 grep). 빌드 무관(문서). 계약(2026-04-27)·본 spec 잔존 확인.

각 단계 독립 커밋. 5(삭제) 전에 1(추출) 완료 필수.

## 9. 성공 기준

- root `DOCS.md` 하나로 "현재 진실/최신/계약/증거" 즉시 식별.
- `docs/superpowers/` = specs/{계약, 본 spec} 만. plans/handoffs/feature-specs/tasks-p 전부 제거.
- durable 4건 ARCHITECTURE 에 보존, 경계표 CLAUDE.md 에 보존.
- kept 문서 깨진 링크 0. 계약 무변경(frozen). git history 에 원본 전부 보존.
- CLAUDE.md 가 신규 historical 누적 금지 정책 명시 → 재발 방지.

## 10. 위험 / 메모

- **되돌리기**: 전부 git history 에 있음(`git mv`/`git rm`). 복구는 git revert/checkout.
- **계약의 깨진 historical 링크**: frozen 편집 금지라 일부 잔존 허용(저위험, DOCS.md 가 "삭제된 historical 은 git" 명시).
- **durable 누락 위험**: 분석이 "이미 HOTFIXES 에 있음" 판정한 handoff/feature-spec 은 삭제 전 grep 재확인.
- **HANDOFF/INDEX 슬림화**가 가장 판단 필요한 content 작업 — phase-level 요약 유지, per-task 상세는 HOTFIXES 가 흡수.
