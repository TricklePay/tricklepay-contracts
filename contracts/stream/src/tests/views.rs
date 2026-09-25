#![cfg(test)]

use crate::{StreamError, StreamStatus};

use super::helpers::StreamTest;

#[test]
fn progress_reports_basis_points() {
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

    assert_eq!(t.contract.progress(&id), 0);
    t.set_time(600);
    assert_eq!(t.contract.progress(&id), 5_000);
    t.set_time(1_100);
    assert_eq!(t.contract.progress(&id), 10_000);
}

#[test]
fn locked_decreases_as_the_stream_vests() {
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

    assert_eq!(t.contract.locked(&id), 1_000);
    t.set_time(600);
    assert_eq!(t.contract.locked(&id), 500);
    t.set_time(1_100);
    assert_eq!(t.contract.locked(&id), 0);
}

// ── Post-cancellation view correctness ──────────────────────────────────────

/// `locked` and `progress` after cancellation must report 0 and 10 000.
#[test]
fn views_are_correct_on_a_cancelled_stream() {
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

    assert_eq!(t.contract.locked(&id), 0);
    assert_eq!(t.contract.progress(&id), 10_000);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);
    assert_eq!(t.contract.withdrawable(&id), 500);
}

/// Views remain correct after the recipient drains a cancelled stream.
#[test]
fn views_remain_correct_after_recipient_drains_cancelled_stream() {
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
    t.set_time(2_000);

    let withdrawn = t.contract.withdraw(&id);
    assert_eq!(withdrawn, 500);

    assert_eq!(t.token.balance(&t.sender), 500);
    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&t.contract.address), 0);

    assert_eq!(t.contract.locked(&id), 0);
    assert_eq!(t.contract.progress(&id), 10_000);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);
    assert_eq!(t.contract.withdrawable(&id), 0);

    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );
}

/// Issue #252 — View functions must not modify stored state.
#[test]
fn test_view_functions_do_not_modify_stored_state() {
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

    t.set_time(600);

    let before = t.contract.get_stream(&id);

    let _ = t.contract.get_stream(&id);
    let _ = t.contract.withdrawable(&id);
    let _ = t.contract.vested(&id);
    let _ = t.contract.locked(&id);
    let _ = t.contract.progress(&id);
    let _ = t.contract.status(&id);
    let _ = t.contract.stream_count();

    let after = t.contract.get_stream(&id);
    assert_eq!(after, before, "a view call must not modify the stored stream");

    assert_eq!(after.withdrawn, before.withdrawn);
    assert_eq!(after.total_amount, before.total_amount);
    assert_eq!(after.start_time, before.start_time);
    assert_eq!(after.cliff_time, before.cliff_time);
    assert_eq!(after.end_time, before.end_time);
    assert_eq!(after.cancelled, before.cancelled);
}

/// Issue #75 — Third-party view access: read-only functions are public and
/// unauthenticated.
#[test]
fn test_third_party_view_access() {
    use soroban_sdk::testutils::Address as _;
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

    let stream = t.contract.get_stream(&id);
    assert_eq!(stream.sender, t.sender);
    assert_eq!(stream.recipient, t.recipient);
    assert_eq!(stream.total_amount, 1_000);

    assert_eq!(t.contract.status(&id), StreamStatus::Streaming);
    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.vested(&id), 500);
    assert_eq!(t.contract.locked(&id), 500);
    assert_eq!(t.contract.progress(&id), 5_000);
    assert_eq!(t.contract.stream_count(), 1);

    let auths = t.env.auths();
    assert!(
        auths.is_empty(),
        "read-only view functions must be unauthenticated and produce empty auth list"
    );
}
