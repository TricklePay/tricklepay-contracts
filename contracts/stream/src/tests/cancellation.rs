#![cfg(test)]

use proptest::prelude::*;
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{Event, Vec};

use crate::events::Cancelled;
use crate::storage::{BUMP_THRESHOLD, ENTRY_TTL};
use crate::vesting::Settlement;
use crate::{StreamError, StreamStatus, MAX_AMOUNT};

use super::helpers::StreamTest;

#[test]
fn cancel_preview_matches_cancel_without_writing_or_publishing() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.set_time(600);
    t.set_sequence(ENTRY_TTL - BUMP_THRESHOLD + 1);
    let ttl_before = t.stream_ttl(id);
    let stream_before = t.persistent_stream(&crate::storage::DataKey::Stream(id));
    let preview = t.contract.cancel_preview(&id, &600);

    assert_eq!(
        preview,
        Settlement {
            refund: 500,
            recipient_remaining: 500,
        }
    );
    assert_eq!(t.stream_ttl(id), ttl_before);
    assert_eq!(
        t.persistent_stream(&crate::storage::DataKey::Stream(id)),
        stream_before
    );
    assert!(t.event_publishers().is_empty());
    assert_eq!(
        t.contract.try_cancel_preview(&99, &600),
        Err(Ok(StreamError::StreamNotFound))
    );

    assert_eq!(t.contract.cancel(&id), preview.refund);
    assert_eq!(
        t.contract.try_cancel_preview(&id, &600),
        Err(Ok(StreamError::AlreadyCancelled))
    );

    let completed_id = t.open_default_stream(1_000);
    t.set_time(1_100);
    assert_eq!(
        t.contract.try_cancel_preview(&completed_id, &1_100),
        Err(Ok(StreamError::StreamAlreadyCompleted))
    );
}

#[test]
fn cancel_preview_before_start_time_yields_full_refund() {
    let t = StreamTest::setup(1_000);
    t.set_time(50);

    // Create stream starting in the future at 200
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &200,
        &1_200,
        &200,
    );

    // Clock is at 100, which is strictly before start_time (200)
    t.set_time(100);

    // 1. Preview at timestamp 100 (before start)
    let preview = t.contract.cancel_preview(&id, &100);
    assert_eq!(
        preview,
        Settlement {
            refund: 1_000,
            recipient_remaining: 0,
        }
    );
    // Preview publishes no events
    assert!(t.event_publishers().is_empty());

    // 2. Real cancel at the same instant
    let sender_balance_before = t.token.balance(&t.sender);
    let refund = t.contract.cancel(&id);
    assert_eq!(refund, preview.refund);
    assert_eq!(refund, 1_000);
    assert_eq!(
        t.token.balance(&t.sender) - sender_balance_before,
        preview.refund
    );
    assert_eq!(t.token.balance(&t.recipient), 0);
    assert_eq!(t.contract.withdrawable(&id), preview.recipient_remaining);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

#[test]
fn cancel_preview_before_unreached_cliff_yields_full_refund() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    // Stream with cliff at 600
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &600,
    );

    // Advance to 400: past start (100) but before cliff (600).
    // Even though 30% of duration elapsed, nothing has vested.
    t.set_time(400);

    // 1. Preview at timestamp 400 (before cliff)
    let preview = t.contract.cancel_preview(&id, &400);
    assert_eq!(
        preview,
        Settlement {
            refund: 1_000,
            recipient_remaining: 0,
        }
    );
    // Preview publishes no events
    assert!(t.event_publishers().is_empty());

    // 2. Real cancel at the same instant
    let sender_balance_before = t.token.balance(&t.sender);
    let refund = t.contract.cancel(&id);
    assert_eq!(refund, preview.refund);
    assert_eq!(refund, 1_000);
    assert_eq!(
        t.token.balance(&t.sender) - sender_balance_before,
        preview.refund
    );
    assert_eq!(t.token.balance(&t.recipient), 0);
    assert_eq!(t.contract.withdrawable(&id), preview.recipient_remaining);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// New test: cancel_preview at a custom timestamp differs from current ledger time.
#[test]
fn cancel_preview_at_custom_timestamp_independent_of_current_time() {
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

    // Current ledger time: 150 (early in the stream, so 50/1000 would vest).
    t.set_time(150);
    let current_vested = t.contract.vested(&id);
    assert_eq!(current_vested, 50);

    // But preview what the settlement would be at timestamp 600 (midpoint).
    let preview_at_600 = t.contract.cancel_preview(&id, &600);
    assert_eq!(
        preview_at_600,
        Settlement {
            refund: 500,
            recipient_remaining: 500,
        }
    );

    // Also preview at an even later timestamp: 850 (three quarters).
    let preview_at_850 = t.contract.cancel_preview(&id, &850);
    assert_eq!(
        preview_at_850,
        Settlement {
            refund: 250,
            recipient_remaining: 750,
        }
    );

    // Current ledger state (time=150) should be unchanged.
    assert_eq!(t.contract.vested(&id), current_vested);
    assert!(t.event_publishers().is_empty());
}

#[test]
fn cancel_preview_after_partial_withdrawal_deducts_withdrawn_from_recipient_remaining() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let id = t.open_default_stream(1_000);

    // Midpoint: 500 vested. Recipient withdraws 200.
    t.set_time(600);
    let withdrawn = t.contract.withdraw_amount(&id, &200);
    assert_eq!(withdrawn, 200);
    assert_eq!(t.token.balance(&t.recipient), 200);

    // 1. Preview at timestamp 600 after 200 has been withdrawn:
    // Total: 1000. Vested: 500. Withdrawn: 200.
    // Unvested refund: 1000 - 500 = 500.
    // Recipient remaining: 500 - 200 = 300.
    let preview = t.contract.cancel_preview(&id, &600);
    assert_eq!(
        preview,
        Settlement {
            refund: 500,
            recipient_remaining: 300,
        }
    );
    // Preview publishes no events
    assert!(t.event_publishers().is_empty());

    // 2. Real cancel at the same instant
    let sender_balance_before = t.token.balance(&t.sender);
    let refund = t.contract.cancel(&id);
    assert_eq!(refund, preview.refund);
    assert_eq!(refund, 500);
    assert_eq!(
        t.token.balance(&t.sender) - sender_balance_before,
        preview.refund
    );

    // Recipient still has exactly recipient_remaining available
    assert_eq!(t.contract.withdrawable(&id), preview.recipient_remaining);
    assert_eq!(t.contract.withdrawable(&id), 300);

    // Recipient can withdraw the remaining 300
    let second_withdrawn = t.contract.withdraw(&id);
    assert_eq!(second_withdrawn, preview.recipient_remaining);
    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

#[test]
fn cancel_preview_matches_cancel_after_midstream_topup() {
    let t = StreamTest::setup(5_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    // At midpoint (t = 600), withdraw 200 and then top up by 1,000.
    // Total is now 2,000; vested at 600 is 1,000.
    // Expected settlement: refund = 2,000 - 1,000 = 1,000,
    // recipient_remaining = 1,000 - 200 = 800.
    t.set_time(600);
    assert_eq!(t.contract.withdraw_amount(&id, &200), 200);
    t.contract.topup_stream(&id, &1_000);

    let sender_balance_before = t.token.balance(&t.sender);
    let recipient_balance_before = t.token.balance(&t.recipient);
    let contract_balance_before = t.token.balance(&t.contract.address);

    let preview = t.contract.cancel_preview(&id);
    assert_eq!(
        preview,
        Settlement {
            refund: 1_000,
            recipient_remaining: 800,
        }
    );

    // Preview does not change any balance or publish events.
    assert_eq!(t.token.balance(&t.sender), sender_balance_before);
    assert_eq!(t.token.balance(&t.recipient), recipient_balance_before);
    assert_eq!(
        t.token.balance(&t.contract.address),
        contract_balance_before
    );
    assert!(t.event_publishers().is_empty());

    let refund = t.contract.cancel(&id);
    assert_eq!(refund, preview.refund);
    assert_eq!(
        t.token.balance(&t.sender) - sender_balance_before,
        preview.refund
    );
    assert_eq!(t.contract.withdrawable(&id), preview.recipient_remaining);

    assert_eq!(t.contract.withdraw(&id), preview.recipient_remaining);
    assert_eq!(
        t.token.balance(&t.recipient) - recipient_balance_before,
        preview.recipient_remaining
    );
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

#[test]
fn cancel_preview_matches_cancel_after_topup_before_start_time() {
    let t = StreamTest::setup(5_000);
    t.set_time(50);

    // Create stream starting in the future at 200.
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &200,
        &1_200,
        &200,
    );

    // Top up before start_time (at t = 100). Total becomes 1,500.
    t.set_time(100);
    t.contract.topup_stream(&id, &500);

    let sender_balance_before = t.token.balance(&t.sender);
    let recipient_balance_before = t.token.balance(&t.recipient);
    let contract_balance_before = t.token.balance(&t.contract.address);

    let preview = t.contract.cancel_preview(&id);
    assert_eq!(
        preview,
        Settlement {
            refund: 1_500,
            recipient_remaining: 0,
        }
    );

    // Preview does not change any balance or publish events.
    assert_eq!(t.token.balance(&t.sender), sender_balance_before);
    assert_eq!(t.token.balance(&t.recipient), recipient_balance_before);
    assert_eq!(
        t.token.balance(&t.contract.address),
        contract_balance_before
    );
    assert!(t.event_publishers().is_empty());

    let refund = t.contract.cancel(&id);
    assert_eq!(refund, preview.refund);
    assert_eq!(
        t.token.balance(&t.sender) - sender_balance_before,
        preview.refund
    );
    assert_eq!(t.contract.withdrawable(&id), preview.recipient_remaining);
    assert_eq!(t.token.balance(&t.recipient), recipient_balance_before);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

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

proptest! {
    /// Issue #624 — `cancel_preview` returns the exact settlement produced by `cancel`,
    /// leaves stored stream records and token balances untouched, matches the emitted
    /// `Cancelled` event values, and returns the same errors as `cancel` for completed
    /// and already-cancelled streams.
    #[test]
    fn cancel_preview_equals_cancel_and_leaves_state_untouched(
        total in 100i128..=MAX_AMOUNT,
        start in 0u64..=1_000_000u64,
        duration in 1u64..=1_000_000u64,
        cliff_offset in 0u64..=1_000_000u64,
        cancel_time in 0u64..=2_500_000u64,
        withdraw_before_cancel in prop::bool::ANY,
    ) {
        let end = start + duration;
        let cliff = start + cliff_offset.min(duration);

        let t = StreamTest::setup(total);
        t.set_time(0);
        let id = t.contract.create_stream(
            &t.sender,
            &t.recipient,
            &t.token_address,
            &total,
            &start,
            &end,
            &cliff,
        );

        // Optionally perform a withdrawal prior to cancellation if eligible.
        if withdraw_before_cancel {
            let withdraw_time = (start + cancel_time) / 2;
            if withdraw_time >= cliff && withdraw_time < cancel_time && withdraw_time < end {
                t.set_time(withdraw_time);
                let avail = t.contract.withdrawable(&id);
                if avail > 0 {
                    t.contract.withdraw(&id);
                }
            }
        }

        t.set_time(cancel_time);

        if cancel_time >= end {
            // Completed streams: both cancel_preview and cancel return StreamAlreadyCompleted.
            prop_assert_eq!(
                t.contract.try_cancel_preview(&id, &cancel_time),
                Err(Ok(StreamError::StreamAlreadyCompleted))
            );
            prop_assert_eq!(
                t.contract.try_cancel(&id),
                Err(Ok(StreamError::StreamAlreadyCompleted))
            );
        } else {
            // Active stream:
            // 1. Snapshot state before calling cancel_preview.
            let stream_before = t.contract.get_stream(&id);
            let sender_bal_before = t.token.balance(&t.sender);
            let recipient_bal_before = t.token.balance(&t.recipient);
            let contract_bal_before = t.token.balance(&t.contract.address);
            let events_count_before = t.env.events().all().events().len();

            // 2. Call cancel_preview with the specified cancel_time.
            let preview = t.contract.cancel_preview(&id, &cancel_time);

            // 3. Assert cancel_preview changes no state: stream record, balances, and events are untouched.
            prop_assert_eq!(t.contract.get_stream(&id), stream_before.clone());
            prop_assert_eq!(t.token.balance(&t.sender), sender_bal_before);
            prop_assert_eq!(t.token.balance(&t.recipient), recipient_bal_before);
            prop_assert_eq!(t.token.balance(&t.contract.address), contract_bal_before);
            prop_assert_eq!(t.env.events().all().events().len(), events_count_before);

            // 4. Call cancel and assert returned refund matches preview.refund.
            let actual_refund = t.contract.cancel(&id);
            prop_assert_eq!(actual_refund, preview.refund);

            // 5. Assert emitted Cancelled event matches preview refund and recipient amounts.
            t.assert_latest_stream_event_topics(
                Cancelled {
                    sender: t.sender.clone(),
                    id,
                    recipient_amount: preview.recipient_remaining,
                    sender_refund: preview.refund,
                }
                .to_xdr(&t.env, &t.contract.address),
            );

            // 6. Assert token balances updated exactly according to preview.
            prop_assert_eq!(
                t.token.balance(&t.sender),
                sender_bal_before + preview.refund
            );
            prop_assert_eq!(
                t.token.balance(&t.contract.address),
                contract_bal_before - preview.refund
            );
            prop_assert_eq!(
                preview.refund + preview.recipient_remaining,
                total - stream_before.withdrawn
            );

            // 7. Cancelled streams: both cancel_preview and cancel return AlreadyCancelled.
            prop_assert_eq!(
                t.contract.try_cancel_preview(&id, &cancel_time),
                Err(Ok(StreamError::AlreadyCancelled))
            );
            prop_assert_eq!(
                t.contract.try_cancel(&id),
                Err(Ok(StreamError::AlreadyCancelled))
            );
        }
    }
}
