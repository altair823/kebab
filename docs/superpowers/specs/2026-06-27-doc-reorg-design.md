---
title: Documentation reorganization — living / frozen / archive 3-zone + SoT map
date: 2026-06-27
status: design
---

# 문서 재정리 설계 (doc-reorg)

## 1. 문제

여러 세션·작업을 거치며 문서가 ~268개(`docs/**` md 165 + `tasks/**` 99 + root 4)로 불어났다. 3개 병렬 explorer 의 전수 특성화 결과, **코어(living + frozen)는 의외로 잘 정리돼 있고 SoT 도 territory 별로 이미 정의**(CLAUDE.md §User-facing docs)돼 있으나:

1. **그 SoT 지도를 한눈에 보여주는 단일 인덱스가 없어** "뭐가 최신/진실인지"가 헷갈린다 — 사용자의 1순위 pain.
2. **historical 실행 아티팩트**(plans 64 · handoffs 8 · 옛 dogfood 1)가 living 문서와 같은 트리에 섞여 시각적으로 "현재 문서"처럼 보인다.
3. **오펀**: `docs/spec/` 7개 스텁(240–534B, frozen 설계로 향하는 포인터, inbound 0).
4. **release-notes**: 9개 `*-draft` 파일, "draft" 오칭, CHANGELOG 부재, 불완전.
5. **stale**: 컴포넌트 README 2개(normalize-chunk·foundation)가 흡수된 crate(kebab-normalize·kebab-parse-types, v0.19.0) 참조.

## 2. 목표 / 비목표

**목표**
- "현재 코드베이스의 진실(SoT)이 뭔지, 어느 문서가 최신인지"를 **단일 지도 문서**로 즉시 알 수 있게 한다.
- 문서를 **living(현재 진실) / frozen(계약) / archive(이력)** 3구역으로 명확히 분리한다.
- 오펀·중복 제거, release-notes 통합, stale 수정으로 트리를 가볍게.

**비목표**
- frozen 거버넌스 앵커(`specs/` 56 + `tasks/p*/` 86) 이동·편집 — CLAUDE.md frozen 규칙상 절대 안 함.
- living 코어 문서의 내용 재작성 — 이미 건강함(SoT 명확, 중복 <20%). stale 2건만 수정.
- SMOKE.md(40KB) 의 scripts/ 추출 리팩터 — 별도 작업으로 미룸(이번 범위 밖).

## 3. 3구역 정의

| 구역 | 의미 | 편집 정책 | 멤버 |
|------|------|-----------|------|
| **living** | 현재 코드베이스의 진실. 코드 변경 시 동기화 필수 | 계속 갱신 | README, HANDOFF, CHANGELOG(신규), CLAUDE.md, DOCS.md(신규), docs/ARCHITECTURE·DOGFOOD·SMOKE·mcp-usage, docs/components/*, docs/wire-schema/v1, tasks/INDEX·HOTFIXES·_template·phase-* |
| **frozen** | 머지 시점 동결된 계약·역사적 spec. 절대 후편집 안 함 | 동결(편집·이동 금지) | docs/superpowers/specs/ (56, 설계 계약 포함), tasks/p0../p10../ (86) |
| **archive** | point-in-time 실행/측정 이력. "현재 진실 아님" | 동결(이력) | docs/archive/** (신규) |

**최우선 규칙**: 동작이 frozen spec 과 다를 때 진실은 **tasks/HOTFIXES.md**(living). archive/ 는 "왜 이렇게 됐나"의 이력일 뿐 현재 상태가 아니다.

## 4. 최종 트리 (after)

```
kebab/
├── README.md                     [SoT·사용법]  (상단에 DOCS.md 포인터 1줄 추가)
├── DOCS.md                       ← NEW 📍 문서 지도 / SoT 인덱스 (root, 별도 파일)
├── HANDOFF.md                    [SoT·진척]
├── CHANGELOG.md                  ← NEW (release-notes 9개 통합, 역순)
├── CLAUDE.md                     [AI 지침] (§docs 갱신)
├── kebab_local_rust_report.md    (frozen·원본 설계 보고서, DOCS.md 에서 "이력"으로 링크)
├── docs/
│   ├── ARCHITECTURE.md           [SoT·구조/결정]
│   ├── DOGFOOD.md  SMOKE.md  mcp-usage.md
│   ├── components/ (13)          [컴포넌트 상세]
│   ├── wire-schema/v1/           [SoT·wire 계약]
│   ├── superpowers/specs/ (56)   [frozen 앵커 — 제자리]
│   └── archive/                  ← NEW "현재 진실 아님"
│       ├── plans/ (64)           ← docs/superpowers/plans/
│       ├── handoffs/ (8)         ← docs/superpowers/handoffs/
│       ├── release-notes/ (9)    ← docs/release-notes/ (CHANGELOG 로 요약된 원본)
│       └── dogfood-runs/v0.18.0/ ← docs/dogfood/v0.18.0/
└── tasks/
    ├── INDEX.md  HOTFIXES.md  _template.md  phase-*.md   (living)
    └── p0../p10../ (86)          [frozen — 제자리]
```

삭제: `docs/spec/` (7 스텁). 빈 `docs/superpowers/plans·handoffs`, `docs/release-notes`, `docs/dogfood` 디렉토리 정리.

## 5. DOCS.md — 핵심 산출물 (SoT 지도)

root `DOCS.md`. 구성:

1. **한 줄 선언**: "현재 코드베이스의 진실 = 코드 + 아래 *living* 문서. *frozen* 은 동결된 계약, *archive* 는 '왜 이렇게 됐나' 이력(현재 상태 아님)."
2. **"알고 싶은 것 → SoT" 표**:

   | 알고 싶은 것 | Source of Truth | 구역 |
   |---|---|---|
   | 어떻게 설치·사용하나 | `README.md` | living |
   | 내부 구조·crate 그래프·기술 결정 | `docs/ARCHITECTURE.md` | living |
   | 지금 어디까지 됐나(진척) | `HANDOFF.md` + `tasks/INDEX.md` | living |
   | **동작이 설계와 다를 때 뭐가 맞나** | `tasks/HOTFIXES.md` | living(최우선) |
   | 버전별 변경 이력 | `CHANGELOG.md` | living |
   | 컴포넌트(crate 그룹) 상세 | `docs/components/<group>/README.md` | living |
   | `--json` wire 계약 | `docs/wire-schema/v1/` | living(frozen 계약) |
   | 설계 원안(12 섹션 계약) | `docs/superpowers/specs/2026-04-27-…-design.md` | frozen |
   | 컴포넌트별 task 명세(원안) | `tasks/p<N>/` | frozen |
   | 옛 실행계획·측정·dogfood run | `docs/archive/**` | 이력(편집 금지) |

3. **3구역 디렉토리 맵** + 각 구역 편집 정책 1줄.
4. **갱신 책임**: "기능/표면 변경 시 README+ARCHITECTURE 동기화는 CLAUDE.md §User-facing docs 가 강제" 로 연결.

## 6. CHANGELOG.md

- 역순(v0.32.0 → v0.20.1). 각 버전: 1–3줄 핵심(사용자 영향 위주) + Gitea release 링크.
- 문서화 안 된 구간(v0.21.0, v0.23.0–v0.27.x, sub-releases)은 한 줄 `> v0.21–v0.27: Gitea releases 참조(개별 draft 없음)` 로 명시(거짓 완전성 회피).
- 원본 9개 draft 는 `docs/archive/release-notes/` 에 보존(증거).
- README 버전 절 + DOCS.md 에서 CHANGELOG 링크.

## 7. 이동/링크 정책 (frozen 충돌 회피 — 가드레일)

1. **이동은 `git mv`** (history 보존).
2. **사전 점검(blocker gate)**: 이동 대상(plans·handoffs·release-notes·dogfood)을 **frozen spec/task 가 링크하는지 전수 grep**. 발견 시 그 frozen 파일은 편집 불가 → 해당 이동 대상은 **옮기지 않고 제자리 유지**(frozen 무결성 > 정리). explorer 분석상 inbound 는 전부 living(HOTFIXES·HANDOFF·INDEX)뿐이라 충돌 0 예상이나 실행 시 확정.
3. **living 문서 inbound 링크(~13: HOTFIXES 8 · HANDOFF 3 · INDEX 2)는 repoint 필수** (living 이라 편집 가능).
4. **archive 내부 문서의 spec 링크**(plan→spec 등)는 best-effort: 깨져도 historical 이라 저-위험. 가능하면 상대경로 보정, 비용 크면 방치 허용(DOCS.md 가 "archive 는 이력" 명시).
5. **검증**: 이동·삭제 후 living 문서(README·HANDOFF·ARCHITECTURE·DOCS·CLAUDE·HOTFIXES·INDEX)에서 깨진 상대 링크 0 (markdown 링크 grep).

## 8. CLAUDE.md 갱신

§User-facing docs 에:
- **DOCS.md = 문서 지도(SoT 인덱스)** 로 명시, "새 문서를 어디 둘지/뭐가 SoT 인지 헷갈리면 DOCS.md 먼저".
- **3구역 정책** 명문화: living=갱신 필수 / frozen=동결 / `docs/archive/`=이력(편집·신규금지, point-in-time 만).
- **CHANGELOG.md** = release 변경 이력 living 문서, release 컷 시 갱신(release 절과 연결).
- 새 historical 산출물(실행계획·측정·dogfood run)은 처음부터 `docs/archive/<kind>/` 에.

## 9. 작업 단위 (구현 시 분할)

1. **stale 수정**(2): normalize-chunk·foundation README 흡수-crate 참조 1줄씩. (독립·무위험)
2. **삭제**: `docs/spec/` 7 스텁. (inbound 0 확인 후)
3. **CHANGELOG 합성** → `CHANGELOG.md`.
4. **archive 이동**: `git mv` plans·handoffs·release-notes·dogfood → `docs/archive/**` (사전 grep gate 통과 후) + living inbound 13 repoint.
5. **DOCS.md 작성**(지도) + README 포인터 1줄.
6. **CLAUDE.md §docs 갱신**.
7. **검증**: living 문서 깨진 링크 0, 트리 확인.

각 단위는 독립 커밋. 4번(이동)이 가장 위험 → 사전 grep gate + 사후 링크 검증 필수.

## 10. 성공 기준

- root `DOCS.md` 하나로 "현재 진실/최신 문서/이력"을 즉시 식별 가능.
- `docs/` 최상위에 historical 아티팩트 0 (전부 `archive/` 하위 또는 frozen `specs/`).
- 오펀 스텁 0, release-notes 단일 CHANGELOG.
- living 문서 깨진 링크 0, frozen specs/tasks 무변경.
- CLAUDE.md 가 3구역 + DOCS.md 지도 정책을 강제.

## 11. 위험 / 메모

- **frozen→이동대상 링크**가 grep 에서 나오면 그 대상은 제자리 유지(부분 정리 허용). frozen 무결성 우선.
- DOCS.md 는 living — 새 문서/구역 추가 시 갱신 필요(CLAUDE.md 가 강제).
- archive 이동으로 git blame 경로가 바뀌나 `git mv` 라 history 추적 가능.
