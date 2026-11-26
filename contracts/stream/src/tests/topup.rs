#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env, Event,
};

use crate::contract::{StreamContract, StreamContractClient};
use crate::events::ToppedUp;
use crate::{StreamError, StreamStatus, MAX_AMOUNT};

use super::helpers::{FeeOnTransferToken, FeeOnTransferTokenClient, StreamTest};

#[test]
fn topup_increases_total_amount_and_preserves_end_time() {
    let t = StreamTest::setup(5_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.contract.topup_stream(&id, &500);

    let stream = t.contract.get_stream(&id);
    assert_eq!(stream.total_amount, 1_500);
    assert_eq!(stream.start_time, 100);
    assert_eq!(stream.end_time, 1_100);
    assert_eq!(stream.cliff_time, 100);
    assert_eq!(stream.withdrawn, 0);
    assert!(!stream.cancelled);
}

#[test]
fn topup_transfers_tokens_from_sender_to_contract() {
    let t = StreamTest::setup(5_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    assert_eq!(t.token.balance(&t.contract.address), 1_000);
    assert_eq!(t.token.balance(&t.sender), 4_000);

    t.contract.topup_stream(&id, &750);

    assert_eq!(t.token.balance(&t.contract.address), 1_750);
    assert_eq!(t.token.balance(&t.sender), 3_250);
}

#[test]
fn topup_emits_topped_up_event() {
    let t = StreamTest::setup(5_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.contract.topup_stream(&id, &500);

    t.assert_latest_stream_event_topics(
        ToppedUp {
            sender: t.sender.clone(),
            stream_id: id,
            added_amount: 500,
            new_total_amount: 1_500,
            new_end_time: 1_100,
        }
        .to_xdr(&t.env, &t.contract.address),
    );
}

#[test]
fn topup_rejected_with_invalid_amount_when_zero_or_negative() {
    let t = StreamTest::setup(5_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    assert_eq!(
        t.contract.try_topup_stream(&id, &0),
        Err(Ok(StreamError::InvalidAmount))
    );
    assert_eq!(
        t.contract.try_topup_stream(&id, &-100),
        Err(Ok(StreamError::InvalidAmount))
    );

    let stream = t.contract.get_stream(&id);
    assert_eq!(stream.total_amount, 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);
}

#[test]
fn topup_rejected_with_stream_not_found_when_id_missing() {
    let t = StreamTest::setup(5_000);
    t.set_time(100);

    assert_eq!(
        t.contract.try_topup_stream(&999, &500),
        Err(Ok(StreamError::StreamNotFound))
    );
}

#[test]
fn topup_rejected_with_already_cancelled_when_stream_cancelled() {
    let t = StreamTest::setup(5_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.set_time(600);
    assert_eq!(t.contract.cancel(&id), 500);

    assert_eq!(
        t.contract.try_topup_stream(&id, &500),
        Err(Ok(StreamError::AlreadyCancelled))
    );
}

#[test]
fn topup_rejected_with_stream_already_completed_when_past_end_time() {
    let t = StreamTest::setup(5_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.set_time(1_100);
    assert_eq!(
        t.contract.try_topup_stream(&id, &500),
        Err(Ok(StreamError::StreamAlreadyCompleted))
    );

    t.set_time(1_500);
    assert_eq!(
        t.contract.try_topup_stream(&id, &500),
        Err(Ok(StreamError::StreamAlreadyCompleted))
    );
}

#[test]
fn topup_succeeds_one_second_before_end_time_and_fails_at_end_time() {
    let t = StreamTest::setup(5_000);
    t.set_time(100);
    // Stream over [100, 1_100] (duration = 1_000) with initial total = 1_000.
    let id = t.open_default_stream(1_000);

    // One second before end_time (1_099 = end_time - 1), top-up is still accepted.
    t.set_time(1_099);
    t.contract.topup_stream(&id, &1_000);
    assert_eq!(t.contract.get_stream(&id).total_amount, 2_000);

    // At t = 1_099, elapsed = 999 -> vested = 2_000 * 999 / 1_000 = 1_998.
    assert_eq!(t.contract.vested(&id), 1_998);

    // At end_time (1_100), top-up is rejected with StreamAlreadyCompleted.
    t.set_time(1_100);
    assert_eq!(
        t.contract.try_topup_stream(&id, &500),
        Err(Ok(StreamError::StreamAlreadyCompleted))
    );
    assert_eq!(t.contract.get_stream(&id).total_amount, 2_000);
}

#[test]
fn topup_rejected_with_amount_too_large_when_exceeding_max_amount() {
    let t = StreamTest::setup(MAX_AMOUNT);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &MAX_AMOUNT,
        &100,
        &1_100,
        &100,
    );

    assert_eq!(
        t.contract.try_topup_stream(&id, &1),
        Err(Ok(StreamError::AmountTooLarge))
    );
}

#[test]
fn topup_pending_stream_succeeds_and_status_remains_pending() {
    let t = StreamTest::setup(5_000);
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

    t.contract.topup_stream(&id, &500);

    assert_eq!(t.contract.status(&id), StreamStatus::Pending);
    assert_eq!(t.contract.get_stream(&id).total_amount, 1_500);
    assert_eq!(t.contract.vested(&id), 0);
    assert_eq!(t.contract.withdrawable(&id), 0);
}

#[test]
fn topup_increases_vested_amount_midstream() {
    let t = StreamTest::setup(5_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    // At midpoint (t = 600 of 100..1100), 500 has vested
    t.set_time(600);
    assert_eq!(t.contract.vested(&id), 500);
    assert_eq!(t.contract.withdrawable(&id), 500);

    // Top up by 1000 tokens. Total is now 2000.
    // At midpoint, 2000 * 500 / 1000 = 1000 has now vested.
    t.contract.topup_stream(&id, &1_000);

    assert_eq!(t.contract.vested(&id), 1_000);
    assert_eq!(t.contract.withdrawable(&id), 1_000);

    // Recipient withdraws 1000
    assert_eq!(t.contract.withdraw(&id), 1_000);
    assert_eq!(t.token.balance(&t.recipient), 1_000);
    assert_eq!(t.contract.withdrawable(&id), 0);
}

#[test]
fn topup_after_partial_withdrawal_increases_withdrawable_balance() {
    let t = StreamTest::setup(5_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    // At t = 600 (50% vested), withdraw 300
    t.set_time(600);
    assert_eq!(t.contract.withdraw_amount(&id, &300), 300);
    assert_eq!(t.contract.withdrawable(&id), 200);

    // Top up by 500. Total is now 1500.
    // At midpoint, vested = 1500 * 500 / 1000 = 750.
    // withdrawable = 750 - 300 = 450.
    t.contract.topup_stream(&id, &500);

    assert_eq!(t.contract.vested(&id), 750);
    assert_eq!(t.contract.withdrawable(&id), 450);

    // Withdraw the remaining 450
    assert_eq!(t.contract.withdraw(&id), 450);
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(t.token.balance(&t.recipient), 750);
}

#[test]
fn topup_then_cancel_refunds_unvested_including_topup() {
    let t = StreamTest::setup(5_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    // At t = 600, top up by 1000. Total = 2000.
    t.set_time(600);
    t.contract.topup_stream(&id, &1_000);

    // Cancel at t = 600.
    // Vested = 2000 * 500 / 1000 = 1000.
    // Refund = 2000 - 1000 = 1000.
    assert_eq!(t.contract.cancel(&id), 1_000);

    // Sender started with 5000, deposited 1000, deposited 1000, got 1000 refunded -> 4000
    assert_eq!(t.token.balance(&t.sender), 4_000);

    // Recipient can still withdraw the 1000 vested tokens
    assert_eq!(t.contract.withdraw(&id), 1_000);
    assert_eq!(t.token.balance(&t.recipient), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

#[test]
fn topup_requires_sender_authorization() {
    let t = StreamTest::setup(5_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.contract.topup_stream(&id, &500);

    let auths = t.env.auths();
    assert!(auths.iter().any(|(addr, _)| addr == &t.sender));
}

#[test]
fn multiple_consecutive_topups_accumulate_correctly() {
    let t = StreamTest::setup(10_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.contract.topup_stream(&id, &200);
    t.contract.topup_stream(&id, &300);
    t.contract.topup_stream(&id, &500);

    let stream = t.contract.get_stream(&id);
    assert_eq!(stream.total_amount, 2_000);
    assert_eq!(t.token.balance(&t.contract.address), 2_000);
    assert_eq!(t.token.balance(&t.sender), 8_000);
}

/// Cross-referenced from `THREAT_MODEL.md § Decision: deposit verification by balance difference`.
///
/// `topup_stream` credits the full `added_amount` to `total_amount` without checking the
/// contract's token balance difference. With a fee-on-transfer token, less arrives than was
/// requested, so a later withdrawal fails once the contract's actual token balance is exhausted.
#[test]
fn topup_with_fee_on_transfer_token_credits_added_amount_and_causes_final_withdrawal_to_fail() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(StreamContract, ());
    let contract = StreamContractClient::new(&env, &contract_id);

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let fee_collector = Address::generate(&env);

    // Register FeeOnTransferToken with 10% fee (1,000 bps).
    let token_address = env.register(FeeOnTransferToken, ());
    let fee_token = FeeOnTransferTokenClient::new(&env, &token_address);
    fee_token.initialize(&1_000, &Some(fee_collector.clone()));

    fee_token.mint(&sender, &2_000);
    env.ledger().set_timestamp(100);

    // Initial deposit of 1,000 delivers 900 to the contract (100 to fee_collector).
    let id = contract.create_stream(
        &sender,
        &recipient,
        &token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(fee_token.balance(&contract_id), 900);
    assert_eq!(contract.get_stream(&id).total_amount, 1_000);

    // Top up by 1,000 at midpoint (t = 600).
    // Only 900 arrives in the contract (1,800 total), yet total_amount is credited the full 1,000 -> 2,000.
    env.ledger().set_timestamp(600);
    contract.topup_stream(&id, &1_000);

    assert_eq!(fee_token.balance(&sender), 0);
    assert_eq!(fee_token.balance(&contract_id), 1_800);
    assert_eq!(fee_token.balance(&fee_collector), 200);
    assert_eq!(contract.get_stream(&id).total_amount, 2_000);

    // At midpoint, 1,000 has vested. Recipient withdraws 900, leaving 900 in the contract.
    assert_eq!(contract.withdraw_amount(&id, &900), 900);
    assert_eq!(fee_token.balance(&contract_id), 900);
    assert_eq!(fee_token.balance(&recipient), 810);
    assert_eq!(fee_token.balance(&fee_collector), 290);

    // At end_time (t = 1,100), full 2,000 has vested so 1,100 remains withdrawable,
    // but the contract only holds 900 -> final withdrawal fails.
    env.ledger().set_timestamp(1_100);
    assert_eq!(contract.withdrawable(&id), 1_100);

    let res = contract.try_withdraw(&id);
    assert!(
        res.is_err(),
        "final withdrawal must fail due to short balance after fee-on-transfer top-up"
    );

    assert_eq!(fee_token.balance(&contract_id), 900);
    assert_eq!(fee_token.balance(&recipient), 810);
    assert_eq!(contract.get_stream(&id).withdrawn, 900);
}
