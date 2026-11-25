#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Bytes,
};

use crate::contract::{StreamContract, StreamContractClient};

use crate::{StreamError, StreamStatus, MAX_AMOUNT, MAX_REFERENCE_LENGTH};

use super::helpers::{MockTokenWithDecimals, MockTokenWithDecimalsClient, StreamTest};

#[test]
fn create_stream_with_reference_stores_and_reports_the_reference() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let reference = Bytes::from_slice(&t.env, b"INV-2026-0042");

    let id = t.contract.create_stream_with_reference(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
        &reference,
    );

    assert_eq!(t.contract.get_stream(&id).reference, reference);
}

#[test]
fn existing_create_stream_uses_an_empty_reference() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    assert_eq!(t.contract.get_stream(&id).reference, Bytes::new(&t.env));
}

#[test]
fn reference_length_is_bounded_without_changing_storage_or_tokens_on_rejection() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let too_long = [65u8; 65];
    let too_long = Bytes::from_slice(&t.env, &too_long);
    assert_eq!(
        t.contract.try_create_stream_with_reference(
            &t.sender,
            &t.recipient,
            &t.token_address,
            &1_000,
            &100,
            &1_100,
            &100,
            &too_long,
        ),
        Err(Ok(StreamError::ReferenceTooLong))
    );
    assert_eq!(t.contract.stream_count(), 0);

    let maximum = [65u8; 64];
    let maximum = Bytes::from_slice(&t.env, &maximum);
    let id = t.contract.create_stream_with_reference(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
        &maximum,
    );
    assert_eq!(
        t.contract.get_stream(&id).reference.len(),
        MAX_REFERENCE_LENGTH
    );
}

#[test]
fn create_stream_locks_funds_and_assigns_id() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    // start == cliff means the stream has no cliff.
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

    // The full amount has moved from the sender into the contract.
    assert_eq!(t.token.balance(&t.sender), 0);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);

    let stream = t.contract.get_stream(&id);
    assert_eq!(stream.sender, t.sender);
    assert_eq!(stream.recipient, t.recipient);
    assert_eq!(stream.token, t.token_address);
    assert_eq!(stream.total_amount, 1_000);
    assert_eq!(stream.withdrawn, 0);
    assert_eq!(stream.start_time, 100);
    assert_eq!(stream.end_time, 1_100);
    assert_eq!(stream.cliff_time, 100);
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
    assert_eq!(first.start_time, 100);
    assert_eq!(first.end_time, 1_100);
    assert_eq!(first.cliff_time, 100);
    assert_eq!(first.withdrawn, 0);

    let second = t.contract.get_stream(&second_id);
    assert_eq!(second.token, second_token_address);
    assert_eq!(second.total_amount, 900);
    assert_eq!(second.start_time, 100);
    assert_eq!(second.end_time, 1_100);
    assert_eq!(second.cliff_time, 100);
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

    // Start is not strictly before end.
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

    // Cliff before the start.
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

    // Cliff after the end.
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

    // None of the rejected calls created state or moved funds.
    assert_eq!(t.contract.stream_count(), 0);
    assert_eq!(t.token.balance(&t.sender), 1_000);

    // The next successful creation receives the still-unused first id.
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

/// `create_stream` must accept exactly `MAX_AMOUNT` — the boundary is
/// inclusive and the stream must work end-to-end without overflow.
#[test]
fn create_stream_accepts_max_amount() {
    let t = StreamTest::setup(MAX_AMOUNT);
    t.set_time(100);

    // A one-second stream maximises elapsed/duration pressure.
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &MAX_AMOUNT,
        &100,
        &101,
        &100,
    );

    // At the midpoint (t == 100, i.e. 0 elapsed out of 1 second) nothing
    // has vested yet.
    assert_eq!(t.contract.withdrawable(&id), 0);

    // At end_time the full amount is vested and withdrawable without panic.
    t.set_time(101);
    assert_eq!(t.contract.vested(&id), MAX_AMOUNT);
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

/// Vesting should never go backwards as time advances. This regression test
/// samples several points across the schedule and asserts the vested amount is
/// non-decreasing at each step.
#[test]
fn vested_amount_never_decreases_over_time() {
    let t = StreamTest::setup(1_000);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    let checkpoints = [(100, 0), (350, 250), (600, 500), (850, 750), (1_100, 1_000)];
    let mut previous = None;

    for (now, expected) in checkpoints {
        t.set_time(now);
        let vested = t.contract.vested(&id);
        assert_eq!(
            vested, expected,
            "vested amount at t={now} should match the schedule"
        );

        if let Some(prev) = previous {
            assert!(
                vested >= prev,
                "vesting decreased from {prev} to {vested} as time advanced to {now}"
            );
        }
        previous = Some(vested);
    }
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
    assert!(
        q > 0 && q < MAX_AMOUNT,
        "quarter-point vested={q} out of range"
    );

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

/// A schedule whose entire window has elapsed is rejected before funding or
/// storage, rather than creating a stream that is already fully vested.
#[test]
fn create_stream_rejects_an_entirely_past_schedule_without_state_change() {
    let t = StreamTest::setup(1_000);
    t.set_time(2_000);

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    assert_eq!(result, Err(Ok(StreamError::StreamWindowInPast)));
    assert_eq!(t.contract.stream_count(), 0);
    assert_eq!(t.token.balance(&t.sender), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);
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

/// When an argument list breaks more than one rule, which error comes back is
/// fixed by the documented order on `create_stream` rather than by the
/// incidental arrangement of the checks. Each case below violates two rules
/// and must report the earlier one.
#[test]
fn create_stream_validation_order_is_deterministic() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    // Participants (2) beat amount (3): stream to the contract with a zero amount.
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &t.contract.address,
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

    // Nothing above moved a token or consumed an id.
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

/// Issue #358 — a stream created at the current ledger time has no elapsed
/// vesting time and therefore starts with zero vested balance.
#[test]
fn stream_created_at_current_ledger_time_vests_from_zero() {
    let t = StreamTest::setup(1_000);
    let start = 500;
    t.set_time(start);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &start,
        &(start + 1_000),
        &start,
    );

    assert_eq!(t.contract.vested(&id), 0);
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(t.contract.progress(&id), 0);
    assert_eq!(t.contract.status(&id), StreamStatus::Streaming);
    assert_eq!(t.contract.get_stream(&id).created_at, start);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);
}

/// The token contract itself may act as a stream participant (sender or
/// recipient) to preserve smart contract composability. This protects the
/// README's statement: "A token contract acting as the `sender` or `recipient`
/// is permitted. Rejecting this could break legitimate smart contract
/// composability."
#[test]
fn token_contract_may_be_a_stream_recipient() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    // The token contract address is the recipient; only the stream contract
    // itself is refused.
    let id = t.contract.create_stream(
        &t.sender,
        &t.token_address,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    assert_eq!(t.contract.stream_count(), 1);
    assert_eq!(t.token.balance(&t.sender), 0);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);

    let stream = t.contract.get_stream(&id);
    assert_eq!(stream.sender, t.sender);
    assert_eq!(stream.recipient, t.token_address);
    assert_eq!(stream.token, t.token_address);
    assert_eq!(stream.total_amount, 1_000);
}

/// A stream where sender == recipient is accepted and behaves normally.
///
/// This test pins the behavior documented in README.md § Deliberately omitted
/// validations: "Sender and recipient being the same address: A stream from an
/// address to itself has the effect of locking the sender's own tokens and
/// handing them back over time. This is a legitimate way to self-stream funds."
///
/// See also docs/DECISIONS.md § ADR: Self-Streams Are Allowed.
#[test]
fn self_stream_is_accepted_and_behaves_normally() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    // Create a stream where sender == recipient (self-stream).
    let self_address = t.sender.clone();
    let id = t.contract.create_stream(
        &self_address,
        &self_address, // recipient is the same as sender
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    // The stream is created successfully.
    assert_eq!(id, 0);
    assert_eq!(t.contract.stream_count(), 1);

    // The full amount is locked in the contract.
    assert_eq!(t.token.balance(&self_address), 0);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);

    // The stream record reflects the self-stream.
    let stream = t.contract.get_stream(&id);
    assert_eq!(stream.sender, self_address);
    assert_eq!(stream.recipient, self_address);
    assert_eq!(stream.token, t.token_address);
    assert_eq!(stream.total_amount, 1_000);
    assert_eq!(stream.withdrawn, 0);
    assert!(!stream.cancelled);

    // Vesting works normally. At the midpoint, 500 tokens have vested.
    t.set_time(600);
    assert_eq!(t.contract.vested(&id), 500);
    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.locked(&id), 500);

    // Withdrawal works: the self-address can withdraw vested funds.
    let withdrawn = t.contract.withdraw(&id);
    assert_eq!(withdrawn, 500);
    assert_eq!(t.token.balance(&self_address), 500);
    assert_eq!(t.token.balance(&t.contract.address), 500);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 500);

    // Create a second self-stream and cancel it.
    let id2 = t.contract.create_stream(
        &self_address,
        &self_address,
        &t.token_address,
        &500, // the 500 remaining from withdrawal above
        &600,
        &1_600,
        &600,
    );
    assert_eq!(id2, 1);
    assert_eq!(t.token.balance(&self_address), 0);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);

    // Cancel the second self-stream at the midpoint.
    t.set_time(1_100);
    let refund = t.contract.cancel(&id2);

    // The refund goes back to the self-address (who is both sender and recipient).
    // At t=1100, elapsed = 500 out of 1000, so vested = 250, refund = 250.
    assert_eq!(refund, 250);
    assert_eq!(t.token.balance(&self_address), 250);
    assert!(t.contract.get_stream(&id2).cancelled);

    // The self-address can still withdraw the vested portion from the cancelled stream.
    assert_eq!(t.contract.withdrawable(&id2), 250);
    let final_withdrawal = t.contract.withdraw(&id2);
    assert_eq!(final_withdrawal, 250);
    assert_eq!(t.token.balance(&self_address), 500);

    // Final accounting: the self-address has withdrawn 500 from the first stream
    // and 500 total from the second stream (250 refund + 250 withdrawal).
    // The contract now holds 500 from the first stream (still unvested).
    assert_eq!(t.token.balance(&t.contract.address), 500);
}

/// Issue #620 — Streams of tokens with different decimals (0, 7, and 18 decimals)
/// created side-by-side vest, withdraw, preview, and cancel strictly in base units.
/// Furthermore, the stream contract is completely decimals-agnostic and never reads
/// the token's `decimals()` method on any code path.
#[test]
fn tokens_with_different_decimals_vest_withdraw_and_cancel_in_base_units() {
    let t = StreamTest::setup(0);

    // 1. Register three mock tokens with 0, 7, and 18 decimals.
    let token_0_id = t.env.register(MockTokenWithDecimals, ());
    let token_0 = MockTokenWithDecimalsClient::new(&t.env, &token_0_id);
    token_0.initialize(&0);
    assert_eq!(token_0.decimals(), 0);

    let token_7_id = t.env.register(MockTokenWithDecimals, ());
    let token_7 = MockTokenWithDecimalsClient::new(&t.env, &token_7_id);
    token_7.initialize(&7);
    assert_eq!(token_7.decimals(), 7);

    let token_18_id = t.env.register(MockTokenWithDecimals, ());
    let token_18 = MockTokenWithDecimalsClient::new(&t.env, &token_18_id);
    token_18.initialize(&18);
    assert_eq!(token_18.decimals(), 18);

    // Reset decimals-read tracking so we can assert the stream contract never accesses it.
    token_0.reset_decimals_read();
    token_7.reset_decimals_read();
    token_18.reset_decimals_read();

    // 2. Mint tokens to sender in their respective base units:
    // - Token 0 (0 decimals): 100 units
    // - Token 7 (7 decimals): 1_000_000_000 base units (100.0000000)
    // - Token 18 (18 decimals): 4_000_000_000_000_000_000 base units (4.000000000000000000)
    //   Note: amounts are bounded by MAX_AMOUNT (i64::MAX as i128 ≈ 9.22 * 10^18 base units).
    let amount_0: i128 = 100;
    let amount_7: i128 = 1_000_000_000;
    let amount_18: i128 = 4_000_000_000_000_000_000;

    token_0.mint(&t.sender, &amount_0);
    token_7.mint(&t.sender, &amount_7);
    token_18.mint(&t.sender, &amount_18);

    // 3. Create three streams side by side with identical schedules:
    // start at 100, cliff at 600 (50% cliff), end at 1100 (duration = 1000s).
    t.set_time(100);

    let id_0 = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &token_0_id,
        &amount_0,
        &100,
        &1_100,
        &600,
    );
    let id_7 = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &token_7_id,
        &amount_7,
        &100,
        &1_100,
        &600,
    );
    let id_18 = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &token_18_id,
        &amount_18,
        &100,
        &1_100,
        &600,
    );

    // Assert funds are locked in the contract in base units.
    assert_eq!(token_0.balance(&t.sender), 0);
    assert_eq!(token_0.balance(&t.contract.address), amount_0);

    assert_eq!(token_7.balance(&t.sender), 0);
    assert_eq!(token_7.balance(&t.contract.address), amount_7);

    assert_eq!(token_18.balance(&t.sender), 0);
    assert_eq!(token_18.balance(&t.contract.address), amount_18);

    // 4. Before the cliff (at t = 350, 25% elapsed):
    t.set_time(350);
    assert_eq!(t.contract.vested(&id_0), 0);
    assert_eq!(t.contract.withdrawable(&id_0), 0);
    assert_eq!(t.contract.locked(&id_0), amount_0);

    assert_eq!(t.contract.vested(&id_7), 0);
    assert_eq!(t.contract.withdrawable(&id_7), 0);
    assert_eq!(t.contract.locked(&id_7), amount_7);

    assert_eq!(t.contract.vested(&id_18), 0);
    assert_eq!(t.contract.withdrawable(&id_18), 0);
    assert_eq!(t.contract.locked(&id_18), amount_18);

    // 5. At the cliff (t = 600, 50% elapsed): exactly 50% has vested in base units.
    t.set_time(600);
    assert_eq!(t.contract.vested(&id_0), 50);
    assert_eq!(t.contract.withdrawable(&id_0), 50);
    assert_eq!(t.contract.locked(&id_0), 50);

    assert_eq!(t.contract.vested(&id_7), 500_000_000);
    assert_eq!(t.contract.withdrawable(&id_7), 500_000_000);
    assert_eq!(t.contract.locked(&id_7), 500_000_000);

    assert_eq!(t.contract.vested(&id_18), 2_000_000_000_000_000_000);
    assert_eq!(t.contract.withdrawable(&id_18), 2_000_000_000_000_000_000);
    assert_eq!(t.contract.locked(&id_18), 2_000_000_000_000_000_000);

    // 6. Perform partial withdrawals across all 3 streams:
    // - Stream 0: withdraw 20 base units
    // - Stream 7: withdraw full 500_000_000 available base units
    // - Stream 18: withdraw 1_000_000_000_000_000_000 base units (1 token)
    let w_0 = t.contract.withdraw_amount(&id_0, &20);
    assert_eq!(w_0, 20);
    assert_eq!(token_0.balance(&t.recipient), 20);
    assert_eq!(token_0.balance(&t.contract.address), 80);

    let w_7 = t.contract.withdraw(&id_7);
    assert_eq!(w_7, 500_000_000);
    assert_eq!(token_7.balance(&t.recipient), 500_000_000);
    assert_eq!(token_7.balance(&t.contract.address), 500_000_000);

    let w_18 = t
        .contract
        .withdraw_amount(&id_18, &1_000_000_000_000_000_000);
    assert_eq!(w_18, 1_000_000_000_000_000_000);
    assert_eq!(token_18.balance(&t.recipient), 1_000_000_000_000_000_000);
    assert_eq!(
        token_18.balance(&t.contract.address),
        3_000_000_000_000_000_000
    );

    // 7. Advance time to t = 850 (75% elapsed).
    // Vested is 75% in base units:
    // - Stream 0: vested = 75. Withdrawn = 20. Remaining withdrawable = 55. Refund = 25.
    // - Stream 7: vested = 750_000_000. Withdrawn = 500_000_000. Remaining withdrawable = 250_000_000. Refund = 250_000_000.
    // - Stream 18: vested = 3_000_000_000_000_000_000. Withdrawn = 1_000_000_000_000_000_000.
    //              Remaining withdrawable = 2_000_000_000_000_000_000. Refund = 1_000_000_000_000_000_000.
    t.set_time(850);

    let preview_0 = t.contract.cancel_preview(&id_0);
    assert_eq!(preview_0.refund, 25);
    assert_eq!(preview_0.recipient_remaining, 55);

    let preview_7 = t.contract.cancel_preview(&id_7);
    assert_eq!(preview_7.refund, 250_000_000);
    assert_eq!(preview_7.recipient_remaining, 250_000_000);

    let preview_18 = t.contract.cancel_preview(&id_18);
    assert_eq!(preview_18.refund, 1_000_000_000_000_000_000);
    assert_eq!(preview_18.recipient_remaining, 2_000_000_000_000_000_000);

    // 8. Cancel all three streams and assert settlements in base units.
    let refund_0 = t.contract.cancel(&id_0);
    assert_eq!(refund_0, 25);
    assert_eq!(token_0.balance(&t.sender), 25);

    let refund_7 = t.contract.cancel(&id_7);
    assert_eq!(refund_7, 250_000_000);
    assert_eq!(token_7.balance(&t.sender), 250_000_000);

    let refund_18 = t.contract.cancel(&id_18);
    assert_eq!(refund_18, 1_000_000_000_000_000_000);
    assert_eq!(token_18.balance(&t.sender), 1_000_000_000_000_000_000);

    // 9. Recipient withdraws remaining vested funds from all three cancelled streams.
    assert_eq!(t.contract.withdraw(&id_0), 55);
    assert_eq!(token_0.balance(&t.recipient), 75);
    assert_eq!(token_0.balance(&t.contract.address), 0);

    assert_eq!(t.contract.withdraw(&id_7), 250_000_000);
    assert_eq!(token_7.balance(&t.recipient), 750_000_000);
    assert_eq!(token_7.balance(&t.contract.address), 0);

    assert_eq!(t.contract.withdraw(&id_18), 2_000_000_000_000_000_000);
    assert_eq!(token_18.balance(&t.recipient), 3_000_000_000_000_000_000);
    assert_eq!(token_18.balance(&t.contract.address), 0);

    // 10. Assert that no code path read decimals() on any of the three tokens.
    assert!(
        !token_0.decimals_were_read(),
        "decimals() should never be read by the contract for token with 0 decimals"
    );
    assert!(
        !token_7.decimals_were_read(),
        "decimals() should never be read by the contract for token with 7 decimals"
    );
    assert!(
        !token_18.decimals_were_read(),
        "decimals() should never be read by the contract for token with 18 decimals"
    );
}
