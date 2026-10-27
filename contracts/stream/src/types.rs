use soroban_sdk::{contracttype, Address};

/// A token stream with a linear schedule and optional cliff.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stream {
    /// Account that funded the stream and may cancel it.
    pub sender: Address,
    /// Account that accrues and withdraws the streamed tokens.
    pub recipient: Address,
    /// Address of the SEP-41 token contract whose balance is being streamed.
    pub token: Address,
    /// Amount available for vesting in the token's smallest unit.
    pub total_amount: i128,
    /// Cumulative amount already withdrawn by the recipient.
    pub withdrawn: i128,
    /// Unix timestamp in seconds at which vesting begins.
    pub start_time: u64,
    /// Unix timestamp in seconds at which the stream is fully vested.
    pub end_time: u64,
    /// Unix timestamp in seconds before which withdrawals are unavailable.
    pub cliff_time: u64,
    /// Whether the sender has cancelled the stream.
    pub cancelled: bool,
}

impl Stream {
    /// Returns the amount vested at `now`.
    pub fn vested_amount(&self, now: u64) -> i128 {
        crate::vesting::vested_amount(
            self.total_amount,
            self.start_time,
            self.end_time,
            self.cliff_time,
            now,
        )
    }
}

/// Lifecycle state derived from a stream and the current ledger time.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamStatus {
    /// The current ledger time is before `start_time`.
    Pending,
    /// The current ledger time is at or after `start_time` and before `end_time`.
    Streaming,
    /// The current ledger time is at or after `end_time`.
    Completed,
    /// The sender cancelled the stream.
    Cancelled,
}
