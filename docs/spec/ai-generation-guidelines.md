# AI generation guidelines

When implementing tasks against this codebase:

- Treat the frozen design doc as the single source of truth. Do not invent
  new fields, traits, or enum variants.
- Prefer editing existing files to creating new ones; reuse types from
  `kb-core` instead of duplicating shapes.
- For each task, follow the task spec under `tasks/p<N>/p<N>-<i>.md`.

Canonical source:
[docs/superpowers/specs/2026-04-27-kb-final-form-design.md](../superpowers/specs/2026-04-27-kb-final-form-design.md), §11 + §12.
