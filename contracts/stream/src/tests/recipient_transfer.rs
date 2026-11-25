#![cfg(test)]

use soroban_sdk::testutils::{Address as _, MockAuthInvoke};
use soroban_sdk::{vec, Address, Event, IntoVal};

use crate::events::RecipientTransferred;
use crate::{StreamError, StreamStatus};

use super::helpers::{StreamTest, END, MIDPOINT, START};

/// The new recipient gains the right to withdraw and the old recipient loses it.
#[test]
fn transfer_moves_withdrawal_rights_to_the_new_recipient() {
    let t = StreamTest::setup(1_000);
    t.set_time(START);
    let id = t.open_default_stream(1_000);
    let new_recipient = Address::generate(&t.env);

    t.set_time(MIDPOINT);
    t.contract.transfer_recipient(&id, &new_recipient);

    assert_eq!(t.contract.get_stream(&id).recipient, new_recipient);
    assert_eq!(t.contract.withdrawable(&id), 500);

    let withdraw_invoke = MockAuthInvoke {
        contract: &t.contract.address,
        fn_name: "withdraw",
        args: vec![&t.env, id.into_val(&t.env)],
        sub_invokes: &[],
    };
    t.env
        .set_auths(&[t.auth_entry(&t.recipient, &withdraw_invoke)]);
    assert!(t.contract.try_withdraw(&id).is_err());

    t.env.mock_all_auths();
    assert_eq!(t.contract.withdraw(&id), 500);
    assert_eq!(t.token.balance(&new_recipient), 500);
    assert_eq!(t.token.balance(&t.recipient), 0);
}

/// Only the current recipient may transfer; the sender's authorization is not enough.
#[test]
fn sender_cannot_transfer_recipient_rights() {
    let t = StreamTest::setup(1_000);
    t.set_time(START);
    let id = t.open_default_stream(1_000);
    let new_recipient = Address::generate(&t.env);
    let before = t.contract.get_stream(&id);

    let transfer_invoke = MockAuthInvoke {
        contract: &t.contract.address,
        fn_name: "transfer_recipient",
        args: vec![&t.env, id.into_val(&t.env), new_recipient.into_val(&t.env)],
        sub_invokes: &[],
    };
    t.env
        .set_auths(&[t.auth_entry(&t.sender, &transfer_invoke)]);

    assert!(t
        .contract
        .try_transfer_recipient(&id, &new_recipient)
        .is_err());
    assert_eq!(t.contract.get_stream(&id), before);
}

/// A call with no authorization at all is rejected before any state changes.
#[test]
fn unauthorized_transfer_leaves_stream_unchanged_and_emits_no_event() {
    let t = StreamTest::setup(1_000);
    t.set_time(START);
    let id = t.open_default_stream(1_000);
    let new_recipient = Address::generate(&t.env);
    let before = t.contract.get_stream(&id);

    t.env.set_auths(&[]);
    assert!(t
        .contract
        .try_transfer_recipient(&id, &new_recipient)
        .is_err());

    assert_eq!(t.contract.get_stream(&id), before);
    t.assert_no_stream_event_topics(
        RecipientTransferred {
            old_recipient: t.recipient.clone(),
            new_recipient: new_recipient.clone(),
            stream_id: id,
        }
        .to_xdr(&t.env, &t.contract.address),
    );
}

/// A missing stream id is reported before authorization, so callers can tell it
/// apart from an authorization failure.
#[test]
fn transferring_a_missing_stream_returns_stream_not_found() {
    let t = StreamTest::setup(1_000);
    let new_recipient = Address::generate(&t.env);

    t.env.set_auths(&[]);
    assert_eq!(
        t.contract.try_transfer_recipient(&999, &new_recipient),
        Err(Ok(StreamError::StreamNotFound))
    );
}

/// The contract rejects transfers to itself and no-op transfers to the current
/// recipient without changing state.
#[test]
fn invalid_new_recipients_are_rejected() {
    let t = StreamTest::setup(1_000);
    t.set_time(START);
    let id = t.open_default_stream(1_000);
    let before = t.contract.get_stream(&id);

    assert_eq!(
        t.contract.try_transfer_recipient(&id, &t.contract.address),
        Err(Ok(StreamError::InvalidParticipant))
    );
    assert_eq!(
        t.contract.try_transfer_recipient(&id, &t.recipient.clone()),
        Err(Ok(StreamError::InvalidParticipant))
    );

    assert_eq!(t.contract.get_stream(&id), before);
    assert_eq!(t.contract.stream_count(), 1);
}

/// Transferring to the sender is allowed: the sender simply becomes the
/// recipient.
#[test]
fn transferring_to_the_sender_is_allowed() {
    let t = StreamTest::setup(1_000);
    t.set_time(START);
    let id = t.open_default_stream(1_000);

    t.contract.transfer_recipient(&id, &t.sender);

    assert_eq!(t.contract.get_stream(&id).recipient, t.sender);
    t.set_time(END);
    assert_eq!(t.contract.withdraw(&id), 1_000);
    assert_eq!(t.token.balance(&t.sender), 1_000);
}

/// Repeated transfers form an ownership chain and only the latest recipient can
/// withdraw. Transferring back restores the original recipient's rights.
#[test]
fn recipient_can_be_transferred_repeatedly_and_back() {
    let t = StreamTest::setup(1_000);
    t.set_time(START);
    let id = t.open_default_stream(1_000);
    let first = Address::generate(&t.env);
    let second = Address::generate(&t.env);

    t.contract.transfer_recipient(&id, &first);
    t.contract.transfer_recipient(&id, &second);
    assert_eq!(t.contract.get_stream(&id).recipient, second);

    let withdraw_invoke = MockAuthInvoke {
        contract: &t.contract.address,
        fn_name: "withdraw",
        args: vec![&t.env, id.into_val(&t.env)],
        sub_invokes: &[],
    };
    t.env.set_auths(&[t.auth_entry(&first, &withdraw_invoke)]);
    assert!(t.contract.try_withdraw(&id).is_err());

    t.env.mock_all_auths();
    t.contract.transfer_recipient(&id, &t.recipient);
    assert_eq!(t.contract.get_stream(&id).recipient, t.recipient);

    t.set_time(MIDPOINT);
    assert_eq!(t.contract.withdraw(&id), 500);
    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&first), 0);
    assert_eq!(t.token.balance(&second), 0);
}

/// A partial withdrawal by the old recipient is preserved; the new recipient can
/// only withdraw what has vested since, and the total stays unchanged.
#[test]
fn withdrawn_amount_is_preserved_across_transfer() {
    let t = StreamTest::setup(1_000);
    t.set_time(START);
    let id = t.open_default_stream(1_000);
    let new_recipient = Address::generate(&t.env);

    t.set_time(MIDPOINT);
    assert_eq!(t.contract.withdraw_amount(&id, &200), 200);
    t.contract.transfer_recipient(&id, &new_recipient);

    t.set_time(END);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 200);
    assert_eq!(t.contract.withdraw(&id), 800);

    assert_eq!(t.token.balance(&t.recipient), 200);
    assert_eq!(t.token.balance(&new_recipient), 800);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// A transfer moves no tokens and changes no schedule or withdrawal fields, so
/// the escrow balance remains `total_amount - withdrawn`.
#[test]
fn transfer_does_not_move_tokens_or_change_the_schedule() {
    let t = StreamTest::setup(1_000);
    t.set_time(START);
    let id = t.open_default_stream(1_000);
    let new_recipient = Address::generate(&t.env);

    t.set_time(MIDPOINT);
    assert_eq!(t.contract.withdraw_amount(&id, &125), 125);

    let before = t.contract.get_stream(&id);
    let sender_balance = t.token.balance(&t.sender);
    let recipient_balance = t.token.balance(&t.recipient);
    let contract_balance = t.token.balance(&t.contract.address);
    let withdrawable = t.contract.withdrawable(&id);

    t.contract.transfer_recipient(&id, &new_recipient);

    let after = t.contract.get_stream(&id);
    assert_eq!(after.recipient, new_recipient);
    assert_eq!(after.sender, before.sender);
    assert_eq!(after.token, before.token);
    assert_eq!(after.total_amount, before.total_amount);
    assert_eq!(after.withdrawn, before.withdrawn);
    assert_eq!(after.start_time, before.start_time);
    assert_eq!(after.end_time, before.end_time);
    assert_eq!(after.cliff_time, before.cliff_time);
    assert_eq!(after.cancelled, before.cancelled);
    assert_eq!(after.created_at, before.created_at);

    assert_eq!(t.token.balance(&t.sender), sender_balance);
    assert_eq!(t.token.balance(&t.recipient), recipient_balance);
    assert_eq!(t.token.balance(&t.contract.address), contract_balance);
    assert_eq!(t.contract.withdrawable(&id), withdrawable);
    assert_eq!(
        contract_balance,
        after.total_amount - after.withdrawn,
        "escrow invariant must hold after a transfer"
    );
}

/// Cancellation freezes the vested amount for whoever is the recipient at that
/// moment, and a cancelled stream can still be transferred so the new recipient
/// can withdraw that frozen amount.
#[test]
fn cancelled_stream_can_be_transferred_and_new_recipient_can_withdraw() {
    let t = StreamTest::setup(1_000);
    t.set_time(START);
    let id = t.open_default_stream(1_000);
    let new_recipient = Address::generate(&t.env);

    t.set_time(MIDPOINT);
    assert_eq!(t.contract.cancel(&id), 500);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);

    t.contract.transfer_recipient(&id, &new_recipient);
    assert_eq!(t.contract.get_stream(&id).recipient, new_recipient);

    t.env.mock_all_auths();
    assert_eq!(t.contract.withdraw(&id), 500);
    assert_eq!(t.token.balance(&new_recipient), 500);
    assert_eq!(t.token.balance(&t.recipient), 0);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// A fully withdrawn stream can still be transferred, but the new recipient
/// finds nothing left to withdraw.
#[test]
fn fully_withdrawn_stream_can_be_transferred() {
    let t = StreamTest::setup(1_000);
    t.set_time(START);
    let id = t.open_default_stream(1_000);
    let new_recipient = Address::generate(&t.env);

    t.set_time(END);
    assert_eq!(t.contract.withdraw(&id), 1_000);
    t.contract.transfer_recipient(&id, &new_recipient);

    assert_eq!(t.contract.get_stream(&id).recipient, new_recipient);
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );
}

/// The event names both sides as topics and carries the stream id as data, and
/// a transfer publishes no token event because no tokens move.
#[test]
fn transfer_emits_recipient_transferred_event_only() {
    let t = StreamTest::setup(1_000);
    t.set_time(START);
    let id = t.open_default_stream(1_000);
    let new_recipient = Address::generate(&t.env);

    t.contract.transfer_recipient(&id, &new_recipient);

    t.assert_latest_stream_event_topics(
        RecipientTransferred {
            old_recipient: t.recipient.clone(),
            new_recipient,
            stream_id: id,
        }
        .to_xdr(&t.env, &t.contract.address),
    );
    assert_eq!(
        t.event_publishers(),
        vec![&t.env, t.contract.address.clone()]
    );
}

/// A rejected transfer publishes no `RecipientTransferred` event.
#[test]
fn rejected_transfer_emits_no_event() {
    let t = StreamTest::setup(1_000);
    t.set_time(START);
    let id = t.open_default_stream(1_000);
    let before = t.contract.get_stream(&id);

    assert_eq!(
        t.contract.try_transfer_recipient(&id, &t.recipient.clone()),
        Err(Ok(StreamError::InvalidParticipant))
    );

    t.assert_no_stream_event_topics(
        RecipientTransferred {
            old_recipient: t.recipient.clone(),
            new_recipient: t.recipient.clone(),
            stream_id: id,
        }
        .to_xdr(&t.env, &t.contract.address),
    );
    assert_eq!(t.contract.get_stream(&id), before);
}
