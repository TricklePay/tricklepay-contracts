#![cfg(test)]

use soroban_sdk::{vec, Event};

use crate::events::{Cancelled, Created, Withdrawn};

use crate::{StreamError, StreamStatus};

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

/// `withdraw_amount` follows the same order as `withdraw`: refund first,
/// event second.
#[test]
fn withdraw_amount_emits_withdrawn_after_the_payout_transfer() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.set_time(600);
    let recipient_balance_before = t.token.balance(&t.recipient);
    let withdrawn = t.contract.withdraw_amount(&id, &200);
    let transferred = withdrawn;

    t.assert_latest_stream_event_topics(
        Withdrawn {
            recipient: t.recipient.clone(),
            id,
            amount: transferred,
            destination: t.recipient.clone(),
        }
        .to_xdr(&t.env, &t.contract.address),
    );

    assert_eq!(t.event_publishers(), t.transfer_then_announce());
    assert_eq!(
        t.token.balance(&t.recipient) - recipient_balance_before,
        transferred
    );
}

/// `withdraw_to` transfers tokens to the destination before announcing.
#[test]
fn withdraw_to_emits_withdrawn_after_the_payout_transfer() {
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Address;

    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    let destination = Address::generate(&t.env);

    t.set_time(600);
    let dest_balance_before = t.token.balance(&destination);
    let recipient_balance_before = t.token.balance(&t.recipient);
    let withdrawn = t.contract.withdraw_to(&id, &destination, &300);

    t.assert_latest_stream_event_topics(
        Withdrawn {
            recipient: t.recipient.clone(),
            id,
            amount: withdrawn,
            destination: destination.clone(),
        }
        .to_xdr(&t.env, &t.contract.address),
    );

    assert_eq!(t.event_publishers(), t.transfer_then_announce());
    assert_eq!(t.token.balance(&destination) - dest_balance_before, 300);
    assert_eq!(t.token.balance(&t.recipient), recipient_balance_before);
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
            destination: t.recipient.clone(),
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
    let recipient_amount = 500;
    let refund = t.contract.cancel(&id);

    assert_eq!(refund, recipient_amount);
    // At the midpoint, the remaining escrow is split cleanly between the
    // recipient's accrued share and the sender's refund.
    assert_eq!(recipient_amount + refund, 1_000);
    t.assert_latest_stream_event_topics(
        Cancelled {
            sender: t.sender.clone(),
            id,
            recipient_amount,
            sender_refund: refund,
        }
        .to_xdr(&t.env, &t.contract.address),
    );

    assert_eq!(t.token.balance(&t.sender), 500);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);
    assert_eq!(t.contract.get_stream(&id).total_amount, 500);
}

/// Issue #361 — each lifecycle event carries the address that acted on the
/// operation, so consumers can attribute the state change without guessing
/// from the stream record.
#[test]
fn lifecycle_events_carry_the_acting_party() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.assert_latest_stream_event_topics(
        Created {
            sender: t.sender.clone(),
            recipient: t.recipient.clone(),
            id,
            token: t.token_address.clone(),
            total_amount: 1_000,
            start_time: 100,
            end_time: 1_100,
            cliff_time: 100,
        }
        .to_xdr(&t.env, &t.contract.address),
    );

    t.set_time(600);
    assert_eq!(t.contract.withdraw(&id), 500);
    t.assert_latest_stream_event_topics(
        Withdrawn {
            recipient: t.recipient.clone(),
            id,
            amount: 500,
            destination: t.recipient.clone(),
        }
        .to_xdr(&t.env, &t.contract.address),
    );

    t.set_time(700);
    assert_eq!(t.contract.cancel(&id), 400);
    t.assert_latest_stream_event_topics(
        Cancelled {
            sender: t.sender.clone(),
            id,
            recipient_amount: 100,
            sender_refund: 400,
        }
        .to_xdr(&t.env, &t.contract.address),
    );
}

/// A creation rejected for an invalid participant publishes nothing: the
/// participant checks run before any token moves, so there is neither a
/// transfer event nor a `Created` event for an indexer to unwind.
#[test]
fn rejected_create_publishes_no_events() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    // This contract's own address is refused by the first validation step.
    let sender = t.sender.clone();
    assert!(t.try_create_stream_for_raw(&sender, &t.contract.address, &t.token_address, 1_000));
    t.assert_create_stream_error(
        &t.contract.address,
        &sender,
        &t.token_address,
        1_000,
        StreamError::InvalidParticipant,
    );

    assert_eq!(t.event_publishers(), vec![&t.env]);
    t.assert_nothing_happened(1_000);
}

/// The same holds for a rejection that happens later in the validation order:
/// an exhausted id counter is caught before the transfer, so the call is
/// silent on the event stream too.
#[test]
fn rejected_create_on_exhausted_counter_publishes_no_events() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    t.set_stream_count(u64::MAX);

    t.assert_create_stream_error(
        &t.sender,
        &t.recipient,
        &t.token_address,
        1_000,
        StreamError::StreamCountExhausted,
    );

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

/// A rejected `withdraw` publishes nothing either: `NothingToWithdraw` is
/// returned before the payout, so no event describes a transfer that did not
/// happen.
#[test]
fn rejected_withdraw_publishes_no_events() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    // Still at start_time, so nothing has vested.
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );

    t.assert_no_stream_event_topics(
        Withdrawn {
            recipient: t.recipient.clone(),
            id,
            amount: 0,
            destination: t.recipient.clone(),
        }
        .to_xdr(&t.env, &t.contract.address),
    );
    assert_eq!(t.event_publishers(), vec![&t.env]);
}

/// A rejected `withdraw_to` publishes nothing.
#[test]
fn rejected_withdraw_to_publishes_no_events() {
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Address;

    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    let destination = Address::generate(&t.env);

    // Amount exceeds withdrawable (0 at start time)
    assert_eq!(
        t.contract.try_withdraw_to(&id, &destination, &100),
        Err(Ok(StreamError::InsufficientBalance))
    );

    t.assert_no_stream_event_topics(
        Withdrawn {
            recipient: t.recipient.clone(),
            id,
            amount: 100,
            destination: destination.clone(),
        }
        .to_xdr(&t.env, &t.contract.address),
    );
    assert_eq!(t.event_publishers(), vec![&t.env]);
}
