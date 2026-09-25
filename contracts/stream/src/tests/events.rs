#![cfg(test)]

use soroban_sdk::vec;

use crate::events::{Cancelled, Created, Withdrawn};
use crate::StreamError;

use super::helpers::StreamTest;

// ── Event ordering ───────────────────────────────────────────────────────────
//
// Every state-changing entry point both moves tokens and publishes an event.
// The order of those two effects is part of the contract's observable
// behaviour: an indexer that reacts to a stream event must be able to assume
// the corresponding transfer has already settled.

/// `create_stream` announces a stream only once the funds are in the contract.
#[test]
fn create_emits_created_after_the_funding_transfer() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    t.open_default_stream(1_000);

    assert_eq!(t.event_publishers(), t.transfer_then_announce());
}

/// `withdraw` pays the recipient before it announces.
#[test]
fn withdraw_emits_withdrawn_after_the_payout_transfer() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.set_time(600);
    assert_eq!(t.contract.withdraw(&id), 500);

    assert_eq!(t.event_publishers(), t.transfer_then_announce());
}

/// `withdraw_amount` follows the same order as `withdraw`.
#[test]
fn withdraw_amount_emits_withdrawn_after_the_payout_transfer() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.set_time(600);
    assert_eq!(t.contract.withdraw_amount(&id, &200), 200);

    assert_eq!(t.event_publishers(), t.transfer_then_announce());
}

/// `cancel` refunds the sender before announcing the cancellation.
#[test]
fn cancel_emits_cancelled_after_the_refund_transfer() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.set_time(600);
    assert_eq!(t.contract.cancel(&id), 500);

    assert_eq!(t.event_publishers(), t.transfer_then_announce());
}

/// Over a full lifecycle the contract's events arrive in operation order.
#[test]
fn lifecycle_events_follow_operation_order() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let id = t.open_default_stream(1_000);
    assert_eq!(t.event_publishers(), t.transfer_then_announce());

    t.set_time(600);
    t.contract.withdraw(&id);
    assert_eq!(t.event_publishers(), t.transfer_then_announce());

    t.set_time(700);
    t.contract.cancel(&id);
    assert_eq!(t.event_publishers(), t.transfer_then_announce());
}

// ── Event topic correctness ──────────────────────────────────────────────────

#[test]
fn created_event_topics_index_sender_and_recipient() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &750,
        &100,
        &1_100,
        &400,
    );

    assert_eq!(id, 0);
    t.assert_latest_stream_event_topics(
        Created {
            sender: t.sender.clone(),
            recipient: t.recipient.clone(),
            id,
            token: t.token_address.clone(),
            total_amount: 750,
            start_time: 100,
            end_time: 1_100,
            cliff_time: 400,
        }
        .to_xdr(&t.env, &t.contract.address),
    );

    let stream = t.contract.get_stream(&id);
    assert_eq!(stream.sender, t.sender);
    assert_eq!(stream.recipient, t.recipient);
    assert_eq!(stream.token, t.token_address);
    assert_eq!(t.token.balance(&t.contract.address), 750);
}

#[test]
fn withdrawn_event_topics_index_recipient() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.set_time(600);
    let amount = t.contract.withdraw(&id);

    assert_eq!(amount, 500);
    t.assert_latest_stream_event_topics(
        Withdrawn {
            recipient: t.recipient.clone(),
            id,
            amount,
        }
        .to_xdr(&t.env, &t.contract.address),
    );

    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 500);
}

#[test]
fn cancelled_event_topics_index_sender() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.set_time(600);
    let refund = t.contract.cancel(&id);

    assert_eq!(refund, 500);
    t.assert_latest_stream_event_topics(
        Cancelled {
            sender: t.sender.clone(),
            id,
            recipient_amount: 500,
            sender_refund: refund,
        }
        .to_xdr(&t.env, &t.contract.address),
    );

    assert_eq!(t.token.balance(&t.sender), 500);
    assert_eq!(t.contract.status(&id), crate::StreamStatus::Cancelled);
    assert_eq!(t.contract.get_stream(&id).total_amount, 500);
}

// ── Silence on rejection ─────────────────────────────────────────────────────

/// A creation rejected for an invalid participant publishes nothing.
#[test]
fn rejected_create_publishes_no_events() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let sender = t.sender.clone();
    assert!(t.try_create_stream_for_raw(&sender, &sender, &t.token_address, 1_000));

    assert_eq!(t.event_publishers(), vec![&t.env]);
    t.assert_nothing_happened(1_000);
}

/// A rejection from an exhausted id counter is also silent.
#[test]
fn rejected_create_on_exhausted_counter_publishes_no_events() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    t.set_stream_count(u64::MAX);

    assert!(t.try_create_stream_for_raw(&t.sender, &t.recipient, &t.token_address, 1_000));

    assert_eq!(t.event_publishers(), vec![&t.env]);
}

/// A non-positive `total_amount` is refused silently too.
#[test]
fn rejected_create_for_invalid_amount_publishes_no_events() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    assert!(t.try_create_stream_for_raw(&t.sender, &t.recipient, &t.token_address, 0));

    assert_eq!(t.event_publishers(), vec![&t.env]);
    t.assert_nothing_happened(1_000);
}

/// A rejected `withdraw` publishes nothing.
#[test]
fn rejected_withdraw_publishes_no_events() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );

    assert_eq!(t.event_publishers(), vec![&t.env]);
}
