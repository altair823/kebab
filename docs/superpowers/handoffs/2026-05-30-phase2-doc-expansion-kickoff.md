---
title: Phase 2 킥오프 — doc-side expansion (색인시 별칭) + 구현 방법론
date: 2026-05-30
status: Phase 1 머지 완료(#193), Phase 2 설계 대기
audience: 새 세션 (자립적 컨텍스트 — 이 문서 + 아래 참조만으로 이어받기 가능)
related:
  - docs/superpowers/handoffs/2026-05-29-query-paraphrase-robustness-phase1-handoff.md
  - docs/superpowers/research/2026-05-30-vocabulary-gap-recall-fix-research.md
  - docs/superpowers/specs/2026-05-29-query-paraphrase-robustness-eval-design.md
  - docs/superpowers/plans/2026-05-29-query-paraphrase-robustness-eval.md
  - memory: project_paraphrase_robustness, project_rerank_experiment, project_crossscript_diagnosis,
            feedback_omc_teams_usage, feedback_teammate_spawn_mode, feedback_teammate_model_routing,
            feedback_worker_completion_polling, feedback_pr_workflow, feedback_search_quality_dogfood,
            feedback_serial_build_only, feedback_skip_user_review_gates, feedback_explain_friendly
---

# Phase 2 킥오프 — doc-side expansion

## 0. TL;DR

같은 의미를 다른 표현으로 물어도 일관된 검색 품질을 내는 게 목표다. **Phase 1(평가 프레임워크)은
main 에 머지됨(#193).** 진단 결과 **어휘격차(B)가 우세** — 같은 뜻 다른 단어(동의어·풀어쓴 문장·
한/영)면 정답이 top-50 pool 에도 안 들어와(recall@50=0) rerank 로는 못 고친다. 딥리서치 + 우리
corpus PoC 가 처방을 **색인시 doc-side expansion**(문서를 넣을 때 "검색용 별칭"을 1회 생성해 붙이기)
으로 확정했다. **Phase 2 = 이 처방을 flag 뒤에 구현하고 `kebab eval variants` 로 효과 재측정.**

## 1. 여기까지 온 경로 (압축)

1. 한/영 음차 검색 불안정 → 원인은 **vector near-tie**([[project_crossscript_diagnosis]]).
2. 완화책으로 **cross-encoder reranker** 실험(`feat/crossscript-rerank`, default off). full chunk text
   까지 시도했으나 **회귀 못 없앰 → 가설 반증**([[project_rerank_experiment]]). rerank 는 pool 안의
   순서만 바꿔서, 정답이 pool 에 없으면 무력.
3. 사용자가 목표 재정의: "한/영뿐 아니라 **같은 의미의 다른 단어·표현**에서도 일관된 품질".
4. **Phase 1**: `kebab-eval` 에 변형 일관성 평가 추가(group + recall@10 vs recall@50 → A/B 분류).
   dogfood 8그룹×32변형 측정 → **B(어휘격차) 우세, 문제는 한/영이 아니라 "어휘 거리"**
   (영어 paraphrase 도 miss, 일부 한국어는 OK). #193 으로 main 머지.
5. **딥리서치**(104 agent, 적대검증): 최선책 = 색인시 doc-side expansion. query-side(HyDE=거부된
   per-query LLM, Vector-PRF=recall 주장 기각) 부적합. learned-sparse(SPLADE/MILCO) CPU/Rust 경로
   없거나 교차언어 약함. **PoC**(dogfood KB): backprop/raft 별칭추가판 ingest → recall@50=0 이던
   3쿼리가 **rank 1~2 부활**(hybrid+vector, 골든 verbatim 아님=일반화). 핵심 미검증 고리 정량 확인.

## 2. Phase 2 설계 방향 (딥리서치 권고 — 합성, 우리 corpus 측정 필수)

**색인시 doc-side expansion** (`docs/superpowers/research/2026-05-30-vocabulary-gap-recall-fix-research.md`):

- **무엇**: 문서/청크를 색인할 때 로컬 LLM(gemma, config `models.llm` = `gemma4:e4b`, endpoint 이미
  설정됨)으로 "이 문서를 찾을 법한 다른 표현/질문"을 **1회** 생성 — 같은언어 paraphrase + **KO↔EN
  번역 별칭** — 해서 **별도 FTS5 필드**에 저장. RRF 가 {원문 body BM25, 별칭 BM25, e5 dense} 융합.
- **왜 우리 제약에 맞나**: (1) 색인시 1회 = 사용자가 거부한 "per-query LLM(밑 빠진 독)" 아님,
  (2) e5-large dense 유지(bge-m3 dense 는 실측 더 나빴음), (3) 별도 필드라 원문 정확매칭(코드 식별자)
  보존, (4) per-query 지연 ~0.
- **핵심 함정**: vanilla mt5 doc2query 는 *같은 언어* query 만 생성 → 한/영 갭 못 메움. 그래서
  **색인시 KO↔EN 번역 별칭 생성**이 추가로 필요(이게 "합성/추론" 부분 — 논문 직접 벤치 없음 →
  우리 corpus 로 반드시 측정).
- **(선택) 보조**: BGE-M3 sparse 채널(fastembed-rs `BGEM3Q`, CPU)을 4th RRF 채널로 — 단일언어
  term-expansion lift, e5 dense 유지. (교차언어는 약하니 선택사항.)

**딥리서치 openQuestions = Phase 2 가 답할 것:**
1. 색인시 KO↔EN 별칭 생성이 *우리 corpus* 에서 recall@50 을 0→양수로 올리나? 생성 예산(별칭 수/문서,
   모델 크기)의 cost/recall knee 는? → **`/build/dogfood` golden + `kebab eval variants` 로 측정.**
2. ONNX/fastembed 호환 교차언어 learned-sparse 체크포인트 있나, 아니면 색인시 expansion 으로만?
3. doc2query 가 FTS5 index 를 얼마나 부풀리나. Doc2Query--/++ 필터 가치 있나.
4. e5 dense 유지 + BGE-M3 **sparse 만** 추가가 순이득인가, 약한 다국어 sparse 가 노이즈인가.

**설계 시 고려(brainstorm 에서 확정):** ingest pipeline 의 어디에 hook(chunk 후?), 별도 FTS5 필드
스키마 + migration(V0XX), gemma 프롬프트(번역 별칭 품질), versioning cascade(별칭은 새
`chunker_version`/별도 version? re-index 정책), flag 이름·default off, 환각·index 팽창 제어(필터).

## 3. 이미 만든 측정 도구 (Phase 2 검증에 그대로 사용)

- **`kebab eval variants <run_id> [--json]`** — 변형 그룹 일관성 진단. recall@10 vs recall@50 →
  `Ok`/`MisRanked`(A)/`Missing`(B) + group rollup + `pool_possibly_truncated`.
- **dogfood golden**: `/build/dogfood/golden_queries.yaml` 에 8 변형그룹×4 = 32 (ownership, raft,
  mvcc, cap_theorem, gradient_descent, backprop, isolation_levels, vector_database). 같은 group =
  동일 `expected_doc_ids`.
- **측정 절차**(⚠️ `KEBAB_EVAL_GOLDEN` 필수 — 미설정 시 default golden → groups=0):
  ```
  KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml \
    kebab eval run --config /build/dogfood/config.toml --mode hybrid --k 50   # k>=50 필수(아니면 진단 bail)
  KEBAB_EVAL_GOLDEN=/build/dogfood/golden_queries.yaml \
    kebab eval variants <run_id> --config /build/dogfood/config.toml
  ```
- **Phase 1 baseline**(처방 전): `groups=8 fully_consistent=2 A_dominant=2 B_dominant=4 spread@10=0.750`.
  Phase 2 목표 = B_dominant↓, fully_consistent↑, spread↓ (처방 on/off 비교).
- **PoC 방법**(참고): 별칭 추가판 문서를 corpus 에 넣고 incremental ingest(기존 skip, +N) → 실패
  쿼리로 새 doc_id 가 top-50 잡히는지. 비파괴적. (`/build/dogfood/logs/2026-05-30-docexpansion-poc-*`)
- ⚠️ dogfood KB 현재 3942 doc (PoC 별칭 2개 잔존, corpus/_poc 삭제됨). variant 골든은 원본 doc_id
  타겟이라 baseline 무영향. pristine 필요 시 `kebab reset` + reingest.

## 4. 구현 방법론 (지금까지와 동일 — 그대로 따를 것)

### 4.1 워크플로 (superpowers)
**brainstorm → spec → plan → subagent 구현.** 각 단계:
- **brainstorm**(`superpowers:brainstorming`): 사용자와 한 번에 하나씩 질문(쉬운 비유·친절히 —
  사용자는 검색/NLP 지식 적음 [[feedback_explain_friendly]]). 핵심 trade-off 만 AskUserQuestion.
- **spec**(`docs/superpowers/specs/YYYY-MM-DD-*.md`): self-review 후 진행. **사용자 컨펌 게이트
  skip** ([[feedback_skip_user_review_gates]]) — self-review 만 + 바로 다음 단계.
- **plan**(`docs/superpowers/plans/*.md`): TDD bite-sized task, 완성 코드(placeholder 금지), self-review.
- **구현**: task 별 OMC teammate (아래).

### 4.2 OMC teammate 실행 ([[feedback_omc_teams_usage]] [[feedback_teammate_spawn_mode]])
- **sequential single-team only** (multi-team spawn 실측 fail). 한 팀 끝 → shutdown → 다음 팀.
- **spawn**: `OMC_TEAM_ROLE_OVERRIDES='{"<role>":{"model":"claude-opus-4-8|claude-sonnet-4-6"}}'
  omc team 1:claude:<role> --no-decompose "Task X: read <brief-abs-path> and execute exactly; write
  result to <result-abs-path>"`. role 예: executor, code-reviewer.
- **brief 파일 패턴**: task 내용을 `.omc/reviews/<date>-<id>-brief.md` 에 자립적으로 작성
  (계획 task 참조 + 빌드/규약 + 결과파일 경로). spawn 의 task 텍스트는 짧게(brief read 지시).
- **완료 감지**: spawn 직후 background polling shell(`run_in_background=true`) — `omc team status
  <slug>` 의 phase=completed/failed 또는 tasks completed>=1 감지 → task notification 자동 알림
  ([[feedback_worker_completion_polling]]). 작은 task sleep 10, 큰 task sleep 20.
- **모델 확인**: spawn 후 worker pane 캡처로 `Model: Sonnet/Opus` 검증 (`tmux capture-pane -pt <pane>`).
- **shutdown**: `omc team shutdown <slug> --force` (non-force 종종 실패). 다음 팀 전 필수.
- ⚠️ `omc team list` 같은 조회 명령 없음 — "list" 를 task 로 해석해 팀이 spawn 됨. 상태는 `omc team
  status <slug>`. 잘못 뜬 팀은 즉시 `shutdown --force`.

### 4.3 모델 라우팅 ([[feedback_teammate_model_routing]])
- **작은 task → sonnet**, **복잡/핵심 로직 → opus**. 리뷰: 핵심 로직 = opus, 작은 변경 = sonnet.
  micro-patch/fix 라운드 = sonnet.
- (실증: Phase 1 에서 opus 리뷰가 H1 실버그 — pool truncation 으로 진단 무력화 — 를 측정 전 차단.)

### 4.4 task 별 사이클
implement(executor) → **review(code-reviewer, 별도 teammate)** → CHANGES 면 fix 라운드 → **독립
검증**. teammate 보고를 신뢰하지 말고 직접 확인: `git show <hash> --stat`, redirect 파일에서 test/
clippy EXIT, 신규 심볼 grep. ([[feedback_serial_build_only]] 의 직렬 빌드 규약도.)

### 4.5 빌드/테스트 규약 (필수 — 어기면 깨진 커밋)
- `CARGO_TARGET_DIR=/build/out/cargo-target/target` (XFS 4TB), `-j 4` (fast mode 8, OOM 시 `-j 1`).
- **결과를 파일로 redirect + exit code 확인 후에만 커밋.** `cargo ... | grep | tail` **금지**
  (pipe exit 가 grep 거라 cargo 실패 마스킹). 빌드는 백그라운드(run_in_background) 권장.
- cargo clean: /build avail<500G 또는 target>500G 일 때만 ([[feedback_cargo_clean_policy]]).

### 4.6 측정 규율 ([[feedback_search_quality_dogfood]] [[project_rerank_experiment]] 교훈)
- **프록시 금지**: overlap 같은 대리 지표 최적화로 헛돈 전적 있음. 진짜 지표(`kebab eval variants`
  recall/일관성)로 처방 효과 측정.
- **측정값 절대 추측 금지**: grep clean 추출 → Read 로 확인한 값만 기록. (Phase 1 전 세션에서 숫자
  fabrication 2회 발생·정정.)
- 처방은 flag off 기본, on/off 비교 측정 + 회귀(전체 golden) 확인.

### 4.7 PR ([[feedback_pr_workflow]])
- **gitea-pr + 리뷰 루프 모드** (단발/루프 묻지 말 것). 스크립트:
  `/home/altair823/.claude/.omc-launch/skills/gitea-ops/bin/gitea-pr{,-status,-diff,-review}`.
  reviewer login `gitea-ops-reviewer` 별도 계정. PR title 정규식 `^(feat|fix|docs|...)(\(scope\))?: .+`,
  브랜치 `^<type>/<kebab>$`, body `## 요약`+`## 검증` 필수. 회차마다 review 등록, 한국어 본문은
  손상 점검(다시 fetch). 머지는 사용자가 UI 에서 (Claude 자동 머지 안 함).
- **user-facing surface 변경 시 같은 PR 에서 README + HANDOFF + ARCHITECTURE 동기화**
  ([[feedback_readme_sync_rule]]): README 는 좁게(사용법+포인터), 상세는 ARCHITECTURE, flag 망라는
  `--help`/in-app 권위 소스 위임(stale 방지). #193 에서 이 정리 수행함.
- **versioning cascade**: chunker/embedding version 등 변경 시 design §9 cascade — re-process job
  또는 breaking bump. 별칭 필드가 새 version 축이면 migration(V0XX) + dogfood trigger.

## 5. 새 세션 첫 작업

1. 이 문서 + §0~4 참조 + 메모리 로드 확인.
2. **brainstorm Phase 2 설계**: doc-side expansion 의 구체(ingest hook 위치, 별도 FTS5 필드 스키마 +
   migration, gemma 번역-별칭 프롬프트, versioning cascade, flag/config, 환각·팽창 제어). §2 의
   openQuestions 를 설계로 흡수.
3. spec → plan → OMC teammate 구현(§4 방법론) → `kebab eval variants` 로 on/off 측정.
4. 효과 확인되면 gitea-pr 리뷰 루프 + README/ARCH sync. flag off 기본.
