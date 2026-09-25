#![cfg(test)]

use soroban_sdk::{testutils::Address as _, token, Address};

use crate::contract::{StreamContract, StreamContractClient};
use crate::{StreamError, StreamStatus, MAX_AMOUNT};

use super::helpers::StreamTest;

#[test]
fn create_stream_locks_funds_and_assigns_id() {
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

    assert_eq!(id, 0);
    assert_eq!(t.contract.stream_count(), 1);

    assert_eq!(t.token.balance(&t.sender), 0);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);

    let stream = t.contract.get_stream(&id);
    assert_eq!(stream.sender, t.sender);
    assert_eq!(stream.recipient, t.recipient);
    assert_eq!(stream.token, t.token_address);
    assert_eq!(stream.total_amount, 1_000);
    assert_eq!(stream.withdrawn, 0);
    assert!(!stream.cancelled);
}

#[test]
fn one_sender_can_stream_multiple_tokens_in_parallel() {
    let t = StreamTest::setup(1_000);
    let second_issuer = Address::generate(&t.env);
    let second_sac = t.env.register_stellar_asset_contract_v2(second_issuer);
    let second_token_address = second_sac.address();
    let second_token = token::TokenClient::new(&t.env, &second_token_address);
    let second_token_admin = token::StellarAssetClient::new(&t.env, &second_token_address);
    second_token_admin.mint(&t.sender, &2_000);
    t.set_time(100);

    let first_id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &400,
        &100,
        &1_100,
        &100,
    );
    let second_id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &second_token_address,
        &900,
        &100,
        &1_100,
        &100,
    );

    assert_eq!(first_id, 0);
    assert_eq!(second_id, 1);
    assert_eq!(t.contract.stream_count(), 2);
    assert_eq!(t.token.balance(&t.sender), 600);
    assert_eq!(second_token.balance(&t.sender), 1_100);
    assert_eq!(t.token.balance(&t.contract.address), 400);
    assert_eq!(second_token.balance(&t.contract.address), 900);

    let first = t.contract.get_stream(&first_id);
    assert_eq!(first.token, t.token_address);
    assert_eq!(first.total_amount, 400);
    assert_eq!(first.withdrawn, 0);

    let second = t.contract.get_stream(&second_id);
    assert_eq!(second.token, second_token_address);
    assert_eq!(second.total_amount, 900);
    assert_eq!(second.withdrawn, 0);

    t.set_time(600);
    assert_eq!(t.contract.withdraw(&first_id), 200);
    assert_eq!(t.contract.withdraw(&second_id), 450);
    assert_eq!(t.token.balance(&t.recipient), 200);
    assert_eq!(second_token.balance(&t.recipient), 450);
    assert_eq!(t.token.balance(&t.contract.address), 200);
    assert_eq!(second_token.balance(&t.contract.address), 450);
    assert_eq!(t.contract.get_stream(&first_id).withdrawn, 200);
    assert_eq!(t.contract.get_stream(&second_id).withdrawn, 450);
}

#[test]
fn create_stream_rejects_invalid_parameters() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let zero_amount = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &0,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(zero_amount, Err(Ok(StreamError::InvalidAmount)));

    let negative_amount = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &-5,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(negative_amount, Err(Ok(StreamError::InvalidAmount)));

    let bad_range = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &1_100,
        &1_100,
        &1_100,
    );
    assert_eq!(bad_range, Err(Ok(StreamError::InvalidTimeRange)));

    let cliff_early = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &50,
    );
    assert_eq!(cliff_early, Err(Ok(StreamError::InvalidCliff)));

    let cliff_late = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &1_200,
    );
    assert_eq!(cliff_late, Err(Ok(StreamError::InvalidCliff)));

    assert_eq!(t.contract.stream_count(), 0);
    assert_eq!(t.token.balance(&t.sender), 1_000);
}

// ── Overflow-guard / MAX_AMOUNT boundary tests ──────────────────────────────

/// `create_stream` must reject `total_amount == MAX_AMOUNT + 1` with
/// `AmountTooLarge`. This is the boundary value: one above the cap.
#[test]
fn create_stream_rejects_amount_above_max() {
    let t = StreamTest::setup(MAX_AMOUNT + 1);
    t.set_time(100);

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &(MAX_AMOUNT + 1),
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::AmountTooLarge)));

    assert_eq!(t.contract.stream_count(), 0);
    assert_eq!(t.token.balance(&t.sender), MAX_AMOUNT + 1);
}

/// `create_stream` must accept exactly `MAX_AMOUNT`.
#[test]
fn create_stream_accepts_max_amount() {
    let t = StreamTest::setup(MAX_AMOUNT);
    t.set_time(100);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &MAX_AMOUNT,
        &100,
        &101,
        &100,
    );

    assert_eq!(t.contract.withdrawable(&id), 0);

    t.set_time(101);
    assert_eq!(t.contract.withdrawable(&id), MAX_AMOUNT);
    assert_eq!(t.contract.withdraw(&id), MAX_AMOUNT);
    assert_eq!(t.token.balance(&t.recipient), MAX_AMOUNT);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// `i128::MAX` is well above `MAX_AMOUNT` and must be rejected.
#[test]
fn create_stream_rejects_i128_max() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &i128::MAX,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::AmountTooLarge)));
}

/// A long-lived stream (duration close to u64::MAX) with an amount at the
/// cap must compute vested amounts without overflow at any point in time.
#[test]
fn vesting_with_max_amount_over_long_duration_does_not_overflow() {
    let duration: u64 = u64::MAX / 2;
    let start: u64 = 0;
    let end: u64 = duration;

    let t = StreamTest::setup(MAX_AMOUNT);
    t.set_time(start);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &MAX_AMOUNT,
        &start,
        &end,
        &start,
    );

    t.set_time(duration / 4);
    let q = t.contract.vested(&id);
    assert!(q > 0 && q < MAX_AMOUNT, "quarter-point vested={q} out of range");

    t.set_time(duration / 2);
    let half = t.contract.vested(&id);
    assert!(half > q, "midpoint must exceed quarter-point");

    t.set_time(end);
    assert_eq!(t.contract.vested(&id), MAX_AMOUNT);
}

// ── Past time-window rejection ───────────────────────────────────────────────

/// A stream whose `end_time` is strictly before the current ledger time must
/// be rejected with `StreamWindowInPast`.
#[test]
fn create_stream_rejects_end_time_in_the_past() {
    let t = StreamTest::setup(1_000);
    t.set_time(1_000);

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &900,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::StreamWindowInPast)));

    assert_eq!(t.contract.stream_count(), 0);
    assert_eq!(t.token.balance(&t.sender), 1_000);
}

/// A stream whose `end_time` equals the current ledger timestamp must also be
/// rejected.
#[test]
fn create_stream_rejects_end_time_equal_to_now() {
    let t = StreamTest::setup(1_000);
    t.set_time(1_000);

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_000,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::StreamWindowInPast)));

    assert_eq!(t.contract.stream_count(), 0);
    assert_eq!(t.token.balance(&t.sender), 1_000);
}

/// A stream whose `start_time` is in the past but `end_time` is in the future
/// is a valid backdated schedule.
#[test]
fn create_stream_accepts_past_start_time_with_future_end_time() {
    let t = StreamTest::setup(1_000);
    t.set_time(600);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    assert_eq!(t.contract.stream_count(), 1);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);

    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.withdraw(&id), 500);
    assert_eq!(t.token.balance(&t.recipient), 500);
}

// ── Timestamp boundary tests ─────────────────────────────────────────────────

/// `start_time == 0` and a future `end_time` is a valid edge case.
#[test]
fn create_stream_accepts_start_time_of_zero() {
    let t = StreamTest::setup(1_000);
    t.set_time(500);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &0,
        &1_000,
        &0,
    );

    assert_eq!(t.contract.stream_count(), 1);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);

    assert_eq!(t.contract.vested(&id), 500);
    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.locked(&id), 500);
}

/// `end_time == now + 1` is the tightest valid window.
#[test]
fn create_stream_accepts_end_time_one_second_in_the_future() {
    let t = StreamTest::setup(1_000);
    t.set_time(1_000);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &999,
        &1_001,
        &999,
    );

    assert_eq!(t.contract.stream_count(), 1);

    t.set_time(1_001);
    assert_eq!(t.contract.withdrawable(&id), 1_000);
}

/// The id counter must never wrap at `u64::MAX`.
#[test]
fn create_stream_rejects_an_exhausted_counter() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    t.set_stream_count(u64::MAX);

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::StreamCountExhausted)));

    assert_eq!(t.contract.stream_count(), u64::MAX);
    assert_eq!(t.token.balance(&t.sender), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// The last id below the ceiling is still usable, and using it takes the
/// counter to exactly `u64::MAX`.
#[test]
fn create_stream_accepts_the_final_id_then_refuses_the_next() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    t.set_stream_count(u64::MAX - 1);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(id, u64::MAX - 1);
    assert_eq!(t.contract.stream_count(), u64::MAX);
    assert_eq!(t.contract.get_stream(&id).total_amount, 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::StreamCountExhausted)));

    assert_eq!(t.contract.get_stream(&id).total_amount, 1_000);
    assert_eq!(t.token.balance(&t.sender), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);
}

// ── Participant validation ───────────────────────────────────────────────────

/// The contract's own address as recipient would lock the tokens forever.
#[test]
fn create_stream_rejects_the_contract_as_recipient() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let contract_address = t.contract.address.clone();

    let result = t.contract.try_create_stream(
        &t.sender,
        &contract_address,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::InvalidParticipant)));
    t.assert_nothing_happened(1_000);
}

/// The contract as sender would let a caller draw on the pooled holdings.
#[test]
fn create_stream_rejects_the_contract_as_sender() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let contract_address = t.contract.address.clone();

    let result = t.contract.try_create_stream(
        &contract_address,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::InvalidParticipant)));
    t.assert_nothing_happened(1_000);
}

/// The contract as the token must be rejected.
#[test]
fn create_stream_rejects_the_contract_as_token() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let contract_address = t.contract.address.clone();

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &contract_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::InvalidParticipant)));
    t.assert_nothing_happened(1_000);
}

/// Using the sender or recipient address as the token input is rejected.
#[test]
fn create_stream_rejects_token_equal_to_sender_or_recipient() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.sender,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::InvalidParticipant)));
    t.assert_nothing_happened(1_000);

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.recipient,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::InvalidParticipant)));
    t.assert_nothing_happened(1_000);
}

/// A stream from an address to itself must be refused.
#[test]
fn create_stream_rejects_a_stream_to_self() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.sender,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::InvalidParticipant)));
    t.assert_nothing_happened(1_000);
}

// ── Validation order ─────────────────────────────────────────────────────────

/// When an argument list breaks more than one rule, the error reported is
/// fixed by the documented order on `create_stream`.
#[test]
fn create_stream_validation_order_is_deterministic() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    // Participants (2) beat amount (3): self-stream with a zero amount.
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &t.sender,
            &t.token_address,
            &0,
            &100,
            &1_100,
            &100
        ),
        Err(Ok(StreamError::InvalidParticipant))
    );

    // Participants (2) beat schedule (4): the contract as recipient, with a
    // window that is also inverted.
    let contract_address = t.contract.address.clone();
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &contract_address,
            &t.token_address,
            &1_000,
            &1_100,
            &100,
            &1_100
        ),
        Err(Ok(StreamError::InvalidParticipant))
    );

    // Amount (3) beats schedule (4): zero amount with an inverted window.
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &t.recipient,
            &t.token_address,
            &0,
            &1_100,
            &100,
            &1_100
        ),
        Err(Ok(StreamError::InvalidAmount))
    );

    // Amount (3) beats capacity (5): an exhausted counter is reported only
    // once the arguments themselves are sound.
    t.set_stream_count(u64::MAX);
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &t.recipient,
            &t.token_address,
            &0,
            &100,
            &1_100,
            &100
        ),
        Err(Ok(StreamError::InvalidAmount))
    );
    // With sound arguments the same counter now surfaces.
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &t.recipient,
            &t.token_address,
            &1_000,
            &100,
            &1_100,
            &100
        ),
        Err(Ok(StreamError::StreamCountExhausted))
    );

    assert_eq!(t.token.balance(&t.sender), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// Within the schedule group the order is also fixed: range, then cliff, then
/// the past-window rule.
#[test]
fn create_stream_schedule_checks_run_in_order() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    // An inverted window whose cliff is also out of bounds reports the range.
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &t.recipient,
            &t.token_address,
            &1_000,
            &1_100,
            &100,
            &50
        ),
        Err(Ok(StreamError::InvalidTimeRange))
    );

    // A cliff past the end, on a window that has also already elapsed,
    // reports the cliff.
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &t.recipient,
            &t.token_address,
            &1_000,
            &10,
            &50,
            &60
        ),
        Err(Ok(StreamError::InvalidCliff))
    );

    // With a well-formed cliff, the elapsed window is what is reported.
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &t.recipient,
            &t.token_address,
            &1_000,
            &10,
            &50,
            &10
        ),
        Err(Ok(StreamError::StreamWindowInPast))
    );

    t.assert_nothing_happened(1_000);
}

/// Secondary counter overflow test: fails closed without wrapping to 0.
#[test]
fn stream_id_counter_overflow_fails_closed() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    t.set_stream_count(u64::MAX);

    let res = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &500,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(res, Err(Ok(StreamError::StreamCountExhausted)));
    assert_eq!(t.contract.stream_count(), u64::MAX);
    assert_eq!(t.token.balance(&t.sender), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// Native asset token compatibility: full lifecycle with a SAC native token.
#[test]
fn native_asset_token_compatibility_lifecycle() {
    use soroban_sdk::Env;
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(StreamContract, ());
    let contract = StreamContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let native_token_address = sac.address();
    let native_token = token::TokenClient::new(&env, &native_token_address);
    let native_admin = token::StellarAssetClient::new(&env, &native_token_address);

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let amount: i128 = 2_000;

    native_admin.mint(&sender, &amount);
    assert_eq!(native_token.balance(&sender), amount);

    env.ledger().set_timestamp(100);

    let id = contract.create_stream(
        &sender,
        &recipient,
        &native_token_address,
        &amount,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(id, 0);

    assert_eq!(native_token.balance(&sender), 0);
    assert_eq!(native_token.balance(&contract.address), amount);

    env.ledger().set_timestamp(600);
    let withdrawn = contract.withdraw(&id);
    assert_eq!(withdrawn, 1_000);
    assert_eq!(native_token.balance(&recipient), 1_000);

    let refund = contract.cancel(&id);
    assert_eq!(refund, 1_000);
    assert_eq!(native_token.balance(&sender), 1_000);
    assert_eq!(contract.status(&id), StreamStatus::Cancelled);
}
