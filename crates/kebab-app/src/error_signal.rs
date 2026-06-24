//! Typed signal re-exports + new signals introduced by fb-27.
//!
//! kebab-cli (and future kebab-desktop) downcast on these to
//! build `error.v1` wire records. The existing signals
//! (`RefusalSignal`, `NoHitSignal`, `DoctorUnhealthy`) live in
//! `doctor_signal.rs` — leave those unchanged and re-export via this
//! module so callers have one place to import from.
//!
//! See `docs/superpowers/specs/2026-05-07-p9-fb-27-introspection-and-error-wire-design.md`.

pub use crate::doctor_signal::{DoctorUnhealthy, NoHitSignal, RefusalSignal};

pub use kebab_config::{ConfigInvalid, ConfigNotFound};
pub use kebab_llm_local::LlmError;
pub use kebab_store_sqlite::NotIndexed;
