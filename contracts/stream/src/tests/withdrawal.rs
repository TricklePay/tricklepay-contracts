#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::contract::{StreamContract, StreamContractClient};
use crate::{StreamError, StreamStatus};

use super::helpers::{FeeOnTransferToken, FeeOnTransferTokenClient, StreamTest};

#[test]
fn withdraw_releases_vested_in_steps() {
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

    // Midpoint: half has vested.
    t.set_time(600);
    let withdrawn = t.contract.withdraw(&id);
    assert_eq!(withdrawn, 500);
    assert_eq!(t.token.balance(&t.recipient), withdrawn);
    // Nothing more is available until the clock advances again.
    assert_eq!(t.contract.withdrawable(&id), 0);

    // Three-quarter point: another 250 has vested.
    t.set_time(850);
    assert_eq!(t.contract.withdraw(&id), 250);
    assert_eq!(t.token.balance(&t.recipient), 750);

    // End: the final 250.
    t.set_time(1_100);
    assert_eq!(t.contract.withdraw(&id), 250);
    assert_eq!(t.token.balance(&t.recipient), 1_000);

    // The contract is drained and the stream is fully settled.
    assert_eq!(t.token.balance(&t.contract.address), 0);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 1_000);
}

#[test]
fn withdraw_at_exact_end() {
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
    assert_eq!(t.contract.withdrawable(&id), 1_000);
    let withdrawn = t.contract.withdraw(&id);
    assert_eq!(withdrawn, 1_000);
    assert_eq!(t.token.balance(&t.recipient), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);
    assert_eq!(t.contract.withdrawable(&id), 0);
}

#[test]
fn withdraw_many_pays_streams_in_order_with_one_transfer_per_stream() {
    let t = StreamTest::setup(4_000);
    t.set_time(100);
    let first_id = t.open_default_stream(1_000);
    let second_id = t.open_default_stream(1_000);
    let other_recipient = Address::generate(&t.env);
    let other_id = t.contract.create_stream(
        &t.sender,
        &other_recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    t.set_time(600);
    let ids = soroban_sdk::vec![&t.env, first_id, second_id, first_id, other_id];
    let amounts = t.contract.withdraw_many(&ids);
    let publishers = t.event_publishers();
    assert_eq!(amounts, soroban_sdk::vec![&t.env, 500, 500, 0, 500]);
    let auths = t.env.auths();
    assert!(auths.iter().any(|(address, _)| address == &t.recipient));
    assert!(auths.iter().any(|(address, _)| address == &other_recipient));
    assert_eq!(t.token.balance(&t.recipient), 1_000);
    assert_eq!(t.token.balance(&other_recipient), 500);
    assert_eq!(t.contract.get_stream(&first_id).withdrawn, 500);
    assert_eq!(t.contract.get_stream(&second_id).withdrawn, 500);
    assert_eq!(t.contract.get_stream(&other_id).withdrawn, 500);
    assert_eq!(
        publishers,
        soroban_sdk::vec![
            &t.env,
            t.token_address.clone(),
            t.contract.address.clone(),
            t.token_address.clone(),
            t.contract.address.clone(),
            t.token_address.clone(),
            t.contract.address.clone(),
        ]
    );
}

#[test]
fn withdraw_many_pays_duplicated_id_only_once() {
    let t = StreamTest::setup(3_000);
    t.set_time(100);
    let a = t.open_default_stream(1_000);
    let b = t.open_default_stream(2_000);

    t.set_time(600);
    let amounts = t.contract.withdraw_many(&soroban_sdk::vec![&t.env, a, b, a]);
    let publishers = t.event_publishers();

    assert_eq!(amounts, soroban_sdk::vec![&t.env, 500, 1_000, 0]);
    assert_eq!(t.token.balance(&t.recipient), 1_500);
    assert_eq!(t.token.balance(&t.contract.address), 1_500);
    assert_eq!(t.contract.get_stream(&a).withdrawn, 500);
    assert_eq!(t.contract.get_stream(&b).withdrawn, 1_000);
    assert_eq!(
        publishers,
        soroban_sdk::vec![
            &t.env,
            t.token_address.clone(),
            t.contract.address.clone(),
            t.token_address.clone(),
            t.contract.address.clone(),
        ]
    );
}

#[test]
fn withdraw_many_rejects_unknown_ids_atomically_and_requires_a_payout() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);
    t.set_time(600);

    let recipient_balance_before = t.token.balance(&t.recipient);
    assert_eq!(
        t.contract
            .try_withdraw_many(&soroban_sdk::vec![&t.env, id, 999]),
        Err(Ok(StreamError::StreamNotFound))
    );
    assert_eq!(t.token.balance(&t.recipient), recipient_balance_before);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 0);
    assert!(t.event_publishers().is_empty());

    t.set_time(100);
    assert_eq!(
        t.contract.try_withdraw_many(&soroban_sdk::vec![&t.env, id]),
        Err(Ok(StreamError::NothingToWithdraw))
    );
    assert!(t.event_publishers().is_empty());
}

#[test]
fn withdraw_many_respects_the_batch_limit() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);
    let mut ids = soroban_sdk::Vec::new(&t.env);
    for _ in 0..=crate::MAX_BATCH_SIZE {
        ids.push_back(id);
    }

    assert_eq!(
        t.contract.try_withdraw_many(&ids),
        Err(Ok(StreamError::BatchTooLarge))
    );
}

#[test]
fn withdraw_many_pays_cancelled_and_active_streams_in_one_batch() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    let cancelled_id = t.open_default_stream(1_000);
    let active_id = t.open_default_stream(1_000);

    // Partially withdraw from the first stream before cancelling it.
    t.set_time(400);
    assert_eq!(t.contract.withdraw_amount(&cancelled_id, &100), 100);

    // Cancel at midpoint (500 vested, 100 already withdrawn => 400 remaining).
    t.set_time(600);
    assert_eq!(t.contract.cancel(&cancelled_id), 500);

    // Advance further; the cancelled stream stays frozen at 500 vested while
    // the active stream accrues to 700.
    t.set_time(800);
    let ids = soroban_sdk::vec![&t.env, cancelled_id, active_id];
    let amounts = t.contract.withdraw_many(&ids);
    assert_eq!(amounts, soroban_sdk::vec![&t.env, 400, 700]);

    assert_eq!(t.token.balance(&t.recipient), 1_200);
    assert_eq!(t.contract.get_stream(&cancelled_id).withdrawn, 500);
    assert_eq!(t.contract.get_stream(&active_id).withdrawn, 700);

    // A second call at the same timestamp has nothing left to withdraw.
    assert_eq!(
        t.contract.try_withdraw_many(&ids),
        Err(Ok(StreamError::NothingToWithdraw))
    );
}

#[test]
fn withdraw_amount_takes_a_partial_balance() {
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

    // Midpoint: 500 vested. Take only 200 of it.
    t.set_time(600);
    let withdrawn = t.contract.withdraw_amount(&id, &200);
    assert_eq!(withdrawn, 200);
    assert_eq!(t.token.balance(&t.recipient), withdrawn);
    // 300 of the vested 500 is still available.
    assert_eq!(t.contract.withdrawable(&id), 300);

    // Taking more than is available is rejected.
    let recipient_balance_before = t.token.balance(&t.recipient);
    let contract_balance_before = t.token.balance(&t.contract.address);
    let withdrawn_before = t.contract.get_stream(&id).withdrawn;
    assert_eq!(
        t.contract.try_withdraw_amount(&id, &400),
        Err(Ok(StreamError::InsufficientBalance))
    );
    assert_eq!(t.token.balance(&t.recipient), recipient_balance_before);
    assert_eq!(
        t.token.balance(&t.contract.address),
        contract_balance_before
    );
    assert_eq!(t.contract.get_stream(&id).withdrawn, withdrawn_before);

    // A non-positive amount is rejected.
    let recipient_balance_before = t.token.balance(&t.recipient);
    let contract_balance_before = t.token.balance(&t.contract.address);
    let withdrawn_before = t.contract.get_stream(&id).withdrawn;
    assert_eq!(
        t.contract.try_withdraw_amount(&id, &0),
        Err(Ok(StreamError::InvalidAmount))
    );
    assert_eq!(t.token.balance(&t.recipient), recipient_balance_before);
    assert_eq!(
        t.token.balance(&t.contract.address),
        contract_balance_before
    );
    assert_eq!(t.contract.get_stream(&id).withdrawn, withdrawn_before);

    let recipient_balance_before = t.token.balance(&t.recipient);
    let contract_balance_before = t.token.balance(&t.contract.address);
    let withdrawn_before = t.contract.get_stream(&id).withdrawn;
    assert_eq!(
        t.contract.try_withdraw_amount(&id, &-1),
        Err(Ok(StreamError::InvalidAmount))
    );
    assert_eq!(t.token.balance(&t.recipient), recipient_balance_before);
    assert_eq!(
        t.token.balance(&t.contract.address),
        contract_balance_before
    );
    assert_eq!(t.contract.get_stream(&id).withdrawn, withdrawn_before);
}

#[test]
fn withdrawable_never_exceeds_vested_across_sampled_times_and_partial_withdrawal() {
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

    for now in [100u64, 300, 600, 850, 1_100] {
        t.set_time(now);
        let vested = t.contract.vested(&id);
        let withdrawable = t.contract.withdrawable(&id);
        assert!(
            withdrawable <= vested,
            "withdrawable={} vested={} at time={}",
            withdrawable,
            vested,
            now
        );
    }

    t.set_time(600);
    assert_eq!(t.contract.withdraw_amount(&id, &200), 200);
    let vested = t.contract.vested(&id);
    let withdrawable = t.contract.withdrawable(&id);
    assert_eq!(vested, 500);
    assert_eq!(withdrawable, 300);
    assert!(withdrawable <= vested);

    t.set_time(850);
    let vested = t.contract.vested(&id);
    let withdrawable = t.contract.withdrawable(&id);
    assert_eq!(vested, 750);
    assert_eq!(withdrawable, 550);
    assert!(withdrawable <= vested);
}

#[test]
fn withdraw_amount_exactly_available_balance_succeeds() {
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
    let available = t.contract.withdrawable(&id);
    assert_eq!(available, 500);

    assert_eq!(t.contract.withdraw_amount(&id, &available), available);
    assert_eq!(t.token.balance(&t.recipient), available);
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(t.contract.get_stream(&id).withdrawn, available);
}

#[test]
fn withdraw_amount_available_plus_one_receives_insufficient_balance() {
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
    let available = t.contract.withdrawable(&id);
    assert_eq!(available, 500);

    assert_eq!(
        t.contract.try_withdraw_amount(&id, &(available + 1)),
        Err(Ok(StreamError::InsufficientBalance))
    );
    assert_eq!(t.token.balance(&t.recipient), 0);
    assert_eq!(t.contract.withdrawable(&id), available);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 0);
}

/// Rejecting the largest request leaves the stream and token balances unchanged.
#[test]
fn withdraw_amount_i128_max_returns_insufficient_balance_without_changes() {
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

    let stream_before = t.contract.get_stream(&id);
    let recipient_balance_before = t.token.balance(&t.recipient);
    let contract_balance_before = t.token.balance(&t.contract.address);
    assert_eq!(t.contract.withdrawable(&id), 500);

    assert_eq!(
        t.contract.try_withdraw_amount(&id, &i128::MAX),
        Err(Ok(StreamError::InsufficientBalance))
    );
    assert_eq!(t.contract.get_stream(&id), stream_before);
    assert_eq!(t.token.balance(&t.recipient), recipient_balance_before);
    assert_eq!(
        t.token.balance(&t.contract.address),
        contract_balance_before
    );
}

/// Issue #57 — Partial withdrawals across multiple calls.
#[test]
fn test_partial_withdrawals_across_multiple_calls() {
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

    // First partial draw midway through: 500 vested, take 200.
    t.set_time(600);
    assert_eq!(t.contract.withdraw_amount(&id, &200), 200);
    assert_eq!(t.token.balance(&t.recipient), 200);
    assert_eq!(t.token.balance(&t.contract.address), 800);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 200);

    // Second partial draw later: 750 vested, take 300 more.
    t.set_time(850);
    assert_eq!(t.contract.withdraw_amount(&id, &300), 300);
    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&t.contract.address), 500);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 500);

    // Third and final draw at the end: the remaining 500 vests and is taken.
    t.set_time(1_100);
    assert_eq!(t.contract.withdraw_amount(&id, &500), 500);
    assert_eq!(t.token.balance(&t.recipient), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 1_000);
}

/// Cliff blocks withdrawal until reached.
#[test]
fn cliff_blocks_withdrawal_until_reached() {
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

    // At the cliff, everything accrued since the start unlocks at once.
    t.set_time(600);
    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.withdraw(&id), 500);
    assert_eq!(t.token.balance(&t.recipient), 500);
}

/// Issue #59 — Withdrawal at exact cliff.
#[test]
fn test_withdraw_at_exact_cliff() {
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
    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.withdraw(&id), 500);

    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&t.contract.address), 500);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 500);

    assert_eq!(t.contract.withdrawable(&id), 0);
}

/// Issue #60 — Withdrawal at exact start.
#[test]
fn test_withdraw_at_exact_start() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let start = 600u64;
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000i128,
        &start,
        &1_100u64,
        &start,
    );

    t.set_time(start - 1);
    assert_eq!(t.contract.withdrawable(&id), 0);

    t.set_time(start);
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );

    assert_eq!(t.token.balance(&t.recipient), 0);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 0);
}

/// Issue #58 — Withdrawal after full vesting.
#[test]
fn test_withdraw_after_full_vesting() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let end = 1_100u64;
    let amount = 1_000i128;
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &amount,
        &100u64,
        &end,
        &100u64,
    );

    t.set_time(end + 1_000);
    assert_eq!(t.contract.withdrawable(&id), amount);

    assert_eq!(t.contract.withdraw(&id), amount);
    assert_eq!(t.token.balance(&t.recipient), amount);
    assert_eq!(t.token.balance(&t.contract.address), 0);
    assert_eq!(t.contract.get_stream(&id).withdrawn, amount);

    assert_eq!(t.contract.withdrawable(&id), 0);
}

#[test]
fn contract_balance_is_zero_after_full_settlement() {
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

    t.set_time(1_100 + 1_000);
    assert_eq!(t.contract.withdraw(&id), 1_000);
    assert_eq!(t.token.balance(&t.recipient), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);

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
    assert_eq!(t.token.balance(&t.sender), 500);
    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.withdraw(&id), 500);
    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

#[test]
fn operations_on_unknown_stream_report_not_found() {
    let t = StreamTest::setup(1_000);

    assert_eq!(
        t.contract.try_get_stream(&99),
        Err(Ok(StreamError::StreamNotFound))
    );
    assert_eq!(
        t.contract.try_withdraw(&99),
        Err(Ok(StreamError::StreamNotFound))
    );
    assert_eq!(
        t.contract.try_cancel(&99),
        Err(Ok(StreamError::StreamNotFound))
    );
    assert_eq!(
        t.contract.try_withdrawable(&99),
        Err(Ok(StreamError::StreamNotFound))
    );
}

#[test]
fn unknown_stream_operations_need_no_authorization_and_change_nothing() {
    let env = Env::default();
    let contract_id = env.register(StreamContract, ());
    let contract = StreamContractClient::new(&env, &contract_id);

    let issuer = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(issuer);
    let token_address = sac.address();
    let token = token::TokenClient::new(&env, &token_address);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let sender_balance = token.balance(&sender);
    let contract_balance = token.balance(&contract.address);
    let recipient_balance = token.balance(&recipient);

    // Keep this aligned with the validation order documented on `StreamContract::cancel`.
    assert_eq!(
        contract.try_cancel(&99),
        Err(Ok(StreamError::StreamNotFound))
    );
    assert_eq!(
        contract.try_withdraw(&99),
        Err(Ok(StreamError::StreamNotFound))
    );
    assert_eq!(
        contract.try_withdraw_amount(&99, &1),
        Err(Ok(StreamError::StreamNotFound))
    );

    assert_eq!(contract.stream_count(), 0);
    assert_eq!(token.balance(&sender), sender_balance);
    assert_eq!(token.balance(&contract.address), contract_balance);
    assert_eq!(token.balance(&recipient), recipient_balance);
}

/// Issue #253 — Withdrawal of a single base unit.
#[test]
fn test_withdraw_single_base_unit() {
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

    t.set_time(101);
    assert_eq!(t.contract.withdrawable(&id), 1);

    let transferred = t.contract.withdraw_amount(&id, &1);
    assert_eq!(transferred, 1);

    assert_eq!(t.token.balance(&t.recipient), 1);
    assert_eq!(t.token.balance(&t.contract.address), 999);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 1);
    assert_eq!(t.contract.withdrawable(&id), 0);
}

/// Issue #254 — Withdrawing leaves the schedule untouched.
#[test]
fn test_withdrawal_leaves_schedule_untouched() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &400,
    );

    let before = t.contract.get_stream(&id);

    t.set_time(600);
    assert_eq!(t.contract.withdraw(&id), 500);

    let after = t.contract.get_stream(&id);

    assert_eq!(after.withdrawn, 500);
    assert_eq!(after.total_amount, before.total_amount);
    assert_eq!(after.start_time, before.start_time);
    assert_eq!(after.cliff_time, before.cliff_time);
    assert_eq!(after.end_time, before.end_time);
    assert_eq!(after.sender, before.sender);
    assert_eq!(after.recipient, before.recipient);
    assert_eq!(after.token, before.token);
    assert_eq!(after.cancelled, before.cancelled);

    t.set_time(850);
    assert_eq!(t.contract.withdraw_amount(&id, &100), 100);

    let after2 = t.contract.get_stream(&id);
    assert_eq!(after2.withdrawn, 600);
    assert_eq!(after2.total_amount, before.total_amount);
    assert_eq!(after2.start_time, before.start_time);
    assert_eq!(after2.cliff_time, before.cliff_time);
    assert_eq!(after2.end_time, before.end_time);
}

/// Issue #255 — Cliff set at the end of the stream acts as a pure lockup.
#[test]
fn test_cliff_at_end_of_stream() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &1_100,
    );

    t.set_time(400);
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );

    t.set_time(1_099);
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );

    t.set_time(1_100);
    assert_eq!(t.contract.withdrawable(&id), 1_000);

    let withdrawn = t.contract.withdraw(&id);
    assert_eq!(withdrawn, 1_000);
    assert_eq!(t.token.balance(&t.recipient), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 1_000);
}

/// Issue #265 — Cancelling one stream does not disturb any other stream.
///
/// Streams are stored under separate `DataKey::Stream(id)` entries, so a
/// cancellation must only mutate the targeted stream. This test opens four
/// independent streams — three with distinct amounts and schedules and one
/// extra that has already had a partial withdrawal — cancels only stream B at
/// its midpoint, and then asserts that A, C, and D are bit-for-bit identical
/// to what they were before the cancel call. A storage-key collision, an
/// off-by-one in the id counter, or any accidental shared mutable state would
/// cause one of those equality checks to fail.
#[test]
fn cancel_does_not_affect_other_streams() {
    // Fund the sender with enough to cover all four streams.
    let t = StreamTest::setup(4_000);
    t.set_time(50);

    // Stream A — 1 000 units, no cliff, window [100, 1100].
    let id_a = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    // Stream B — 500 units, no cliff, window [100, 1100].  This is the one
    // that will be cancelled.
    let id_b = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &500,
        &100,
        &1_100,
        &100,
    );

    // Stream C — 800 units, cliff at midpoint, window [100, 1100].
    let id_c = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &800,
        &100,
        &1_100,
        &600,
    );

    // Stream D — 700 units, no cliff, window [100, 1100].  The recipient
    // makes a partial withdrawal before stream B is cancelled so we can
    // confirm that `withdrawn` is also unaffected.
    let id_d = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &700,
        &100,
        &1_100,
        &100,
    );

    // Sanity: ids are assigned sequentially and all four streams exist.
    assert_eq!(id_a, 0);
    assert_eq!(id_b, 1);
    assert_eq!(id_c, 2);
    assert_eq!(id_d, 3);
    assert_eq!(t.contract.stream_count(), 4);

    // Advance to the midpoint and take a partial withdrawal from stream D.
    t.set_time(600);
    assert_eq!(t.contract.withdraw_amount(&id_d, &200), 200);

    // Snapshot the state of the streams that must not change.
    let a_before = t.contract.get_stream(&id_a);
    let c_before = t.contract.get_stream(&id_c);
    let d_before = t.contract.get_stream(&id_d);

    // Cancel only stream B at the midpoint. Half of 500 has vested, so the
    // refund is 250.
    let refund = t.contract.cancel(&id_b);
    assert_eq!(refund, 250);
    assert!(t.contract.get_stream(&id_b).cancelled);

    // ── Stream A must be completely unchanged ────────────────────────────────
    let a_after = t.contract.get_stream(&id_a);
    assert_eq!(a_after.sender, a_before.sender);
    assert_eq!(a_after.recipient, a_before.recipient);
    assert_eq!(a_after.token, a_before.token);
    assert_eq!(a_after.total_amount, a_before.total_amount);
    assert_eq!(a_after.withdrawn, a_before.withdrawn);
    assert_eq!(a_after.cancelled, a_before.cancelled);
    assert_eq!(a_after.start_time, a_before.start_time);
    assert_eq!(a_after.cliff_time, a_before.cliff_time);
    assert_eq!(a_after.end_time, a_before.end_time);

    // A is still live and vesting normally after B is cancelled.
    assert_eq!(t.contract.withdrawable(&id_a), 500);
    assert_eq!(t.contract.status(&id_a), StreamStatus::Streaming);

    // ── Stream C must be completely unchanged ────────────────────────────────
    let c_after = t.contract.get_stream(&id_c);
    assert_eq!(c_after.sender, c_before.sender);
    assert_eq!(c_after.recipient, c_before.recipient);
    assert_eq!(c_after.token, c_before.token);
    assert_eq!(c_after.total_amount, c_before.total_amount);
    assert_eq!(c_after.withdrawn, c_before.withdrawn);
    assert_eq!(c_after.cancelled, c_before.cancelled);
    assert_eq!(c_after.start_time, c_before.start_time);
    assert_eq!(c_after.cliff_time, c_before.cliff_time);
    assert_eq!(c_after.end_time, c_before.end_time);

    // C has its cliff at 600, so at now == 600 it just became withdrawable.
    assert_eq!(t.contract.withdrawable(&id_c), 400);
    assert_eq!(t.contract.status(&id_c), StreamStatus::Streaming);

    // ── Stream D must be completely unchanged (including prior withdrawal) ───
    let d_after = t.contract.get_stream(&id_d);
    assert_eq!(d_after.sender, d_before.sender);
    assert_eq!(d_after.recipient, d_before.recipient);
    assert_eq!(d_after.token, d_before.token);
    assert_eq!(d_after.total_amount, d_before.total_amount);
    assert_eq!(d_after.withdrawn, d_before.withdrawn);
    assert_eq!(d_after.cancelled, d_before.cancelled);
    assert_eq!(d_after.start_time, d_before.start_time);
    assert_eq!(d_after.cliff_time, d_before.cliff_time);
    assert_eq!(d_after.end_time, d_before.end_time);

    // D vested 350 at the midpoint; 200 were already withdrawn, so 150 remain.
    assert_eq!(t.contract.withdrawable(&id_d), 150);
    assert_eq!(t.contract.status(&id_d), StreamStatus::Streaming);
}

/// Issue #298 — Test that the recipient balance rises by the withdrawn amount.
///
/// Asserts that when a withdrawal occurs, the recipient token balance increases by exactly
/// the returned withdrawn amount, and the contract token balance decreases by the same amount.
#[test]
fn test_recipient_balance_rises_by_withdrawn_amount() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    // Initial balances before any withdrawal
    let recipient_before = t.token.balance(&t.recipient);
    let contract_before = t.token.balance(&t.contract.address);
    assert_eq!(recipient_before, 0);
    assert_eq!(contract_before, 1_000);

    // Advance to 600 (500 vested) and withdraw a specific partial amount (200)
    t.set_time(600);
    let withdrawn_partial = t.contract.withdraw_amount(&id, &200);
    assert_eq!(withdrawn_partial, 200);

    let recipient_after_partial = t.token.balance(&t.recipient);
    let contract_after_partial = t.token.balance(&t.contract.address);
    assert_eq!(
        recipient_after_partial,
        recipient_before + withdrawn_partial
    );
    assert_eq!(contract_after_partial, contract_before - withdrawn_partial);

    // Perform full withdrawal of the remaining available (300)
    let withdrawn_full = t.contract.withdraw(&id);
    assert_eq!(withdrawn_full, 300);

    let recipient_after_full = t.token.balance(&t.recipient);
    let contract_after_full = t.token.balance(&t.contract.address);
    assert_eq!(
        recipient_after_full,
        recipient_after_partial + withdrawn_full
    );
    assert_eq!(contract_after_full, contract_after_partial - withdrawn_full);
}

/// Issue #309 — Test that two streams in the same token settle independently.
///
/// Streams created with the same token all share the same contract wallet, so a
/// withdrawal from one stream must not modify the escrow, `withdrawn` total, or
/// remaining withdrawable balance of any other stream that uses that token.
#[test]
fn test_two_streams_sharing_a_token_settle_independently() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);

    let id_a = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    let id_b = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    assert_eq!(t.contract.stream_count(), 2);
    assert_eq!(t.token.balance(&t.contract.address), 2_000);

    t.set_time(600);

    // Stream A has 500 vested at this ledger time, so a 200 withdrawal should
    // change only its own accounting and the contract's aggregate balance.
    assert_eq!(t.contract.withdraw_amount(&id_a, &200), 200);

    let stream_a_after = t.contract.get_stream(&id_a);
    let stream_b_after = t.contract.get_stream(&id_b);
    assert_eq!(stream_a_after.withdrawn, 200);
    assert_eq!(stream_b_after.withdrawn, 0);
    assert_eq!(t.contract.withdrawable(&id_a), 300);
    assert_eq!(t.contract.withdrawable(&id_b), 500);
    assert_eq!(t.token.balance(&t.recipient), 200);
    assert_eq!(t.token.balance(&t.contract.address), 1_800);

    // A withdrawal from stream B must not alter A's recorded withdrawal total or
    // the remaining escrow attached to A.
    assert_eq!(t.contract.withdraw_amount(&id_b, &100), 100);

    let stream_a_final = t.contract.get_stream(&id_a);
    let stream_b_final = t.contract.get_stream(&id_b);
    assert_eq!(stream_a_final.withdrawn, 200);
    assert_eq!(stream_b_final.withdrawn, 100);
    assert_eq!(t.contract.withdrawable(&id_a), 300);
    assert_eq!(t.contract.withdrawable(&id_b), 400);
    assert_eq!(t.token.balance(&t.recipient), 300);
    assert_eq!(t.token.balance(&t.contract.address), 1_700);
}

/// Issue #299 — Test that cancellation refunds the stored sender.
///
/// Asserts that when a stream is cancelled, the unvested refund is credited exclusively
/// to the stored sender address recorded on the stream, and no other account (such as
/// a third-party caller or the recipient) receives the refund tokens.
#[test]
fn test_cancellation_refunds_stored_sender() {
    let t = StreamTest::setup(1_000);

    t.set_time(100);

    let stored_sender = t.sender.clone();
    let third_party = Address::generate(&t.env);

    let id = t.contract.create_stream(
        &stored_sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(t.contract.stream_count(), 1);

    // Initial balances after stream creation
    assert_eq!(t.token.balance(&stored_sender), 0);
    assert_eq!(t.token.balance(&t.recipient), 0);
    assert_eq!(t.token.balance(&third_party), 0);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);

    // Halfway through: 500 vested, 500 unvested refund
    t.set_time(600);
    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 500);

    // Assert that the refund was credited to the stored sender address
    assert_eq!(t.token.balance(&stored_sender), 500);

    // Assert that no other account received tokens
    assert_eq!(t.token.balance(&third_party), 0);
    assert_eq!(t.token.balance(&t.recipient), 0);
    // The contract retains only the unwithdrawn vested remainder for the recipient
    assert_eq!(t.token.balance(&t.contract.address), 500);
}

#[test]
fn counter_is_unchanged_by_withdrawals() {
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

    let initial_count = t.contract.stream_count();

    t.set_time(600);
    t.contract.withdraw_amount(&id, &200);
    assert_eq!(t.contract.stream_count(), initial_count);

    t.contract.withdraw(&id);
    assert_eq!(t.contract.stream_count(), initial_count);
}

/// Issue #300 — Test that nothing is withdrawable straight after a full withdrawal.
///
/// Drawing the entire available balance must immediately leave `withdrawable(&id)` at 0
/// until more time passes, and an immediate subsequent withdrawal attempt at the same
/// timestamp must be rejected with `NothingToWithdraw`.
#[test]
fn test_nothing_withdrawable_straight_after_full_withdrawal() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    // Advance to midpoint (500 vested)
    t.set_time(600);
    assert_eq!(t.contract.withdrawable(&id), 500);

    // Perform a full withdrawal of all currently available funds
    let withdrawn = t.contract.withdraw(&id);
    assert_eq!(withdrawn, 500);

    // Immediately after full withdrawal at the same timestamp:
    // 1. Withdrawable figure is zero
    assert_eq!(t.contract.withdrawable(&id), 0);

    // 2. A further withdrawal at the same time is rejected
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );
    assert_eq!(
        t.contract.try_withdraw_amount(&id, &1),
        Err(Ok(StreamError::InsufficientBalance))
    );
}

/// Issue #301 — Test that a cliff equal to the start behaves as no cliff.
///
/// Setting `cliff_time == start_time` is the documented representation of an uncliffed
/// stream. Assert that vesting begins immediately after start_time without being blocked,
/// and that the vested and withdrawable amounts match an equivalent uncliffed schedule
/// across intermediate points and at completion.
#[test]
fn test_cliff_equal_to_start_behaves_as_no_cliff() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);

    let start = 100u64;
    let end = 1_100u64;
    let amount = 1_000i128;

    // Stream 1: explicit cliff equal to start
    let id_cliff = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &amount,
        &start,
        &end,
        &start,
    );

    // Stream 2: standard uncliffed schedule over [100, 1100]
    let id_uncliffed = t.open_default_stream(amount);

    // At start, vested amount is 0 for both
    assert_eq!(t.contract.vested(&id_cliff), 0);
    assert_eq!(t.contract.vested(&id_uncliffed), 0);

    // Immediately after start (t = 200, 100s elapsed out of 1000s)
    // Vesting has begun immediately (not blocked by a cliff)
    t.set_time(200);
    assert_eq!(t.contract.vested(&id_cliff), 100);
    assert_eq!(t.contract.vested(&id_uncliffed), 100);
    assert_eq!(t.contract.withdrawable(&id_cliff), 100);
    assert_eq!(t.contract.withdrawable(&id_uncliffed), 100);

    // Midpoint: t = 600 (500s elapsed out of 1000s)
    t.set_time(600);
    assert_eq!(t.contract.vested(&id_cliff), 500);
    assert_eq!(t.contract.vested(&id_uncliffed), 500);
    assert_eq!(t.contract.withdrawable(&id_cliff), 500);
    assert_eq!(t.contract.withdrawable(&id_uncliffed), 500);

    // At end: t = 1100
    t.set_time(1100);
    assert_eq!(t.contract.vested(&id_cliff), 1_000);
    assert_eq!(t.contract.vested(&id_uncliffed), 1_000);
    assert_eq!(t.contract.withdrawable(&id_cliff), 1_000);
    assert_eq!(t.contract.withdrawable(&id_uncliffed), 1_000);
}

#[test]
fn status_ends_pending_at_start_time() {
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

    t.set_time(100);
    assert_ne!(t.contract.status(&id), StreamStatus::Pending);

    t.set_time(101);
    assert_ne!(t.contract.status(&id), StreamStatus::Pending);
}

#[test]
fn cancel_fully_drawn_stream_refunds_nothing() {
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

    // Fully draw the stream
    t.set_time(1_100);
    let withdrawn = t.contract.withdraw(&id);
    assert_eq!(withdrawn, 1_000);

    // The stream is fully drawn. Attempt to cancel.
    let res = t.contract.try_cancel(&id);

    // Cancelling should refuse (or return 0). In this case it refuses because it's already completed.
    assert_eq!(res, Err(Ok(StreamError::StreamAlreadyCompleted)));

    // No tokens move back to the sender
    assert_eq!(t.token.balance(&t.sender), 0);
    assert_eq!(t.token.balance(&t.contract.address), 0);
    assert_eq!(t.token.balance(&t.recipient), 1_000);
}

/// The expected outcome is cross-referenced from the threat model decision on balance-difference
/// checks (see `THREAT_MODEL.md § Decision: deposit verification by balance difference`).
///
/// The contract trusts the SEP-41 token interface and deliberately rejects reading the token balance
/// before and after `transfer` during `create_stream` to avoid cross-contract call overhead for conforming tokens.
/// When a non-conforming fee-on-transfer token is used:
/// 1. The contract receives less than `total_amount` (here, 1,000 sent with 10% fee delivers only 900).
/// 2. The stream record stores the full requested `total_amount = 1,000`.
/// 3. Intermediate withdrawals succeed as long as the contract has sufficient token balance (e.g. 400 withdrawn).
/// 4. The final withdrawal fails because the contract's actual balance is less than the remaining vested amount.
#[test]
fn fee_on_transfer_token_short_deposit_causes_last_withdrawal_to_fail() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(StreamContract, ());
    let contract = StreamContractClient::new(&env, &contract_id);

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let fee_collector = Address::generate(&env);

    // Register FeeOnTransferToken with 10% fee (1,000 bps)
    let token_address = env.register(FeeOnTransferToken, ());
    let fee_token = FeeOnTransferTokenClient::new(&env, &token_address);
    fee_token.initialize(&1_000, &Some(fee_collector.clone()));

    // Mint 1,000 tokens to sender
    fee_token.mint(&sender, &1_000);
    assert_eq!(fee_token.balance(&sender), 1_000);
    assert_eq!(fee_token.balance(&contract_id), 0);
    assert_eq!(fee_token.balance(&recipient), 0);
    assert_eq!(fee_token.balance(&fee_collector), 0);

    // Set time to start of stream
    env.ledger().set_timestamp(100);

    // Create stream for 1,000 tokens over [100, 1100]
    let id = contract.create_stream(
        &sender,
        &recipient,
        &token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    // Due to 10% fee on transfer, contract received only 900 tokens; fee collector got 100.
    assert_eq!(fee_token.balance(&sender), 0);
    assert_eq!(fee_token.balance(&contract_id), 900);
    assert_eq!(fee_token.balance(&fee_collector), 100);
    assert_eq!(fee_token.balance(&recipient), 0);

    // But the contract recorded total_amount as 1,000 without verifying balance difference:
    let stream = contract.get_stream(&id);
    assert_eq!(stream.total_amount, 1_000);

    // At midpoint (t = 600), 500 has vested. Recipient withdraws 400 via withdraw_amount.
    // Contract transfers 400: contract balance drops by 400 to 500.
    // With 10% fee on withdrawal transfer, recipient receives 360, and fee collector gets +40.
    env.ledger().set_timestamp(600);
    let withdrawn = contract.withdraw_amount(&id, &400);
    assert_eq!(withdrawn, 400);

    assert_eq!(fee_token.balance(&contract_id), 500);
    assert_eq!(fee_token.balance(&recipient), 360);
    assert_eq!(fee_token.balance(&fee_collector), 140);

    // At end of stream (t = 1100), full 1,000 has vested.
    // Withdrawn so far: 400. Remaining according to contract: 600.
    // But contract only holds 500!
    env.ledger().set_timestamp(1_100);
    assert_eq!(contract.withdrawable(&id), 600);

    // Final withdrawal attempt fails because contract has insufficient token balance (500 < 600)
    let res = contract.try_withdraw(&id);
    assert!(
        res.is_err(),
        "final withdrawal must fail due to insufficient contract token balance"
    );

    // Assert resulting balances remain unchanged following the failed withdrawal
    assert_eq!(fee_token.balance(&sender), 0);
    assert_eq!(fee_token.balance(&recipient), 360);
    assert_eq!(fee_token.balance(&contract_id), 500);
    assert_eq!(fee_token.balance(&fee_collector), 140);

    // Contract accounting still shows withdrawn = 400 because failed withdrawal reverted atomically
    let stream_after = contract.get_stream(&id);
    assert_eq!(stream_after.withdrawn, 400);
}

/// The expected outcome is cross-referenced from the threat model decision on balance-difference
/// checks (see `THREAT_MODEL.md § Decision: deposit verification by balance difference`).
///
/// Demonstrates that when multiple streams share a fee-on-transfer token, earlier full withdrawals
/// can silently drain the collateral deposited for later streams, leaving the contract insolvent
/// when subsequent recipients attempt to withdraw.
#[test]
fn fee_on_transfer_token_short_deposit_drains_concurrent_streams() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(StreamContract, ());
    let contract = StreamContractClient::new(&env, &contract_id);

    let sender_1 = Address::generate(&env);
    let recipient_1 = Address::generate(&env);
    let sender_2 = Address::generate(&env);
    let recipient_2 = Address::generate(&env);
    let fee_collector = Address::generate(&env);

    // Register FeeOnTransferToken with 10% fee (1,000 bps)
    let token_address = env.register(FeeOnTransferToken, ());
    let fee_token = FeeOnTransferTokenClient::new(&env, &token_address);
    fee_token.initialize(&1_000, &Some(fee_collector.clone()));

    fee_token.mint(&sender_1, &1_000);
    fee_token.mint(&sender_2, &1_000);

    env.ledger().set_timestamp(100);

    // Stream 1 deposits 1,000 -> contract gets 900, fee collector gets 100
    let id_1 = contract.create_stream(
        &sender_1,
        &recipient_1,
        &token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    // Stream 2 deposits 1,000 -> contract gets 900, fee collector gets 100
    let id_2 = contract.create_stream(
        &sender_2,
        &recipient_2,
        &token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    // Total contract balance is 1,800 (900 + 900), while promised total across streams is 2,000
    assert_eq!(fee_token.balance(&contract_id), 1_800);
    assert_eq!(fee_token.balance(&fee_collector), 200);

    // Advance to end of streams
    env.ledger().set_timestamp(1_100);

    // Stream 1 recipient withdraws full 1,000.
    // Contract has 1,800 >= 1,000, so withdrawal succeeds!
    // But this drains 100 tokens of Stream 2's underlying deposit from the contract pool.
    let withdrawn_1 = contract.withdraw(&id_1);
    assert_eq!(withdrawn_1, 1_000);

    // Contract balance is now 800 (1,800 - 1,000).
    // Recipient 1 receives 900 (1,000 - 10% fee), fee collector has 300.
    assert_eq!(fee_token.balance(&contract_id), 800);
    assert_eq!(fee_token.balance(&recipient_1), 900);
    assert_eq!(fee_token.balance(&fee_collector), 300);

    // Stream 2 recipient attempts to withdraw their promised 1,000 tokens:
    // Contract only holds 800, so the transfer fails!
    let res_2 = contract.try_withdraw(&id_2);
    assert!(
        res_2.is_err(),
        "Stream 2 withdrawal must fail due to collateral drained by Stream 1"
    );

    // Assert resulting balances:
    assert_eq!(fee_token.balance(&recipient_2), 0);
    assert_eq!(fee_token.balance(&contract_id), 800);
    assert_eq!(fee_token.balance(&recipient_1), 900);
    assert_eq!(fee_token.balance(&fee_collector), 300);
    assert_eq!(fee_token.balance(&sender_1), 0);
    assert_eq!(fee_token.balance(&sender_2), 0);

    // Stream 2 state remains untouched with withdrawn = 0
    let stream_2 = contract.get_stream(&id_2);
    assert_eq!(stream_2.withdrawn, 0);
    assert_eq!(contract.status(&id_2), StreamStatus::Completed);
}

// ── withdraw_to tests ────────────────────────────────────────────────────────

#[test]
fn withdraw_to_happy_path_to_third_party() {
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

    let destination = Address::generate(&t.env);
    t.set_time(600); // 500 vested

    let withdrawn = t.contract.withdraw_to(&id, &destination, &300);
    assert_eq!(withdrawn, 300);
    assert_eq!(t.token.balance(&destination), 300);
    assert_eq!(t.token.balance(&t.recipient), 0);
    assert_eq!(t.token.balance(&t.contract.address), 700);

    let stream = t.contract.get_stream(&id);
    assert_eq!(stream.withdrawn, 300);
    assert_eq!(t.contract.withdrawable(&id), 200);
}

#[test]
fn withdraw_to_destination_equals_recipient() {
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
    let withdrawn = t.contract.withdraw_to(&id, &t.recipient, &400);
    assert_eq!(withdrawn, 400);
    assert_eq!(t.token.balance(&t.recipient), 400);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 400);
    assert_eq!(t.contract.withdrawable(&id), 100);
}

#[test]
fn withdraw_to_requires_recipient_authorization() {
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

    let destination = Address::generate(&t.env);
    t.set_time(600);
    t.contract.withdraw_to(&id, &destination, &250);

    let auths = t.env.auths();
    assert!(auths.iter().any(|(addr, _)| addr == &t.recipient));
    assert!(!auths.iter().any(|(addr, _)| addr == &destination));
}

#[test]
fn withdraw_to_zero_or_negative_amount_fails() {
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

    let destination = Address::generate(&t.env);
    t.set_time(600);

    assert_eq!(
        t.contract.try_withdraw_to(&id, &destination, &0),
        Err(Ok(StreamError::InvalidAmount))
    );
    assert_eq!(
        t.contract.try_withdraw_to(&id, &destination, &-50),
        Err(Ok(StreamError::InvalidAmount))
    );
}

#[test]
fn withdraw_to_contract_as_destination_fails() {
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
    assert_eq!(
        t.contract.try_withdraw_to(&id, &t.contract.address, &100),
        Err(Ok(StreamError::InvalidParticipant))
    );
}

#[test]
fn withdraw_to_exceeding_balance_fails() {
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

    let destination = Address::generate(&t.env);
    t.set_time(600); // 500 vested

    assert_eq!(
        t.contract.try_withdraw_to(&id, &destination, &501),
        Err(Ok(StreamError::InsufficientBalance))
    );
}

#[test]
fn withdraw_to_nonexistent_stream_fails() {
    let t = StreamTest::setup(1_000);
    let destination = Address::generate(&t.env);

    assert_eq!(
        t.contract.try_withdraw_to(&999, &destination, &100),
        Err(Ok(StreamError::StreamNotFound))
    );
}

#[test]
fn withdraw_to_then_cancel_preserves_accounting() {
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

    let destination = Address::generate(&t.env);
    t.set_time(600); // 500 vested

    // Recipient withdraws 200 to destination
    assert_eq!(t.contract.withdraw_to(&id, &destination, &200), 200);

    // Sender cancels at time 600
    // Vested is 500, refund is 500 (1000 - 500), recipient still has 300 claimable
    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 500);
    assert_eq!(t.token.balance(&t.sender), 500);
    assert_eq!(t.contract.withdrawable(&id), 300);

    // Recipient withdraws remaining 300 to destination
    assert_eq!(t.contract.withdraw_to(&id, &destination, &300), 300);
    assert_eq!(t.token.balance(&destination), 500);
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

#[test]
fn withdraw_to_on_cancelled_stream_pays_up_to_frozen_vested_balance() {
    use soroban_sdk::Event;

    use crate::events::Withdrawn;

    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    // Cancel at midpoint so vested balance freezes at 500.
    t.set_time(600);
    assert_eq!(t.contract.cancel(&id), 500);

    // Advancing past the original end time does not unfreeze the vested cap.
    t.set_time(1_200);
    let destination = Address::generate(&t.env);

    assert_eq!(
        t.contract.try_withdraw_to(&id, &destination, &501),
        Err(Ok(StreamError::InsufficientBalance))
    );

    assert_eq!(t.contract.withdraw_to(&id, &destination, &500), 500);
    t.assert_latest_stream_event_topics(
        Withdrawn {
            recipient: t.recipient.clone(),
            id,
            amount: 500,
            destination: destination.clone(),
        }
        .to_xdr(&t.env, &t.contract.address),
    );
    assert_eq!(t.token.balance(&destination), 500);
    assert_eq!(t.token.balance(&t.recipient), 0);
    assert_eq!(t.token.balance(&t.contract.address), 0);
    assert_eq!(t.contract.withdrawable(&id), 0);
}
