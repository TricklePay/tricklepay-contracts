#![no_std]

//! TricklePay stream contract.
//!
//! This contract implements a linear token streaming and vesting mechanism on
//! Soroban (Stellar). A sender deposits tokens upfront into the contract to
//! create a stream for a recipient. Tokens vest continuously and linearly over
//! time from `start_time` to `end_time`, subject to an optional `cliff_time`.
//!
//! # Core Streaming Model
//!
//! - **Upfront Deposit & Guarantee**: When [`StreamContract::create_stream`] is called,
//!   the full `total_amount` is transferred from the sender into the contract.
//!   This guarantees that funds are fully backed for the life of the stream.
//! - **Linear Vesting**: Tokens vest linearly across the duration `[start_time, end_time]`.
//!   Before `start_time` or `cliff_time`, no tokens are vested. Once `cliff_time` passes,
//!   tokens accrued since `start_time` become available for withdrawal.
//! - **Withdrawals**: The recipient can claim accrued vested tokens at any time
//!   via [`StreamContract::withdraw`] (full withdrawable balance) or
//!   [`StreamContract::withdraw_amount`] (partial withdrawal).
//! - **Cancellation**: The sender can cancel an active stream prior to `end_time` using
//!   [`StreamContract::cancel`]. The unvested remainder of funds is refunded to the sender,
//!   while all tokens vested up to the cancellation timestamp remain claimable by the recipient.
//!
//! # Public Entry Points
//!
//! Contract interaction occurs through [`StreamContract`]:
//!
//! - **Stream Lifecycle**:
//!   - [`StreamContract::create_stream`]: Open a new token stream with schedule & cliff parameters.
//!   - [`StreamContract::withdraw`]: Pull all currently withdrawable vested tokens.
//!   - [`StreamContract::withdraw_amount`]: Pull a specific portion of vested tokens.
//!   - [`StreamContract::cancel`]: Stop an active stream early and receive a refund of unvested tokens.
//! - **State & Balance Queries**:
//!   - [`StreamContract::get_stream`]: Fetch full stream metadata and state.
//!   - [`StreamContract::withdrawable`]: Query tokens vested but not yet withdrawn.
//!   - [`StreamContract::vested`]: Query total tokens vested since creation.
//!   - [`StreamContract::locked`]: Query unvested tokens still held by the contract.
//!   - [`StreamContract::progress`]: Query vesting progress in basis points (0–10000).
//!   - [`StreamContract::status`]: Query current lifecycle status ([`StreamStatus`]).
//!   - [`StreamContract::stream_count`]: Query total number of streams created.
//!
//! For calculation details on linear vesting, see the [`vesting`] module.

mod contract;
mod error;
mod events;
mod storage;
mod types;
pub mod vesting;

#[cfg(test)]
mod tests;

pub use contract::{StreamContract, StreamContractClient, MAX_AMOUNT};
pub use error::StreamError;
pub use types::{Stream, StreamStatus};
