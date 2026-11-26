//! Contract limits and scaling constants.
//!
//! This module defines the numerical limits and scaling factors that govern the
//! stream contract:
//!
//! - [`MAX_AMOUNT`]: The deposit ceiling for individual streams.
//! - [`BPS_SCALE`]: The basis-points denominator used for percentage progress.
//! - [`MAX_BATCH_SIZE`]: The maximum number of stream IDs allowed in a batch query.

/// Maximum `total_amount` accepted when creating a stream.
///
/// Set to `i64::MAX as i128` (9,223,372,036,854,775,807) to prevent integer
/// overflow when multiplying amounts by durations or basis points in vesting
/// calculations, while still accommodating the maximum 64-bit integer values
/// supported by the Stellar Asset Contract.
pub const MAX_AMOUNT: i128 = i64::MAX as i128;

/// Basis points in 100%, the scale [`StreamContract::progress`](crate::StreamContract::progress) reports on.
///
/// 10,000 basis points equals 100.00% (1 bp = 0.01%).
pub const BPS_SCALE: u32 = 10_000;

/// Maximum number of stream ids accepted by [`StreamContract::get_streams`](crate::StreamContract::get_streams).
///
/// Capped at 20 ids to protect against unbounded ledger read footprint growth and
/// CPU instruction limit exhaustion in a single transaction invocation.
pub const MAX_BATCH_SIZE: u32 = 20;

/// Maximum length of an optional stream reference, in bytes.
///
/// References are opaque byte strings rather than text types. Integrators may
/// encode invoice or payroll identifiers in UTF-8, but the contract does not
/// validate or normalize their contents. At most 64 bytes are stored.
pub const MAX_REFERENCE_LENGTH: u32 = 64;
