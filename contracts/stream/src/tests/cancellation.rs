#![cfg(test)]

use soroban_sdk::{Event, Vec};

use crate::events::Cancelled;
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

    // The sender gets the unvested half back immediately.
    assert_eq!(t.token.balance(&t.sender), 500);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);

    // The recipient's vested half stays claimable, even much later.
    t.set_time(2_000);
    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.withdraw(&id), 500);
    assert_eq!(t.token.balance(&t.recipient), 500);

    // The split adds up to the original total and the contract is drained.
    assert_eq!(t.token.balance(&t.contract.address), 0);

    // A stream cannot be cancelled twice.
    assert_eq!(
        t.contract.try_cancel(&id),
        Err(Ok(StreamError::AlreadyCancelled))
    );
    assert_eq!(t.event_publishers(), Vec::new(&t.env));
}

/// Cancel a stream the recipient has already partially withdrawn from.
///
/// The recipient keeps what they took, the sender gets back only the still
/// unvested remainder, and the withdrawn balance survives into the frozen
/// record so the same tokens can never be refunded twice. This pins the
/// interaction between `withdraw_amount` and `cancel`; a change to vesting,
/// authorization, or lifecycle accounting that breaks it fails here.
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

    // Midpoint: 500 vested, 500 still locked. The recipient takes 200 of the
    // vested half and leaves 300 behind.
    t.set_time(600);
    let withdrawn = t.contract.withdraw_amount(&id, &200);
    assert_eq!(withdrawn, 200);
    assert_eq!(t.token.balance(&t.recipient), withdrawn);

    // The sender cancels. The refund is the unvested half; the 200 already
    // withdrawn is not double-refunded to the sender.
    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 500);
    assert_eq!(t.token.balance(&t.sender), refund);
    assert_eq!(t.token.balance(&t.contract.address), 300);

    // The stored stream is frozen at the vested amount with the prior
    // withdrawal still accounted for.
    let stream = t.contract.get_stream(&id);
    assert!(stream.cancelled);
    assert_eq!(stream.total_amount, 500);
    assert_eq!(stream.withdrawn, 200);
    assert_eq!(stream.end_time, 600);

    // The recipient can still claim the rest of the vested balance; together
    // with the 200 already taken that is the full vested 500, the split adds
    // up to the original total, and the contract is drained.
    assert_eq!(t.contract.withdrawable(&id), 300);
    let remaining_withdrawal = t.contract.withdraw(&id);
    assert_eq!(remaining_withdrawal, 300);
    assert_eq!(
        t.token.balance(&t.recipient),
        withdrawn + remaining_withdrawal
    );
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

#[test]
fn cancel_pending_stream_refunds_full_deposit() {
    let t = StreamTest::setup(1_000);
    let now = 50;
    t.set_time(now);
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

    let refund = t.contract.cancel(&id);

    assert_eq!(refund, 1_000);
    t.assert_latest_stream_event_topics(
        Cancelled {
            sender: t.sender.clone(),
            id,
            recipient_amount: 0,
            sender_refund: 1_000,
        }
        .to_xdr(&t.env, &t.contract.address),
    );

    assert_eq!(t.token.balance(&t.sender), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);
    assert_eq!(t.token.balance(&t.recipient), 0);

    let stream = t.contract.get_stream(&id);
    assert!(stream.cancelled);
    assert_eq!(stream.start_time, now);
    assert_eq!(stream.cliff_time, now);
    assert_eq!(stream.end_time, now);

    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(t.contract.progress(&id), 10_000);
}

/// Cancelling at the exact start leaves no elapsed time to vest, so the full
/// deposit returns to the sender and nothing is claimable by the recipient.
#[test]
fn cancel_at_start_time_refunds_the_full_amount() {
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

    // No time has elapsed at start_time, so none of the deposit has vested.
    assert_eq!(t.contract.cancel(&id), 1_000);
    assert_eq!(t.token.balance(&t.sender), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);
    assert_eq!(t.token.balance(&t.recipient), 0);
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );
}

#[test]
fn cancel_millisecond_timestamp_stream_refunds_full_deposit() {
    let t = StreamTest::setup(1_000);
    t.set_time(1_700_000_000_000);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &1_700_000_001_000,
        &1_700_000_003_000,
        &1_700_000_002_000,
    );

    assert_eq!(t.token.balance(&t.sender), 0);
    assert_eq!(t.contract.cancel(&id), 1_000);
    assert_eq!(t.token.balance(&t.sender), 1_000);

    let stream = t.contract.get_stream(&id);
    assert!(stream.cancelled);
    assert_eq!(stream.total_amount, 0);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );
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

#[test]
fn second_withdraw_at_same_timestamp_is_rejected() {
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
    assert_eq!(t.contract.withdraw(&id), 500);
    let withdrawn_before = t.contract.get_stream(&id).withdrawn;

    // Withdrawing again with no time elapsed releases nothing.
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );
    assert_eq!(t.contract.get_stream(&id).withdrawn, withdrawn_before);
    assert_eq!(t.token.balance(&t.recipient), 500);
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
