#![cfg(test)]

use soroban_sdk::testutils::Ledger as _;

use crate::storage::{DataKey, BUMP_THRESHOLD, ENTRY_TTL};
use crate::StreamError;

use super::helpers::StreamTest;

// --- Storage key encoding ---------------------------------------------------
//
// `DataKey` is the only thing standing between a stream record and the wrong
// slot in storage. `StreamCount` and `Stream(id)` must encode to different
// keys, every id must encode to its own key across the whole `u64` range, and
// the two live in different storage types. None of that is checked by the
// entry points, which report `StreamNotFound` whether a key is missing or
// merely holds something unexpected — so these tests read the raw keys.

/// Each stream id encodes to its own persistent key. Three streams written in
/// a row occupy `Stream(0)`, `Stream(1)` and `Stream(2)` and do not overwrite
/// one another; the id one past the last is absent rather than aliasing.
#[test]
fn stream_ids_map_to_distinct_persistent_keys() {
    let t = StreamTest::setup(6_000);
    t.set_time(100);

    // Distinct amounts so an aliased key shows up as a wrong value, not just a
    // wrong count.
    let first = t.open_default_stream(1_000);
    assert_eq!(t.contract.stream_count(), 1);
    assert_eq!(t.contract.get_stream(&first).total_amount, 1_000);

    let second = t.open_default_stream(2_000);
    assert_eq!(t.contract.stream_count(), 2);
    assert_eq!(t.contract.get_stream(&second).total_amount, 2_000);

    let third = t.open_default_stream(3_000);
    assert_eq!(t.contract.stream_count(), 3);
    assert_eq!(t.contract.get_stream(&third).total_amount, 3_000);
    assert_eq!((first, second, third), (0, 1, 2));

    assert!(t.persistent_has(&DataKey::Stream(0)));
    assert!(t.persistent_has(&DataKey::Stream(1)));
    assert!(t.persistent_has(&DataKey::Stream(2)));
    assert!(!t.persistent_has(&DataKey::Stream(3)));

    // Each key holds its own record.
    assert_eq!(
        t.persistent_stream(&DataKey::Stream(0))
            .unwrap()
            .total_amount,
        1_000
    );
    assert_eq!(
        t.persistent_stream(&DataKey::Stream(1))
            .unwrap()
            .total_amount,
        2_000
    );
    assert_eq!(
        t.persistent_stream(&DataKey::Stream(2))
            .unwrap()
            .total_amount,
        3_000
    );
}

/// `StreamCount` and `Stream(0)` must not share a key or storage type.
#[test]
fn stream_count_and_stream_zero_do_not_share_a_key() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    t.open_default_stream(1_000);

    assert!(t.instance_has(&DataKey::StreamCount));
    assert!(!t.persistent_has(&DataKey::StreamCount));

    assert!(t.persistent_has(&DataKey::Stream(0)));
    assert!(!t.instance_has(&DataKey::Stream(0)));

    assert_eq!(t.contract.stream_count(), 1);
}

/// Key encoding holds at the top of the id range too.
#[test]
fn boundary_stream_ids_encode_to_their_own_keys() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    t.set_stream_count(u64::MAX - 1);

    let id = t.open_default_stream(1_000);
    assert_eq!(id, u64::MAX - 1);

    assert!(t.persistent_has(&DataKey::Stream(u64::MAX - 1)));
    assert!(!t.persistent_has(&DataKey::Stream(u64::MAX)));
    assert!(!t.persistent_has(&DataKey::Stream(0)));

    assert_eq!(
        t.persistent_stream(&DataKey::Stream(u64::MAX - 1))
            .unwrap()
            .total_amount,
        1_000
    );
}

/// A stream round-trips through its key unchanged.
#[test]
fn stream_records_round_trip_under_their_key() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &200,
        &1_200,
        &400,
    );

    let stored = t.persistent_stream(&DataKey::Stream(id)).unwrap();
    assert_eq!(stored, t.contract.get_stream(&id));

    assert_eq!(stored.start_time, 200);
    assert_eq!(stored.cliff_time, 400);
    assert_eq!(stored.end_time, 1_200);
    assert_eq!(stored.withdrawn, 0);
    assert!(!stored.cancelled);
}

/// Issue #363 — validation must not rewrite any part of the accepted schedule
/// before it is persisted. This catches accidental normalization of timestamps
/// or cliff boundaries between validation and storage.
#[test]
fn schedule_is_stored_exactly_as_validated() {
    let t = StreamTest::setup(1_000);
    t.set_time(1_000);

    let start_time = 1_234;
    let end_time = 9_876;
    let cliff_time = 5_678;
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &start_time,
        &end_time,
        &cliff_time,
    );

    let stored = t.persistent_stream(&DataKey::Stream(id)).unwrap();
    assert_eq!(stored.start_time, start_time);
    assert_eq!(stored.end_time, end_time);
    assert_eq!(stored.cliff_time, cliff_time);
}

/// A creation rejected because the contract's own address was passed as
/// `recipient` writes no key at all. The participant checks run before the
/// transfer and before storage, so `Stream(0)` stays empty and the id is not
/// consumed.
#[test]
fn rejected_create_writes_no_storage_key() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let contract_address = t.contract.address.clone();
    t.assert_create_stream_error(
        &t.sender,
        &contract_address,
        &t.token_address,
        1_000,
        StreamError::InvalidParticipant,
    );

    assert!(!t.persistent_has(&DataKey::Stream(0)));
    t.assert_nothing_happened(1_000);
}

// ── Persistent entry TTL ─────────────────────────────────────────────────────

/// A freshly created stream starts with the full `ENTRY_TTL` window.
#[test]
fn create_stream_gives_the_record_the_full_entry_ttl() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let default_ttl = t.env.ledger().get().min_persistent_entry_ttl;
    assert!(default_ttl < ENTRY_TTL);

    let id = t.open_default_stream(1_000);
    assert_eq!(t.stream_ttl(id), ENTRY_TTL);
}

/// The entry's remaining life decays one ledger at a time.
#[test]
fn a_stream_entry_ttl_decays_with_the_ledger_sequence() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    let elapsed = 1_000;
    t.set_sequence(elapsed);
    assert_eq!(t.stream_ttl(id), ENTRY_TTL - elapsed);
}

/// Reading a stream below `BUMP_THRESHOLD` restores the full window.
#[test]
fn reading_a_stream_below_the_threshold_restores_the_full_ttl() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);
    assert!(ENTRY_TTL - elapsed < BUMP_THRESHOLD);

    t.contract.get_stream(&id);
    assert_eq!(t.stream_ttl(id), ENTRY_TTL);
}

/// A read above the threshold does nothing to the TTL.
#[test]
fn reading_a_stream_above_the_threshold_leaves_the_ttl_alone() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    let elapsed = ENTRY_TTL - BUMP_THRESHOLD - 1;
    t.set_sequence(elapsed);
    assert!(ENTRY_TTL - elapsed > BUMP_THRESHOLD);

    t.contract.get_stream(&id);
    assert_eq!(t.stream_ttl(id), ENTRY_TTL - elapsed);
}

/// Issue #433: the module docs used to claim a 24-day touch cadence kept a
/// stream alive. It does not. A touch only re-extends an entry when fewer than
/// `BUMP_THRESHOLD` ledgers remain; a cadence shorter than that but longer than
/// `ENTRY_TTL - BUMP_THRESHOLD` lands above the threshold every time, so the
/// countdown is untouched and the entry expires on its original schedule.
#[test]
fn a_touch_above_the_threshold_does_not_postpone_expiry() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    // Twenty days ≈ 345_600 ledgers at a five-second close, which is closer
    // together than the old "24-day window" advice and still leaves ~172_800
    // ledgers, well above `BUMP_THRESHOLD`.
    let twenty_days = 345_600;
    t.set_sequence(twenty_days);
    let remaining = t.stream_ttl(id);
    assert_eq!(remaining, ENTRY_TTL - twenty_days);
    assert!(remaining > BUMP_THRESHOLD);

    // The touch is a no-op: the countdown is exactly what it was before.
    t.contract.get_stream(&id);
    assert_eq!(t.stream_ttl(id), remaining);

    // Only once the remaining life falls below the threshold does a touch
    // reset it — the guarantee the docs now state.
    t.set_sequence(ENTRY_TTL - BUMP_THRESHOLD + 1);
    assert!(t.stream_ttl(id) < BUMP_THRESHOLD);
    t.contract.get_stream(&id);
    assert_eq!(t.stream_ttl(id), ENTRY_TTL);
}

/// Withdrawing restores a decayed stream entry TTL.
#[test]
fn withdrawing_restores_a_decayed_stream_ttl() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);

    t.set_time(600);
    assert_eq!(t.contract.withdraw(&id), 500);

    assert_eq!(t.stream_ttl(id), ENTRY_TTL);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 500);
}

/// Cancelling restores a decayed stream entry TTL.
#[test]
fn cancelling_restores_a_decayed_stream_ttl() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);

    t.set_time(600);
    assert_eq!(t.contract.cancel(&id), 500);

    assert_eq!(t.stream_ttl(id), ENTRY_TTL);
    assert_eq!(t.contract.withdrawable(&id), 500);
}

/// A cancelled stream still holds the recipient's accrued balance, so a
/// plain read of it — not just the write inside `cancel` itself — must keep
/// refreshing the entry, or the record could be archived out from under a
/// recipient who has not yet come back to withdraw (issue #314).
#[test]
fn reading_a_cancelled_stream_below_the_threshold_restores_its_ttl() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.set_time(600);
    assert_eq!(t.contract.cancel(&id), 500);
    // cancel() itself writes, which already refreshes the entry.
    assert_eq!(t.stream_ttl(id), ENTRY_TTL);

    // Let the now-cancelled entry decay again.
    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);
    assert!(ENTRY_TTL - elapsed < BUMP_THRESHOLD);

    // A plain view call on the cancelled stream still bumps it.
    let stream = t.contract.get_stream(&id);
    assert!(stream.cancelled);
    assert_eq!(t.stream_ttl(id), ENTRY_TTL);

    // The stream keeps answering well past where it would have been
    // archived without that bump.
    t.set_sequence(elapsed + ENTRY_TTL - BUMP_THRESHOLD + 1);
    assert!(t.contract.get_stream(&id).cancelled);
}

/// Views authorize nothing, write nothing, and move no tokens, so two
/// consecutive reads of a stream nobody has touched must be byte-identical.
/// A refresh that mutated a field (rather than only the entry's TTL) would
/// show up here (issue #317).
#[test]
fn reading_a_stream_twice_returns_identical_data() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    let first = t.contract.get_stream(&id);
    let second = t.contract.get_stream(&id);

    assert_eq!(first, second);
}

/// Touching one stream does not extend another.
#[test]
fn bumping_one_stream_does_not_extend_another() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    let busy = t.open_default_stream(1_000);
    let idle = t.open_default_stream(1_000);

    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);

    t.contract.get_stream(&busy);

    assert_eq!(t.stream_ttl(busy), ENTRY_TTL);
    assert_eq!(t.stream_ttl(idle), ENTRY_TTL - elapsed);
}

/// Repeated bumps keep a stream entry alive indefinitely.
#[test]
fn repeated_bumps_keep_a_stream_entry_alive_indefinitely() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    let step = ENTRY_TTL - BUMP_THRESHOLD + 1;
    for cycle in 1..=3u32 {
        t.set_sequence(step * cycle);
        t.contract.get_stream(&id);
        assert_eq!(t.stream_ttl(id), ENTRY_TTL);
    }

    assert!(step * 3 > ENTRY_TTL);
    assert_eq!(t.contract.get_stream(&id).total_amount, 1_000);
}

// ── Instance TTL ─────────────────────────────────────────────────────────────

/// `create_stream` extends the instance TTL.
#[test]
fn create_stream_extends_the_instance_ttl() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let default_ttl = t.instance_ttl();
    assert!(default_ttl < ENTRY_TTL);

    t.open_default_stream(1_000);
    assert_eq!(t.instance_ttl(), ENTRY_TTL);
}

/// The stream count survives a ledger advance past the default TTL.
#[test]
fn stream_count_survives_a_ledger_advance_past_the_default_ttl() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    let first = t.open_default_stream(1_000);

    let default_ttl = t.env.ledger().get().min_persistent_entry_ttl;
    let advanced_to = default_ttl * 2;
    t.set_sequence(advanced_to);

    assert_eq!(t.instance_ttl(), ENTRY_TTL - advanced_to);
    assert_eq!(t.contract.stream_count(), 1);

    let second = t.open_default_stream(1_000);
    assert_eq!(second, first + 1);
    assert_eq!(t.contract.stream_count(), 2);
    assert_eq!(t.contract.get_stream(&first).total_amount, 1_000);
}

/// Creating a stream lifts the instance to the full `ENTRY_TTL` window.
#[test]
fn create_stream_lifts_the_instance_to_the_full_entry_ttl() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let default_ttl = t.instance_ttl();
    assert!(default_ttl < ENTRY_TTL);

    let id = t.open_default_stream(1_000);

    assert_eq!(t.instance_ttl(), ENTRY_TTL);
    assert_eq!(t.stream_ttl(id), ENTRY_TTL);
}

/// The instance TTL decays one ledger at a time.
#[test]
fn the_instance_ttl_decays_with_the_ledger_sequence() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    t.open_default_stream(1_000);

    let elapsed = 1_000;
    t.set_sequence(elapsed);
    assert_eq!(t.instance_ttl(), ENTRY_TTL - elapsed);
}

/// A later creation restores a decayed instance to the full window.
#[test]
fn a_later_create_restores_a_decayed_instance_ttl() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    t.open_default_stream(1_000);

    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);
    assert_eq!(t.instance_ttl(), ENTRY_TTL - elapsed);

    t.open_default_stream(1_000);
    assert_eq!(t.instance_ttl(), ENTRY_TTL);
}

/// View calls do not extend the instance TTL.
#[test]
fn view_calls_do_not_extend_the_instance_ttl() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);
    t.set_time(600);

    t.contract.get_stream(&id);
    t.contract.withdrawable(&id);
    t.contract.stream_count();
    assert_eq!(t.stream_ttl(id), ENTRY_TTL);

    assert_eq!(t.instance_ttl(), ENTRY_TTL - elapsed);
}

/// Withdrawing and cancelling do not extend the instance TTL.
#[test]
fn withdraw_and_cancel_do_not_extend_the_instance_ttl() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    let first = t.open_default_stream(1_000);
    let second = t.open_default_stream(1_000);

    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);
    t.set_time(600);

    t.contract.withdraw(&first);
    t.contract.cancel(&second);

    assert_eq!(t.instance_ttl(), ENTRY_TTL - elapsed);
}

/// Repeated creates keep the instance alive indefinitely.
#[test]
fn repeated_creates_keep_the_instance_alive_indefinitely() {
    let t = StreamTest::setup(5_000);
    t.set_time(100);
    t.open_default_stream(1_000);

    let step = ENTRY_TTL - BUMP_THRESHOLD + 1;
    for cycle in 1..=3u32 {
        t.set_sequence(step * cycle);
        t.open_default_stream(1_000);
        assert_eq!(t.instance_ttl(), ENTRY_TTL);
    }

    assert!(step * 3 > ENTRY_TTL);
    assert_eq!(t.contract.stream_count(), 4);
    assert_eq!(t.open_default_stream(1_000), 4);
}

/// Ids never restart after a long idle gap.
#[test]
fn ids_never_restart_after_a_long_idle_gap() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    let first = t.open_default_stream(1_000);
    assert_eq!(first, 0);

    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);

    let second = t.open_default_stream(1_000);
    assert_eq!(second, 1);
    assert_eq!(t.contract.stream_count(), 2);

    assert!(t.persistent_has(&DataKey::Stream(0)));
    assert!(t.persistent_has(&DataKey::Stream(1)));
    assert_eq!(t.contract.get_stream(&first).total_amount, 1_000);
}
