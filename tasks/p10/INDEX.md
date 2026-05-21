# Phase 10 — Code Ingest

| ID | Subject | Status |
|----|---------|--------|
| 1A-1 | code ingest framework (wire schema, parse-code crate skeleton, filter flags, skip policy, config 절) | ✅ 머지 |
| 1A-2 | Rust AST chunker | ✅ 머지 |
| 1B | Python + TS/JS AST chunkers | 🟡 PR 오픈 (코드 완성, 머지 대기) |
| 1C-Go | Go AST chunker (`code-go-ast-v1`) | 🟡 PR 오픈 (v0.12.0) |
| 1C-JavaKotlin | Java + Kotlin AST chunkers (`code-java-ast-v1` / `code-kotlin-ast-v1`) | 🟢 PR 오픈 (v0.13.0) |
| 1D | C + C++ AST chunkers | ⏳ |
| 2 | Tier 2 resource-aware (k8s / Dockerfile / manifest) | ✅ 머지 (v0.14.0) |
| 3 | Tier 3 paragraph + line-window fallback | ✅ 머지 (v0.15.0) |

Design: [2026-05-15-kebab-code-ingest-design.md](../../docs/superpowers/specs/2026-05-15-kebab-code-ingest-design.md)
