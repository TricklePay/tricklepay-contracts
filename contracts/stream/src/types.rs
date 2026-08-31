use soroban_sdk::{contracttype, Address};

/// A single token stream from a sender to a recipient.
///
/// Tokens vest linearly from `start_time` to `end_time`. The recipient may
/// withdraw whatever has vested but not yet been taken at any point. The
/// sender may cancel, which stops further vesting and returns the unvested
/// remainder.
///
/// All amounts are in the token's smallest unit (stroops for the native
/// asset). All times are Unix timestamps in seconds, matching the ledger
/// clock exposed by `env.ledger().timestamp()`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stream {
    /// Account that funded the stream and may cancel it.
    ///
    /// Only this address is authorised to call `cancel`.
    pub sender: Address,
    /// Account that accrues and withdraws the streamed tokens.
    ///
    /// Only this address is authorised to call `withdraw` and
    /// `withdraw_amount`.
    pub recipient: Address,
    /// Address of the SEP-41 token contract whose balance is being streamed.
    pub token: Address,
    /// The amount available for vesting, in the token's smallest unit
    /// (stroops for the native asset).
    ///
    /// On an active stream this equals the amount locked at creation. On a
    /// cancelled stream this field is frozen at the vested amount at the
    /// moment of cancellation — the unvested remainder has already been
    /// refunded to the sender and is no longer reflected here. Callers
    /// must not assume this equals the original deposit after cancellation.
    pub total_amount: i128,
    /// Cumulative amount the recipient has already withdrawn, in the token's
    /// smallest unit (stroops for the native asset).
    ///
    /// The currently withdrawable balance is `vested_amount - withdrawn`,
    /// where `vested_amount` is computed from the stream schedule and the
    /// current ledger time. This field only grows; it is never reduced.
    pub withdrawn: i128,
    /// Unix timestamp in seconds at which linear vesting begins.
    ///
    /// Before this moment nothing has vested. A `start_time` in the past is
    /// accepted at creation; the elapsed portion vests immediately.
    pub start_time: u64,
    /// Unix timestamp in seconds at which the stream is fully vested.
    ///
    /// At or after this moment the entire `total_amount` has vested and the
    /// recipient may withdraw the remaining balance in one call. Must be
    /// strictly greater than `start_time` and strictly in the future at the
    /// time `create_stream` is called.
    pub end_time: u64,
    /// Unix timestamp in seconds before which nothing may be withdrawn.
    ///
    /// When the ledger time reaches `cliff_time`, all tokens that have
    /// accrued since `start_time` unlock at once and vesting continues
    /// linearly from there. Set equal to `start_time` for a stream with no
    /// cliff; in that case the gate is vacuous and vesting proceeds linearly
    /// from the start. Must lie within `[start_time, end_time]`.
    pub cliff_time: u64,
    /// `true` once the sender has cancelled the stream.
    ///
    /// When `true`, `total_amount` holds only the vested portion at the
    /// moment of cancellation (the unvested remainder was refunded to the
    /// sender). The recipient may still withdraw any remaining vested balance
    /// after cancellation. A cancelled stream cannot be cancelled again.
    pub cancelled: bool,
}

/// The lifecycle state of a stream, derived from its fields and the current
/// ledger time. Returned by view calls so clients do not have to recompute
/// the same logic.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamStatus {
    /// The current ledger time is before `start_time`; nothing has vested
    /// yet.
    Pending,
    /// The current ledger time is at or after `start_time` and before
    /// `end_time`; tokens are actively vesting.
    Streaming,
    /// The current ledger time is at or after `end_time`; the stream is
    /// fully vested.
    Completed,
    /// The sender cancelled the stream. Cancellation can only happen while
    /// a stream is `Pending` or `Streaming` (a `Completed` stream can no
    /// longer be cancelled), but once cancelled the stream reports
    /// `Cancelled` permanently, overriding what the time-based state would
    /// otherwise be.
    Cancelled,
}
