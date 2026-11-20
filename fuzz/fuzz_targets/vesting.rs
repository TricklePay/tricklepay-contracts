#![no_main]

use libfuzzer_sys::fuzz_target;
use tricklepay_stream::vesting::{settlement, vested_amount, withdrawable_amount};

const MAX_AMOUNT: i128 = i64::MAX as i128;

fuzz_target!(|data: &[u8]| {
    // Requires at least 64 bytes for:
    // - total_amount (16 bytes)
    // - start_time (8 bytes)
    // - end_time (8 bytes)
    // - cliff_time (8 bytes)
    // - now (8 bytes)
    // - withdrawn (16 bytes)
    if data.len() < 64 {
        return;
    }

    let raw_total = i128::from_le_bytes(data[0..16].try_into().unwrap());
    let raw_start = u64::from_le_bytes(data[16..24].try_into().unwrap());
    let raw_end = u64::from_le_bytes(data[24..32].try_into().unwrap());
    let raw_cliff = u64::from_le_bytes(data[32..40].try_into().unwrap());
    let now = u64::from_le_bytes(data[40..48].try_into().unwrap());
    let raw_withdrawn = i128::from_le_bytes(data[48..64].try_into().unwrap());

    // Restrict inputs to values create_stream accepts:
    // 1. total_amount > 0 && total_amount <= MAX_AMOUNT
    let total_amount = (raw_total.abs() % MAX_AMOUNT) + 1;

    // 2. Schedule: start_time < end_time
    let start_time = raw_start % (u64::MAX - 1);
    let duration = (raw_end % (u64::MAX - start_time)).max(1);
    let end_time = start_time + duration;

    // 3. cliff_time >= start_time && cliff_time <= end_time
    let cliff_offset = raw_cliff % (duration + 1);
    let cliff_time = start_time + cliff_offset;

    // Invariants enforced by create_stream:
    assert!(total_amount > 0 && total_amount <= MAX_AMOUNT);
    assert!(start_time < end_time);
    assert!(cliff_time >= start_time && cliff_time <= end_time);

    // Call vested_amount
    let vested = vested_amount(total_amount, start_time, end_time, cliff_time, now);

    // Invariants for vested_amount:
    assert!(
        vested >= 0,
        "vested_amount returned negative value: {vested}"
    );
    assert!(
        vested <= total_amount,
        "vested_amount exceeded total_amount: {vested} > {total_amount}"
    );

    if now < cliff_time || now < start_time {
        assert_eq!(vested, 0, "vested before cliff or start must be 0");
    }
    if now >= end_time {
        assert_eq!(
            vested, total_amount,
            "vested at or after end must equal total_amount"
        );
    }

    // Call withdrawable_amount for bounded withdrawn amounts
    let withdrawn = raw_withdrawn.abs() % (total_amount + 1);
    let withdrawable = withdrawable_amount(vested, withdrawn);

    assert!(
        withdrawable >= 0,
        "withdrawable_amount returned negative value: {withdrawable}"
    );
    assert!(
        withdrawable <= vested,
        "withdrawable_amount exceeded vested: {withdrawable} > {vested}"
    );

    if withdrawn <= vested {
        assert_eq!(
            withdrawable,
            vested - withdrawn,
            "withdrawable must equal vested - withdrawn when withdrawn <= vested"
        );
    } else {
        assert_eq!(
            withdrawable, 0,
            "withdrawable must be 0 when withdrawn > vested"
        );
    }

    // Call settlement
    let s = settlement(total_amount, vested, withdrawn);
    assert_eq!(
        s.refund,
        total_amount - vested,
        "settlement refund must equal total_amount - vested"
    );
    assert_eq!(
        s.recipient_remaining,
        vested - withdrawn,
        "settlement recipient_remaining must equal vested - withdrawn"
    );

    if withdrawn <= vested {
        assert_eq!(
            s.refund + s.recipient_remaining,
            total_amount - withdrawn,
            "settlement must conserve unwithdrawn escrow"
        );
    }
});
