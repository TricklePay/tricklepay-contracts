#![no_std]

//! TricklePay stream contract.
//!
//! Holds tokens on behalf of a sender and releases them to a recipient
//! linearly over time. See the module docs in [`contract`] for the public
//! entry points and [`vesting`] for the release schedule.

mod contract;
mod error;
mod events;
mod storage;
mod types;
mod vesting;

#[cfg(test)]
mod test;

pub use contract::{StreamContract, StreamContractClient};
pub use error::StreamError;
pub use types::{Stream, StreamStatus};
