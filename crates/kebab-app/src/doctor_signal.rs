//! Signal types used by `kb-cli`'s `exit_code` mapping (§10).
//!
//! These are *not* errors per se: a doctor failure is normal output, just
//! signalled out-of-band so the CLI can exit with the right status.

use std::fmt;

#[derive(Debug)]
pub struct DoctorUnhealthy;

impl fmt::Display for DoctorUnhealthy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("doctor unhealthy")
    }
}

impl std::error::Error for DoctorUnhealthy {}

#[derive(Debug)]
pub struct RefusalSignal;

impl fmt::Display for RefusalSignal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("refusal")
    }
}

impl std::error::Error for RefusalSignal {}

#[derive(Debug)]
pub struct NoHitSignal;

impl fmt::Display for NoHitSignal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no hit")
    }
}

impl std::error::Error for NoHitSignal {}

/// `kebab eval compare --max-drop` found a metric that regressed past
/// the tolerance. Like the others here this is normal output, not a
/// failure — it just has to reach the shell as a non-zero status so a
/// golden set can actually gate something.
#[derive(Debug)]
pub struct EvalRegression;

impl fmt::Display for EvalRegression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("eval regression")
    }
}

impl std::error::Error for EvalRegression {}
