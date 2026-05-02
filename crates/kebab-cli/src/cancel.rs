//! `kebab ingest` SIGINT (Ctrl-C) handler — flips a shared
//! `Arc<AtomicBool>` so `kebab_app::ingest_with_config_cancellable`
//! can break at the next step boundary.
//!
//! Per spec §10: the second Ctrl-C is a hard exit (130 = SIGINT
//! conventional). We count signal arrivals via a private atomic and
//! call `std::process::exit` on the second arrival — past the point
//! where the user has signalled both "let me out gracefully" and
//! "no, really, get me out now". This sidesteps the indicatif
//! cleanup path; the terminal may be left in a slightly odd state
//! after a hard exit, which is the acceptable tradeoff for "really
//! exit now".
//!
//! `ctrlc` is the only cross-platform SIGINT helper that doesn't
//! drag in a tokio runtime; it registers a single OS-level handler
//! per process. Because the handler is process-global, calling
//! `install` more than once per `kebab` invocation is forbidden
//! (would clobber the previous handler) — `Cmd::Ingest` is the only
//! caller today, but a future `kebab eval run` etc. would need to
//! either share the same atomic or deliberately re-install before
//! its run begins.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Install a SIGINT handler that:
/// - on first signal: sets `cancel.store(true)` so the cooperative
///   cancel loop in `kebab_app::ingest_with_config_cancellable`
///   breaks at its next step boundary.
/// - on second signal: hard-exits with code 130 (SIGINT
///   convention).
///
/// Returns the same `Arc<AtomicBool>` for the caller to thread
/// through to the facade. Errors only on duplicate install; first
/// caller wins.
pub fn install_sigint_cancel() -> anyhow::Result<Arc<AtomicBool>> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_handler = cancel.clone();
    // Per-process count of received SIGINTs. Static so the closure
    // owns no extra state; first signal flips cancel, second exits.
    //
    // Process-lifetime: never reset. ctrlc::set_handler rejects
    // multi-install with `Err(MultipleHandlers)`, so this counter
    // is effectively single-use per `kebab` invocation. A future
    // command that needs its own cancel token (e.g. `kebab eval
    // run --with-cancel`) must factor the install path into a
    // helper that takes the token as an arg and shares it across
    // callers — not call `install_sigint_cancel` twice.
    static SIGNAL_COUNT: AtomicU8 = AtomicU8::new(0);
    ctrlc::set_handler(move || {
        let prev = SIGNAL_COUNT.fetch_add(1, Ordering::Relaxed);
        if prev == 0 {
            cancel_for_handler.store(true, Ordering::Relaxed);
            // Helpful hint on stderr — the run loop will surface
            // its own "aborting…" line once the cancel propagates.
            let _ = std::io::Write::write_all(
                &mut std::io::stderr().lock(),
                b"\nreceived Ctrl-C; aborting after current asset (press again to force quit)\n",
            );
        } else {
            // Second signal → bail. 130 is the canonical SIGINT
            // exit code (128 + signal number).
            std::process::exit(130);
        }
    })?;
    Ok(cancel)
}

#[cfg(test)]
mod tests {
    // The handler is process-global and can only be installed once
    // per binary invocation (ctrlc constraint), so unit-testing the
    // happy path here is brittle — see `tests/ingest_cancel_cli.rs`
    // for the integration coverage that runs the bin in a fresh
    // subprocess.

    #[test]
    fn cancel_module_compiles() {
        // Trivial sanity — confirm the module compiles in dev profile
        // (the install function is exercised by the CLI integration
        // test, not directly here).
    }
}
