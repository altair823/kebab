//! Thread-cap test (spec §7). Own integration binary → clean process, so the
//! one-shot global rayon pool is initialized exactly once, by us.
//!
//! Verifies that `apply_thread_cap(4)` sizes the global rayon pool to 4, which
//! is the lever that keeps candle's CPU backend NUMA-safe (vs onnxruntime's
//! hard-coded 48 intra-op threads).

use kebab_embed_candle::apply_thread_cap;

#[test]
fn thread_cap_sizes_global_rayon_pool() {
    // Must run before any other rayon use in this process. As the only test in
    // this binary that touches rayon, that holds.
    let applied = apply_thread_cap(4);
    assert!(applied, "first build_global call should succeed");
    assert_eq!(
        rayon::current_num_threads(),
        4,
        "global rayon pool must be capped at the requested 4 threads"
    );

    // A second cap attempt is a no-op (pool already built), not a panic.
    assert!(
        !apply_thread_cap(8),
        "second build_global must report not-applied"
    );
    assert_eq!(
        rayon::current_num_threads(),
        4,
        "thread count must stay at the first cap"
    );
}
