#![cfg(test)]

use crate::{StreamError, StreamStatus};

use super::helpers::StreamTest;

#[test]
fn cancel_refunds_unvested_and_preserves_vested() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    // Halfway through: 500 vested, 500 still locked.
    t.set_time(600);
    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 500);

    assert_eq!(t.token.balance(&t.sender), 500);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);

    // The recipient's vested half stays claimable, even much later.
    t.set_time(2_000);
    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.withdraw(&id), 500);
    assert_eq!(t.token.balance(&t.recipient), 500);

    assert_eq!(t.token.balance(&t.contract.address), 0);

    assert_eq!(
        t.contract.try_cancel(&id),
        Err(Ok(StreamError::AlreadyCancelled))
    );
}

/// Cancel a stream the recipient has already partially withdrawn from.
#[test]
fn cancel_after_partial_withdrawal() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    t.set_time(600);
    assert_eq!(t.contract.withdraw_amount(&id, &200), 200);
    assert_eq!(t.token.balance(&t.recipient), 200);

    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 500);
    assert_eq!(t.token.balance(&t.sender), 500);
    assert_eq!(t.token.balance(&t.contract.address), 300);

    let stream = t.contract.get_stream(&id);
    assert!(stream.cancelled);
    assert_eq!(stream.total_amount, 500);
    assert_eq!(stream.withdrawn, 200);
    assert_eq!(stream.end_time, 600);

    assert_eq!(t.contract.withdrawable(&id), 300);
    assert_eq!(t.contract.withdraw(&id), 300);
    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// Cancel in the last instant still allowed: one second before `end_time`.
#[test]
fn cancel_immediately_before_end_time() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    t.set_time(1_099);
    assert_eq!(t.contract.status(&id), StreamStatus::Streaming);

    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 1);
    assert_eq!(t.token.balance(&t.sender), 1);
    assert_eq!(t.token.balance(&t.contract.address), 999);

    let stream = t.contract.get_stream(&id);
    assert!(stream.cancelled);
    assert_eq!(stream.total_amount, 999);
    assert_eq!(stream.withdrawn, 0);
    assert_eq!(stream.end_time, 1_099);

    assert_eq!(t.contract.withdrawable(&id), 999);
    assert_eq!(t.contract.withdraw(&id), 999);
    assert_eq!(t.token.balance(&t.recipient), 999);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// Cancel the instant a stream has started.
#[test]
fn cancel_immediately_after_start() {
    let t = StreamTest::setup(1_000);
    t.set_time(50);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(t.contract.status(&id), StreamStatus::Pending);

    t.set_time(101);
    assert_eq!(t.contract.status(&id), StreamStatus::Streaming);

    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 999);
    assert_eq!(t.token.balance(&t.sender), 999);
    assert_eq!(t.token.balance(&t.contract.address), 1);

    let stream = t.contract.get_stream(&id);
    assert!(stream.cancelled);
    assert_eq!(stream.total_amount, 1);
    assert_eq!(stream.withdrawn, 0);
    assert_eq!(stream.start_time, 100);
    assert_eq!(stream.cliff_time, 100);
    assert_eq!(stream.end_time, 101);

    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);
    assert_eq!(t.contract.withdrawable(&id), 1);
    assert_eq!(t.contract.withdraw(&id), 1);
    assert_eq!(t.token.balance(&t.recipient), 1);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// Cancel a stream before its cliff has been reached.
#[test]
fn cancel_with_cliff_not_reached() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &600,
    );

    t.set_time(400);
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );

    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 1_000);
    assert_eq!(t.token.balance(&t.sender), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);

    let stream = t.contract.get_stream(&id);
    assert!(stream.cancelled);
    assert_eq!(stream.total_amount, 0);
    assert_eq!(stream.withdrawn, 0);
    assert_eq!(stream.start_time, 100);
    assert_eq!(stream.cliff_time, 400);
    assert_eq!(stream.end_time, 400);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);

    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );
    assert_eq!(t.token.balance(&t.recipient), 0);
}

#[test]
fn cancel_on_stream_at_end_time_is_rejected() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    t.set_time(1_100);
    assert_eq!(t.contract.status(&id), StreamStatus::Completed);

    assert_eq!(
        t.contract.try_cancel(&id),
        Err(Ok(StreamError::StreamAlreadyCompleted))
    );

    assert_eq!(t.contract.status(&id), StreamStatus::Completed);
    assert_eq!(t.token.balance(&t.sender), 0);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);
}

#[test]
fn cancel_past_end_time_is_rejected_and_status_stays_completed() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    t.set_time(5_000);
    assert_eq!(t.contract.status(&id), StreamStatus::Completed);

    assert_eq!(
        t.contract.try_cancel(&id),
        Err(Ok(StreamError::StreamAlreadyCompleted))
    );

    assert_eq!(t.contract.status(&id), StreamStatus::Completed);

    assert_eq!(t.contract.withdrawable(&id), 1_000);
    assert_eq!(t.contract.withdraw(&id), 1_000);
    assert_eq!(t.token.balance(&t.recipient), 1_000);
}

// ── Vesting cancel scenarios ─────────────────────────────────────────────────

/// Issue #69 — Cancel at exact cliff.
#[test]
fn test_cancel_at_exact_cliff() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000i128,
        &100u64,
        &1_100u64,
        &600u64,
    );

    t.set_time(600);
    let refund = t.contract.cancel(&id);

    assert_eq!(refund, 500);
    assert_eq!(t.token.balance(&t.sender), 500);
    assert_eq!(t.token.balance(&t.contract.address), 500);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);

    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.vested(&id), 500);
    assert_eq!(t.contract.locked(&id), 0);
}

/// Issue #70 — Cancel refund rounding.
#[test]
fn test_cancel_refund_rounding() {
    let t = StreamTest::setup(10);
    t.set_time(0);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &10i128,
        &0u64,
        &3u64,
        &0u64,
    );

    t.set_time(1);
    let refund = t.contract.cancel(&id);

    // vested = floor(10 * 1 / 3) = 3; refund = 10 - 3 = 7
    assert_eq!(refund, 7);
    assert_eq!(t.token.balance(&t.sender), 7);
    assert_eq!(t.token.balance(&t.contract.address), 3);
    assert_eq!(t.contract.withdrawable(&id), 3);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);
}

/// Issue #72 — Repeated cancel failure.
#[test]
fn test_repeated_cancel_fails() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    t.set_time(600);
    assert_eq!(t.contract.status(&id), StreamStatus::Streaming);

    t.contract.cancel(&id);
    let auths = t.env.auths();

    assert!(
        auths.iter().any(|(addr, _)| addr == &t.sender),
        "cancel must require sender authorization"
    );
    assert!(
        !auths.iter().any(|(addr, _)| addr == &t.recipient),
        "cancel must not require or accept recipient authorization"
    );

    // A second cancel must be rejected.
    assert_eq!(
        t.contract.try_cancel(&id),
        Err(Ok(StreamError::AlreadyCancelled))
    );
}
