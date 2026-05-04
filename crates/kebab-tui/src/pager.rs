//! p9-fb-24: page-step constant shared by Ask + Inspect PgUp/PgDn.
//!
//! Fixed `10` rows per page (independent of viewport height). The
//! design doc considered viewport-aware paging but deliberately
//! deferred it — Inspect already shipped with `+/-10`, so unifying
//! on the same constant is the smallest path that closes the
//! "Ask has no PgUp/PgDn" feedback. A future viewport-aware upgrade
//! lives behind this single edit point.

/// Rows scrolled per `PgUp` / `PgDn` keystroke.
pub(crate) const PAGE_STEP: u16 = 10;
