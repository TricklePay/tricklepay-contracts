//! Pure lifecycle status derivation.
//!
//! [`stream_status`] reads only a stream's own fields and the timestamp it is
//! handed, so the lifecycle rules can be reasoned about and tested without an
//! environment or storage. The contract layer is responsible for loading a
//! stream and reading the ledger clock.

use crate::types::{Stream, StreamStatus};

/// Lifecycle status of `stream` at the time `now`.
///
/// [`StreamStatus::Cancelled`] takes precedence: a cancelled stream reports
/// `Cancelled` at every time, whatever the clock would otherwise say.
/// Otherwise the status follows the schedule — [`StreamStatus::Pending`]
/// before `start_time`, [`StreamStatus::Completed`] at or after `end_time`,
/// and [`StreamStatus::Streaming`] in between.
///
/// # Examples
///
/// ```
/// use soroban_sdk::testutils::Address as _;
/// use soroban_sdk::{Address, Env};
/// use tricklepay_stream::status::stream_status;
/// use tricklepay_stream::{Stream, StreamStatus};
///
/// let env = Env::default();
/// let stream = Stream {
///     sender: Address::generate(&env),
///     recipient: Address::generate(&env),
///     token: Address::generate(&env),
///     total_amount: 1_000,
///     withdrawn: 0,
///     start_time: 100,
///     end_time: 1_100,
///     cliff_time: 100,
///     cancelled: false,
/// };
///
/// assert_eq!(stream_status(&stream, 50), StreamStatus::Pending);
/// assert_eq!(stream_status(&stream, 600), StreamStatus::Streaming);
/// assert_eq!(stream_status(&stream, 1_100), StreamStatus::Completed);
/// ```
pub fn stream_status(stream: &Stream, now: u64) -> StreamStatus {
    if stream.cancelled {
        return StreamStatus::Cancelled;
    }
    if now < stream.start_time {
        return StreamStatus::Pending;
    }
    if now >= stream.end_time {
        return StreamStatus::Completed;
    }
    StreamStatus::Streaming
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    /// A stream of 1000 units over `[100, 1100]`, no cliff (`cliff == start`).
    fn stream(cancelled: bool) -> Stream {
        let env = Env::default();
        Stream {
            sender: Address::generate(&env),
            recipient: Address::generate(&env),
            token: Address::generate(&env),
            total_amount: 1_000,
            withdrawn: 0,
            start_time: 100,
            end_time: 1_100,
            cliff_time: 100,
            cancelled,
        }
    }

    #[test]
    fn pending_before_start() {
        assert_eq!(stream_status(&stream(false), 99), StreamStatus::Pending);
    }

    /// `start_time` is inclusive: the stream is already streaming at the moment
    /// it opens.
    #[test]
    fn streaming_at_start() {
        assert_eq!(stream_status(&stream(false), 100), StreamStatus::Streaming);
    }

    #[test]
    fn streaming_before_end() {
        assert_eq!(
            stream_status(&stream(false), 1_099),
            StreamStatus::Streaming
        );
    }

    /// `end_time` is inclusive: the stream is completed at the moment the
    /// window closes.
    #[test]
    fn completed_at_end() {
        assert_eq!(
            stream_status(&stream(false), 1_100),
            StreamStatus::Completed
        );
    }

    #[test]
    fn completed_after_end() {
        assert_eq!(
            stream_status(&stream(false), 9_999),
            StreamStatus::Completed
        );
    }

    /// Cancellation outranks the clock. A stream cancelled before it opened
    /// still reports `Cancelled` rather than `Pending`.
    #[test]
    fn cancelled_before_start_reports_cancelled() {
        assert_eq!(stream_status(&stream(true), 50), StreamStatus::Cancelled);
    }

    #[test]
    fn cancelled_while_streaming_reports_cancelled() {
        assert_eq!(stream_status(&stream(true), 600), StreamStatus::Cancelled);
    }

    /// `cancel` refuses to run once `now >= end_time`, so in practice a
    /// cancelled stream never also looks completed. The status function does
    /// not depend on that: cancellation wins either way.
    #[test]
    fn cancelled_after_end_reports_cancelled() {
        assert_eq!(stream_status(&stream(true), 1_100), StreamStatus::Cancelled);
    }

    /// The boundaries are exhaustive and mutually exclusive, so every instant
    /// of a stream's life lands on exactly one status.
    #[test]
    fn boundaries_cover_every_instant_without_gaps_or_overlap() {
        let s = stream(false);
        assert_eq!(stream_status(&s, 0), StreamStatus::Pending);
        assert_eq!(stream_status(&s, 99), StreamStatus::Pending);
        assert_eq!(stream_status(&s, 100), StreamStatus::Streaming);
        assert_eq!(stream_status(&s, 1_099), StreamStatus::Streaming);
        assert_eq!(stream_status(&s, 1_100), StreamStatus::Completed);
        assert_eq!(stream_status(&s, u64::MAX), StreamStatus::Completed);
    }

    /// A cliff does not appear in the status derivation: it gates withdrawal,
    /// not the lifecycle. A stream before its cliff is still `Streaming`.
    #[test]
    fn cliff_does_not_affect_status() {
        let mut s = stream(false);
        s.cliff_time = 600;

        assert_eq!(stream_status(&s, 300), StreamStatus::Streaming);
        assert_eq!(stream_status(&s, 600), StreamStatus::Streaming);
        assert_eq!(stream_status(&s, 1_099), StreamStatus::Streaming);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env};

    prop_compose! {
        fn valid_schedules()(
            start_time in 0u64..=u64::MAX / 2,
            duration in 1u64..=u64::MAX / 2,
        ) -> (u64, u64) {
            (start_time, start_time + duration)
        }
    }

    fn build(start_time: u64, end_time: u64, cancelled: bool) -> Stream {
        let env = Env::default();
        Stream {
            sender: Address::generate(&env),
            recipient: Address::generate(&env),
            token: Address::generate(&env),
            total_amount: 1_000,
            withdrawn: 0,
            start_time,
            end_time,
            cliff_time: start_time,
            cancelled,
        }
    }

    proptest! {
        /// Cancellation outranks the clock at every instant.
        #[test]
        fn cancelled_always_wins(
            (start, end) in valid_schedules(),
            now in any::<u64>(),
        ) {
            let s = build(start, end, true);
            prop_assert_eq!(stream_status(&s, now), StreamStatus::Cancelled);
        }

        #[test]
        fn before_start_is_pending(
            (start, end) in valid_schedules(),
            now in any::<u64>(),
        ) {
            prop_assume!(now < start);
            let s = build(start, end, false);
            prop_assert_eq!(stream_status(&s, now), StreamStatus::Pending);
        }

        #[test]
        fn within_the_window_is_streaming(
            (start, end) in valid_schedules(),
            now in any::<u64>(),
        ) {
            prop_assume!(now >= start);
            prop_assume!(now < end);
            let s = build(start, end, false);
            prop_assert_eq!(stream_status(&s, now), StreamStatus::Streaming);
        }

        #[test]
        fn at_or_after_end_is_completed(
            (start, end) in valid_schedules(),
            now in any::<u64>(),
        ) {
            prop_assume!(now >= end);
            let s = build(start, end, false);
            prop_assert_eq!(stream_status(&s, now), StreamStatus::Completed);
        }
    }
}
