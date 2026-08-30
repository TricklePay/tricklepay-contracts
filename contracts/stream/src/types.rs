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
    pub sender: Address,
    /// Account that accrues and withdraws the streamed tokens.
    pub recipient: Address,
    /// Address of the token contract being streamed.
    pub token: Address,
    /// Total amount locked into the stream at creation.
    pub total_amount: i128,
    /// Amount the recipient has already withdrawn.
    pub withdrawn: i128,
    /// Unix second at which vesting begins.
    pub start_time: u64,
    /// Unix second at which the stream is fully vested.
    pub end_time: u64,
    /// Unix second before which nothing may be withdrawn. Equal to
    /// `start_time` when the stream has no cliff.
    pub cliff_time: u64,
    /// Whether the stream has been cancelled.
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
