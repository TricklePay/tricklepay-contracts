#![cfg(test)]

use soroban_sdk::testutils::Address as _;

use crate::{StreamError, StreamStatus};

use super::helpers::StreamTest;

/// Issue #356 — authorization is checked before create_stream can transfer
/// tokens, allocate an id, or write a stream record.
#[test]
fn create_requires_authorization_before_any_state_change() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let sender_balance = t.token.balance(&t.sender);
    t.env.set_auths(&[]);

    assert!(t
        .contract
        .try_create_stream(
            &t.sender,
            &t.recipient,
            &t.token_address,
            &1_000,
            &100,
            &1_100,
            &100,
        )
        .is_err());

    assert_eq!(t.contract.stream_count(), 0);
    assert_eq!(t.token.balance(&t.sender), sender_balance);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

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

/// Issue #360 — a caller without sender authorization cannot cancel a stream
/// or move any of its escrowed tokens.
#[test]
fn unauthorized_cancel_leaves_escrow_intact() {
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
    let before = t.contract.get_stream(&id);
    let sender_balance = t.token.balance(&t.sender);
    let contract_balance = t.token.balance(&t.contract.address);

    // Disable the fixture's all-auth mock. No authorization entry means the
    // sender's require_auth() must reject the cancellation.
    t.env.set_auths(&[]);
    assert!(t.contract.try_cancel(&id).is_err());

    assert_eq!(t.contract.get_stream(&id), before);
    assert_eq!(t.token.balance(&t.sender), sender_balance);
    assert_eq!(t.token.balance(&t.contract.address), contract_balance);
    assert_eq!(t.contract.withdrawable(&id), 500);
}

/// Issue #621 — Smart-wallet (custom account) contracts authorize with their own
/// `__check_auth`. Verify that `create_stream`, `withdraw`, and `cancel` succeed
/// when custom account contracts acting as sender and recipient approve authorization.
#[test]
fn custom_account_authorisation_succeeds_when_approved() {
    use super::helpers::{CustomAccountContract, CustomAccountContractClient};
    use soroban_sdk::testutils::MockAuthInvoke;
    use soroban_sdk::token::StellarAssetClient;
    use soroban_sdk::IntoVal;

    let t = StreamTest::setup(1_000);
    let sender_account = t.env.register(CustomAccountContract, ());
    let recipient_account = t.env.register(CustomAccountContract, ());

    // Fund the custom account sender.
    let token_admin = StellarAssetClient::new(&t.env, &t.token_address);
    token_admin.mint(&sender_account, &1_000);

    // Both accounts are configured to approve authorizations.
    CustomAccountContractClient::new(&t.env, &sender_account).set_allowed(&true);
    CustomAccountContractClient::new(&t.env, &recipient_account).set_allowed(&true);

    // 1. create_stream: sender requires auth on create_stream and inner transfer.
    let create_invoke = MockAuthInvoke {
        contract: &t.contract.address,
        fn_name: "create_stream",
        args: soroban_sdk::vec![
            &t.env,
            sender_account.into_val(&t.env),
            recipient_account.into_val(&t.env),
            t.token_address.into_val(&t.env),
            1_000_i128.into_val(&t.env),
            100_u64.into_val(&t.env),
            1_100_u64.into_val(&t.env),
            100_u64.into_val(&t.env),
        ],
        sub_invokes: &[MockAuthInvoke {
            contract: &t.token_address,
            fn_name: "transfer",
            args: soroban_sdk::vec![
                &t.env,
                sender_account.into_val(&t.env),
                t.contract.address.into_val(&t.env),
                1_000_i128.into_val(&t.env),
            ],
            sub_invokes: &[],
        }],
    };
    t.env
        .set_auths(&[t.auth_entry(&sender_account, &create_invoke)]);

    let id = t.contract.create_stream(
        &sender_account,
        &recipient_account,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(id, 0);
    assert_eq!(t.contract.stream_count(), 1);
    assert_eq!(t.token.balance(&sender_account), 0);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);

    // Advance to midpoint (t=600): 500 vested.
    t.set_time(600);

    // 2. withdraw: recipient requires auth on withdraw.
    let withdraw_invoke = MockAuthInvoke {
        contract: &t.contract.address,
        fn_name: "withdraw",
        args: soroban_sdk::vec![&t.env, id.into_val(&t.env)],
        sub_invokes: &[],
    };
    t.env
        .set_auths(&[t.auth_entry(&recipient_account, &withdraw_invoke)]);

    let withdrawn = t.contract.withdraw(&id);
    assert_eq!(withdrawn, 500);
    assert_eq!(t.token.balance(&recipient_account), 500);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 500);

    // 3. cancel: sender requires auth on cancel.
    let cancel_invoke = MockAuthInvoke {
        contract: &t.contract.address,
        fn_name: "cancel",
        args: soroban_sdk::vec![&t.env, id.into_val(&t.env)],
        sub_invokes: &[],
    };
    t.env
        .set_auths(&[t.auth_entry(&sender_account, &cancel_invoke)]);

    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 500);
    assert!(t.contract.get_stream(&id).cancelled);
    assert_eq!(t.token.balance(&sender_account), 500);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// Issue #621 — Verify that when a custom account contract denies authorization
/// in `__check_auth`, `create_stream`, `withdraw`, and `cancel` fail closed and leave
/// token balances and stored stream records completely unchanged.
#[test]
fn custom_account_authorisation_fails_and_leaves_state_unchanged_when_denied() {
    use super::helpers::{CustomAccountContract, CustomAccountContractClient};
    use soroban_sdk::testutils::MockAuthInvoke;
    use soroban_sdk::token::StellarAssetClient;
    use soroban_sdk::IntoVal;

    let t = StreamTest::setup(1_000);
    let sender_account = t.env.register(CustomAccountContract, ());
    let recipient_account = t.env.register(CustomAccountContract, ());

    let token_admin = StellarAssetClient::new(&t.env, &t.token_address);
    token_admin.mint(&sender_account, &1_000);

    // 1. create_stream fails when sender denies auth.
    CustomAccountContractClient::new(&t.env, &sender_account).set_allowed(&false);
    let create_invoke = MockAuthInvoke {
        contract: &t.contract.address,
        fn_name: "create_stream",
        args: soroban_sdk::vec![
            &t.env,
            sender_account.into_val(&t.env),
            recipient_account.into_val(&t.env),
            t.token_address.into_val(&t.env),
            1_000_i128.into_val(&t.env),
            100_u64.into_val(&t.env),
            1_100_u64.into_val(&t.env),
            100_u64.into_val(&t.env),
        ],
        sub_invokes: &[MockAuthInvoke {
            contract: &t.token_address,
            fn_name: "transfer",
            args: soroban_sdk::vec![
                &t.env,
                sender_account.into_val(&t.env),
                t.contract.address.into_val(&t.env),
                1_000_i128.into_val(&t.env),
            ],
            sub_invokes: &[],
        }],
    };
    t.env
        .set_auths(&[t.auth_entry(&sender_account, &create_invoke)]);

    let res_create = t.contract.try_create_stream(
        &sender_account,
        &recipient_account,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert!(
        res_create.is_err(),
        "create_stream must fail when custom account denies auth"
    );
    // Assert all state and balances remain untouched.
    assert_eq!(t.contract.stream_count(), 0);
    assert_eq!(t.token.balance(&sender_account), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);

    // Now allow creation to set up the stream for withdraw and cancel rejection checks.
    CustomAccountContractClient::new(&t.env, &sender_account).set_allowed(&true);
    t.env
        .set_auths(&[t.auth_entry(&sender_account, &create_invoke)]);
    let id = t.contract.create_stream(
        &sender_account,
        &recipient_account,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(id, 0);

    // Advance to midpoint (t=600): 500 vested.
    t.set_time(600);
    let stream_before_withdraw = t.contract.get_stream(&id);
    let recipient_balance_before = t.token.balance(&recipient_account);
    let contract_balance_before = t.token.balance(&t.contract.address);

    // 2. withdraw fails when recipient custom account denies auth.
    CustomAccountContractClient::new(&t.env, &recipient_account).set_allowed(&false);
    let withdraw_invoke = MockAuthInvoke {
        contract: &t.contract.address,
        fn_name: "withdraw",
        args: soroban_sdk::vec![&t.env, id.into_val(&t.env)],
        sub_invokes: &[],
    };
    t.env
        .set_auths(&[t.auth_entry(&recipient_account, &withdraw_invoke)]);

    let res_withdraw = t.contract.try_withdraw(&id);
    assert!(
        res_withdraw.is_err(),
        "withdraw must fail when custom account denies auth"
    );
    // Assert state and balances remain untouched after failed withdraw.
    assert_eq!(t.contract.get_stream(&id), stream_before_withdraw);
    assert_eq!(
        t.token.balance(&recipient_account),
        recipient_balance_before
    );
    assert_eq!(
        t.token.balance(&t.contract.address),
        contract_balance_before
    );

    // 3. cancel fails when sender custom account denies auth.
    CustomAccountContractClient::new(&t.env, &sender_account).set_allowed(&false);
    let cancel_invoke = MockAuthInvoke {
        contract: &t.contract.address,
        fn_name: "cancel",
        args: soroban_sdk::vec![&t.env, id.into_val(&t.env)],
        sub_invokes: &[],
    };
    t.env
        .set_auths(&[t.auth_entry(&sender_account, &cancel_invoke)]);

    let sender_balance_before_cancel = t.token.balance(&sender_account);
    let res_cancel = t.contract.try_cancel(&id);
    assert!(
        res_cancel.is_err(),
        "cancel must fail when custom account denies auth"
    );
    // Assert stream record and balances remain untouched after failed cancel.
    let stream_after_failed_cancel = t.contract.get_stream(&id);
    assert_eq!(stream_after_failed_cancel, stream_before_withdraw);
    assert!(!stream_after_failed_cancel.cancelled);
    assert_eq!(
        t.token.balance(&sender_account),
        sender_balance_before_cancel
    );
    assert_eq!(
        t.token.balance(&t.contract.address),
        contract_balance_before
    );
}
