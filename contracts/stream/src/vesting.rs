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
/// overflow for extreme inputs, which the contract guards against by bounding
/// amounts at creation.
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
