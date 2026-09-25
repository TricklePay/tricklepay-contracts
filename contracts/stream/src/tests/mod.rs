//! Integration and unit tests for the stream contract.
//!
//! Each module covers a single functional area so that a failing test
//! immediately signals which part of the contract is broken:
//!
//! | Module          | Covers                                                        |
//! |-----------------|---------------------------------------------------------------|
//! | `helpers`       | `StreamTest` fixture and shared utilities                     |
//! | `creation`      | `create_stream`: validation, parameters, participant checks   |
//! | `withdrawal`    | `withdraw`, `withdraw_amount`: vesting, cliff, partial draws  |
//! | `cancellation`  | `cancel`: refunds, freezing, cliff scenarios, lifecycle       |
//! | `views`         | Read-only queries: `progress`, `locked`, `status`, etc.       |
//! | `events`        | Event ordering and topic correctness for every entry point    |
//! | `storage`       | `DataKey` encoding, persistent and instance TTL bumps         |
//! | `auth`          | Authorization requirements for every mutating entry point     |

pub mod helpers;

mod auth;
mod cancellation;
mod creation;
mod events;
mod storage;
mod views;
mod withdrawal;
