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
