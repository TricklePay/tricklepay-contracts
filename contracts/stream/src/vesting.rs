//! Pure vesting calculations.
//!
//! These functions take plain numbers rather than a [`Stream`] so they can be
//! reasoned about and tested without an environment or storage. The contract
//! layer is responsible for loading a stream and feeding its fields in.
//!
//! [`Stream`]: crate::types::Stream

/// Amount vested at `now` for a stream that runs linearly from `start_time`
/// to `end_time`, with nothing available before `cliff_time`.
///
/// The result is clamped to the closed range `[0, total_amount]`:
/// - before the cliff or start, nothing has vested;
/// - at or after the end time, the full amount has vested;
/// - in between, the amount grows linearly with elapsed time.
///
/// Realistic token amounts and durations stay well inside `i128`. The
/// intermediate `total_amount * elapsed` product is the one place that could
/// overflow for extreme inputs; `create_stream` guards against this by
/// rejecting any `total_amount` above `i64::MAX`. Because `elapsed` is
/// bounded by `u64::MAX`, the product `i64::MAX * u64::MAX` fits inside
/// `i128::MAX` and the multiplication is unconditionally safe.
///
/// # Examples
///
/// ```
/// use tricklepay_stream::vesting::vested_amount;
///
/// let total = 1000;
/// let start = 100;
/// let end = 1100;
/// let cliff = 600;
///
/// // Before the cliff, nothing has vested.
/// assert_eq!(vested_amount(total, start, end, cliff, 300), 0);
///
/// // At or after the end time, the full amount has vested.
/// assert_eq!(vested_amount(total, start, end, cliff, 1100), 1000);
/// ```
pub fn vested_amount(
    total_amount: i128,
    start_time: u64,
    end_time: u64,
    cliff_time: u64,
    now: u64,
) -> i128 {
    if now < cliff_time || now < start_time {
        return 0;
    }
    if now >= end_time {
        return total_amount;
    }
    let elapsed = (now - start_time) as i128;
    let duration = (end_time - start_time) as i128;
    total_amount * elapsed / duration
}

/// Amount the recipient can withdraw right now: whatever has vested minus
/// whatever has already been taken. Never negative.
///
/// # Examples
///
/// ```
/// use tricklepay_stream::vesting::withdrawable_amount;
///
/// assert_eq!(withdrawable_amount(500, 200), 300);
/// assert_eq!(withdrawable_amount(200, 500), 0);
/// ```
pub fn withdrawable_amount(vested: i128, withdrawn: i128) -> i128 {
    let available = vested - withdrawn;
    if available < 0 {
        0
    } else {
        available
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A reference stream: 1000 units over [100, 1100], no cliff (cliff == start).
    const TOTAL: i128 = 1_000;
    const START: u64 = 100;
    const END: u64 = 1_100;

    #[test]
    fn nothing_vests_before_start() {
        assert_eq!(vested_amount(TOTAL, START, END, START, 50), 0);
    }

    #[test]
    fn nothing_vests_before_cliff() {
        // Past the start but before a cliff set at the midpoint.
        assert_eq!(vested_amount(TOTAL, START, END, 600, 300), 0);
    }

    #[test]
    fn half_vests_at_midpoint() {
        assert_eq!(vested_amount(TOTAL, START, END, START, 600), 500);
    }

    #[test]
    fn quarter_vests_at_quarter_point() {
        assert_eq!(vested_amount(TOTAL, START, END, START, 350), 250);
    }

    #[test]
    fn full_amount_vests_at_end() {
        assert_eq!(vested_amount(TOTAL, START, END, START, END), TOTAL);
    }

    #[test]
    fn full_amount_vests_after_end() {
        assert_eq!(vested_amount(TOTAL, START, END, START, 9_999), TOTAL);
    }

    #[test]
    fn cliff_releases_accrued_amount_at_once() {
        // At the cliff, the linearly accrued amount since start becomes
        // available in one step: 500 of 1000 at the midpoint.
        assert_eq!(vested_amount(TOTAL, START, END, 600, 600), 500);
    }

    #[test]
    fn three_quarters_vest_at_the_three_quarter_point() {
        assert_eq!(vested_amount(TOTAL, START, END, START, 850), 750);
    }

    /// `cliff == start` is how a caller expresses "no cliff". It is not
    /// special-cased: the cliff gate collapses into the start check, so it
    /// behaves exactly like a cliff that has already passed.
    #[test]
    fn cliff_equal_to_start_behaves_as_no_cliff() {
        for now in [50, 100, 350, 600, 850, END, 9_999] {
            assert_eq!(
                vested_amount(TOTAL, START, END, START, now),
                vested_amount(TOTAL, START, END, 0, now),
                "cliff == start must match a cliff already in the past"
            );
        }
    }

    /// The two schedules documented in the README. A cliff changes neither the
    /// rate nor the total; it withholds the earlier portion and then releases
    /// it in one step, after which the schedules are identical.
    #[test]
    fn cliff_schedule_matches_the_readme_example() {
        const CLIFF: u64 = 600;

        assert_eq!(vested_amount(TOTAL, START, END, CLIFF, 300), 0);
        assert_eq!(vested_amount(TOTAL, START, END, CLIFF, CLIFF), 500);
        assert_eq!(vested_amount(TOTAL, START, END, CLIFF, 850), 750);
        assert_eq!(vested_amount(TOTAL, START, END, CLIFF, END), TOTAL);

        for now in [CLIFF, 850, END] {
            assert_eq!(
                vested_amount(TOTAL, START, END, CLIFF, now),
                vested_amount(TOTAL, START, END, START, now),
                "from the cliff onward both schedules must agree"
            );
        }
    }

    /// A cliff at the end time is a pure lockup: nothing vests until the
    /// window closes, then the whole amount lands at once.
    #[test]
    fn cliff_at_the_end_withholds_everything_until_the_window_closes() {
        assert_eq!(vested_amount(TOTAL, START, END, END, END - 1), 0);
        assert_eq!(vested_amount(TOTAL, START, END, END, END), TOTAL);
    }

    #[test]
    fn integer_division_rounds_down() {
        // 10 * 1 / 3 = 3.33, truncated to 3.
        assert_eq!(vested_amount(10, 0, 3, 0, 1), 3);
    }

    #[test]
    fn withdrawable_subtracts_withdrawn() {
        assert_eq!(withdrawable_amount(500, 200), 300);
    }

    #[test]
    fn withdrawable_is_never_negative() {
        assert_eq!(withdrawable_amount(200, 500), 0);
    }

    #[test]
    fn withdrawable_is_zero_when_fully_taken() {
        assert_eq!(withdrawable_amount(300, 300), 0);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    prop_compose! {
        fn valid_stream_params()(
            total_amount in 0i128..=i64::MAX as i128,
            start_time in 0u64..=u64::MAX / 2,
            duration in 1u64..=u64::MAX / 2,
            cliff_offset in 0u64..=u64::MAX / 2,
        ) -> (i128, u64, u64, u64) {
            let end_time = start_time + duration;
            let cliff_time = start_time + cliff_offset.min(duration);
            (total_amount, start_time, end_time, cliff_time)
        }
    }

    proptest! {
        #[test]
        fn vested_between_zero_and_total(
            (total, start, end, cliff) in valid_stream_params(),
            now in any::<u64>(),
        ) {
            let v = vested_amount(total, start, end, cliff, now);
            prop_assert!(v >= 0, "vested must be >= 0");
            prop_assert!(v <= total, "vested must be <= total");
        }

        #[test]
        fn vested_at_end_equals_total(
            (total, start, end, cliff) in valid_stream_params(),
        ) {
            prop_assert_eq!(vested_amount(total, start, end, cliff, end), total);
        }

        #[test]
        fn vested_is_monotonic_in_now(
            (total, start, end, cliff) in valid_stream_params(),
            now_a in any::<u64>(),
            now_b in any::<u64>(),
        ) {
            let (a, b) = if now_a <= now_b {
                (now_a, now_b)
            } else {
                (now_b, now_a)
            };
            let va = vested_amount(total, start, end, cliff, a);
            let vb = vested_amount(total, start, end, cliff, b);
            prop_assert!(va <= vb, "vested must be monotonic: va={} vb={} a={} b={}", va, vb, a, b);
        }

        #[test]
        fn withdrawable_between_zero_and_vested(
            vested in 0i128..=i64::MAX as i128,
            withdrawn in 0i128..=i64::MAX as i128,
        ) {
            let w = withdrawable_amount(vested, withdrawn);
            prop_assert!(w >= 0, "withdrawable must be >= 0");
            prop_assert!(w <= vested, "withdrawable must be <= vested");
        }

        #[test]
        fn withdrawable_equals_vested_minus_withdrawn_when_withdrawn_le_vested(
            vested in 0i128..=i64::MAX as i128,
            withdrawn in 0i128..=i64::MAX as i128,
        ) {
            let (v, wd) = if withdrawn <= vested {
                (vested, withdrawn)
            } else {
                (withdrawn, vested)
            };
            prop_assert_eq!(withdrawable_amount(v, wd), v - wd);
        }
    }
}
