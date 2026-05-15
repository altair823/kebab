# kebab — Code Ingest Design

기준일: 2026-05-15
대상: kebab 워크스페이스를 **코드 corpus** 로 확장 (`Tier 1` AST per-language + `Tier 2` resource-aware + `Tier 3` paragraph fallback). frozen design doc `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` 의 후속이자, 그 §11 비-스코프 중 "모든 파일 포맷의 완벽한 parsing" 을 **부분적으로** 깨는 첫 spec. 단, multi-workspace / watch mode 같은 다른 비-스코프는 그대로 유지.

대상 사용자 시나리오: 한 부모 디렉토리 (`workspace.root`) 아래 *수십 개의 git repo* 를 clone 한 상태에서, 그 corpus 전체에 의미 검색 + RAG 를 한 곳에서 수행.

---

## 0. 동결된 결정 요약

| # | 결정 | 값 | 근거 |
|---|------|-----|------|
| C1 | 스코프 단위 | 한 `workspace.root` 아래 여러 repo (multi-workspace 안 함) | frozen Q10 / §11 그대로 — 부모 디렉토리 한 줄로 커버 |
| C2 | chunking 전략 | Tier 1 = AST per-language, Tier 2 = resource-aware, Tier 3 = paragraph + line-window | 의미 단위가 언어별로 다름. 일률 적용 불가 |
| C3 | embedding 모델 | 기존 `multilingual-e5-large` 유지 | 코드 + 문서 동일 벡터 공간 → cross-corpus 검색. embedding_version cascade 회피 |
| C4 | ignore 통합 | `.gitignore` 자동 honor + `.kebabignore` 추가 layer + 최소 built-in safety net | 사용자 mental model 자연스러움. `.gitignore` 가 source of truth |
| C5 | repo 인식 | `.git/` walk-up 자동 감지, identifier = dir 이름 | 단순 / deterministic / git remote 미설정 repo 도 안 깨짐 |
| C6 | branch 처리 | working tree only. branch 변경 후 ingest 는 blake3 hash 차이로 incremental reprocess | git history aware 색인은 §3 도메인 모델 크게 흔듦 — P+ |
| C7 | Citation variant | 새 `code` variant 도입 (line/page/region/caption/time/**code**) | 의미 분리 명확 — agent / consumer 분기 깔끔 |
| C8 | search hit 추가 | `SearchHit.repo`, `SearchHit.code_lang` (optional, additive minor) | repo 격리 / 통계 / filter |
| C9 | 새 filter | `--media code`, `--code-lang <list>`, `--repo <name>` | search ergonomics |
| C10 | chunker_version | per-language (`code-rust-ast-v1` 등) | 언어별 chunker 독립 진화, §9 cascade rule 깔끔 |
| C11 | crate 구조 | 새 crate `kebab-parse-code` (모든 언어 mod) + 기존 `kebab-chunk` 모듈 확장 | 22 crates 한 번만 증가. 언어 추가는 모듈 한 쌍 |
| C12 | symbol path | per-language convention (`mod::fn` / `pkg.cls.method` / `module/Class.method` …) | 각 언어 self-reference 관습 그대로 |
| C13 | RAG prompt | Phase 1A 는 `rag-v2` 유지. 측정 후 `rag-v3` 도입 검토 | YAGNI |
| C14 | 특수 파일 | manifest 류 (`Cargo.toml` 등) 는 파일 통째로 1 chunk (`manifest-file-v1`) | 작은 파일은 전체 보기가 더 유용 |
| C15 | Phase 분할 | 1A Rust → 1B Python+TS/JS → 1C Go+Java/Kotlin → 1D C/C++ → 2 Tier 2 → 3 Tier 3 | 점진 도입, dogfooding 가능 |
| C16 | built-in skip 최소 | 5 entries: `node_modules/` `target/` `__pycache__/` `.venv/` `venv/` `env/` | `.gitignore` 가 메인 — built-in 은 safety net |
| C17 | generated header sniff | `@generated` / `DO NOT EDIT` 등 marker 6 종 — 첫 ~500 byte read | 첫 도그푸딩 비용 차단 (protobuf 등) |
| C18 | size cap | `max_file_bytes = 262144` (256 KiB), `max_file_lines = 5000` default | 대용량 fixture / minified 차단 |

---

## 1. 스코프 + 비-스코프

### 1.1 스코프 (이 spec 으로 동결되는 것)

- 코드 / 설정 파일 ingest 파이프라인 (parse → chunk → embed → store → retrieve → answer)
- 새 Citation variant `code`
- 새 SearchHit 필드 (`repo`, `code_lang`)
- 새 search filter (`--media code`, `--code-lang`, `--repo`)
- 새 chunker_version 라벨 family (`code-{lang}-ast-v1`, `k8s-manifest-resource-v1`, `dockerfile-file-v1`, `manifest-file-v1`, `code-text-paragraph-v1`)
- 새 crate `kebab-parse-code`
- 기존 `kebab-chunk` 모듈 확장
- repo 자동 감지 + `metadata.repo` / `git_branch` / `git_commit`
- ignore 통합 정책 (`.gitignore` honor + `.kebabignore` + built-in)
- generated / vendored / size cap skip 정책
- IngestReport 카운트 분류 확장
- 새 config 절 `[ingest.code]`
- Phase 분할 (1A → 1B → 1C → 1D → 2 → 3)

### 1.2 비-스코프 (이 spec 으로 명시적으로 *안 다루는* 것)

- **Multi-workspace** — 여전히 single `workspace.root`. 사용자가 직접 부모 디렉토리 정렬.
- **Watch mode** — 여전히 명시 ingest 만.
- **git history aware indexing** — branch / commit 별 snapshot 색인 안 함. working tree 한 시점만.
- **LSP / go-to-definition / find-references** — 코드 *내비게이션* 은 IDE / CC 가 잘 함. kebab 은 *의미 검색* + *RAG* 만.
- **Code-specific embedding 모델** — Phase 2+ 측정 후 검토. 현재 spec 에선 e5-large 유지.
- **`rag-v3` (code-aware prompt)** — Phase 2+ 측정 후 검토.
- **서브모듈 / git worktree** — `.git/` 가 dir 인 normal repo 만 인식. submodule (`.git` file) 은 metadata.repo 만 null 또는 부모 repo 이름 fallback.
- **Cross-repo 의도적 dedup** — blake3 content hash 의 우연 dedup 만 존재. 명시적 dedup 로직 안 함.
- **`kebab://` URL handler** — frozen §11 그대로 P+.

---

## 2. Phase 분할 + 마일스톤

각 phase = 별도 task spec (`tasks/p10/p10-1a-1-code-ingest-framework.md` 등) + 별도 PR. **Phase 1A-1** 이 *프레임워크 일체* (새 crate skeleton, 새 Citation variant, repo metadata, 새 filter, ignore 정책 전체, skip 정책, IngestReport 세분화) 를 들고 들어가는 가장 무거운 phase. 1A-2 이후는 *언어 / chunker 추가* 만.

| Phase | 내용 | 새 crate / 모듈 | 새 chunker_version | 마일스톤 |
|-------|------|----------------|--------------------|----------|
| **1A-1** | 프레임워크 일체 — Citation `code` variant, SearchHit `repo`/`code_lang`, 새 filter (`--media code` / `--code-lang` / `--repo`), ignore 통합 정책, skip 정책 (built-in/generated/size), IngestReport 세분화, config `[ingest.code]` 절. `kebab-parse-code` crate **skeleton** (lang/repo/skip 모듈만, 언어 parser 없음) | `kebab-parse-code` 신설 — infrastructure only, language parser 모듈 없음 | *없음* (chunker 추가 0) | wire schema additive minor commit. 기존 markdown corpus 무영향 검증 (regression test). 코드 ingest 아직 활성 안 됨 |
| **1A-2** | Rust AST chunker 자체 + tree-sitter-rust 도입. Rust 파일 ingest 활성화 | 동일 crate 에 `rust.rs` parser 모듈 + `kebab-chunk/code_rust_ast_v1.rs` | `code-rust-ast-v1` | kebab 자기 자신 dogfooding 가능 |
| **1B** | Python + TS/JS AST ingest | 동일 crate 에 `python.rs` / `typescript.rs` / `javascript.rs` 모듈 + chunker 추가 | `code-python-ast-v1`, `code-ts-ast-v1`, `code-js-ast-v1` | 사내 ML 코드 + 웹 코드 검색 |
| **1C** | Go + Java + Kotlin AST ingest | 동일 crate 에 모듈 추가 | `code-go-ast-v1`, `code-java-ast-v1`, `code-kotlin-ast-v1` | 사내 backend 검색 |
| **1D** | C + C++ AST ingest | 동일 crate 에 모듈 추가 | `code-c-ast-v1`, `code-cpp-ast-v1` | system code 검색 (마지막) |
| **2** | Tier 2 resource-aware: k8s manifest + Dockerfile + 일반 manifest | 동일 crate 에 모듈 추가 | `k8s-manifest-resource-v1`, `dockerfile-file-v1`, `manifest-file-v1` | k8s 운영 / DevOps 검색 |
| **3** | Tier 3 fallback: shell + 미지원 확장자 | 동일 crate 에 모듈 추가 | `code-text-paragraph-v1` | 잡 텍스트 fallback |

**Phase 1A 가 1A-1 / 1A-2 로 쪼개진 이유**: 1A 가 들고 들어가는 *프레임워크 surface* (Citation variant, SearchHit 필드, filter 3종, skip 정책, config 절, IngestReport 세분화, 새 crate) 가 *언어 chunker 자체* 와 독립적으로 검증 가능. 1A-1 머지 후 기존 markdown corpus 가 *byte-level identical* 한 출력을 내는지 regression test 로 검증 — 코드 ingest 가 활성화되지 않은 상태에서 wire schema 변경 안전성을 별도 확인. 1A-2 는 Rust chunker 자체에만 집중, dogfooding 가능 지점 = 1A-2 머지.

**Binary version bump 트리거 정리**:
- **1A-1 머지**: bump 없음. wire 의 additive minor 변경 (CLAUDE.md "wire 의 additive minor 변경 은 backward-compat 이라 본 트리거에 해당 안 됨" 적용). 코드 ingest 미활성 — 사용자 도그푸드 surface 변경 없음.
- **1A-2 머지**: minor bump (예: `0.6` → `0.7`). 사용자 도그푸딩 가능 = bump 트리거.
- 이후 phase (1B/1C/1D/2/3) 의 bump 여부는 각 phase 의 task spec 에서 결정 — wire / flag 추가 없으면 patch bump.

---

## 3. 도메인 모델 영향

frozen design `docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` 의 §3 (도메인 모델) 과 §2 (wire schema) 를 *additive minor* 로 확장. breaking 변경 없음. 영향 받는 frozen design 섹션은 [§10 cascade](#10-변경-영향--cascade) 에 정리.

### 3.1 새 Citation variant: `code`

frozen design §2.1 의 5 variant (`line` / `page` / `region` / `caption` / `time`) 에 `code` 추가 → 총 6 variant.

```json
{
  "schema_version": "citation.v1",
  "kind": "code",
  "path": "kebab/crates/kebab-chunk/src/md_heading_v1.rs",
  "uri":  "kebab/crates/kebab-chunk/src/md_heading_v1.rs#L142-L168",

  "code": {
    "line_start": 142,
    "line_end": 168,
    "symbol": "MdHeadingV1Chunker::chunk_doc",
    "lang": "rust"
  }
}
```

`code.symbol` 은 nullable — Tier 1 AST chunk 면 채움, Tier 2/3 면 비움 (`null`).
`code.lang` 은 `--code-lang` filter 와 같은 식별자 (lowercase). null 가능.

기존 5 variant 와 마찬가지로 `path` + `uri` 는 항상 채움. `uri` 는 `path#L<start>-L<end>` (W3C Media Fragments) 그대로.

### 3.2 SearchHit 신규 optional 필드

frozen design §2.2 의 SearchHit 에 두 필드 추가, 모두 optional / nullable, additive minor:

```json
{
  "schema_version": "search_hit.v1",
  "rank": 1,
  "score": 0.78,
  "score_kind": "rrf",
  "chunk_id": "...",
  "doc_id": "...",
  "doc_path": "kebab/crates/kebab-chunk/src/md_heading_v1.rs",
  "heading_path": ["src", "md_heading_v1"],
  "section_label": "MdHeadingV1Chunker::chunk_doc",
  "snippet": "...",
  "citation": { "kind": "code", "...": "citation.v1" },

  "repo": "kebab",          // ← 신규 optional. .git/ walk-up 결과.
  "code_lang": "rust",      // ← 신규 optional. Tier 1/2/3 모두 채움 (Tier 2 의 yaml 등 포함).

  "retrieval": { "...": "..." },
  "index_version": "v1.0",
  "embedding_model": "multilingual-e5-large",
  "chunker_version": "code-rust-ast-v1"
}
```

기존 consumer (Claude Code skill 등) 는 두 필드 미인지 시 무시 — backwards-compat.

Markdown / PDF / 이미지 hit 는 두 필드 모두 null. 코드 hit 도 *repo 외부 single-file ingest* (`kebab ingest-file`) 인 경우 `repo` null 가능.

### 3.3 chunker_version 명명 (per-language)

frozen design §3.2 의 `chunker_version` 라벨 family 확장. **per-language 독립** — 언어 chunker 버그 픽스가 다른 언어 chunks 무효화 안 함.

```text
기존:
  md-heading-v1
  pdf-page-v1

Phase 1A 추가:
  code-rust-ast-v1

Phase 1B 추가:
  code-python-ast-v1
  code-ts-ast-v1
  code-js-ast-v1

Phase 1C 추가:
  code-go-ast-v1
  code-java-ast-v1
  code-kotlin-ast-v1

Phase 1D 추가:
  code-c-ast-v1
  code-cpp-ast-v1

Phase 2 추가:
  k8s-manifest-resource-v1
  dockerfile-file-v1
  manifest-file-v1

Phase 3 추가:
  code-text-paragraph-v1
```

cascade rule (frozen design §9):
- 한 언어 chunker 버그 픽스 → 해당 `code-{lang}-ast-vN` 만 bump → `embedding_records` 의 해당 chunk 만 invalidate → 다음 ingest 에서 해당 언어 파일만 reprocess.
- 공통 코드 (예: tree-sitter wrapper) 변경 → 영향 받는 모든 언어 chunker 동시 bump.

### 3.4 symbol path 포맷 (per-language convention)

`Citation.code.symbol` 의 값. 각 언어의 *self-reference 관습* 그대로.

| 언어 | 포맷 | 예시 |
|------|------|------|
| Rust | `mod::sub::fn_name`, `impl Type::method`, `Trait::method` | `chunk::md_heading_v1::MdHeadingV1Chunker::chunk_doc` |
| Python | `pkg.module.Class.method`, `pkg.module.func` | `kebab_eval.metrics.compute_mrr` |
| TS/JS | `module/Class.method`, `module/func`, `module/default` | `src/search/retriever/Retriever.search` |
| Go | `package.Func`, `package.(Receiver).Method` | `chunk.(*MdHeadingV1Chunker).ChunkDoc` |
| Java/Kotlin | `package.Class.method` | `com.kebab.chunk.MdHeadingV1Chunker.chunkDoc` |
| C | `func_name` | `parse_blocks` |
| C++ | `namespace::Class::method`, `namespace::func` | `kebab::chunk::MdHeadingV1Chunker::chunk_doc` |

**top-level scope** (top-level fn / struct / class 정의 외부의 code, 예: Rust `use` / Python `import` block) 는 `<top-level>` 로 표기. null 아님 — chunk 가 의미 단위 *없는* 영역임을 명시.

**module / namespace 만 있고 symbol 없는 경우** (예: Rust mod 선언만 모인 `lib.rs`): `<module>` 로 표기.

### 3.5 metadata 확장

frozen design §3.6 (Metadata / Provenance) 에 코드 ingest 시 채워지는 필드:

```rust
pub struct Metadata {
    // 기존 필드 ...
    pub lang: Option<String>,         // BCP-47 (자연어). 코드 파일은 보통 null. 코드 안의 주석 dominant lang detection 안 함.
    pub tags: Vec<String>,
    // ...

    // 신규 (코드 ingest)
    pub repo: Option<String>,         // .git/ walk-up 결과. dir 이름.
    pub git_branch: Option<String>,   // ingest 시점 HEAD branch.
    pub git_commit: Option<String>,   // ingest 시점 HEAD commit SHA (full 40 hex).
    pub code_lang: Option<String>,    // tree-sitter parser 이름과 매칭. lowercase.
}
```

`code_lang` 식별자 정규화 (이 spec 의 canonical 정의):
- Rust 파일 (`.rs`) → `rust`
- Python (`.py`, `.pyi`) → `python`
- TypeScript (`.ts`, `.tsx`) → `typescript`
- JavaScript (`.js`, `.jsx`, `.mjs`, `.cjs`) → `javascript`
- Go (`.go`) → `go`
- Java (`.java`) → `java`
- Kotlin (`.kt`, `.kts`) → `kotlin`
- C (`.c`, `.h`) → `c`
- C++ (`.cpp`, `.cc`, `.cxx`, `.hpp`, `.hh`, `.hxx`) → `cpp`
- YAML / k8s manifest (`.yaml`, `.yml`) → `yaml`
- Dockerfile (`Dockerfile`, `*.dockerfile`) → `dockerfile`
- TOML (`.toml`) → `toml`
- JSON (`.json`) → `json`
- Shell (`.sh`, `.bash`, `.zsh`) → `shell`
- Make (`Makefile`, `*.mk`) → `make`
- 미지원 / Tier 3 fallback → null

확장자 sniff 는 `kebab-parse-code` 의 단일 함수 `code_lang_for_path(path: &Path) -> Option<&'static str>` 에서 결정. 이 함수가 *유일한 source of truth*.

---

## 4. Wire schema v1 변경 (모두 additive minor)

### 4.1 변경 요약 표

| schema | 변경 | 영향 |
|--------|------|------|
| `citation.v1` | `kind = "code"` variant 추가 + `code: { line_start, line_end, symbol, lang }` 키 추가 | additive minor (기존 consumer 미인지 시 빠짐) |
| `search_hit.v1` | `repo`, `code_lang` 두 optional 필드 추가 | additive minor |
| `ingest_report.v1` | `skipped_generated`, `skipped_size_exceeded`, `skipped_builtin_blacklist`, `skipped_gitignore` 카운트 + `skip_examples` 추가 | additive minor |
| `schema.v1` | `media_breakdown` 에 `code` 카테고리 추가, 새 `code_lang_breakdown` 표 추가 | additive minor |
| `doctor.v1` | (변경 없음) | — |
| `answer.v1` | (Phase 1A 변경 없음. citation 객체가 code variant 일 수 있다는 점만 implicit) | — |
| `fetch_result.v1` | (변경 없음, kind=chunk / doc / span 그대로) | — |

### 4.2 JSON Schema 파일 수정 위치

```
docs/wire-schema/v1/
  citation.schema.json          ← code variant 추가
  search_hit.schema.json        ← repo / code_lang 추가
  ingest_report.schema.json     ← skip 카운트 + skip_examples 추가
  schema.schema.json            ← code_lang_breakdown 추가
```

각 schema 파일에 `"additionalProperties": false` 가 켜져 있으면 새 필드 정의 추가만으로 valid 가 안 됨 — 새 필드를 `properties` 에 명시하고 `required` 는 그대로 유지 (optional).

### 4.3 `--json` 출력 호환성 검증

Phase 1A 구현 시 기존 markdown corpus 의 hit / answer 가 *예전과 byte-level identical* 한 출력 내는지 단위 테스트 추가:

- `search_hit.v1` 의 `repo` / `code_lang` 필드는 markdown hit 에서 *output 에 등장하지 않음* (snake-case omit-null serialization).
- `ingest_report.v1` 의 새 카운트 필드는 코드 ingest 가 실행되지 않으면 `0` 으로 채워짐 (또는 omit-zero — task spec 단계에서 결정).
- `citation.v1` 의 `code` 키는 `kind != "code"` variant 에서 항상 absent.

---

## 5. Ingest 파이프라인 변경

### 5.1 Repo 자동 감지

```text
fn detect_repo(path: &Path) -> Option<RepoMeta> {
    // path 의 부모 디렉토리에서 위로 .git/ (dir) 만날 때까지 walk.
    // workspace.root 위로는 안 올라감 (boundary).
    // .git/ 가 file 인 경우 (worktree marker / submodule) → metadata.repo = None,
    //   metadata.git_branch / commit = None.
    // .git/ 가 dir 이면:
    //   - repo_name = .git/ 의 부모 dir 이름
    //   - branch = git symbolic-ref HEAD (없으면 detached HEAD → "detached")
    //   - commit = git rev-parse HEAD (40 hex 또는 None if empty repo)
}
```

`git` binary 호출 vs `gix` (gitoxide) library 사용 — task spec 에서 결정. 단 `git` binary 호출은 PATH 의존성 도입 (kebab 의 다른 곳엔 없음) → `gix` 선호.

repo 감지는 ingest 시 *파일당 한 번* 만 — repo 별 캐시 (in-memory HashMap) 로 같은 repo 의 두 번째 파일부터는 lookup hit.

### 5.2 ignore 통합 (`.gitignore` + `.kebabignore` + built-in)

**우선순위** (앞이 강함):
1. **Built-in safety net** — 항상 적용, 사용자 negate 가능 (`.kebabignore` 의 `!pattern`)
2. **`.gitignore`** — repo 의 `.gitignore` 자동 honor. nested `.gitignore` 도 적용 (디렉토리 단위 cascade).
3. **`.kebabignore`** — kebab 만의 추가 layer. workspace.root + 각 디렉토리 별 가능 (현재 동작 그대로).

**Built-in safety net (5 entries 만)**:
```text
**/node_modules/
**/target/
**/__pycache__/
**/.venv/
**/venv/
**/env/
```

`env/` 가 모호하지만 (사용자 자식 디렉토리가 우연히 "env" 일 수 있음) Python virtualenv 관습 강해서 포함. 사용자 override 는 `.kebabignore` 의 `!env/` 로.

**구현**:
- 기존 `kebab-source-fs` 의 `.kebabignore` 처리 코드를 확장.
- `ignore` crate (gitignore syntax) 그대로 사용. `.gitignore` + `.kebabignore` 를 같은 `Override` 빌더에 add — `ignore` crate 가 둘 다 표준으로 처리.
- built-in 은 hardcoded `WalkBuilder.add_custom_ignore_filename` 또는 코드 내 `OverrideBuilder` 로.

### 5.3 Generated / vendored skip 정책

**Generated header sniff** — `kebab-source-fs` 의 file scan 단계에서 *blake3 hash 계산 전* 에 실행 (incremental ingest 의 빠른 path 유지):

```text
fn is_generated_file(path: &Path) -> io::Result<bool> {
    let mut buf = [0u8; 512];
    let n = File::open(path)?.read(&mut buf)?;
    let head = std::str::from_utf8(&buf[..n]).unwrap_or("");

    // 줄 단위 markers — case-insensitive 매칭 (다양한 ecosystem 관습 수용).
    head.lines().take(10).any(|line| {
        let l = line.to_ascii_lowercase();
        l.contains("@generated") ||
        l.contains("code generated by") ||
        l.contains("do not edit") ||
        l.contains("do not modify") ||
        l.contains("automatically generated") ||
        l.contains("auto-generated") ||
        l.contains("autogenerated")
    })
}
```

비용: 파일당 1 read syscall (≤512 byte). 이미 `.gitignore` / built-in 으로 빠진 파일은 이 단계 도달 안 함.

**Skip 시 IngestReport 에 sample 등록** — 디버깅 용 (사용자 "왜 X 파일이 색인 안 됐지?" 시 즉시 답):
```json
{
  "skip_examples": {
    "generated": [
      "kebab/crates/proto/src/api.pb.rs",
      "..."
    ],
    "size_exceeded": [
      "vendor/data/large-fixture.json"
    ],
    "builtin_blacklist": ["..."],
    "gitignore": ["..."]
  }
}
```
각 카테고리당 처음 5건만. CLI text 모드에서는 카운트만 표시, `--json` 이면 위 schema 그대로.

### 5.4 Size cap

```text
[ingest.code]
max_file_bytes = 262144         # 256 KiB
max_file_lines = 5000           # 둘 중 먼저 hit
```

- byte cap 은 `fs::metadata().len()` 한 번 — 매우 빠름.
- line cap 은 byte cap 통과 후 streaming read 로 5000 line 까지 count, 초과 시 skip.
- 둘 다 `IngestReport.skipped_size_exceeded` 로 카운트, `skip_examples.size_exceeded` 에 sample.

기본값 근거:
- 256 KiB → 보통 코드 파일 (Rust fn, Python class) 의 100배 이상. minified JS / 대용량 fixture / generated client 의 일반적 사이즈 (수 MB) 는 차단.
- 5000 line → 한 파일이 한 사람이 이해할 수 있는 한계 근처. 그 이상은 보통 generated.

사용자 override:
```toml
[ingest.code]
max_file_bytes = 1048576    # 1 MiB 로 풀고 싶을 때
max_file_lines = 20000
```

### 5.5 IngestReport 세분화

기존 `skipped_by_extension` 옆에 추가:

```json
{
  "schema_version": "ingest_report.v1",
  "indexed": 1234,
  "unchanged": 5678,
  "updated": 12,
  "deleted": 3,
  "skipped_by_extension": 45,
  "skipped_gitignore": 2104,
  "skipped_kebabignore": 8,
  "skipped_builtin_blacklist": 567,
  "skipped_generated": 89,
  "skipped_size_exceeded": 4,
  "skip_examples": {
    "generated": ["..."],
    "size_exceeded": ["..."],
    "builtin_blacklist": ["..."],
    "gitignore": ["..."]
  },
  "warnings": [],
  "duration_ms": 12345
}
```

`skipped_by_extension` 은 *지원 안 되는 확장자* — 코드 ingest 후로는 Tier 3 fallback (`code-text-paragraph-v1`) 이 잡아내는 폭이 넓어져서 비율이 줄 것. Tier 3 도 못 잡는 binary 등이 남음.

human 출력 (TTY) 에서는 한 줄 요약:
```text
✓ indexed 1234 chunks (unchanged 5678, updated 12, deleted 3)
  skipped: 2104 .gitignore, 567 built-in, 89 generated, 45 unsupported, 8 .kebabignore, 4 too-large
  duration: 12.3s
```

---

## 6. Crate 구조

### 6.1 새 crate `kebab-parse-code`

```text
crates/kebab-parse-code/
├── Cargo.toml
└── src/
    ├── lib.rs                  # 공통 entry, dispatch by extension
    ├── lang.rs                 # code_lang_for_path(), 식별자 정규화
    ├── repo.rs                 # detect_repo() — gix wrapper
    ├── skip.rs                 # generated header sniff, size cap
    ├── rust.rs                 # tree-sitter-rust → CanonicalDocument (Phase 1A)
    ├── python.rs               # tree-sitter-python → ...    (Phase 1B)
    ├── typescript.rs           # ...                          (Phase 1B)
    ├── javascript.rs           # ...                          (Phase 1B)
    ├── go.rs                   # ...                          (Phase 1C)
    ├── java.rs                 # ...                          (Phase 1C)
    ├── kotlin.rs               # ...                          (Phase 1C)
    ├── c.rs                    # ...                          (Phase 1D)
    ├── cpp.rs                  # ...                          (Phase 1D)
    ├── yaml_k8s.rs             # k8s manifest resource-aware  (Phase 2)
    ├── dockerfile.rs           # ...                          (Phase 2)
    ├── manifest.rs             # Cargo.toml / package.json 1-chunk (Phase 2)
    └── text_paragraph.rs       # Tier 3 fallback              (Phase 3)
```

**의존성**:
- 각 phase 별로 `tree-sitter-*` dep 추가. Phase 1A 는 `tree-sitter-rust` + `tree-sitter` (core) 만.
- `gix` (gitoxide) — Phase 1A 부터.
- `kebab-core`, `kebab-parse-types` (CanonicalDocument / Block / SourceSpan).

**의존성 제약** (frozen design §8 inheritance):
- `kebab-parse-code` 는 다른 `kebab-parse-*` 크레이트와 동일한 격리 규칙 — store / embed / llm / rag 직접 import 금지.
- UI crate (`kebab-cli` / `kebab-tui` / `kebab-mcp`) 는 이 crate 직접 import 금지. `kebab-app` facade 통해서만.

### 6.2 `kebab-chunk` 모듈 확장

```text
crates/kebab-chunk/src/
├── lib.rs                          # export 추가 (per phase 누적)
├── md_heading_v1.rs                # 기존
├── pdf_page_v1.rs                  # 기존
├── code_rust_ast_v1.rs             # Phase 1A
├── code_python_ast_v1.rs           # Phase 1B
├── code_ts_ast_v1.rs               # ...
├── code_js_ast_v1.rs               # ...
├── code_go_ast_v1.rs               # Phase 1C
├── code_java_ast_v1.rs             # ...
├── code_kotlin_ast_v1.rs           # ...
├── code_c_ast_v1.rs                # Phase 1D
├── code_cpp_ast_v1.rs              # ...
├── k8s_manifest_resource_v1.rs     # Phase 2
├── dockerfile_file_v1.rs           # ...
├── manifest_file_v1.rs             # ...
└── code_text_paragraph_v1.rs       # Phase 3
```

각 모듈 = 한 chunker 구현체 + `pub use` 로 lib 에 노출. 기존 패턴 (md_heading_v1 / pdf_page_v1) 그대로.

**Chunker trait 변경 없음** — 기존 `Chunker` trait (frozen §7.2) 가 `CanonicalDocument → Vec<Chunk>` 시그니처라 코드도 같은 trait 로 동작.

### 6.3 의존성 그래프 변경

```text
기존:
  kebab-app → kebab-parse-md, kebab-parse-pdf, kebab-parse-image
            → kebab-chunk
            → ...

추가 (Phase 1A):
  kebab-app → kebab-parse-code (신규)
            → kebab-chunk (모듈 추가)
```

추가 의존성:
- `kebab-app → kebab-parse-code`
- `kebab-parse-code → tree-sitter`, `tree-sitter-rust`, `gix`
- 빌드 영향: `kebab-parse-code` 추가 → workspace `cargo test -p` 단위 한 개 추가. `-j 1` 정책 (frozen CLAUDE.md) 그대로 적용.

### 6.4 `target/` 디스크 영향

frozen CLAUDE.md 에 "target/ 가 90 GB+ 까지 balloon" 경고 있음. 이 spec 으로 22 → 새 모듈들 추가 시 *integration test* 마다 새 binary linkage 추가 → 더 부풀어. **각 phase 머지 후 `cargo clean` 강제 권장** — CLAUDE.md 의 기존 rule 그대로 적용, phase 끝마다 명시.

---

## 7. Search / RAG 표면

### 7.1 새 search filter

`kebab search` 의 기존 filter (`--tag` / `--lang` / `--path-glob` / `--media` / `--ingested-after` / `--trust-min` / `--doc-id`) 에 세 종 추가:

```text
--media code                       # umbrella — 모든 code Tier 의 chunk
--code-lang <list>                 # 반복 / comma — rust,python 식. OR 매칭.
--repo <name>                      # 반복 가능. OR 매칭.
```

기존 정책 일관:
- 반복 가능 flag 는 OR 매칭 (`--repo kebab --repo other`).
- `--code-lang rs` 같은 alias 는 미지원 — *full identifier* (`rust`) 만. 일관성 위해.
- 모르는 `--code-lang` 값 → empty hits (`--media` 와 동일 정책).
- filter flags 간은 AND (`--media code --code-lang rust` → 코드이면서 Rust).

### 7.2 `kebab schema` stats 확장

frozen design §2.5 / p9-fb-37 의 `stats.media_breakdown` 에 `code` 카테고리 추가:

```json
{
  "schema_version": "schema.v1",
  "stats": {
    "media_breakdown": {
      "markdown": 1234,
      "pdf": 56,
      "image": 78,
      "audio": 0,
      "code": 4567,       // ← 신규
      "other": 12
    },
    "lang_breakdown": {       // 기존 — 자연어
      "ko": 1100,
      "en": 234,
      "null": 134
    },
    "code_lang_breakdown": {  // ← 신규 — 프로그래밍 언어 (chunk 수)
      "rust": 2345,
      "python": 1234,
      "typescript": 567,
      "yaml": 89,
      "go": 332
    },
    "repo_breakdown": {       // ← 신규 — repo 별 chunk 수
      "kebab": 1234,
      "internal-api": 567,
      "...": "..."
    },
    "index_bytes": 1234567890,
    "stale_doc_count": 12
  }
}
```

`repo_breakdown` 도 추가하기로 — 사용자가 "어느 repo 가 가장 많이 색인 됐지?" 확인 가능.

### 7.3 RAG prompt (Phase 1A 는 `rag-v2` 그대로)

Phase 1A 에서는 코드 chunk 가 *일반 도큐먼트* 로 prompt 에 들어감:

```text
[#1] (code: kebab::chunk::md_heading_v1::MdHeadingV1Chunker::chunk_doc)
fn chunk_doc(&self, doc: &CanonicalDocument) -> Result<Vec<Chunk>> {
    ...
}

[#2] (code: kebab::chunk::pdf_page_v1::PdfPageV1Chunker::chunk_doc)
...
```

prompt 의 source identifier 가 *file path + symbol* 둘 다 들어가게 — symbol 이 있으면 *symbol* 을 우선 표시, 없으면 file path.

`rag-v2` 의 기존 규칙 ("fact 인용 시 [#번호] 앞에 chunk 속 원문 큰따옴표") 은 코드에서 좀 어색할 수 있음 (코드의 큰따옴표는 string literal). 측정 후 어색하면 Phase 2+ 에서 `rag-v3` (code-aware) 도입.

### 7.4 `kebab inspect` / `kebab fetch` 영향

기존 `kebab inspect chunk <id>` 출력에서 `Citation::Code` variant 의 `symbol` / `code_lang` 표시. text 모드 출력 변경:

```text
chunk_id:        abc123...
doc_path:        kebab/crates/kebab-chunk/src/md_heading_v1.rs
line range:      L142-L168
symbol:          MdHeadingV1Chunker::chunk_doc          ← 신규 (code variant 에서만)
code_lang:       rust                                   ← 신규
repo:            kebab                                  ← 신규
chunker_version: code-rust-ast-v1
```

`kebab fetch chunk` / `kebab fetch span` 은 변경 없음 — 본문 byte 그대로 반환.

---

## 8. Config 변경

### 8.1 신규 `[ingest.code]` 절

```toml
[ingest.code]
# Generated header sniff 활성화. 첫 ~500 byte 의 6 markers 중 하나 발견 시 skip.
skip_generated_header = true

# 파일당 max byte (bytes). 초과 시 skip.
max_file_bytes = 262144     # 256 KiB

# 파일당 max line. 초과 시 skip. byte cap 통과 후 검사.
max_file_lines = 5000

# 사용자 추가 skip 패턴. gitignore 문법. built-in / .gitignore / .kebabignore 외 추가.
extra_skip_globs = []

# AST chunk 가 너무 길 때 fallback line-window 적용 임계.
# 단일 fn / class 가 이 라인 수 넘으면 paragraph fallback 적용.
ast_chunk_max_lines = 200    # 단일 chunk 최대 라인

# Tier 3 fallback (paragraph + line-window) 시 line-window 사이즈.
fallback_lines_per_chunk = 80
fallback_lines_overlap = 20
```

기본값 근거:
- `skip_generated_header = true` — 안전 default. 미스 케이스 (사용자가 generated 도 색인 원함) 는 명시적 false.
- `max_file_bytes = 262144` — minified JS / 대용량 generated 차단 충분.
- `max_file_lines = 5000` — 한 사람이 한 번에 이해할 수 있는 코드 한계 근처.
- `ast_chunk_max_lines = 200` — 사람 인지 한계 + retrieval token budget.
- `fallback_lines_per_chunk = 80`, `overlap = 20` — RAG 컨벤션의 보수적 default.

### 8.2 기본값 + override 정책

- 모든 키 optional. 누락 시 위 default.
- `KEBAB_*` env override 안 지원 (이건 dev / debug 가 아닌 정책 설정).
- `--config <path>` 로 격리 테스트 가능 (XDG 의존 안 함).

### 8.3 `config.toml` 의 기존 `[workspace]` 절 영향

변경 없음. `workspace.root`, `exclude` 그대로. `.gitignore` / `.kebabignore` honor 정책은 *기본 동작* 으로 config 키 없이 active — 사용자가 끄고 싶으면 `.kebabignore` 의 `!pattern` 으로 override.

---

## 9. Tier 별 chunker 상세

### 9.1 Tier 1 — AST per-language

**입력**: `CanonicalDocument` with `Block::Code { lang: Some("rust"), code: "..." }`.
**출력**: `Vec<Chunk>` — 각 chunk 가 AST 의 *top-level 의미 단위* 또는 fallback unit.

**Rust 예시 (Phase 1A)**:

tree-sitter 의미 단위:
- `function_item` → 1 chunk
- `impl_item` 의 각 `function_item` → 1 chunk per method
- `struct_item` / `enum_item` / `trait_item` → 1 chunk (선언 + doc comment)
- `mod_item` 의 *내용물* → 재귀 분해
- top-level `use` / `extern crate` / `const` / `static` block → 한 chunk 로 모음 (`<top-level>` symbol)

**ast_chunk_max_lines 초과 시 fallback**:
- 단일 fn 이 200 line 넘으면 paragraph (blank-line) 기반으로 split.
- 각 sub-chunk 의 symbol 은 `function_name [part 1/N]` 식으로 표기.
- 이 동작은 `kebab-chunk/src/code_rust_ast_v1.rs` 내부에서.

**citation 의 line range**: tree-sitter node 의 `start_position.row` / `end_position.row` (0-indexed → +1 로 1-based).

**예시 input → output**:
```rust
// src/lib.rs
pub fn parse(input: &str) -> Result<Doc> {
    // 50 lines
}

impl Chunker for Foo {
    fn chunk_doc(&self, doc: &Doc) -> Vec<Chunk> {
        // 80 lines
    }

    fn name(&self) -> &str { "foo-v1" }
}
```

→ chunks:
1. `parse`, lines 1-50, symbol = `parse`
2. `Foo::chunk_doc`, lines 53-132, symbol = `Foo::chunk_doc` (impl 의 method)
3. `Foo::name`, lines 134-134, symbol = `Foo::name`

### 9.2 Tier 2 — resource-aware

**k8s-manifest-resource-v1**:
- YAML multi-document split (`---` separator).
- 각 document 마다 1 chunk.
- chunk metadata: `kind: Deployment`, `apiVersion`, `metadata.name`, `metadata.namespace`.
- citation 의 `symbol` 필드: `<kind>/<namespace>/<name>` (e.g., `Deployment/prod/api-server`). namespace 없으면 `<kind>/<name>`.
- yaml 파싱 실패 (invalid YAML, 또는 k8s schema 가 아닌 일반 yaml) 시: `code-text-paragraph-v1` 로 fallback 처리.

**dockerfile-file-v1**:
- Dockerfile 전체 = 1 chunk.
- symbol = `<dockerfile>`.
- citation 의 line range = 1 ~ EOF.
- ARG / FROM / RUN / COPY / CMD 등은 chunk 내부 plain text 로 보존.

**manifest-file-v1** (Cargo.toml, package.json, pyproject.toml, go.mod, pom.xml, build.gradle, tsconfig.json 등):
- 파일 통째로 1 chunk.
- symbol = `<manifest>`.
- citation 의 line range = 1 ~ EOF.

### 9.3 Tier 3 — paragraph + line-window fallback

**code-text-paragraph-v1** — shell script, 미지원 확장자, AST 실패 시 fallback:
- 빈 줄 (blank line) 기준으로 paragraph 분할.
- paragraph 가 `fallback_lines_per_chunk` (default 80) 넘으면 line-window split with `fallback_lines_overlap` (default 20).
- symbol 은 null. citation 은 `Citation::Code { symbol: None, lang: Some("shell") }` 또는 lang 미지정.

---

## 10. 변경 영향 / cascade

### 10.1 Frozen design doc 갱신 (이 spec 머지와 동시)

`docs/superpowers/specs/2026-04-27-kebab-final-form-design.md` 의 다음 섹션 갱신:

| 섹션 | 갱신 내용 |
|------|-----------|
| §0 동결된 결정 요약 | "코드 ingest 추가" 1줄. cross-link to 2026-05-15 spec |
| §2.1 Citation | 5 → 6 variants, `code` 추가 |
| §2.2 SearchHit | `repo`, `code_lang` optional 필드 |
| §2.4 IngestReport | skip 카운트 4종 + skip_examples |
| §2 schema.v1 (fb-37 추가분) | `code` media + `code_lang_breakdown` + `repo_breakdown` |
| §3.2 Versions / labels | chunker_version family 확장 (per-language pattern) |
| §3.6 Metadata | `repo`, `git_branch`, `git_commit`, `code_lang` 필드 |
| §8 모듈 경계 | `kebab-parse-code` 추가 + 의존성 규칙 inheritance |
| §11 동결 범위 | "code ingest" 가 더 이상 비-스코프 아님 명시. 단 multi-workspace / watch / history aware 는 그대로 비-스코프 |

### 10.2 cascade rule (frozen §9) 영향

- `parser_version` cascade: 각 phase 의 새 parser version (`code-rust-parse-v1` 등) 추가. 기존 markdown / pdf 무영향.
- `chunker_version` cascade: per-language 라벨 → 한 언어 chunker 변경이 다른 언어 chunks 무효화 안 함.
- `embedding_version` cascade: 변경 없음 (`multilingual-e5-large` 그대로).
- `prompt_template_version` cascade: Phase 1A 는 `rag-v2` 그대로 → 무영향.
- `index_version` cascade: SQLite DDL 변경 없으면 무영향. metadata.repo / git_branch / git_commit 필드는 *Metadata* 의 JSON blob 안에 추가 — DDL 변경 안 필요 (frozen §5 의 `documents.metadata_json TEXT` 가 free-form).

### 10.3 V00X migration?

SQLite DDL 변경 없음 → V00X migration 불요. `documents.metadata_json` 의 free-form 내부에 새 키 (`repo`, `git_branch`, `git_commit`, `code_lang`) 가 들어감. 기존 markdown / pdf chunk 들의 metadata_json 은 그대로.

### 10.4 Binary version bump

[§2 Phase 분할 표 하단 "Binary version bump 트리거 정리"](#2-phase-분할--마일스톤) 참조. 요지:
- **1A-1 머지** → bump 없음 (wire additive minor + 사용자 surface 변경 없음).
- **1A-2 머지** → minor bump (`0.6` → `0.7`, 사용자 도그푸딩 시작).
- 이후 phase 는 각 task spec 에서 결정 (wire / flag 추가 없으면 patch bump).

---

## 11. Open questions (Phase 1A task spec 단계에서 픽스)

이 spec 은 *프레임워크* 까지만 동결. 다음 항목은 Phase 1A 의 task spec 작성 시 결정:

1. **AST chunk 의 minimum size** — 5-line fn 도 한 chunk? 또는 minimum threshold (예: ≥ 10 line) 미만은 인접 fn 과 merge? *영향*: chunk 수 폭증 vs retrieval miss.

2. **doc_id 충돌 위험** — `Cargo.toml` 두 repo 의 content 가 우연히 동일 → blake3 hash 동일 → 같은 doc? frozen §4.2 의 ID recipe 확인 필요. *영향*: 한 doc 이 두 repo 에서 출처 표시. 해결: doc_id recipe 에 repo / path 포함 여부 확인.

3. **`--code-lang` 식별자 정규화 (canonical)** — `rust` / `python` / `typescript` 의 풀네임만 vs `rs` / `py` / `ts` 짧은 alias 도 허용? 이 spec 은 풀네임만 — task spec 에서 alias 매핑 명시.

4. **TUI surface 변경 시점** — Phase 1A 에 포함 vs 별도 Phase 4 (TUI code rendering)? *영향*: TUI 의 Library/Inspect 패널에서 code citation 의 symbol/lang/repo 렌더. 일단 Phase 1A 에 *최소 변경* (citation 표시) 만 포함, 별도 인터랙션 (예: `g` 키로 LSP 식 navigation) 은 P+.

5. **AST chunk symbol path 의 *depth 한계*** — Rust 의 nested impl / nested mod 가 깊으면 `outer::inner::deepest::method` 식 path 가 길어짐. 60 char cap + 중간 생략 (`outer::…::method`)? Phase 1A 의 task spec 에서 cap 정책 결정.

6. **`gix` 의 binary size 영향** — `kebab-parse-code` → `gix` dep 도입이 release binary 크기에 얼마나 영향? `git2` (libgit2) 는 C dep 이라 안 쓰기로 — `gix` 가 pure rust. binary size 영향 측정 후 결정.

7. **k8s manifest 의 `kind` 인식 범위** — `Deployment` / `Service` / `ConfigMap` 등 표준 외 *CRD* (custom resource) 처리? Phase 2 의 task spec 에서 결정. 일단 *모든 yaml document 의 `kind` 필드 그대로* 사용 (CRD 포함 자동 처리).

---

## 12. 다음 단계

1. **이 spec 의 사용자 검토** — 빠진 결정 / 모순 / 추가 우려 확인.
2. 검토 통과 시 `tasks/p10/` 디렉토리 신설 + `tasks/p10/INDEX.md` 추가 + `tasks/INDEX.md` 에 phase 10 entry.
3. **Phase 1A-1 task spec 작성** (먼저) — `tasks/p10/p10-1a-1-code-ingest-framework.md`. `contract_sections` 로 `[§2.1, §2.2, §2.4, §2 schema.v1, §3.6, §8, §11]` (chunker 추가 없음 — §3.2 chunker_version 갱신은 1A-2 와 함께).
4. **Phase 1A-2 task spec 작성** (1A-1 머지 후) — `tasks/p10/p10-1a-2-rust-ast-chunker.md`. `contract_sections` 로 `[§2.1 (code variant 실 사용), §3.2 (code-rust-ast-v1 추가), §3.4 (Rust symbol path)]`.
5. Frozen design doc (2026-04-27) 갱신을 *Phase 1A-1 PR* 에 동봉 (이 spec 의 §10.1 표 그대로, 단 §3.2 chunker_version 부분은 1A-2 에서).
6. writing-plans skill 로 Phase 1A-1 의 구현 계획 (작업 단위) 작성.
7. Phase 1A-1 머지 후 regression test 통과 확인 → Phase 1A-2 구현 계획 작성 → 머지 → kebab 자기 자신 dogfooding → 측정 → 다음 phase 진행 결정.

---

## 부록 A — 의사 결정 회의록 (이 spec 작성 시 사용자와의 brainstorming 요약)

이 spec 작성에 들어간 결정들의 *왜* 를 짧게 (감사용 / 미래 재고 시 참조):

- **시나리오**: "한 부모 dir 아래 수십 개 repo + 의미 검색 + RAG" — kebab 의 cross-corpus 가치를 코드까지 확장.
- **chunking 전략**: 사용자가 길 B (AST per-language) 명시 선택. 작성자 추천은 길 C (A 로 시작 측정 후 승급) 였으나 사용자 결정 존중.
- **언어 범위**: 사용자 초기 답 (Rust/Python/TS-JS/Go-Java-Kotlin/C/C++/Shell/Dockerfile/yaml) 을 Tier 1/2/3 으로 재분류 → AST 가 의미 있는 곳에만 AST 적용. 작성자 push back 결과.
- **embedding 모델**: e5-large 유지. cross-corpus 가치 + cascade 비용 회피.
- **Citation variant**: 사용자가 `(a)-2` (새 `code` variant) 선택. 작성자 추천은 `(a)-1` (line variant 재사용) 였으나 의미 분리 명확함이 결정 요인.
- **built-in blacklist**: 사용자가 *축소* 요청 → 5 entry 최종. `.gitignore` 가 source of truth, built-in 은 safety net 만.
- **Phase 분할**: 사용자가 "되도록 많은 디테일 spec → Phase 1A 부터 구현" 명시. 이 spec 이 그 프레임워크 동결, phase 별 구현은 별도 task spec.
