//! Integration and unit tests for the stream contract.
//!
//! Each module covers a single functional area so that a failing test
//! immediately signals which part of the contract is broken:
//!
//! | Module            | Covers                                                        |
//! |-------------------|---------------------------------------------------------------|
//! | `helpers`         | `StreamTest` fixture and shared utilities                     |
//! | `budget`          | Resource budget regression tests and entry point ceilings     |
//! | `creation`        | `create_stream`: validation, parameters, participant checks   |
//! | `withdrawal`      | `withdraw`, `withdraw_amount`: vesting, cliff, partial draws  |
//! | `cancellation`    | `cancel`: refunds, freezing, cliff scenarios, lifecycle       |
//! | `views`           | Read-only queries: `progress`, `locked`, `status`, etc.       |
//! | `documentation`   | README error-code table and interface snapshot checks         |
//! | `budget`          | Measured resource costs and budget regression ceilings        |
//! | `events`          | Event ordering and topic correctness for every entry point    |
//! | `storage`         | `DataKey` encoding, persistent and instance TTL bumps         |
//! | `auth`            | Authorization requirements for every mutating entry point     |
//! | `proptest_escrow` | Property tests for escrow balance invariant                   |
//! | `reentrancy`      | Token re-entrancy protection tests                            |

pub mod helpers;

mod auth;
mod budget;
mod cancellation;
mod creation;
mod documentation;
mod events;
mod proptest_escrow;
mod readme_examples;
mod reentrancy;
mod storage;
mod views;
mod withdrawal;
