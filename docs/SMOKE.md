---
title: "kb 스모크 실행 가이드"
date: 2026-05-01
---

# kb 스모크 실행 가이드

P3-5 머지 후 (`kb-app::ingest` / `search` / `list` / `inspect` 와이어링) 부터, 그리고 P4-3 머지 후 (`kb ask` 와이어링) 부터 사용자가 자기 설치본을 직접 검증할 수 있다. 이 문서는 사용자 환경 (`~/.config/kb/`, `~/.local/share/kb/`) 을 건드리지 않고 임시 디렉토리에 격리된 KB 를 띄워 전체 파이프라인을 1세션 안에 한 번 돌리는 절차다.

## 준비

빌드:

```bash
cargo build --release -p kb-cli   # debug 도 무방. 디버그가 더 빠르게 빌드됨.
```

원격 Ollama (선택, `kb ask` 만 필요):

```bash
# Mac 등 별도 호스트에서
OLLAMA_HOST=0.0.0.0:11434 ollama serve
ollama pull gemma4:26b           # 또는 qwen2.5:32b 등 — 자세한 비교는 README
```

본 머신에서 reachability 검증:

```bash
curl http://<host>:11434/api/tags
```

`{"models": [...]}` 가 나오면 네트워크 + 방화벽 OK.

## 격리된 워크스페이스 생성

```bash
mkdir -p /tmp/kb-smoke/{workspace,data}
cat > /tmp/kb-smoke/workspace/intro.md <<'EOF'
---
title: 인사말
tags: [demo]
lang: ko
---
# 안녕

이 문서는 스모크 테스트 fixture 다.
EOF
```

여러 파일을 시드하고 싶으면 본인 KB 일부를 `cp -r` 으로 복사해도 좋다 (다음 절차는 6개 markdown 가정).

## 격리된 config

`/tmp/kb-smoke/config.toml`:

```toml
schema_version = 1

[workspace]
root = "/tmp/kb-smoke/workspace"
include = ["**/*.md"]
exclude = [".git/**", "node_modules/**", ".obsidian/**"]

[storage]
data_dir = "/tmp/kb-smoke/data"
sqlite = "{data_dir}/kb.sqlite"
vector_dir = "{data_dir}/lancedb"
asset_dir = "{data_dir}/assets"
artifact_dir = "{data_dir}/artifacts"
model_dir = "{data_dir}/models"
runs_dir = "{data_dir}/runs"
copy_threshold_mb = 100

[indexing]
max_parallel_extractors = 2
max_parallel_embeddings = 1
watch_filesystem = false

[chunking]
target_tokens = 500
overlap_tokens = 80
respect_markdown_headings = true
chunker_version = "md-heading-v1"

[models.embedding]
provider = "fastembed"               # "none" 으로 두면 lexical-only — Ollama 불필요
model = "multilingual-e5-small"
version = "v1"
dimensions = 384
batch_size = 64

[models.llm]
provider = "ollama"
model = "gemma4:26b"                 # 사용자 환경에 맞춰 교체
context_tokens = 16384
endpoint = "http://192.168.0.47:11434"
temperature = 0.2
seed = 42

[search]
default_k = 10
hybrid_fusion = "rrf"
rrf_k = 60
snippet_chars = 220

[rag]
prompt_template_version = "rag-v1"
score_gate = 0.05                    # RRF 정규화 후 [0, 1] 범위라 default 그대로 OK
explain_default = false
max_context_tokens = 6000
```

`KB_*` 환경변수로 override 가능 (`KB_MODELS_LLM_MODEL=qwen2.5:32b kb …` 등). 자세한 키 목록은 `crates/kb-config/src/lib.rs` 의 `apply_env` 매치 암.

## 명령 시퀀스

```bash
KB() { ./target/debug/kb --config /tmp/kb-smoke/config.toml "$@"; }

KB doctor                                          # 1. health check
KB ingest                                          # 2. 워크스페이스 색인
KB list docs                                       # 3. 색인 결과 목록
KB search --mode lexical "코루틴" --k 3            # 4. lexical 검색
KB search --mode vector "memory safety" --k 3      # 5. vector 검색
KB search --mode hybrid "Cargo workspace" --k 3    # 6. hybrid 검색
KB inspect chunk <chunk_id>                        # 7. raw chunk 보기
KB ask "이 KB 안에서 ..." --mode hybrid --k 5     # 8. RAG 답변 (Ollama 필요)
KB --json ask "..." --mode hybrid                  # 9. 기계 친화 출력 검증
```

각 명령은 0 종료 코드면 정상. `kb ask` 는 거절 시 종료 코드 1 (`RefusalSignal`) — 의도된 동작.

## 검증 체크리스트

- `kb doctor` 가 `--config` path 를 honor 하고 그 안의 `storage.data_dir` 를 출력 (XDG default 가 아님).
- `kb ingest` idempotent — 두 번째 실행이 `new=0 updated=N`.
- `kb list docs` 출력에 frontmatter 의 `title` 이 아닌 deterministic `doc_id` (32-hex) + `workspace_path` 가 보임.
- `kb search --mode hybrid` 의 `fusion_score` 가 `[0, 1]` 범위 (top-1 종종 1.0 — 두 retriever 모두 rank 1 일 때).
- `kb ask` JSON 응답에 `model.id` 가 config 의 모델 (`gemma4:26b` 등) 과 일치, `embedding.id = multilingual-e5-small`, `citations[].marker` 가 `[1]` / `[2]` 형식 (square-bracketed bare index).
- 코퍼스에 없는 주제로 `kb ask` → `refusal_reason: "llm_self_judge"` (또는 `no_chunks` / `score_gate`) + `grounded: false`.

## 정리

```bash
rm -rf /tmp/kb-smoke/data        # 데이터만 날리고 다시 ingest 가능
rm -rf /tmp/kb-smoke              # 통째로 정리
```

`~/.config/kb/` 와 `~/.local/share/kb/` 는 한 번도 터치되지 않는다 (`--config` flag 가 정확히 honor 되는 경우 — P3-5 hotfix 이후 보장).

## 알려진 동작

- 첫 `kb ingest` 시 fastembed 모델 다운로드 (~470MB) — `data_dir/models/fastembed/` 에 캐시.
- `kb ask` 응답 시간 = LLM 토큰 throughput 에 종속. M4 Pro 48GB + gemma4:26b 기준 답변 50–100 토큰에 20–55초.
- `--config` path 가 존재하지 않거나 malformed 면 `kb doctor` 가 hard fail (defaults 가 silently mask 하지 않게 하는 hotfix 동작).
- 매 CLI invocation 마다 fastembed 모델 init 비용 (~4초) — process-level 캐시 부재 때문. P9 TUI 진입 시 `App` 의 `OnceLock` 으로 세션 동안 한 번만 init.

자세한 history 와 발견된 버그는 [tasks/HOTFIXES.md](../tasks/HOTFIXES.md) 참조.
