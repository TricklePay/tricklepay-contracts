#![cfg(test)]

use crate::StreamError;

use super::helpers::StreamTest;

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
    assert_eq!(t.contract.withdraw(&id), 500);
    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.contract.withdrawable(&id), 0);

    // Three-quarter point: another 250 has vested.
    t.set_time(850);
    assert_eq!(t.contract.withdraw(&id), 250);
    assert_eq!(t.token.balance(&t.recipient), 750);

    // End: the final 250.
    t.set_time(1_100);
    assert_eq!(t.contract.withdraw(&id), 250);
    assert_eq!(t.token.balance(&t.recipient), 1_000);

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
    assert_eq!(t.contract.withdraw_amount(&id, &200), 200);
    assert_eq!(t.token.balance(&t.recipient), 200);
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
    assert_eq!(t.token.balance(&t.contract.address), contract_balance_before);
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
    assert_eq!(t.token.balance(&t.contract.address), contract_balance_before);
    assert_eq!(t.contract.get_stream(&id).withdrawn, withdrawn_before);

    let recipient_balance_before = t.token.balance(&t.recipient);
    let contract_balance_before = t.token.balance(&t.contract.address);
    let withdrawn_before = t.contract.get_stream(&id).withdrawn;
    assert_eq!(
        t.contract.try_withdraw_amount(&id, &-1),
        Err(Ok(StreamError::InvalidAmount))
    );
    assert_eq!(t.token.balance(&t.recipient), recipient_balance_before);
    assert_eq!(t.token.balance(&t.contract.address), contract_balance_before);
    assert_eq!(t.contract.get_stream(&id).withdrawn, withdrawn_before);
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
fn second_withdraw_without_progress_is_rejected() {
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

    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );
    assert_eq!(t.token.balance(&t.recipient), 500);
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
