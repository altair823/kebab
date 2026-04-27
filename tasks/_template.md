---
phase: P<N>
component: <crate-or-module-name>
task_id: p<N>-<i>
title: "<Component title>"
status: planned
depends_on: []                            # other task_ids
unblocks: []                              # other task_ids
contract_source: ../docs/superpowers/specs/2026-04-27-kb-final-form-design.md
contract_sections: []                     # e.g. [§3.5, §5.5, §7.2]
---

# <task_id> — <Component title>

## Goal

<One sentence. The user-facing outcome of this task.>

## Why now / why this size

<One paragraph. Why this is the right unit of work and how it slots into the phase.>

## Allowed dependencies

- `kb-core`
- <other crates per design §8>
- <external crates with versions>

## Forbidden dependencies

- <list — every crate banned per design §8 Allowed/Forbidden table>

If any item here is needed during implementation, STOP and update the frozen design doc first.

## Inputs

| input | type | source |
|-------|------|--------|
| ...   | ...  | ...    |

## Outputs

| output | type | downstream consumer |
|--------|------|---------------------|
| ...    | ...  | ...                 |

## Public surface (signatures only — no new types)

```rust
// Cite only types/traits already defined in the frozen design doc.
// If a new helper is needed, mark it "internal" and keep it crate-private.
```

## Behavior contract

- <bullet list of must-hold invariants>
- <reference to design doc section numbers>
- <determinism / version recording / error policy>

## Storage / wire effects

- DB tables touched (read/write)
- LanceDB tables touched (read/write)
- Filesystem paths created/read
- Wire schema objects emitted (must conform to `*.v1`)

## Test plan

| kind | description | fixture / data |
|------|-------------|----------------|
| unit | ...         | ...            |
| snapshot | ... (JSON freeze) | `fixtures/...` |
| contract | trait round-trip | mock impls |
| integration | end-to-end via `kb-app` facade | tmp workspace |

All tests must run under `cargo test -p <crate>` and not require external network or Ollama unless explicitly stated.

## Definition of Done

- [ ] `cargo check -p <crate>` passes
- [ ] `cargo test -p <crate>` passes
- [ ] No imports outside Allowed dependencies
- [ ] All emitted wire JSON validates against `docs/wire-schema/v1/<schema>.schema.json` (when applicable)
- [ ] All record version fields populated per design §9
- [ ] PR body links the relevant design section numbers

## Out of scope

- <explicit list — features that other tasks cover>
- <future-phase work>

## Risks / notes

- <one paragraph max — known traps, version coupling, perf concerns>
