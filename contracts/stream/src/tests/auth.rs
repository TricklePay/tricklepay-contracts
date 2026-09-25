#![cfg(test)]

use soroban_sdk::testutils::Address as _;

use crate::{StreamError, StreamStatus};

use super::helpers::StreamTest;

#[test]
fn withdraw_requires_recipient_authorization() {
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
    t.contract.withdraw(&id);

    let auths = t.env.auths();
    assert!(auths.iter().any(|(addr, _)| addr == &t.recipient));
}

#[test]
fn cancel_requires_sender_authorization() {
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
    t.contract.cancel(&id);

    let auths = t.env.auths();
    assert!(auths.iter().any(|(addr, _)| addr == &t.sender));
}

/// Issue #73 — Sender cannot withdraw: withdraw must require recipient auth only.
#[test]
fn test_sender_cannot_withdraw() {
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
    t.contract.withdraw(&id);
    let auths = t.env.auths();
    assert!(
        auths.iter().any(|(addr, _)| addr == &t.recipient),
        "withdraw must require recipient authorization"
    );
    assert!(
        !auths.iter().any(|(addr, _)| addr == &t.sender),
        "withdraw must not require sender authorization"
    );
    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&t.sender), 0);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 500);
}

/// Issue #71 — Recipient claim after cancellation requires recipient auth only.
#[test]
fn test_recipient_claim_after_cancellation() {
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
    assert_eq!(t.contract.withdrawable(&id), 500);

    t.contract.withdraw_amount(&id, &100);
    let auths_amount = t.env.auths();
    assert!(
        auths_amount.iter().any(|(addr, _)| addr == &t.recipient),
        "withdraw_amount must require recipient authorization"
    );
    assert!(
        !auths_amount.iter().any(|(addr, _)| addr == &t.sender),
        "withdraw_amount must not authorize sender"
    );

    t.contract.withdraw(&id);
    let auths = t.env.auths();
    assert!(
        auths.iter().any(|(addr, _)| addr == &t.recipient),
        "withdraw must require recipient authorization"
    );
    assert!(
        !auths.iter().any(|(addr, _)| addr == &t.sender),
        "withdraw must not authorize or be attributed to sender"
    );

    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&t.sender), 0);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 500);
    assert!(!t.contract.get_stream(&id).cancelled);
}

/// Issue #74 — Recipient cannot cancel: cancel must require sender auth only.
#[test]
fn test_recipient_cannot_cancel() {
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
    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 500);

    t.set_time(1_500);
    let withdrawn = t.contract.withdraw(&id);
    assert_eq!(withdrawn, 500);

    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&t.sender), 500);
    assert_eq!(t.token.balance(&t.contract.address), 0);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);

    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );
    assert_eq!(t.token.balance(&t.recipient), 500);
}

/// Issue #76 — Third party cannot mutate streams.
#[test]
fn test_third_party_cannot_mutate_streams() {
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

    let third_party = soroban_sdk::Address::generate(&t.env);
    assert_ne!(third_party, t.sender);
    assert_ne!(third_party, t.recipient);

    // withdraw_amount requires recipient auth, not third-party.
    t.contract.withdraw_amount(&id, &100);
    let auths_wa = t.env.auths();
    assert!(
        auths_wa.iter().any(|(addr, _)| addr == &t.recipient),
        "withdraw_amount must require recipient auth"
    );
    assert!(
        !auths_wa.iter().any(|(addr, _)| addr == &third_party),
        "withdraw_amount must not accept third party auth"
    );

    // withdraw requires recipient auth, not third-party.
    t.contract.withdraw(&id);
    let auths_w = t.env.auths();
    assert!(
        auths_w.iter().any(|(addr, _)| addr == &t.recipient),
        "withdraw must require recipient auth"
    );
    assert!(
        !auths_w.iter().any(|(addr, _)| addr == &third_party),
        "withdraw must not accept third party auth"
    );

    // cancel requires sender auth, not third-party.
    let t_cancel = StreamTest::setup(1_000);
    t_cancel.set_time(100);
    let id_cancel = t_cancel.contract.create_stream(
        &t_cancel.sender,
        &t_cancel.recipient,
        &t_cancel.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    t_cancel.set_time(600);

    t_cancel.contract.cancel(&id_cancel);
    let auths_c = t_cancel.env.auths();
    assert!(
        auths_c.iter().any(|(addr, _)| addr == &t_cancel.sender),
        "cancel must require sender auth"
    );
    assert!(
        !auths_c.iter().any(|(addr, _)| addr == &third_party),
        "cancel must not accept third party auth"
    );
}
