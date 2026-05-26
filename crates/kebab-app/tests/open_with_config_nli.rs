//! Tests for `App::open_with_config`'s NLI verifier construction path.
//!
//! Coverage:
//! 1. `open_with_config_nli_fails_when_model_dir_unwritable_and_threshold_positive` —
//!    when `rag.nli_threshold > 0` and `storage.model_dir` is unwritable,
//!    `open_with_config` returns `Err` with "OnnxNliVerifier" in the
//!    error chain.
//! 2. `open_with_config_nli_skipped_when_threshold_zero` —
//!    same bad `model_dir`, but `rag.nli_threshold = 0.0` (gate disabled),
//!    so `OnnxNliVerifier::new` is never called and `open_with_config`
//!    succeeds.
//!
//! `/proc/1/root` is the init process's filesystem root; on Linux it is
//! owned by root and not traversable by unprivileged users, making
//! `create_dir_all` fail with `EACCES` — a reliable "unwritable path"
//! that requires no test setup beyond the path literal.

use kebab_config::Config;

/// Return a `Config` whose `data_dir` lives in a fresh `TempDir`
/// (so `SqliteStore::open` succeeds) and whose `model_dir` is set to
/// `/proc/1/root` (unwritable by non-root processes on Linux).
///
/// The `TempDir` is returned alongside the config so the caller keeps
/// it alive until the test completes — dropping it early would delete
/// the data directory before any assertions run.
fn config_with_unwritable_model_dir() -> (tempfile::TempDir, Config) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut cfg = Config::defaults();
    // Valid data_dir → SqliteStore::open + run_migrations succeed.
    cfg.storage.data_dir = tmp.path().to_string_lossy().into_owned();
    // /proc/1/root is only accessible to root; create_dir_all will
    // return EACCES for any unprivileged user, which is exactly the
    // failure mode we want to exercise.
    cfg.storage.model_dir = "/proc/1/root".to_string();
    (tmp, cfg)
}

// ── 1. Failure path: threshold > 0 + unwritable model_dir ─────────────────

#[test]
fn open_with_config_nli_fails_when_model_dir_unwritable_and_threshold_positive() {
    let (_tmp, mut cfg) = config_with_unwritable_model_dir();
    cfg.rag.nli_threshold = 0.5; // gate enabled → OnnxNliVerifier::new runs

    let result = kebab_app::App::open_with_config(cfg);

    let Err(err) = result else {
        panic!(
            "App::open_with_config must fail when model_dir is unwritable and nli_threshold > 0"
        );
    };
    // The error chain must identify the OnnxNliVerifier as the source so
    // an operator reading logs can trace the failure to the NLI config.
    let err_chain = format!("{err:?}");
    assert!(
        err_chain.contains("OnnxNliVerifier"),
        "error chain must mention OnnxNliVerifier; full chain: {err_chain}"
    );
}

// ── 2. Success path: threshold = 0.0 → NLI verifier never constructed ──────

#[test]
fn open_with_config_nli_skipped_when_threshold_zero() {
    let (_tmp, cfg) = config_with_unwritable_model_dir();
    // Default nli_threshold is 0.0 — gate disabled, verifier skipped.
    assert!(
        (cfg.rag.nli_threshold - 0.0).abs() < f32::EPSILON,
        "precondition: default nli_threshold must be 0.0 (gate disabled)"
    );

    // A bad model_dir must NOT cause a failure when the NLI gate is off.
    let result = kebab_app::App::open_with_config(cfg);
    assert!(
        result.is_ok(),
        "App::open_with_config must succeed when nli_threshold = 0.0 \
         (OnnxNliVerifier is never constructed); err: {:?}",
        result.err()
    );
}
