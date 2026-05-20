# p10-2 Tier 2 Resource-Aware Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Activate Tier 2 resource-aware chunkers (k8s manifest + Dockerfile + 7-file manifest set) in a single PR. AST 가 아닌 file/document-level chunking. 머지 시점부터 `.yaml` / `Dockerfile` / 매니페스트 7종 dogfooding 가능.

**Architecture:** 3 self-contained chunker 모듈을 `kebab-chunk` 에 추가. `kebab-parse-code` 의 lang.rs 만 갱신 (Tier 2 = AST 없음). `kebab-source-fs/src/media.rs` 의 inline 확장자 match 를 `code_lang_for_path` 호출로 통일 (1A-1 부터 누적된 duplication 정리). `ingest_one_code_asset` 의 match 가 Tier 2 lang 7종 (`yaml` / `dockerfile` / `toml` / `json` / `xml` / `groovy` / `go-mod`) 을 새 chunker 로 라우팅. parser_version = `"none-v1"` 통일.

**Tech Stack:** Rust 2024 workspace, `serde_yaml = "0.9"` (이미 workspace.dependencies). 1A-2 / 1B / 1C 인프라 변경 없음.

**Memory note:** Host has been OOM'd previously (재부팅 사례 있음). Per-crate cargo only. ONE full-suite + clippy invocation in Task J. NO `cargo test --workspace` outside that gate.

---

## Pre-flight

Branch `feat/p10-2-tier2-resource` 이미 존재 (spec commit 47857b2 포함).

- [ ] **Disk hygiene**: `df -h /` 점검. 90% 넘으면 `cargo clean` (last cleanup recovered 38.7 GB).

Reference files:
- 1A-2 chunker: `crates/kebab-chunk/src/code_rust_ast_v1.rs` — `AST_CHUNK_MAX_LINES = 200` / `POLICY_HASH_HEX_LEN = 16` / `BYTES_PER_TOKEN = 3` 상수 + `Document → Vec<Chunk>` 패턴.
- 1C-JK dispatch generalization: `crates/kebab-app/src/lib.rs::ingest_one_code_asset` (~L1794). 현재 7-arm match (rust|python|typescript|javascript|go|java|kotlin). Tier 2 분기 추가 자리.
- 1A-1 code_lang_for_path: `crates/kebab-parse-code/src/lang.rs`. basename 우선 매칭 패턴 신설.
- 1A-1 media.rs: `crates/kebab-source-fs/src/media.rs`. inline `match extension` duplication.
- spec: `tasks/p10/p10-2-tier2-resource-aware.md`.

---

## Task A: kebab-chunk 에 serde_yaml dep 추가

**Files:**
- Modify: `crates/kebab-chunk/Cargo.toml` (dependencies 절)

- [ ] **Step 1**: `crates/kebab-chunk/Cargo.toml` 의 `[dependencies]` 절에 추가 (serde_json 다음 줄):

```toml
serde_yaml = { workspace = true }
```

- [ ] **Step 2**: `cargo build -p kebab-chunk` → clean (unused dep warning 무시 — Task D 에서 사용).

- [ ] **Step 3**: Commit:

```bash
git add crates/kebab-chunk/Cargo.toml
git commit -m "$(cat <<'EOF'
build(p10-2): add serde_yaml dep to kebab-chunk for k8s-manifest-resource-v1

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task B: lang.rs — basename + 확장자 추가 + Tier 2 매핑

**Files:**
- Modify: `crates/kebab-parse-code/src/lang.rs`
- Test: same file's test module

- [ ] **Step 1 (failing test)**: `lang.rs` 의 `#[cfg(test)] mod tests` 에 추가 (기존 테스트 옆):

```rust
#[test]
fn tier2_basename_takes_precedence_over_extension() {
    assert_eq!(code_lang_for_path(Path::new("Dockerfile")),         Some("dockerfile"));
    assert_eq!(code_lang_for_path(Path::new("foo/Dockerfile.dev")), Some("dockerfile"));
    assert_eq!(code_lang_for_path(Path::new("myapp.dockerfile")),   Some("dockerfile"));
    assert_eq!(code_lang_for_path(Path::new("repo/Cargo.toml")),    Some("toml"));
    assert_eq!(code_lang_for_path(Path::new("pyproject.toml")),     Some("toml"));
    assert_eq!(code_lang_for_path(Path::new("repo/package.json")),  Some("json"));
    assert_eq!(code_lang_for_path(Path::new("tsconfig.json")),      Some("json"));
    assert_eq!(code_lang_for_path(Path::new("go.mod")),             Some("go-mod"));
    assert_eq!(code_lang_for_path(Path::new("pom.xml")),            Some("xml"));
    assert_eq!(code_lang_for_path(Path::new("build.gradle")),       Some("groovy"));
}

#[test]
fn tier2_extension_fallback() {
    assert_eq!(code_lang_for_path(Path::new("k8s/deploy.yaml")),    Some("yaml"));
    assert_eq!(code_lang_for_path(Path::new("k8s/deploy.yml")),     Some("yaml"));
    assert_eq!(code_lang_for_path(Path::new("foo/bar.toml")),       Some("toml"));
    assert_eq!(code_lang_for_path(Path::new("foo/bar.json")),       Some("json"));
    assert_eq!(code_lang_for_path(Path::new("foo/bar.xml")),        Some("xml"));
    assert_eq!(code_lang_for_path(Path::new("foo/bar.gradle")),     Some("groovy"));
}
```

- [ ] **Step 2**: Run → FAIL.

```bash
cargo test -p kebab-parse-code lang::tests::tier2 -- --nocapture
```

Expected: function returns `None` for all new inputs.

- [ ] **Step 3 (impl)**: `code_lang_for_path` 본문을 다음 형태로 갱신 (기존 확장자 매칭은 유지하고 basename 분기를 *맨 앞* 으로):

```rust
pub fn code_lang_for_path(path: &Path) -> Option<&'static str> {
    // p10-2: basename takes precedence over extension (Dockerfile, Cargo.toml, …).
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    match file_name {
        "Dockerfile" | "Cargo.toml" | "pyproject.toml" | "package.json"
            | "tsconfig.json" | "go.mod" | "pom.xml" | "build.gradle" => {
            return Some(match file_name {
                "Dockerfile"                          => "dockerfile",
                "Cargo.toml" | "pyproject.toml"       => "toml",
                "package.json" | "tsconfig.json"      => "json",
                "go.mod"                              => "go-mod",
                "pom.xml"                             => "xml",
                "build.gradle"                        => "groovy",
                _ => unreachable!(),
            });
        }
        _ => {}
    }
    // Dockerfile.* prefix variant (Dockerfile.dev, Dockerfile.prod, …).
    if let Some(rest) = file_name.strip_prefix("Dockerfile.") {
        if !rest.is_empty() {
            return Some("dockerfile");
        }
    }

    // Extension fallback.
    let ext = path.extension().and_then(|e| e.to_str())?;
    let lang = match ext {
        "rs"                                   => "rust",
        "py" | "pyi"                           => "python",
        "ts" | "tsx" | "mts" | "cts"           => "typescript",
        "js" | "mjs" | "cjs" | "jsx"           => "javascript",
        "go"                                   => "go",
        "java"                                 => "java",
        "kt" | "kts"                           => "kotlin",
        // p10-2: Tier 2 extensions.
        "yaml" | "yml"                         => "yaml",
        "dockerfile"                           => "dockerfile",
        "toml"                                 => "toml",
        "json"                                 => "json",
        "xml"                                  => "xml",
        "gradle"                               => "groovy",
        _ => return None,
    };
    Some(lang)
}
```

(기존 함수의 확장자 절 그대로 보존하고 위 7줄만 추가. 기존 코드 형식이 다르면 그 형식 유지 + Tier 2 라인만 추가.)

- [ ] **Step 4**: Run → PASS.

```bash
cargo test -p kebab-parse-code lang::tests -- --nocapture
```

Expected: 모든 lang::tests 통과.

- [ ] **Step 5**: Clippy + commit:

```bash
cargo clippy -p kebab-parse-code --all-targets -- -D warnings
git add crates/kebab-parse-code/src/lang.rs
git commit -m "$(cat <<'EOF'
feat(p10-2): extend code_lang_for_path with Tier 2 basenames + extensions

Adds basename-first matching for Dockerfile / Cargo.toml / pyproject.toml /
package.json / tsconfig.json / go.mod / pom.xml / build.gradle plus
Dockerfile.* prefix variant. Extension fallback adds .yaml/.yml/.dockerfile/
.toml/.json/.xml/.gradle → yaml/dockerfile/toml/json/xml/groovy.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task C: media.rs — code_lang_for_path 호출로 inline match 통일

**Files:**
- Modify: `crates/kebab-source-fs/src/media.rs`

design §3.5 의 "code_lang_for_path 가 *유일한 source of truth*" 룰 적용. 1A-1 부터 누적된 duplication 정리.

- [ ] **Step 1**: 현재 `media_type_for` 함수의 `match extension` 의 code 매칭 절을 모두 한 줄로 교체:

```rust
pub fn media_type_for(path: &Path) -> MediaType {
    // p10-2: code_lang_for_path is the single source of truth for code lang.
    if let Some(lang) = kebab_parse_code::code_lang_for_path(path) {
        return MediaType::Code(lang.to_string());
    }

    // 기존 비-code 확장자 매칭 (markdown / pdf / images / etc.) 은 그대로 유지.
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        // ... 기존 비-code 절 그대로 ...
        "md" | "markdown" => MediaType::Markdown,
        // (기존 코드 그대로 — code lang 절 만 삭제)
        _ => MediaType::Other(ext.to_string()),
    }
}
```

(전체 함수 본문은 현재 파일을 Read 한 후 code 절만 삭제하고 위 if-let 을 함수 맨 앞에 추가.)

- [ ] **Step 2**: 기존 media.rs 의 test 모듈 보존 — `code_files_map_to_media_code`, `go_files_map_to_media_code_go`, `java_kotlin_files_map_to_media_code` 등 모두 통과해야 함. 추가 Tier 2 테스트:

```rust
#[test]
fn tier2_files_map_to_media_code() {
    assert_eq!(media_type_for(Path::new("a/deploy.yaml")), MediaType::Code("yaml".into()));
    assert_eq!(media_type_for(Path::new("a/Dockerfile")), MediaType::Code("dockerfile".into()));
    assert_eq!(media_type_for(Path::new("a/Cargo.toml")), MediaType::Code("toml".into()));
    assert_eq!(media_type_for(Path::new("a/pom.xml")), MediaType::Code("xml".into()));
    assert_eq!(media_type_for(Path::new("a/build.gradle")), MediaType::Code("groovy".into()));
    assert_eq!(media_type_for(Path::new("a/go.mod")), MediaType::Code("go-mod".into()));
}
```

- [ ] **Step 3**: `cargo test -p kebab-source-fs` → 기존 + 신규 테스트 모두 PASS. 만약 비-code 확장자 (md/pdf/etc.) 매칭이 깨졌으면 Step 1 의 비-code 절 보존 누락 — 다시 확인.

- [ ] **Step 4**: Clippy + commit:

```bash
cargo clippy -p kebab-source-fs --all-targets -- -D warnings
git add crates/kebab-source-fs/src/media.rs
git commit -m "$(cat <<'EOF'
refactor(p10-2): media.rs delegates code lang to code_lang_for_path

Replaces 1A-1 era inline match block with a single call to
kebab_parse_code::code_lang_for_path, per design §3.5 single-source-of-truth
rule. Adds Tier 2 routing test (yaml / dockerfile / toml / json / xml /
groovy / go-mod) and preserves all non-code extension branches.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task D: k8s-manifest-resource-v1 chunker

**Files:**
- Create: `crates/kebab-chunk/src/k8s_manifest_resource_v1.rs`
- Create: `crates/kebab-chunk/tests/fixtures/sample_k8s.yaml`
- Create: `crates/kebab-chunk/tests/k8s_manifest_resource_v1.rs`
- Modify: `crates/kebab-chunk/src/lib.rs` (pub use)

가장 복잡한 chunker. pre-split + serde_yaml deserialize + identify + emit + oversize fallback.

- [ ] **Step 1 (fixture)**: `crates/kebab-chunk/tests/fixtures/sample_k8s.yaml` 생성. 3 document (2 k8s + 1 비-k8s):

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api-server
  namespace: prod
spec:
  replicas: 3
  selector:
    matchLabels:
      app: api-server
  template:
    metadata:
      labels:
        app: api-server
    spec:
      containers:
      - name: api
        image: example/api:1.2.3
---
apiVersion: v1
kind: Service
metadata:
  name: api-server
  namespace: prod
spec:
  selector:
    app: api-server
  ports:
  - port: 80
    targetPort: 8080
---
# Non-k8s document — apiVersion missing
kind: ClusterIP
foo: bar
```

- [ ] **Step 2 (failing test)**: `crates/kebab-chunk/tests/k8s_manifest_resource_v1.rs` 생성:

```rust
use kebab_chunk::{ChunkPolicy, Chunker, K8sManifestResourceV1Chunker};
use kebab_core::{Asset, AssetId, Document, MediaType, ParserVersion, SourceSpan};
use std::path::PathBuf;

fn read_fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).expect("read fixture")
}

fn make_doc(lang: &str, text: &str) -> Document {
    // Tier 2 makes a Document directly (no extractor). Mirror what
    // ingest_one_code_asset will do for Tier 2 in Task G.
    use kebab_core::{Block, Inline};
    Document {
        doc_id: "test-doc".to_string(),
        asset: Asset {
            asset_id: AssetId("test".to_string()),
            workspace_path: format!("test.{lang}"),
            byte_len: text.len() as u64,
            content_hash: "deadbeef".to_string(),
            media_type: MediaType::Code(lang.to_string()),
        },
        parser_version: ParserVersion("none-v1".to_string()),
        metadata: Default::default(),
        blocks: vec![Block::Code {
            text: text.to_string(),
            lang: Some(lang.to_string()),
            span: SourceSpan::Line { start: 1, end: text.lines().count() as u32 },
        }],
    }
}

#[test]
fn k8s_multi_doc_emits_one_chunk_per_resource() {
    let text = read_fixture("sample_k8s.yaml");
    let doc = make_doc("yaml", &text);
    let policy = ChunkPolicy::default();
    let chunks = K8sManifestResourceV1Chunker.chunk(&doc, &policy).unwrap();

    // 2 k8s resources accepted, 1 non-k8s skipped.
    assert_eq!(chunks.len(), 2, "expected 2 k8s chunks, got {}", chunks.len());

    let symbols: Vec<_> = chunks.iter()
        .map(|c| c.source_span_symbol().unwrap_or_default().to_string())
        .collect();
    assert_eq!(symbols, vec![
        "Deployment/prod/api-server".to_string(),
        "Service/prod/api-server".to_string(),
    ]);

    // Each chunk's lang field is "yaml".
    for c in &chunks {
        assert_eq!(c.source_span_lang().as_deref(), Some("yaml"));
    }
}

#[test]
fn k8s_invalid_yaml_emits_zero_chunks() {
    let invalid = "apiVersion: v1\nkind: Service\n\tbadtab: x\n";  // invalid YAML
    let doc = make_doc("yaml", invalid);
    let policy = ChunkPolicy::default();
    let chunks = K8sManifestResourceV1Chunker.chunk(&doc, &policy).unwrap();
    assert!(chunks.is_empty(), "invalid yaml -> 0 chunks");
}

#[test]
fn k8s_cluster_scoped_resource_symbol() {
    let cluster = r#"apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: cluster-admin
rules: []
"#;
    let doc = make_doc("yaml", cluster);
    let policy = ChunkPolicy::default();
    let chunks = K8sManifestResourceV1Chunker.chunk(&doc, &policy).unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(
        chunks[0].source_span_symbol().unwrap_or_default(),
        "ClusterRole/cluster-admin"
    );
}
```

(`Chunk` 의 `source_span_symbol()` / `source_span_lang()` 는 helper. 실제 API 가 다르면 — 예: `chunk.source_spans[0]` 의 `SourceSpan::Code { symbol, lang, .. }` 직접 접근 — 1A-2 의 snapshot test 형식 참고하여 동일 패턴으로 작성. Step 4 impl 작성 후 API 일치 확인.)

- [ ] **Step 3**: `cargo test -p kebab-chunk k8s_manifest_resource_v1` → FAIL ("K8sManifestResourceV1Chunker not found").

- [ ] **Step 4a (shared helper)**: 먼저 `crates/kebab-chunk/src/tier2_shared.rs` 생성 — Task D/E/F 가 모두 사용할 oversize-aware chunk emit helper. impl 작성 전 `crates/kebab-chunk/src/code_rust_ast_v1.rs` 의 Chunk 생성 코드 (hash, token count, ChunkPolicy 적용 부분) Read 하고 동일 패턴 미러링:

```rust
//! p10-2: Tier 2 chunker shared helpers (oversize fallback + Chunk build).

use crate::ChunkPolicy;
use anyhow::Result;
use kebab_core::{Chunk, Document, SourceSpan};

pub(crate) const AST_CHUNK_MAX_LINES: u32 = 200;

/// Push 1+ chunks for a region. ≤200 lines → 1 chunk. >200 → line-window
/// split with same symbol (only line range varies). Mirrors 1A-2's oversize
/// fallback.
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_chunks_with_oversize(
    out: &mut Vec<Chunk>,
    doc: &Document,
    policy: &ChunkPolicy,
    text: &str,
    line_start: u32,
    line_end: u32,
    symbol: &str,
    lang: &str,
    chunker_version: &str,
) -> Result<()> {
    let n_lines = (line_end - line_start + 1).max(1);
    if n_lines <= AST_CHUNK_MAX_LINES {
        out.push(build_chunk(doc, policy, text, line_start, line_end, symbol, lang, chunker_version)?);
        return Ok(());
    }
    let lines: Vec<&str> = text.lines().collect();
    let mut window_start = line_start;
    let mut i = 0usize;
    while i < lines.len() {
        let take = (AST_CHUNK_MAX_LINES as usize).min(lines.len() - i);
        let window_text = lines[i..i + take].join("\n");
        let window_end = window_start + take as u32 - 1;
        out.push(build_chunk(doc, policy, &window_text, window_start, window_end, symbol, lang, chunker_version)?);
        i += take;
        window_start = window_end + 1;
    }
    Ok(())
}

/// Build a single Chunk from a (text, line range, symbol, lang) tuple.
/// MUST mirror code_rust_ast_v1.rs's Chunk construction so hash / token /
/// ChunkPolicy semantics stay identical across Tier 1 and Tier 2.
fn build_chunk(
    doc: &Document,
    policy: &ChunkPolicy,
    text: &str,
    line_start: u32,
    line_end: u32,
    symbol: &str,
    lang: &str,
    chunker_version: &str,
) -> Result<Chunk> {
    let span = SourceSpan::Code {
        line_start,
        line_end,
        symbol: Some(symbol.to_string()),
        lang: Some(lang.to_string()),
    };
    // TODO at impl time: replicate 1A-2's exact Chunk { id, text, source_spans,
    // chunker_version, parser_version, policy_hash, token_count, content_hash, ... }
    // field-fill, computing id / content_hash / policy_hash via the same helpers
    // (blake3 + serde_json_canonicalizer) used by code_rust_ast_v1. The exact
    // function names are in code_rust_ast_v1.rs's `chunk` impl; mirror them
    // here.
    todo!("mirror code_rust_ast_v1's Chunk construction; see Task D Step 4a comment")
}
```

(`todo!()` 는 placeholder 표시 — impl 단계에서 `code_rust_ast_v1.rs` 의 실제 chunk 생성 부분을 그대로 옮김. 1A-2 의 Chunk 생성이 ~30 줄 정도면 그것을 build_chunk 안으로 옮기되 `span` / `chunker_version` 을 인자 형태로 받음.)

- [ ] **Step 4b (k8s chunker impl)**: `crates/kebab-chunk/src/k8s_manifest_resource_v1.rs` 작성:

```rust
//! p10-2: k8s manifest resource-aware chunker.
//!
//! YAML multi-document split with `apiVersion` + `kind` identification.
//! 1 chunk per recognized resource, symbol `<kind>/<namespace>/<name>`.
//! Invalid YAML or non-k8s document → 0 chunks (handled by p10-3 fallback).

use crate::tier2_shared::push_chunks_with_oversize;
use crate::{Chunker, ChunkPolicy};
use anyhow::Result;
use kebab_core::{Block, Chunk, Document};

pub const VERSION_LABEL: &str = "k8s-manifest-resource-v1";

pub struct K8sManifestResourceV1Chunker;

impl Chunker for K8sManifestResourceV1Chunker {
    fn chunker_version(&self) -> &'static str { VERSION_LABEL }

    fn chunk(&self, doc: &Document, policy: &ChunkPolicy) -> Result<Vec<Chunk>> {
        let Some(Block::Code { text, .. }) = doc.blocks.first() else {
            return Ok(vec![]);
        };

        // Pre-split on ^---\s*$ to track line numbers (serde_yaml's
        // multi-doc iterator doesn't expose line offsets).
        let slices = split_yaml_documents(text);

        let mut chunks = Vec::new();
        for slice in slices {
            // Any parse error → skip whole file (return empty; p10-3 fallback later).
            let value: serde_yaml::Value = match serde_yaml::from_str(slice.text) {
                Ok(v) => v,
                Err(_) => return Ok(vec![]),
            };
            let Some(mapping) = value.as_mapping() else { continue };

            let api = mapping.get("apiVersion").and_then(|v| v.as_str()).unwrap_or("");
            let kind = mapping.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            if api.is_empty() || kind.is_empty() {
                continue;
            }

            let metadata = mapping.get("metadata").and_then(|v| v.as_mapping());
            let name = metadata
                .and_then(|m| m.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("<unnamed>");
            let namespace = metadata
                .and_then(|m| m.get("namespace"))
                .and_then(|v| v.as_str());

            let symbol = match namespace {
                Some(ns) if !ns.is_empty() => format!("{kind}/{ns}/{name}"),
                _                          => format!("{kind}/{name}"),
            };

            push_chunks_with_oversize(
                &mut chunks, doc, policy,
                slice.text, slice.line_start, slice.line_end,
                &symbol, "yaml", VERSION_LABEL,
            )?;
        }
        Ok(chunks)
    }
}

struct YamlSlice<'a> {
    text: &'a str,
    line_start: u32,
    line_end: u32,
}

fn split_yaml_documents(text: &str) -> Vec<YamlSlice<'_>> {
    let mut slices = Vec::new();
    let lines: Vec<&str> = text.lines().collect();

    let mut separators: Vec<usize> = lines.iter().enumerate()
        .filter_map(|(i, l)| {
            let trimmed = l.trim_end();
            if trimmed == "---" || trimmed.starts_with("--- ") || trimmed.starts_with("---\t") {
                Some(i)
            } else { None }
        })
        .collect();
    separators.push(lines.len());  // sentinel after last line

    let mut doc_start_line: usize = 0;  // 0-indexed
    for sep_line in separators {
        if sep_line > doc_start_line {
            let start_byte = byte_offset_of_line(text, doc_start_line);
            let end_byte = byte_offset_of_line(text, sep_line);
            let slice_text = &text[start_byte..end_byte];
            if !slice_text.trim().is_empty() {
                slices.push(YamlSlice {
                    text: slice_text,
                    line_start: (doc_start_line + 1) as u32,
                    line_end:   sep_line as u32,
                });
            }
        }
        doc_start_line = sep_line + 1;
    }
    slices
}

fn byte_offset_of_line(text: &str, line_idx: usize) -> usize {
    if line_idx == 0 { return 0; }
    let mut count = 0usize;
    for (i, c) in text.char_indices() {
        if c == '\n' {
            count += 1;
            if count == line_idx { return i + 1; }
        }
    }
    text.len()
}
```

- [ ] **Step 5**: `crates/kebab-chunk/src/lib.rs` 에 추가 (tier2_shared 은 pub 아님 — crate-internal):

```rust
mod tier2_shared;
pub mod k8s_manifest_resource_v1;
pub use k8s_manifest_resource_v1::K8sManifestResourceV1Chunker;
```

- [ ] **Step 6**: `cargo test -p kebab-chunk k8s_manifest_resource_v1 -- --nocapture` → PASS. fixture 의 2 k8s chunk + 비-k8s skip + invalid yaml 0 chunk + cluster-scoped symbol 검증.

- [ ] **Step 7**: Clippy + commit:

```bash
cargo clippy -p kebab-chunk --all-targets -- -D warnings
git add crates/kebab-chunk/src/k8s_manifest_resource_v1.rs \
        crates/kebab-chunk/src/lib.rs \
        crates/kebab-chunk/tests/fixtures/sample_k8s.yaml \
        crates/kebab-chunk/tests/k8s_manifest_resource_v1.rs
git commit -m "$(cat <<'EOF'
feat(p10-2): k8s-manifest-resource-v1 chunker (YAML multi-doc + apiVersion+kind identification)

Splits multi-document YAML by ^---\s*$, requires apiVersion + kind string
fields per document, emits 1 chunk per recognized k8s resource. Symbol =
<kind>/<namespace>/<name> or <kind>/<name> (cluster-scoped). Invalid YAML
returns 0 chunks (handled by p10-3 paragraph fallback). Oversize >200 lines
splits into line-windows sharing the same symbol.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task E: dockerfile-file-v1 chunker

**Files:**
- Create: `crates/kebab-chunk/src/dockerfile_file_v1.rs`
- Create: `crates/kebab-chunk/tests/fixtures/sample.dockerfile`
- Create: `crates/kebab-chunk/tests/dockerfile_file_v1.rs`
- Modify: `crates/kebab-chunk/src/lib.rs`

- [ ] **Step 1 (fixture)**: `crates/kebab-chunk/tests/fixtures/sample.dockerfile` (5 줄):

```dockerfile
FROM rust:1.94-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release
CMD ["/app/target/release/kebab"]
```

- [ ] **Step 2 (failing test)**: `crates/kebab-chunk/tests/dockerfile_file_v1.rs`:

```rust
use kebab_chunk::{ChunkPolicy, Chunker, DockerfileFileV1Chunker};
use kebab_core::{Asset, AssetId, Block, Document, MediaType, ParserVersion, SourceSpan};
use std::path::PathBuf;

fn read_fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures").join(name);
    std::fs::read_to_string(path).unwrap()
}
fn make_doc(lang: &str, text: &str) -> Document {
    Document {
        doc_id: "test".into(),
        asset: Asset {
            asset_id: AssetId("a".into()),
            workspace_path: "Dockerfile".into(),
            byte_len: text.len() as u64,
            content_hash: "deadbeef".into(),
            media_type: MediaType::Code(lang.into()),
        },
        parser_version: ParserVersion("none-v1".into()),
        metadata: Default::default(),
        blocks: vec![Block::Code {
            text: text.into(),
            lang: Some(lang.into()),
            span: SourceSpan::Line { start: 1, end: text.lines().count() as u32 },
        }],
    }
}

#[test]
fn dockerfile_emits_single_chunk() {
    let text = read_fixture("sample.dockerfile");
    let doc = make_doc("dockerfile", &text);
    let chunks = DockerfileFileV1Chunker.chunk(&doc, &ChunkPolicy::default()).unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].source_span_symbol().as_deref(), Some("<dockerfile>"));
    assert_eq!(chunks[0].source_span_lang().as_deref(), Some("dockerfile"));
    // line_start = 1, line_end = 5 (5-line fixture).
    let (ls, le) = chunks[0].source_span_lines();
    assert_eq!((ls, le), (1, 5));
}
```

(`source_span_lines()` / `source_span_symbol()` API 가 1A-2 와 동일한지 확인 후 미러링.)

- [ ] **Step 3**: `cargo test -p kebab-chunk dockerfile_file_v1` → FAIL.

- [ ] **Step 4 (impl)**: `crates/kebab-chunk/src/dockerfile_file_v1.rs`. Task D Step 4a 의 `tier2_shared::push_chunks_with_oversize` 재사용:

```rust
//! p10-2: dockerfile whole-file chunker (Tier 2).

use crate::tier2_shared::push_chunks_with_oversize;
use crate::{Chunker, ChunkPolicy};
use anyhow::Result;
use kebab_core::{Block, Chunk, Document};

pub const VERSION_LABEL: &str = "dockerfile-file-v1";

pub struct DockerfileFileV1Chunker;

impl Chunker for DockerfileFileV1Chunker {
    fn chunker_version(&self) -> &'static str { VERSION_LABEL }

    fn chunk(&self, doc: &Document, policy: &ChunkPolicy) -> Result<Vec<Chunk>> {
        let Some(Block::Code { text, .. }) = doc.blocks.first() else {
            return Ok(vec![]);
        };
        let total_lines = text.lines().count().max(1) as u32;
        let mut chunks = Vec::new();
        push_chunks_with_oversize(
            &mut chunks, doc, policy,
            text, 1, total_lines,
            "<dockerfile>", "dockerfile", VERSION_LABEL,
        )?;
        Ok(chunks)
    }
}
```

- [ ] **Step 5**: `crates/kebab-chunk/src/lib.rs` 갱신:

```rust
pub mod dockerfile_file_v1;
pub use dockerfile_file_v1::DockerfileFileV1Chunker;
```

- [ ] **Step 6**: `cargo test -p kebab-chunk dockerfile_file_v1` → PASS.

- [ ] **Step 7**: Clippy + commit:

```bash
cargo clippy -p kebab-chunk --all-targets -- -D warnings
git add crates/kebab-chunk/src/dockerfile_file_v1.rs \
        crates/kebab-chunk/src/lib.rs \
        crates/kebab-chunk/tests/fixtures/sample.dockerfile \
        crates/kebab-chunk/tests/dockerfile_file_v1.rs
git commit -m "$(cat <<'EOF'
feat(p10-2): dockerfile-file-v1 chunker (whole-file 1 chunk, symbol <dockerfile>)

Reads entire Dockerfile / Dockerfile.* / *.dockerfile content and emits a
single Chunk with symbol "<dockerfile>", code_lang "dockerfile", line range
1..EOF. Oversize >200 lines splits into line-windows sharing the symbol.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task F: manifest-file-v1 chunker

**Files:**
- Create: `crates/kebab-chunk/src/manifest_file_v1.rs`
- Create: `crates/kebab-chunk/tests/fixtures/sample_cargo.toml`
- Create: `crates/kebab-chunk/tests/fixtures/sample_package.json`
- Create: `crates/kebab-chunk/tests/fixtures/sample_pom.xml`
- Create: `crates/kebab-chunk/tests/fixtures/sample_go.mod`
- Create: `crates/kebab-chunk/tests/manifest_file_v1.rs`
- Modify: `crates/kebab-chunk/src/lib.rs`

- [ ] **Step 1 (fixtures)**: 4 작은 fixture (각 ~10 줄):

`sample_cargo.toml`:

```toml
[package]
name = "demo"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1"
```

`sample_package.json`:

```json
{
  "name": "demo",
  "version": "0.1.0",
  "dependencies": {
    "react": "^18.0.0"
  }
}
```

`sample_pom.xml`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.demo</groupId>
  <artifactId>demo</artifactId>
  <version>0.1.0</version>
</project>
```

`sample_go.mod`:

```
module example.com/demo

go 1.22

require github.com/spf13/cobra v1.8.0
```

- [ ] **Step 2 (failing test)**: `crates/kebab-chunk/tests/manifest_file_v1.rs` 에 4 테스트 (각 fixture 마다):

```rust
use kebab_chunk::{ChunkPolicy, Chunker, ManifestFileV1Chunker};
use kebab_core::{Asset, AssetId, Block, Document, MediaType, ParserVersion, SourceSpan};
use std::path::PathBuf;

fn read_fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures").join(name);
    std::fs::read_to_string(path).unwrap()
}
fn make_doc(lang: &str, text: &str) -> Document {
    // Same as Task E's helper. Copy verbatim.
    Document {
        doc_id: "test".into(),
        asset: Asset {
            asset_id: AssetId("a".into()),
            workspace_path: format!("test-manifest"),
            byte_len: text.len() as u64,
            content_hash: "deadbeef".into(),
            media_type: MediaType::Code(lang.into()),
        },
        parser_version: ParserVersion("none-v1".into()),
        metadata: Default::default(),
        blocks: vec![Block::Code {
            text: text.into(),
            lang: Some(lang.into()),
            span: SourceSpan::Line { start: 1, end: text.lines().count() as u32 },
        }],
    }
}

#[test]
fn cargo_toml_single_chunk_with_toml_lang() {
    let text = read_fixture("sample_cargo.toml");
    let doc = make_doc("toml", &text);
    let chunks = ManifestFileV1Chunker.chunk(&doc, &ChunkPolicy::default()).unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].source_span_symbol().as_deref(), Some("<manifest>"));
    assert_eq!(chunks[0].source_span_lang().as_deref(), Some("toml"));
}

#[test]
fn package_json_single_chunk_with_json_lang() {
    let text = read_fixture("sample_package.json");
    let doc = make_doc("json", &text);
    let chunks = ManifestFileV1Chunker.chunk(&doc, &ChunkPolicy::default()).unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].source_span_symbol().as_deref(), Some("<manifest>"));
    assert_eq!(chunks[0].source_span_lang().as_deref(), Some("json"));
}

#[test]
fn pom_xml_single_chunk_with_xml_lang() {
    let text = read_fixture("sample_pom.xml");
    let doc = make_doc("xml", &text);
    let chunks = ManifestFileV1Chunker.chunk(&doc, &ChunkPolicy::default()).unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].source_span_symbol().as_deref(), Some("<manifest>"));
    assert_eq!(chunks[0].source_span_lang().as_deref(), Some("xml"));
}

#[test]
fn go_mod_single_chunk_with_go_mod_lang() {
    let text = read_fixture("sample_go.mod");
    let doc = make_doc("go-mod", &text);
    let chunks = ManifestFileV1Chunker.chunk(&doc, &ChunkPolicy::default()).unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].source_span_symbol().as_deref(), Some("<manifest>"));
    assert_eq!(chunks[0].source_span_lang().as_deref(), Some("go-mod"));
}
```

- [ ] **Step 3**: `cargo test -p kebab-chunk manifest_file_v1` → FAIL.

- [ ] **Step 4 (impl)**: `crates/kebab-chunk/src/manifest_file_v1.rs`:

```rust
//! p10-2: manifest whole-file chunker (Tier 2). Cargo.toml / package.json / etc.

use crate::tier2_shared::push_chunks_with_oversize;
use crate::{Chunker, ChunkPolicy};
use anyhow::Result;
use kebab_core::{Block, Chunk, Document};

pub const VERSION_LABEL: &str = "manifest-file-v1";

pub struct ManifestFileV1Chunker;

impl Chunker for ManifestFileV1Chunker {
    fn chunker_version(&self) -> &'static str { VERSION_LABEL }

    fn chunk(&self, doc: &Document, policy: &ChunkPolicy) -> Result<Vec<Chunk>> {
        let Some(Block::Code { text, lang, .. }) = doc.blocks.first() else {
            return Ok(vec![]);
        };
        let lang_str = lang.as_deref().unwrap_or("");
        let total_lines = text.lines().count().max(1) as u32;
        let mut chunks = Vec::new();
        push_chunks_with_oversize(
            &mut chunks, doc, policy,
            text, 1, total_lines,
            "<manifest>", lang_str, VERSION_LABEL,
        )?;
        Ok(chunks)
    }
}
```

- [ ] **Step 5**: `crates/kebab-chunk/src/lib.rs` 갱신:

```rust
pub mod manifest_file_v1;
pub use manifest_file_v1::ManifestFileV1Chunker;
```

- [ ] **Step 6**: `cargo test -p kebab-chunk manifest_file_v1` → 4 테스트 PASS.

- [ ] **Step 7**: Clippy + commit:

```bash
cargo clippy -p kebab-chunk --all-targets -- -D warnings
git add crates/kebab-chunk/src/manifest_file_v1.rs \
        crates/kebab-chunk/src/lib.rs \
        crates/kebab-chunk/tests/fixtures/sample_cargo.toml \
        crates/kebab-chunk/tests/fixtures/sample_package.json \
        crates/kebab-chunk/tests/fixtures/sample_pom.xml \
        crates/kebab-chunk/tests/fixtures/sample_go.mod \
        crates/kebab-chunk/tests/manifest_file_v1.rs
git commit -m "$(cat <<'EOF'
feat(p10-2): manifest-file-v1 chunker (whole-file 1 chunk, symbol <manifest>)

Emits 1 Chunk per manifest file (Cargo.toml / pyproject.toml / package.json /
tsconfig.json / pom.xml / build.gradle / go.mod). Symbol unified to
"<manifest>"; manifest type distinguished by code_lang (toml / json / xml /
groovy / go-mod). Oversize >200 lines splits into line-windows.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task G: ingest_one_code_asset Tier 2 routing

**Files:**
- Modify: `crates/kebab-app/src/lib.rs` (`ingest_one_code_asset` 함수 + 호출 부 allowlist)

현재 7-arm match (rust|python|typescript|javascript|go|java|kotlin) 옆에 Tier 2 분기. Tier 2 는 `extract` 단계 없음 — `RawAsset` bytes 로 직접 `Document` 생성.

- [ ] **Step 1**: 함수 본문의 parser_version match 갱신:

```rust
let parser_version = match code_lang {
    "rust"       => ParserVersion(kebab_parse_code::RUST_PARSER_VERSION.to_string()),
    // ... 기존 7 줄 그대로 ...
    "kotlin" => ParserVersion(kebab_parse_code::KOTLIN_PARSER_VERSION.to_string()),
    // p10-2: Tier 2 has no parse step.
    "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod"
        => ParserVersion("none-v1".to_string()),
    other => anyhow::bail!("unsupported code_lang: {other}"),
};
```

- [ ] **Step 2**: chunker_version match 갱신 (Tier 2 분기 추가):

```rust
let chunker_version = match code_lang {
    "rust"       => CodeRustAstV1Chunker.chunker_version(),
    // ... 기존 ...
    "kotlin"     => CodeKotlinAstV1Chunker.chunker_version(),
    "yaml"       => K8sManifestResourceV1Chunker.chunker_version(),
    "dockerfile" => DockerfileFileV1Chunker.chunker_version(),
    "toml" | "json" | "xml" | "groovy" | "go-mod"
                 => ManifestFileV1Chunker.chunker_version(),
    other => anyhow::bail!("unreachable chunker_version: {other}"),
};
```

- [ ] **Step 3**: extract / chunk 단계 분리. 현재는 `let mut canonical = match code_lang { ... extract ... };` 후 `let chunks = match code_lang { ... chunk(&canonical) ... };`. Tier 2 는 extract 없이 직접 Document 생성:

```rust
// p10-1B/1C: Tier 1 extractors return a canonical Document.
// p10-2: Tier 2 has no parser — synthesize a Document with a single
// Block::Code carrying the whole file text. The chunker does the work.
let mut canonical = match code_lang {
    "rust"       => RustAstExtractor::new().extract(&ctx, &bytes).context("...")?,
    // ... 기존 7 줄 ...
    "kotlin"     => KotlinAstExtractor::new().extract(&ctx, &bytes).context("...")?,
    "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod" => {
        // Tier 2: no extractor. Build a minimal Document.
        synthesize_tier2_document(asset, &bytes, code_lang, &parser_version)?
    }
    other => anyhow::bail!("unreachable (extract): {other}"),
};

let chunks = match code_lang {
    "rust"       => CodeRustAstV1Chunker.chunk(&canonical, chunk_policy).context("...")?,
    // ... 기존 ...
    "kotlin"     => CodeKotlinAstV1Chunker.chunk(&canonical, chunk_policy).context("...")?,
    "yaml"       => K8sManifestResourceV1Chunker.chunk(&canonical, chunk_policy).context("kb-chunk::K8sManifestResourceV1Chunker::chunk")?,
    "dockerfile" => DockerfileFileV1Chunker.chunk(&canonical, chunk_policy).context("kb-chunk::DockerfileFileV1Chunker::chunk")?,
    "toml" | "json" | "xml" | "groovy" | "go-mod"
                 => ManifestFileV1Chunker.chunk(&canonical, chunk_policy).context("kb-chunk::ManifestFileV1Chunker::chunk")?,
    other => anyhow::bail!("unreachable (chunk): {other}"),
};
```

`synthesize_tier2_document` helper 를 같은 파일 (kebab-app/src/lib.rs) 안에 추가:

```rust
fn synthesize_tier2_document(
    asset: &RawAsset,
    bytes: &[u8],
    code_lang: &str,
    parser_version: &ParserVersion,
) -> anyhow::Result<kebab_core::Document> {
    use kebab_core::{Asset, AssetId, Block, Document, MediaType, SourceSpan};
    let text = std::str::from_utf8(bytes)
        .with_context(|| format!("tier2 doc not utf-8: {}", asset.workspace_path))?
        .to_string();
    let n_lines = text.lines().count().max(1) as u32;
    Ok(Document {
        doc_id: asset.asset_id.0.clone(),  // tentative — will be overwritten downstream
        asset: Asset {
            asset_id: AssetId(asset.asset_id.0.clone()),
            workspace_path: asset.workspace_path.clone(),
            byte_len: asset.byte_len,
            content_hash: asset.content_hash.clone(),
            media_type: MediaType::Code(code_lang.to_string()),
        },
        parser_version: parser_version.clone(),
        metadata: Default::default(),
        blocks: vec![Block::Code {
            text,
            lang: Some(code_lang.to_string()),
            span: SourceSpan::Line { start: 1, end: n_lines },
        }],
    })
}
```

(`Document` / `Asset` 의 정확한 필드 — Read `crates/kebab-core/src/document.rs` 후 미러링. 위 코드의 필드명이 다르면 정정.)

- [ ] **Step 4**: 호출 부 allowlist 갱신 (현재 `matches!(lang.as_str(), "rust" | "python" | ...)`):

```rust
if matches!(lang.as_str(),
    "rust" | "python" | "typescript" | "javascript"
    | "go" | "java" | "kotlin"
    | "yaml" | "dockerfile" | "toml" | "json" | "xml" | "groovy" | "go-mod"
) {
    return ingest_one_code_asset(...);
}
```

- [ ] **Step 5**: Build + per-crate test:

```bash
cargo build -p kebab-app
cargo test -p kebab-app --lib -- --nocapture 2>&1 | tail -20
```

Expected: build clean, 기존 unit test (있다면) 그대로 PASS.

- [ ] **Step 6**: Clippy + commit:

```bash
cargo clippy -p kebab-app --all-targets -- -D warnings
git add crates/kebab-app/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(p10-2): activate Tier 2 chunkers in ingest_one_code_asset dispatch

Adds yaml / dockerfile / toml / json / xml / groovy / go-mod arms to the
existing 7-arm AST match. parser_version unified to "none-v1" for Tier 2.
synthesize_tier2_document builds a minimal Document (single Block::Code
with raw file text) since Tier 2 has no parse step. allowlist in
ingest_one_asset extended to admit Tier 2 langs.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task H: code_ingest_smoke integration tests (Tier 2)

**Files:**
- Modify: `crates/kebab-app/tests/code_ingest_smoke.rs` (3 새 test)

기존 9 테스트 옆에 yaml / dockerfile / manifest 통합 ingest 검증 1개씩 추가.

- [ ] **Step 1 (failing test)** — 파일 끝에 추가:

```rust
#[test]
fn tier2_k8s_yaml_ingest_searchable() {
    let kb = isolated_kb();  // TempDir KB helper, existing in this file
    let path = kb.workspace_root().join("k8s/deploy.yaml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: api\n  namespace: prod\nspec:\n  replicas: 1\n").unwrap();
    kb.ingest_via_cli().expect("ingest");

    let hits = kb.search_via_cli_json("--code-lang yaml api");
    assert!(!hits.is_empty(), "expected at least 1 yaml hit");
    assert_eq!(hits[0]["citation"]["lang"].as_str(), Some("yaml"));
    assert_eq!(hits[0]["citation"]["symbol"].as_str(), Some("Deployment/prod/api"));
}

#[test]
fn tier2_dockerfile_ingest_searchable() {
    let kb = isolated_kb();
    let path = kb.workspace_root().join("Dockerfile");
    std::fs::write(&path, "FROM rust:1.94\nRUN cargo install foo\n").unwrap();
    kb.ingest_via_cli().expect("ingest");

    let hits = kb.search_via_cli_json("--code-lang dockerfile cargo");
    assert!(!hits.is_empty());
    assert_eq!(hits[0]["citation"]["lang"].as_str(), Some("dockerfile"));
    assert_eq!(hits[0]["citation"]["symbol"].as_str(), Some("<dockerfile>"));
}

#[test]
fn tier2_cargo_toml_ingest_searchable() {
    let kb = isolated_kb();
    let path = kb.workspace_root().join("Cargo.toml");
    std::fs::write(&path, "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n").unwrap();
    kb.ingest_via_cli().expect("ingest");

    let hits = kb.search_via_cli_json("--code-lang toml demo");
    assert!(!hits.is_empty());
    assert_eq!(hits[0]["citation"]["lang"].as_str(), Some("toml"));
    assert_eq!(hits[0]["citation"]["symbol"].as_str(), Some("<manifest>"));
}
```

(helper API — `isolated_kb()`, `workspace_root()`, `ingest_via_cli()`, `search_via_cli_json()` — 는 기존 9 테스트가 쓰는 그대로. 명명이 다르면 그 파일의 패턴 미러링.)

- [ ] **Step 2**: 실행 → FAIL ("yaml symbol not present").

```bash
cargo test -p kebab-app --test code_ingest_smoke tier2 -- --nocapture
```

- [ ] **Step 3**: Task D-G 가 다 완료된 상태이므로 코드 변경 없이 PASS 해야 함. FAIL 이면 디버그:
- citation_helper 의 `Citation::Code` mapping (1A-1) 이 `lang` / `symbol` 을 wire 에 채우는지 확인.
- `code_lang_for_path` 가 호출되는지 확인 (kebab-source-fs/media.rs).

- [ ] **Step 4**: 9 + 3 = 12 테스트 통과 후 commit:

```bash
git add crates/kebab-app/tests/code_ingest_smoke.rs
git commit -m "$(cat <<'EOF'
test(p10-2): integration smoke tests for Tier 2 (k8s yaml + Dockerfile + Cargo.toml)

Three new tests in code_ingest_smoke.rs verifying isolated-TempDir ingest +
--code-lang filter + Citation::Code.lang / .symbol shape for each Tier 2
chunker. Brings the suite to 12 tests (Rust 3 + Python 1 + TS 1 + JS 1 +
Go 1 + Java 1 + Kotlin 1 + yaml 1 + dockerfile 1 + manifest 1).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task I: frozen design §3.5 + §10.1 갱신

**Files:**
- Modify: `docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md`

- [ ] **Step 1**: §3.5 의 `code_lang` 매핑 표 끝부분에 3 줄 추가 (Shell / Make 줄 사이 적절한 위치 — `code_lang_for_path` 정의 직전):

```diff
 - YAML / k8s manifest (`.yaml`, `.yml`) → `yaml`
 - Dockerfile (`Dockerfile`, `*.dockerfile`) → `dockerfile`
 - TOML (`.toml`) → `toml`
 - JSON (`.json`) → `json`
+- XML (`.xml`, `pom.xml`) → `xml`
+- Groovy (`build.gradle`, `.gradle`) → `groovy`
+- Go module (`go.mod`) → `go-mod`
 - Shell (`.sh`, `.bash`, `.zsh`) → `shell`
```

- [ ] **Step 2**: §10.1 의 deactivation log 표 (또는 줄 목록) 끝에 추가 (1C-Go, 1C-JK 활성화 줄 다음):

```
| p10-2 | Tier 2 활성화 — k8s-manifest-resource-v1 + dockerfile-file-v1 + manifest-file-v1 chunker 3종. code_lang 추가 매핑 (xml / groovy / go-mod). | 2026-05-20 |
```

(§10.1 의 정확한 형식 — table vs bullet list — 은 현재 파일을 Read 한 후 그 형식에 맞게.)

- [ ] **Step 3**: commit:

```bash
git add docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md
git commit -m "$(cat <<'EOF'
docs(p10-2): activate Tier 2 in code-ingest design §10.1 + §3.5 mappings

§3.5: add code_lang_for_path mappings xml / groovy / go-mod.
§10.1: add deactivation log entry for p10-2 (3 Tier 2 chunkers active).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task J: README + HANDOFF + ARCHITECTURE + SMOKE + tasks/INDEX + tasks/p10/INDEX + full-suite gate

**Files:**
- Modify: `README.md`
- Modify: `HANDOFF.md`
- Modify: `docs/ARCHITECTURE.md`
- Modify: `docs/SMOKE.md`
- Modify: `tasks/INDEX.md`
- Modify: `tasks/p10/INDEX.md`

- [ ] **Step 1 — README.md**: **명령** 표의 ingest 행에 Tier 2 7종 언급 추가. 예 (기존 행이 "지원 lang: rust / python / typescript / javascript / go / java / kotlin" 형식이면):

```diff
-지원 lang: rust / python / typescript / javascript / go / java / kotlin
+지원 lang: rust / python / typescript / javascript / go / java / kotlin / yaml (k8s) / dockerfile / toml / json / xml / groovy / go-mod
```

Configuration 섹션 변경 없음 (gating flag 신설 없음). Mermaid 다이어그램은 변경 없음 (code 카테고리 이미 존재).

- [ ] **Step 2 — HANDOFF.md**: phase 표의 p10-2 행 ⏳ → ✅. "머지 후 발견된 버그 / 결정 (요약)" 섹션 변경 불필요 (post-merge 발견 시 별도 PR).

- [ ] **Step 3 — docs/ARCHITECTURE.md**: 디렉토리 트리의 `crates/kebab-chunk/src/` 트리에 3 줄 추가:

```
crates/kebab-chunk/src/
├── code_*_ast_v1.rs       (Tier 1, 7개)
├── k8s_manifest_resource_v1.rs   (Tier 2, p10-2)
├── dockerfile_file_v1.rs         (Tier 2, p10-2)
├── manifest_file_v1.rs           (Tier 2, p10-2)
├── tier2_shared.rs               (Tier 2 helper, p10-2)
└── ...
```

- [ ] **Step 4 — docs/SMOKE.md**: 한 줄 추가 — Tier 2 smoke 검증 (yaml + Dockerfile + Cargo.toml ingest → search --code-lang yaml/dockerfile/toml).

- [ ] **Step 5 — tasks/INDEX.md** + **tasks/p10/INDEX.md**: p10-2 status ⏳ → ✅.

- [ ] **Step 6 — Full-suite gate** (memory-conscious):

```bash
df -h /     # 공간 확인
cargo clean # heavy 면
cargo test --workspace --no-fail-fast -j 1 2>&1 | tail -60
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -30
```

Expected: 모든 테스트 PASS, clippy clean.

- [ ] **Step 7**: commit:

```bash
git add README.md HANDOFF.md docs/ARCHITECTURE.md docs/SMOKE.md tasks/INDEX.md tasks/p10/INDEX.md
git commit -m "$(cat <<'EOF'
docs(p10-2): README/HANDOFF/ARCHITECTURE/SMOKE/INDEX + tasks/p10/INDEX

User-visible surface sync per the docs-split rule: README adds Tier 2 langs
in the command table; HANDOFF flips p10-2 to ✅; ARCHITECTURE adds the new
chunker modules + tier2_shared.rs to the directory tree; SMOKE adds a
yaml/Dockerfile/Cargo.toml smoke step; both INDEX files flip p10-2 to ✅.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task K: version bump 0.13.0 → 0.14.0 + gitea PR

**Files:**
- Modify: `Cargo.toml` (workspace `version`)
- Modify: `Cargo.lock` (자동 갱신)

- [ ] **Step 1**: `Cargo.toml` 의 `[workspace.package] version = "0.13.0"` → `"0.14.0"`.

- [ ] **Step 2**: `cargo build -p kebab` 한 번 — `Cargo.lock` 갱신.

- [ ] **Step 3**: commit:

```bash
git add Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
chore: bump version 0.13.0 → 0.14.0 (p10-2 Tier 2 resource-aware)

Minor bump — additive code_lang values (xml / groovy / go-mod) + 3 new
chunker_version labels (k8s-manifest-resource-v1 / dockerfile-file-v1 /
manifest-file-v1) + frozen design §3.5 deltas. No DB migration, no wire
schema major bump.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 4**: gitea PR open via gitea-ops skill. Branch `feat/p10-2-tier2-resource` → `main`. Title: `feat(p10-2): Tier 2 resource-aware chunkers (k8s + Dockerfile + manifest)`.

- [ ] **Step 5**: 사용자가 APPROVE 하면 즉시 머지 (memory: feedback_pr_workflow). 머지 후 main pull + branch 정리 + `gitea-release v0.14.0` (gitea-ops skill).

---

## Verification matrix (final, after Task K merge)

| 검증 | 명령 | 기대 |
|------|------|------|
| Tier 2 lang routing | `kebab schema --json \| jq '.stats.code_lang_breakdown'` (Tier 2 파일 ingest 후) | yaml / dockerfile / toml / json / xml / groovy / go-mod 카운트 등장 |
| k8s symbol shape | `kebab search --code-lang yaml --json` | citation.symbol = `<kind>/<namespace>/<name>` |
| Dockerfile chunk | `kebab search --code-lang dockerfile --json` | citation.symbol = `<dockerfile>`, line 1..EOF |
| manifest chunk | `kebab search --code-lang toml --json` | citation.symbol = `<manifest>`, lang 매핑 |
| 비-k8s YAML skip | docker-compose.yml ingest | 0 chunk, IngestReport.skipped 카운트 +1 |
| Invalid YAML skip | 의도적 invalid yaml ingest | 0 chunk, IngestReport.skipped + warning |

`docs/SMOKE.md` 의 Tier 2 절을 따라 수동 검증 가능.

---

## Risks 재요약 (구현 중 주의)

- `^---\s*$` regex 가 너무 좁음 — YAML 표준 상 `---` 뒤 공백 + comment 가능. fixture 로 검증 + 필요시 regex 완화.
- `serde_yaml::Value::as_str()` 가 boolean / number 에 None 반환 — apiVersion/kind 가 string 임을 강제. 이미 spec 명시.
- pre-1.0 의 Cargo.toml workspace version 위치 — `[workspace.package]` 가 맞는지 현재 파일 Read 후 확인.
- `synthesize_tier2_document` 의 `doc_id` 가 임시값 (`asset_id.0`) — downstream 의 진짜 doc_id 생성 로직과 충돌 가능. 1A-2 의 extractor return 형식과 같은 doc_id 정책 적용. Step 3 impl 작성 전 RustAstExtractor 의 Document 생성 코드 확인.
- pom.xml 거대 fixture 가 200 줄 넘어가면 oversize split 검증 좋음 — 필요시 Task F 의 sample_pom.xml 을 일부러 길게.
