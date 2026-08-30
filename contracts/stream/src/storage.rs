use soroban_sdk::{contracttype, Env};

use crate::types::Stream;

/// Number of ledgers an entry lives before it must be bumped. At the standard
/// five second close time this is roughly thirty days, which gives active
/// streams plenty of headroom between touches.
pub(crate) const ENTRY_TTL: u32 = 518_400;
/// When an accessed entry has fewer than this many ledgers left, extend it
/// back up to `ENTRY_TTL`. Above this mark an access is a no-op, so an entry
/// touched often does not pay to be re-extended on every read.
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
