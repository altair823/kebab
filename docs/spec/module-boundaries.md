# Module boundaries

`kebab-core` is leaf — every other crate depends on it. Parsers depend on
`kebab-parse-types` (not on `kebab-normalize`); `kebab-normalize` depends on
`kebab-parse-types` (not on parsers). UI crates depend only on `kebab-app`.

Canonical source:
[docs/superpowers/specs/2026-04-27-kebab-final-form-design.md](../superpowers/specs/2026-04-27-kebab-final-form-design.md), §8.
