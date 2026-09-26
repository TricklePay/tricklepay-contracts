//! Persistent storage keys and time-to-live management.
//!
//! # Overview
//!
//! This module is the single source of truth for every ledger entry the
//! contract touches. All reads and writes go through the helpers here so that
//! TTL management is handled in one place and entry points never call the SDK
//! storage API directly.
//!
//! The contract keeps exactly two kinds of entry:
//!
//! | Key | Storage kind | Purpose |
//! |-----|-------------|---------|
//! | [`DataKey::StreamCount`] | Instance | Monotonic id counter |
//! | [`DataKey::Stream`]`(id)` | Persistent | One full [`crate::types::Stream`] record |
//!
//! # Keys
//!
//! **`StreamCount`** lives in *instance* storage alongside the contract's
//! WASM. It is a single `u64` whose current value is the id to assign to the
//! next stream. Because instance storage is not bumped by read-only contract
//! calls, it is extended explicitly inside `create_stream`; a contract that is
//! only queried and never written to will run its instance down over time.
//!
//! **`Stream(id)`** entries live in *persistent* storage, one per stream,
//! keyed by the numeric id. Each entry holds the full [`crate::types::Stream`]
//! struct: participants, token, schedule, withdrawn amount, and cancelled flag.
//! The entry is extended on every read or write, so any contact with a stream
//! resets its countdown. Ids are never reused.
//!
//! # Lifetimes
//!
//! Both entry kinds share the same TTL constants:
//!
//! - [`ENTRY_TTL`] — `518_400` ledgers (≈ 30 days at a 5-second ledger close).
//!   This is the target lifetime to which an entry is extended when bumped.
//! - [`BUMP_THRESHOLD`] — `103_680` ledgers (≈ 6 days; one fifth of
//!   `ENTRY_TTL`). An entry is only re-extended when its remaining TTL falls
//!   below this mark, so a frequently accessed entry is not charged for a bump
//!   on every call.
//!
//! A stream touched at least once per 24-day window will never approach
//! archival. A stream that is abandoned for longer than `ENTRY_TTL` ledgers
//! will be archived by the network; it must be restored off-chain before the
//! contract can use it again.

use soroban_sdk::{contracttype, Env};

use crate::types::Stream;

/// Number of ledgers an entry lives before it must be bumped.
///
/// Derived from a target of thirty days at the nominal five second ledger
/// close time: `30 days * 86_400 s/day / 5 s/ledger = 518_400` ledgers. Close
/// times drift, so the wall-clock lifetime is approximate; slower ledgers
/// stretch it and faster ones shorten it. Thirty days covers a monthly payroll
/// or subscription cycle, so a stream that is touched at least once per cycle
/// never approaches archival.
pub(crate) const ENTRY_TTL: u32 = 518_400;
/// When an accessed entry has fewer than this many ledgers left, extend it
/// back up to `ENTRY_TTL`. Above this mark an access is a no-op, so an entry
/// touched often does not pay to be re-extended on every read.
///
/// Set to one fifth of `ENTRY_TTL`, `518_400 / 5 = 103_680` ledgers, or about
/// six days at five seconds per ledger. An entry is therefore re-extended at
/// most about once every twenty-four days of activity, and never has less than
/// six days of headroom left after being touched.
pub(crate) const BUMP_THRESHOLD: u32 = 103_680;

/// Keys for entries the contract keeps in storage.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Monotonic counter holding the id to assign to the next stream.
    ///
    /// Stored in **instance** storage. It is initialised to `0` on the first
    /// `create_stream` call and incremented by one each time a new stream is
    /// opened. Ids are never reused: once a value has been assigned it stays
    /// consumed even if the corresponding stream is cancelled or fully vested.
    ///
    /// Because instance storage is not bumped by read-only calls, this entry
    /// is extended explicitly by [`extend_instance_ttl`] inside `create_stream`
    /// to keep it alive for as long as the streams it numbers.
    StreamCount,
    /// A single stream record, keyed by its numeric id.
    ///
    /// Stored in **persistent** storage. Each entry holds the full [`Stream`]
    /// struct — participants, token, schedule, withdrawn amount, and cancelled
    /// flag — for one stream. The entry is extended to [`ENTRY_TTL`] ledgers
    /// whenever it is read or written, so any view or mutating call on a stream
    /// resets its countdown. A stream that is never touched for longer than
    /// [`ENTRY_TTL`] ledgers will be archived by the network and must be
    /// restored off-contract before it can be used again.
    Stream(u64),
}

/// Read the next stream id, defaulting to zero on a fresh contract.
pub fn stream_count(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::StreamCount)
        .unwrap_or(0)
}

/// Persist the next stream id.
pub fn set_stream_count(env: &Env, count: u64) {
    env.storage().instance().set(&DataKey::StreamCount, &count);
}

/// Refresh the contract instance's time to live.
///
/// The instance holds [`DataKey::StreamCount`], the source of every stream id.
/// Unlike a stream entry, nothing bumps it as a side effect of being read, so
/// a contract left untouched past its lifetime would be archived and take the
/// id sequence with it. Extending on the same schedule as stream entries keeps
/// the counter alive for as long as the streams it numbers.
pub fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(BUMP_THRESHOLD, ENTRY_TTL);
}

/// Look up a stream by id, if one exists.
pub fn get_stream(env: &Env, id: u64) -> Option<Stream> {
    let key = DataKey::Stream(id);
    let stream = env.storage().persistent().get(&key);
    if stream.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, BUMP_THRESHOLD, ENTRY_TTL);
    }
    stream
}

/// Write a stream and refresh its time to live.
pub fn set_stream(env: &Env, id: u64, stream: &Stream) {
    let key = DataKey::Stream(id);
    env.storage().persistent().set(&key, stream);
    env.storage()
        .persistent()
        .extend_ttl(&key, BUMP_THRESHOLD, ENTRY_TTL);
}
