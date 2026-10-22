use soroban_sdk::contracterror;

/// Stable error codes returned by the stream contract.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum StreamError {
    /// No stream exists for the requested identifier (code 1).
    StreamNotFound = 1,
    /// The start time is not strictly before the end time (code 3).
    InvalidTimeRange = 3,
    /// The total amount is zero or negative (code 4).
    InvalidAmount = 4,
    /// The cliff is outside the stream's start and end window (code 5).
    InvalidCliff = 5,
    /// The stream has already been cancelled (code 6).
    AlreadyCancelled = 6,
    /// Nothing is currently available to withdraw (code 7).
    NothingToWithdraw = 7,
    /// The requested withdrawal exceeds the available balance (code 8).
    InsufficientBalance = 8,
    /// The stream is complete and cannot be cancelled (code 9).
    StreamAlreadyCompleted = 9,
    /// `total_amount` exceeds `MAX_AMOUNT` (code 10).
    AmountTooLarge = 10,
    /// The stream's end time is not in the future (code 11).
    StreamWindowInPast = 11,
    /// The stream id counter cannot be incremented without wrapping (code 12).
    StreamCountExhausted = 12,
    /// A participant address is invalid for its role (code 13).
    InvalidParticipant = 13,
}
