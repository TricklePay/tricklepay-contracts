#![cfg(test)]

use soroban_sdk::{vec, Vec};

use crate::storage::{BUMP_THRESHOLD, ENTRY_TTL};
use crate::{StreamError, StreamStatus, MAX_BATCH_SIZE};

use super::helpers::StreamTest;

#[test]
fn get_streams_returns_in_request_order_and_none_for_unknown_ids() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let first_id = t.open_default_stream(400);
    let second_id = t.open_default_stream(600);

    t.set_sequence(ENTRY_TTL - BUMP_THRESHOLD + 1);
    let first_ttl = t.stream_ttl(first_id);
    let second_ttl = t.stream_ttl(second_id);
    let streams = t
        .contract
        .get_streams(&vec![&t.env, second_id, 99, first_id]);
    assert_eq!(streams.len(), 3);
    assert_eq!(streams.get(0).unwrap().unwrap().total_amount, 600);
    assert_eq!(streams.get(1).unwrap(), None);
    assert_eq!(streams.get(2).unwrap().unwrap().total_amount, 400);
    assert_eq!(t.stream_ttl(first_id), first_ttl);
    assert_eq!(t.stream_ttl(second_id), second_ttl);
    assert_eq!(t.event_publishers(), vec![&t.env]);
}

#[test]
fn get_streams_accepts_empty_input_and_rejects_over_limit() {
    let t = StreamTest::setup(1_000);
    let empty: Vec<u64> = Vec::new(&t.env);
    assert!(t.contract.get_streams(&empty).is_empty());

    let mut oversized = Vec::new(&t.env);
    for id in 0..=MAX_BATCH_SIZE {
        oversized.push_back(u64::from(id));
    }
    assert_eq!(
        t.contract.try_get_streams(&oversized),
        Err(Ok(StreamError::BatchTooLarge))
    );
}

#[test]
fn max_amount_view_returns_i64_max() {
    let t = StreamTest::setup(1);

    assert_eq!(t.contract.max_amount(), i64::MAX as i128);
}

#[test]
fn stream_exists_reports_existing_unknown_cancelled_and_completed_streams() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let completed_id = t.open_default_stream(500);
    let cancelled_id = t.open_default_stream(500);

    let below_bump_threshold = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(below_bump_threshold);
    let original_ttl = t.stream_ttl(completed_id);
    assert!(t.contract.stream_exists(&completed_id));
    assert_eq!(t.event_publishers(), vec![&t.env]);
    assert_eq!(t.stream_ttl(completed_id), original_ttl);
    assert!(!t.contract.stream_exists(&99));
    assert_eq!(t.event_publishers(), vec![&t.env]);

    t.set_time(600);
    t.contract.cancel(&cancelled_id);
    t.set_sequence(below_bump_threshold * 2);
    let cancelled_ttl = t.stream_ttl(cancelled_id);
    assert!(t.contract.stream_exists(&cancelled_id));
    assert_eq!(t.event_publishers(), vec![&t.env]);
    assert_eq!(t.stream_ttl(cancelled_id), cancelled_ttl);

    t.set_time(1_100);
    assert!(t.contract.stream_exists(&completed_id));
    assert_eq!(t.event_publishers(), vec![&t.env]);
}

#[test]
fn vested_at_projects_the_schedule_and_freezes_after_cancellation() {
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

    assert_eq!(t.contract.vested_at(&id, &50), 0);
    assert_eq!(t.contract.vested_at(&id, &300), 0);
    assert_eq!(t.contract.vested_at(&id, &1_100), 1_000);

    t.set_time(700);
    assert_eq!(t.contract.vested_at(&id, &700), t.contract.vested(&id));

    t.set_time(600);
    t.contract.cancel(&id);
    assert_eq!(t.contract.vested_at(&id, &700), 500);
    assert_eq!(t.contract.vested_at(&id, &1_100), 500);
}

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

    // Nothing vested at the start.
    assert_eq!(t.contract.progress(&id), 0);
    // Halfway is 50 percent, in basis points.
    t.set_time(600);
    assert_eq!(t.contract.progress(&id), 5_000);
    // One second before the end, progress must still be below the maximum.
    t.set_time(1_099);
    assert_eq!(t.contract.progress(&id), 9_990);
    assert!(t.contract.progress(&id) < 10_000);
    // Fully vested at the end.
    t.set_time(1_100);
    assert_eq!(t.contract.progress(&id), 10_000);
}

#[test]
fn progress_never_decreases_as_time_advances() {
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

    let mut previous = 0;
    for timestamp in [50, 100, 101, 250, 350, 600, 850, 1_099, 1_100, 1_200] {
        t.set_time(timestamp);
        let progress = t.contract.progress(&id);
        assert!(
            progress >= previous,
            "progress decreased from {previous} to {progress} at timestamp {timestamp}"
        );
        previous = progress;
    }
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
        &600,
    );

    // Before the cliff, nothing is vested and the whole amount is locked.
    t.set_time(300);
    assert_eq!(t.contract.locked(&id), 1_000);
    assert_eq!(t.contract.vested(&id), 0);
    assert_eq!(t.contract.locked(&id) + t.contract.vested(&id), 1_000);

    // Mid-stream, vested and locked remain complementary.
    t.set_time(850);
    assert_eq!(t.contract.locked(&id), 250);
    assert_eq!(t.contract.vested(&id), 750);
    assert_eq!(t.contract.locked(&id) + t.contract.vested(&id), 1_000);

    // After the end, all value is vested and none is locked.
    t.set_time(1_200);
    assert_eq!(t.contract.locked(&id), 0);
    assert_eq!(t.contract.vested(&id), 1_000);
    assert_eq!(t.contract.locked(&id) + t.contract.vested(&id), 1_000);
}

#[test]
fn locked_never_goes_negative_across_sampled_times() {
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

    for now in [100u64, 300, 600, 850, 1_100, 1_200] {
        t.set_time(now);
        let vested = t.contract.vested(&id);
        let locked = t.contract.locked(&id);
        assert!(
            locked >= 0,
            "locked={} vested={} at time={}",
            locked,
            vested,
            now
        );
        assert_eq!(locked, (1_000 - vested).max(0));
    }

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

/// Same four view assertions, but run again after the recipient has drained
/// the remaining vested balance. Once the recipient withdraws, withdrawable
/// must fall to 0 and the other views must stay stable. This also confirms
/// the token balances add up and a second withdraw is rejected.
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

    // Cancel at the midpoint and then advance time well past the original end
    // to confirm the frozen state does not change with the clock.
    t.set_time(600);
    t.contract.cancel(&id);
    assert_eq!(t.contract.get_stream(&id).total_amount, 500);
    t.set_time(2_000);

    assert_eq!(t.contract.get_stream(&id).total_amount, 500);
    assert_eq!(t.contract.vested(&id), 500);

    // Recipient drains their share.
    let withdrawn = t.contract.withdraw(&id);
    assert_eq!(withdrawn, 500);

    // Token balances add up to the original total — nothing was lost.
    assert_eq!(t.token.balance(&t.sender), 500);
    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&t.contract.address), 0);

    // Views must remain consistent after the drain.
    assert_eq!(t.contract.locked(&id), 0);
    assert_eq!(t.contract.progress(&id), 10_000);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);

    // withdrawable() must now be 0 — the recipient took everything.
    assert_eq!(t.contract.withdrawable(&id), 0);

    // A second withdraw attempt must be rejected.
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
    let _ = t.contract.vested_at(&id, &600);
    let _ = t.contract.stream_exists(&id);
    let _ = t.contract.locked(&id);
    let _ = t.contract.progress(&id);
    let _ = t.contract.status(&id);
    let _ = t.contract.stream_count();
    let _ = t.contract.max_amount();

    let after = t.contract.get_stream(&id);
    assert_eq!(
        after, before,
        "a view call must not modify the stored stream"
    );

    assert_eq!(after.withdrawn, before.withdrawn);
    assert_eq!(after.total_amount, before.total_amount);
    assert_eq!(after.start_time, before.start_time);
    assert_eq!(after.cliff_time, before.cliff_time);
    assert_eq!(after.end_time, before.end_time);
    assert_eq!(after.cancelled, before.cancelled);
}

/// Issue #313 — Every read-only entry point must stay silent on the event
/// stream. An indexer must never record activity that did not really happen.
#[test]
fn test_view_functions_publish_no_events() {
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

    let _ = t.contract.get_stream(&id);
    assert_eq!(t.event_publishers(), vec![&t.env]);

    let _ = t.contract.withdrawable(&id);
    assert_eq!(t.event_publishers(), vec![&t.env]);

    let _ = t.contract.vested(&id);
    assert_eq!(t.event_publishers(), vec![&t.env]);

    let _ = t.contract.vested_at(&id, &600);
    assert_eq!(t.event_publishers(), vec![&t.env]);

    let _ = t.contract.stream_exists(&id);
    assert_eq!(t.event_publishers(), vec![&t.env]);

    let _ = t.contract.locked(&id);
    assert_eq!(t.event_publishers(), vec![&t.env]);

    let _ = t.contract.progress(&id);
    assert_eq!(t.event_publishers(), vec![&t.env]);

    let _ = t.contract.status(&id);
    assert_eq!(t.event_publishers(), vec![&t.env]);

    let _ = t.contract.stream_count();
    assert_eq!(t.event_publishers(), vec![&t.env]);

    let _ = t.contract.max_amount();
    assert_eq!(t.event_publishers(), vec![&t.env]);
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
    assert_eq!(t.contract.vested_at(&id, &600), 500);
    assert!(t.contract.stream_exists(&id));
    assert_eq!(t.contract.locked(&id), 500);
    assert_eq!(t.contract.progress(&id), 5_000);
    assert_eq!(t.contract.stream_count(), 1);
    assert_eq!(t.contract.max_amount(), i64::MAX as i128);

    let auths = t.env.auths();
    assert!(
        auths.is_empty(),
        "read-only view functions must be unauthenticated and produce empty auth list"
    );
}

/// Issue #359 — all views agree at every sampled point in a stream's
/// lifecycle: vested + locked equals the deposit, withdrawable never exceeds
/// vested, and progress tracks the same vesting fraction.
#[test]
fn views_agree_at_every_sampled_time() {
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

    for now in [50u64, 100, 101, 250, 500, 600, 850, 1_099, 1_100, 1_200] {
        t.set_time(now);
        let vested = t.contract.vested(&id);
        let locked = t.contract.locked(&id);
        let withdrawable = t.contract.withdrawable(&id);
        let progress = t.contract.progress(&id);

        assert_eq!(vested + locked, 1_000, "time={now}");
        assert!(withdrawable >= 0 && withdrawable <= vested, "time={now}");

        let expected_progress = if now <= 100 {
            0
        } else if now >= 1_100 {
            10_000
        } else {
            ((now - 100) * 10_000 / 1_000) as u32
        };
        assert_eq!(progress, expected_progress, "time={now}");
    }
}
