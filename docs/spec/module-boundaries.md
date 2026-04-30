# Module boundaries

`kb-core` is leaf — every other crate depends on it. Parsers depend on
`kb-parse-types` (not on `kb-normalize`); `kb-normalize` depends on
`kb-parse-types` (not on parsers). UI crates depend only on `kb-app`.

Canonical source:
[docs/superpowers/specs/2026-04-27-kb-final-form-design.md](../superpowers/specs/2026-04-27-kb-final-form-design.md), §8.
