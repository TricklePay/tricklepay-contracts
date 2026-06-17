use soroban_sdk::{contracttype, Env};

use crate::types::Stream;

/// Number of ledgers a stream entry lives before it must be bumped. At the
/// standard five second close time this is roughly thirty days, which gives
/// active streams plenty of headroom between touches.
const STREAM_TTL: u32 = 518_400;
/// When an accessed stream has fewer than this many ledgers left, extend it
/// back up to `STREAM_TTL`.
const STREAM_BUMP_THRESHOLD: u32 = 103_680;

/// Keys for entries the contract keeps in storage.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Monotonic counter holding the id to assign to the next stream.
    StreamCount,
    /// A stream record, keyed by its id.
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

/// Look up a stream by id, if one exists.
pub fn get_stream(env: &Env, id: u64) -> Option<Stream> {
    let key = DataKey::Stream(id);
    let stream = env.storage().persistent().get(&key);
    if stream.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, STREAM_BUMP_THRESHOLD, STREAM_TTL);
    }
    stream
}

/// Write a stream and refresh its time to live.
pub fn set_stream(env: &Env, id: u64, stream: &Stream) {
    let key = DataKey::Stream(id);
    env.storage().persistent().set(&key, stream);
    env.storage()
        .persistent()
        .extend_ttl(&key, STREAM_BUMP_THRESHOLD, STREAM_TTL);
}
