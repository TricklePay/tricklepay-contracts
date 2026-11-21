#![cfg(test)]

use proptest::prelude::*;
use soroban_sdk::{vec, Vec};

use crate::storage::{BUMP_THRESHOLD, ENTRY_TTL};
use crate::{StreamError, StreamStatus, MAX_AMOUNT, MAX_BATCH_SIZE};

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
fn get_streams_accepts_exactly_max_batch_size_and_rejects_over_limit() {
    let t = StreamTest::setup(25_000);
    t.set_time(100);

    // Empty vector succeeds and returns an empty list.
    let empty: Vec<u64> = Vec::new(&t.env);
    assert!(t.contract.get_streams(&empty).is_empty());

    // Create 20 distinct streams to test exactly MAX_BATCH_SIZE.
    let mut exact_batch = Vec::new(&t.env);
    for _ in 0..MAX_BATCH_SIZE {
        exact_batch.push_back(t.open_default_stream(100));
    }
    assert_eq!(exact_batch.len(), MAX_BATCH_SIZE);

    // Exactly 20 ids succeed and return all 20 records.
    let results = t.contract.get_streams(&exact_batch);
    assert_eq!(results.len(), MAX_BATCH_SIZE);
    for i in 0..MAX_BATCH_SIZE {
        let stream = results.get(i).unwrap().expect("stream must be present");
        assert_eq!(stream.total_amount, 100);
    }

    // Exactly 21 ids (MAX_BATCH_SIZE + 1) fail with BatchTooLarge.
    let mut oversized = exact_batch.clone();
    oversized.push_back(999);
    assert_eq!(oversized.len(), MAX_BATCH_SIZE + 1);
    assert_eq!(
        t.contract.try_get_streams(&oversized),
        Err(Ok(StreamError::BatchTooLarge))
    );
}

#[test]
fn get_streams_returns_duplicate_records_in_request_order() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    let id_a = t.open_default_stream(300);
    let id_b = t.open_default_stream(700);

    // Duplicate ids in arbitrary order: [id_a, id_b, id_a, id_a, id_b]
    let query = vec![&t.env, id_a, id_b, id_a, id_a, id_b];
    let streams = t.contract.get_streams(&query);
    assert_eq!(streams.len(), 5);
    assert_eq!(streams.get(0).unwrap().unwrap().total_amount, 300);
    assert_eq!(streams.get(1).unwrap().unwrap().total_amount, 700);
    assert_eq!(streams.get(2).unwrap().unwrap().total_amount, 300);
    assert_eq!(streams.get(3).unwrap().unwrap().total_amount, 300);
    assert_eq!(streams.get(4).unwrap().unwrap().total_amount, 700);

    // Exactly 20 duplicate ids of a single stream succeeding at the batch ceiling
    let mut all_same = Vec::new(&t.env);
    for _ in 0..MAX_BATCH_SIZE {
        all_same.push_back(id_a);
    }
    let repeated = t.contract.get_streams(&all_same);
    assert_eq!(repeated.len(), MAX_BATCH_SIZE);
    for i in 0..MAX_BATCH_SIZE {
        assert_eq!(repeated.get(i).unwrap().unwrap().total_amount, 300);
    }
}

#[test]
fn get_streams_handles_mixture_of_existing_unknown_and_cancelled_ids() {
    let t = StreamTest::setup(3_000);
    t.set_time(100);

    let active_id = t.open_default_stream(500);

    let cancelled_id = t.open_default_stream(600);
    t.set_time(600);
    t.contract.cancel(&cancelled_id);

    let completed_id = t.open_default_stream(700);
    t.set_time(1_200);

    let unknown_id_1 = 999_u64;
    let unknown_id_2 = 1_000_u64;

    // Mixture: unknown, active, cancelled, unknown, completed, active duplicate
    let batch = vec![
        &t.env,
        unknown_id_1,
        active_id,
        cancelled_id,
        unknown_id_2,
        completed_id,
        active_id,
    ];

    let results = t.contract.get_streams(&batch);
    assert_eq!(results.len(), 6);

    // [0]: unknown -> None
    assert_eq!(results.get(0).unwrap(), None);

    // [1]: active stream -> Some
    let active_stream = results.get(1).unwrap().unwrap();
    assert_eq!(active_stream.total_amount, 500);
    assert!(!active_stream.cancelled);

    // [2]: cancelled stream -> Some with frozen vested amount and cancelled flag true
    let cancelled_stream = results.get(2).unwrap().unwrap();
    assert_eq!(cancelled_stream.total_amount, 300);
    assert!(cancelled_stream.cancelled);

    // [3]: unknown -> None without failing the batch
    assert_eq!(results.get(3).unwrap(), None);

    // [4]: completed stream -> Some
    let completed_stream = results.get(4).unwrap().unwrap();
    assert_eq!(completed_stream.total_amount, 700);

    // [5]: duplicate active stream in request order
    let active_dup = results.get(5).unwrap().unwrap();
    assert_eq!(active_dup.total_amount, 500);

    // Read queries publish zero events
    assert_eq!(t.event_publishers(), vec![&t.env]);
}

#[test]
fn withdrawable_many_returns_balances_for_mixed_stream_states() {
    let t = StreamTest::setup(5_000);
    t.set_time(100);

    let active_id = t.open_default_stream(1_000);
    let cancelled_id = t.open_default_stream(1_000);
    let completed_id = t.open_default_stream(1_000);
    let pending_id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &2_000,
        &3_000,
        &2_000,
    );

    t.set_time(600);
    assert_eq!(t.contract.withdraw_amount(&active_id, &200), 200);
    t.contract.cancel(&cancelled_id);

    t.set_time(1_100);
    let batch = vec![
        &t.env,
        active_id,
        cancelled_id,
        completed_id,
        pending_id,
        999,
    ];
    let amounts = t.contract.withdrawable_many(&batch);
    assert_eq!(amounts, vec![&t.env, 800, 500, 1_000, 0, 0]);
}

#[test]
fn withdrawable_many_respects_limit_and_is_read_only() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    let mut exact_batch = Vec::new(&t.env);
    for _ in 0..MAX_BATCH_SIZE {
        exact_batch.push_back(id);
    }

    t.set_sequence(ENTRY_TTL - BUMP_THRESHOLD + 1);
    let ttl_before = t.stream_ttl(id);
    let amounts = t.contract.withdrawable_many(&exact_batch);
    assert_eq!(amounts.len(), MAX_BATCH_SIZE);
    assert_eq!(amounts.get(0), Some(0));
    assert_eq!(t.stream_ttl(id), ttl_before);
    assert!(t.event_publishers().is_empty());

    let mut oversized = exact_batch;
    oversized.push_back(id);
    assert_eq!(
        t.contract.try_withdrawable_many(&oversized),
        Err(Ok(StreamError::BatchTooLarge))
    );
    assert!(t
        .contract
        .withdrawable_many(&Vec::<u64>::new(&t.env))
        .is_empty());
}

#[test]
fn rate_per_second_handles_one_second_and_long_stream_rounding() {
    let t = StreamTest::setup(2_000_000_003);
    t.set_time(100);

    let one_second = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &9,
        &100,
        &101,
        &100,
    );
    assert_eq!(t.contract.rate_per_second(&one_second), 9);

    let long_stream = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000_000_003,
        &100,
        &1_000_100,
        &100,
    );
    assert_eq!(t.contract.rate_per_second(&long_stream), 1_000);
    t.set_time(1_000_100);
    assert_eq!(t.contract.vested(&long_stream), 1_000_000_003);
}

#[test]
fn rate_per_second_returns_zero_for_cancelled_streams() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.set_time(600);
    t.contract.cancel(&id);
    assert_eq!(t.contract.rate_per_second(&id), 0);
    assert_eq!(
        t.contract.try_rate_per_second(&999),
        Err(Ok(StreamError::StreamNotFound))
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
fn fresh_contract_handles_id_zero_and_extreme_ids() {
    let t = StreamTest::setup(1_000);

    // On a fresh contract before any streams are created:
    assert_eq!(t.contract.stream_count(), 0);

    // ID 0 does not exist yet
    assert!(!t.contract.stream_exists(&0));
    assert!(!t.contract.stream_exists(&1));
    assert!(!t.contract.stream_exists(&u64::MAX));

    // get_streams with 0, 1, and u64::MAX returns None for each
    let query = vec![&t.env, 0, 1, u64::MAX];
    let results = t.contract.get_streams(&query);
    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap(), None);
    assert_eq!(results.get(1).unwrap(), None);
    assert_eq!(results.get(2).unwrap(), None);

    // Single-id views return StreamNotFound
    assert_eq!(
        t.contract.try_get_stream(&0),
        Err(Ok(StreamError::StreamNotFound))
    );
    assert_eq!(
        t.contract.try_get_stream(&u64::MAX),
        Err(Ok(StreamError::StreamNotFound))
    );
}

#[test]
fn stream_exists_and_get_streams_at_extreme_and_counter_boundary_ids() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);

    // Create 2 streams: ID 0 and ID 1. Current counter becomes 2.
    let id_0 = t.open_default_stream(500);
    let id_1 = t.open_default_stream(700);
    assert_eq!(id_0, 0);
    assert_eq!(id_1, 1);
    assert_eq!(t.contract.stream_count(), 2);

    let counter = t.contract.stream_count(); // 2
    let one_past_counter = counter; // ID 2
    let two_past_counter = counter + 1; // ID 3

    // Existing streams assert true
    assert!(t.contract.stream_exists(&id_0));
    assert!(t.contract.stream_exists(&id_1));

    // One past counter and extreme ids assert false
    assert!(!t.contract.stream_exists(&one_past_counter));
    assert!(!t.contract.stream_exists(&two_past_counter));
    assert!(!t.contract.stream_exists(&(u64::MAX / 2)));
    assert!(!t.contract.stream_exists(&(u64::MAX - 1)));
    assert!(!t.contract.stream_exists(&u64::MAX));

    // Batch get_streams asserting Some for valid ids and None for boundary / extreme ids
    let query = vec![
        &t.env,
        id_0,
        one_past_counter,
        id_1,
        two_past_counter,
        u64::MAX - 1,
        u64::MAX,
    ];
    let results = t.contract.get_streams(&query);
    assert_eq!(results.len(), 6);
    assert_eq!(results.get(0).unwrap().unwrap().total_amount, 500);
    assert_eq!(results.get(1).unwrap(), None);
    assert_eq!(results.get(2).unwrap().unwrap().total_amount, 700);
    assert_eq!(results.get(3).unwrap(), None);
    assert_eq!(results.get(4).unwrap(), None);
    assert_eq!(results.get(5).unwrap(), None);
}

#[test]
fn read_views_do_not_panic_for_any_u64_id() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let _id = t.open_default_stream(500);

    let test_ids = [
        0_u64,
        1_u64,
        2_u64,
        3_u64,
        99_u64,
        1_000_u64,
        i64::MAX as u64,
        (i64::MAX as u64) + 1,
        u64::MAX / 2,
        u64::MAX - 100,
        u64::MAX - 1,
        u64::MAX,
    ];

    for &test_id in &test_ids {
        // stream_exists never panics
        let exists = t.contract.stream_exists(&test_id);
        if test_id == 0 {
            assert!(exists);
        } else {
            assert!(!exists);
        }

        // get_streams never panics
        let batch = t.contract.get_streams(&vec![&t.env, test_id]);
        assert_eq!(batch.len(), 1);
        if test_id == 0 {
            assert!(batch.get(0).unwrap().is_some());
        } else {
            assert_eq!(batch.get(0).unwrap(), None);
        }

        // try_get_stream never panics
        let single = t.contract.try_get_stream(&test_id);
        if test_id == 0 {
            assert!(single.is_ok());
        } else {
            assert_eq!(single, Err(Ok(StreamError::StreamNotFound)));
        }
    }
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

proptest! {
    /// Issue #623 — `vested_at(id, timestamp)` is monotonic, bounded in [0, total],
    /// zero before start and cliff, total at/after end, and agrees with `vested(id)`
    /// when evaluated at the current ledger timestamp.
    #[test]
    fn vested_at_is_monotonic_and_agrees_with_vested(
        total in 1i128..=MAX_AMOUNT,
        start in 0u64..=1_000_000u64,
        duration in 1u64..=1_000_000u64,
        cliff_offset in 0u64..=1_000_000u64,
        t1 in 0u64..=2_500_000u64,
        t2 in 0u64..=2_500_000u64,
    ) {
        let end = start + duration;
        let cliff = start + cliff_offset.min(duration);

        let t = StreamTest::setup(total);
        t.set_time(0);
        let id = t.contract.create_stream(
            &t.sender,
            &t.recipient,
            &t.token_address,
            &total,
            &start,
            &end,
            &cliff,
        );

        let (early, late) = if t1 <= t2 { (t1, t2) } else { (t2, t1) };

        let v1 = t.contract.vested_at(&id, &early);
        let v2 = t.contract.vested_at(&id, &late);

        // Monotonicity: vested_at never decreases as timestamp advances
        prop_assert!(
            v1 <= v2,
            "vested_at not monotonic: v1 ({}) at {} > v2 ({}) at {}",
            v1,
            early,
            v2,
            late
        );

        // Bounds: [0, total]
        prop_assert!(
            v1 >= 0 && v1 <= total,
            "v1 ({}) out of [0, {}]",
            v1,
            total
        );
        prop_assert!(
            v2 >= 0 && v2 <= total,
            "v2 ({}) out of [0, {}]",
            v2,
            total
        );

        // Zero before start or cliff
        if early < start || early < cliff {
            prop_assert_eq!(
                v1,
                0,
                "vested_at should be 0 before start/cliff at {}",
                early
            );
        }
        if late < start || late < cliff {
            prop_assert_eq!(
                v2,
                0,
                "vested_at should be 0 before start/cliff at {}",
                late
            );
        }

        // Total at and after end
        if early >= end {
            prop_assert_eq!(
                v1,
                total,
                "vested_at should be total at/after end at {}",
                early
            );
        }
        if late >= end {
            prop_assert_eq!(
                v2,
                total,
                "vested_at should be total at/after end at {}",
                late
            );
        }

        // Agreement with vested():
        // Evaluating vested() at the corresponding ledger timestamps matches vested_at().
        t.set_time(early);
        prop_assert_eq!(
            t.contract.vested(&id),
            v1,
            "vested() does not agree with vested_at at early ({})",
            early
        );

        t.set_time(late);
        prop_assert_eq!(
            t.contract.vested(&id),
            v2,
            "vested() does not agree with vested_at at late ({})",
            late
        );
    }
}
