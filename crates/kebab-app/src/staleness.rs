//! p9-fb-32 staleness helpers.

use time::{Duration, OffsetDateTime};

use kebab_core::SearchHit;

/// Returns `true` iff `now - indexed_at > threshold_days * 24h`.
/// `threshold_days = 0` always returns `false` (feature disabled).
/// Strict `>` so that exactly `threshold_days` old returns `false`.
pub fn compute_stale(
    indexed_at: OffsetDateTime,
    now: OffsetDateTime,
    threshold_days: u32,
) -> bool {
    if threshold_days == 0 {
        return false;
    }
    let threshold = Duration::days(i64::from(threshold_days));
    (now - indexed_at) > threshold
}

/// Sets `stale` on each hit in place using `compute_stale`.
pub fn mark_stale_in_place(
    hits: &mut [SearchHit],
    now: OffsetDateTime,
    threshold_days: u32,
) {
    for h in hits {
        h.stale = compute_stale(h.indexed_at, now, threshold_days);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn now() -> OffsetDateTime {
        datetime!(2026-05-09 12:00:00 UTC)
    }

    #[test]
    fn threshold_zero_always_fresh() {
        let very_old = datetime!(2020-01-01 00:00:00 UTC);
        assert!(!compute_stale(very_old, now(), 0));
    }

    #[test]
    fn just_under_threshold_is_fresh() {
        // 29 days, 23h, 59m old — under 30d.
        let indexed = now() - Duration::days(29) - Duration::hours(23) - Duration::minutes(59);
        assert!(!compute_stale(indexed, now(), 30));
    }

    #[test]
    fn exactly_threshold_is_fresh() {
        // strict `>` boundary: exactly 30d old is still fresh.
        let indexed = now() - Duration::days(30);
        assert!(!compute_stale(indexed, now(), 30));
    }

    #[test]
    fn one_minute_past_threshold_is_stale() {
        let indexed = now() - Duration::days(30) - Duration::minutes(1);
        assert!(compute_stale(indexed, now(), 30));
    }

    #[test]
    fn future_indexed_at_is_fresh() {
        // clock skew safety: future timestamps must not be stale.
        let future = now() + Duration::hours(1);
        assert!(!compute_stale(future, now(), 30));
    }
}
