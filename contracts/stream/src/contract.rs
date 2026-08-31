use soroban_sdk::{contract, contractimpl, token::TokenClient, Address, Env};

use crate::error::StreamError;
use crate::events;
use crate::storage;
use crate::types::{Stream, StreamStatus};
use crate::vesting;

/// Maximum value accepted for `total_amount` at stream creation.
///
/// The vesting arithmetic computes `total_amount * elapsed / duration` where
/// `elapsed` can be at most `u64::MAX` seconds (the full range of the ledger
/// clock). To guarantee that intermediate `i128` multiplication never
/// overflows — regardless of stream duration — amounts are capped at
/// `i64::MAX` (≈ 9.2 × 10¹⁸ stroops). The bound is well above the total
/// supply of any realistic token and satisfies:
///
///   `i64::MAX as i128 * u64::MAX as i128 < i128::MAX`
pub const MAX_AMOUNT: i128 = i64::MAX as i128;

#[contract]
pub struct StreamContract;

#[contractimpl]
impl StreamContract {
    /// Open a new stream from `sender` to `recipient`.
    ///
    /// The full `total_amount` is pulled from the sender into the contract at
    /// creation, so the recipient is guaranteed the funds exist for the life
    /// of the stream. Vesting runs linearly from `start_time` to `end_time`;
    /// pass `cliff_time == start_time` for a stream with no cliff.
    ///
    /// Returns the id assigned to the new stream.
    ///
    /// # Validation order
    ///
    /// Arguments are checked in a fixed order, and **all of it happens before
    /// any tokens move or any storage is written**. A call that is rejected
    /// leaves no trace: no transfer, no stream record, no id consumed. When an
    /// argument list violates more than one rule, the first matching rule below
    /// determines the error, so the result is deterministic rather than an
    /// artefact of how the checks happen to be ordered in the body:
    ///
    /// 1. **Authorization** — `sender` must authorize the call.
    /// 2. **Participants** — [`StreamError::InvalidParticipant`] if `sender`
    ///    equals `recipient`, or if `token` equals `sender` or `recipient`,
    ///    or if any of `sender`, `recipient`, or `token` is this contract's
    ///    own address.
    /// 3. **Amount** — [`StreamError::InvalidAmount`] if `total_amount` is not
    ///    positive, then [`StreamError::AmountTooLarge`] if it exceeds
    ///    [`MAX_AMOUNT`].
    /// 4. **Schedule** — [`StreamError::InvalidTimeRange`] if `start_time` is
    ///    not strictly before `end_time`, then [`StreamError::InvalidCliff`]
    ///    if `cliff_time` falls outside `[start_time, end_time]`, then
    ///    [`StreamError::StreamWindowInPast`] if `end_time` is not in the
    ///    future.
    /// 5. **Capacity** — [`StreamError::StreamCountExhausted`] if the id
    ///    counter has reached `u64::MAX`. The operation fails closed without
    ///    wrapping to zero or reusing any previously assigned stream id.
    ///
    /// Only once all five pass are tokens transferred and the stream stored.
    // A contract entry point: every field is part of the public call shape,
    // so bundling them into a struct would only obscure the interface.
    // The too-many-arguments threshold is raised to 8 in clippy.toml to
    // accommodate this function without an inline allow attribute.
    pub fn create_stream(
        env: Env,
        sender: Address,
        recipient: Address,
        token: Address,
        total_amount: i128,
        start_time: u64,
        end_time: u64,
        cliff_time: u64,
    ) -> Result<u64, StreamError> {
        sender.require_auth();

        // 1. Participants. Identity is the most fundamental precondition and
        //    these are pure comparisons, so they run first.
        //
        //    A stream from an address to itself has no effect other than
        //    locking the sender's own tokens and handing them back over time.
        //    It is almost always a mistake — a swapped argument or an unset
        //    field — so it is refused rather than silently accepted.
        if sender == recipient {
            return Err(StreamError::InvalidParticipant);
        }
        //    A token contract cannot act as a stream participant, and attempting
        //    to stream a token to or from its own address is refused.
        if token == sender || token == recipient {
            return Err(StreamError::InvalidParticipant);
        }
        //    This contract's own address is not valid in any role. Each case
        //    fails differently — an unclaimable recipient, a token with no
        //    `transfer` entry point, a sender drawing on the holdings that
        //    back every other stream — so all three are refused here.
        let this = env.current_contract_address();
        if sender == this || recipient == this || token == this {
            return Err(StreamError::InvalidParticipant);
        }

        // 2. Amount.
        if total_amount <= 0 {
            return Err(StreamError::InvalidAmount);
        }
        if total_amount > MAX_AMOUNT {
            return Err(StreamError::AmountTooLarge);
        }
        // 3. Schedule.
        if start_time >= end_time {
            return Err(StreamError::InvalidTimeRange);
        }
        if cliff_time < start_time || cliff_time > end_time {
            return Err(StreamError::InvalidCliff);
        }
        // Reject a window that is entirely in the past. A stream whose
        // end_time has already passed would be 100 % vested on creation —
        // effectively an immediate transfer with extra ceremony. Callers who
        // genuinely need that should use a token transfer directly.
        if end_time <= env.ledger().timestamp() {
            return Err(StreamError::StreamWindowInPast);
        }

        // 4. Capacity. Reserve the id before any tokens move. The counter is
        //    the source of every id and never reuses one, so if it were
        //    allowed to wrap the next stream would be written over a record
        //    that already exists. Checking here means an exhausted counter
        //    costs the caller nothing.
        let id = storage::stream_count(&env);
        let next_id = id.checked_add(1).ok_or(StreamError::StreamCountExhausted)?;

        // 5. Effects. Every rejection above returns before this point, so a
        //    failed creation never moves tokens or touches storage.
        TokenClient::new(&env, &token).transfer(
            &sender,
            env.current_contract_address(),
            &total_amount,
        );

        let stream = Stream {
            sender: sender.clone(),
            recipient: recipient.clone(),
            token: token.clone(),
            total_amount,
            withdrawn: 0,
            start_time,
            end_time,
            cliff_time,
            cancelled: false,
        };
        storage::set_stream(&env, id, &stream);
        storage::set_stream_count(&env, next_id);
        storage::extend_instance_ttl(&env);

        events::Created {
            sender: sender.clone(),
            recipient: recipient.clone(),
            id,
            token: token.clone(),
            total_amount,
            start_time,
            end_time,
            cliff_time,
        }
        .publish(&env);

        Ok(id)
    }

    /// Withdraw everything that has vested but not yet been taken.
    ///
    /// Only the recipient may call this. The amount sent is whatever has
    /// vested up to the current ledger time minus what was withdrawn before.
    /// Returns the amount transferred.
    pub fn withdraw(env: Env, id: u64) -> Result<i128, StreamError> {
        let mut stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        stream.recipient.require_auth();

        let now = env.ledger().timestamp();
        let vested = vesting::vested_amount(
            stream.total_amount,
            stream.start_time,
            stream.end_time,
            stream.cliff_time,
            now,
        );
        let available = vesting::withdrawable_amount(vested, stream.withdrawn);
        if available <= 0 {
            return Err(StreamError::NothingToWithdraw);
        }

        stream.withdrawn += available;
        storage::set_stream(&env, id, &stream);

        TokenClient::new(&env, &stream.token).transfer(
            &env.current_contract_address(),
            &stream.recipient,
            &available,
        );

        events::Withdrawn {
            recipient: stream.recipient.clone(),
            id,
            amount: available,
        }
        .publish(&env);

        Ok(available)
    }

    /// Withdraw a specific amount, up to what has vested.
    ///
    /// Only the recipient may call this. It behaves like [`Self::withdraw`] but
    /// lets the caller take less than the full available balance, which is
    /// useful for drawing a fixed sum or leaving a buffer in the stream. Fails
    /// if the requested amount exceeds the currently withdrawable balance.
    /// Returns the amount transferred.
    pub fn withdraw_amount(env: Env, id: u64, amount: i128) -> Result<i128, StreamError> {
        let mut stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        stream.recipient.require_auth();

        if amount <= 0 {
            return Err(StreamError::InvalidAmount);
        }

        let now = env.ledger().timestamp();
        let vested = vesting::vested_amount(
            stream.total_amount,
            stream.start_time,
            stream.end_time,
            stream.cliff_time,
            now,
        );
        let available = vesting::withdrawable_amount(vested, stream.withdrawn);
        if amount > available {
            return Err(StreamError::InsufficientBalance);
        }

        stream.withdrawn += amount;
        storage::set_stream(&env, id, &stream);

        TokenClient::new(&env, &stream.token).transfer(
            &env.current_contract_address(),
            &stream.recipient,
            &amount,
        );

        events::Withdrawn {
            recipient: stream.recipient.clone(),
            id,
            amount,
        }
        .publish(&env);

        Ok(amount)
    }

    /// Cancel a stream and refund the unvested remainder to the sender.
    ///
    /// Only the sender may call this. Whatever has vested up to the current
    /// ledger time stays claimable by the recipient through [`Self::withdraw`];
    /// the rest is returned to the sender. Once cancelled, no further tokens
    /// vest. Returns the amount refunded to the sender.
    pub fn cancel(env: Env, id: u64) -> Result<i128, StreamError> {
        let mut stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        stream.sender.require_auth();

        if stream.cancelled {
            return Err(StreamError::AlreadyCancelled);
        }

        let now = env.ledger().timestamp();

        if now >= stream.end_time {
            return Err(StreamError::StreamAlreadyCompleted);
        }
        let vested = vesting::vested_amount(
            stream.total_amount,
            stream.start_time,
            stream.end_time,
            stream.cliff_time,
            now,
        );
        let refund = stream.total_amount - vested;
        let recipient_remaining = vested - stream.withdrawn;

        // Freeze the stream at the vested amount. With the total reduced to
        // what has vested and the window closed at `now`, no further tokens
        // vest, but the recipient can still withdraw their accrued share.
        stream.total_amount = vested;
        stream.start_time = stream.start_time.min(now);
        stream.cliff_time = stream.cliff_time.min(now);
        stream.end_time = now;
        stream.cancelled = true;
        storage::set_stream(&env, id, &stream);

        if refund > 0 {
            TokenClient::new(&env, &stream.token).transfer(
                &env.current_contract_address(),
                &stream.sender,
                &refund,
            );
        }

        events::Cancelled {
            sender: stream.sender.clone(),
            id,
            recipient_amount: recipient_remaining,
            sender_refund: refund,
        }
        .publish(&env);

        Ok(refund)
    }

    /// Fetch a stream by id.
    pub fn get_stream(env: Env, id: u64) -> Result<Stream, StreamError> {
        storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)
    }

    /// Amount the recipient can withdraw right now.
    pub fn withdrawable(env: Env, id: u64) -> Result<i128, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        let vested = vesting::vested_amount(
            stream.total_amount,
            stream.start_time,
            stream.end_time,
            stream.cliff_time,
            env.ledger().timestamp(),
        );
        Ok(vesting::withdrawable_amount(vested, stream.withdrawn))
    }

    /// Total amount vested so far, including anything already withdrawn.
    pub fn vested(env: Env, id: u64) -> Result<i128, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        Ok(vesting::vested_amount(
            stream.total_amount,
            stream.start_time,
            stream.end_time,
            stream.cliff_time,
            env.ledger().timestamp(),
        ))
    }

    /// Amount not yet vested: the portion still locked in the contract that the
    /// recipient cannot withdraw yet.
    ///
    /// Locked behavior across stream lifecycle:
    /// - Before `start_time` or `cliff_time`: returns `total_amount` (entire amount locked).
    /// - Between `start_time` and `end_time`: decreases linearly as tokens vest (`total_amount - vested`).
    /// - At or after `end_time`: returns `0` (0% locked).
    /// - A cancelled stream returns `0` because cancellation freezes `total_amount` at `vested`.
    ///
    /// Rejections and Error Behavior:
    /// - Returns [`StreamError::StreamNotFound`] if `id` does not exist in storage
    ///   (e.g. an unknown id or an id from a creation call rejected for invalid participants).
    /// - `locked` is a read-only view function: it does not alter state or move tokens.
    pub fn locked(env: Env, id: u64) -> Result<i128, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        let vested = vesting::vested_amount(
            stream.total_amount,
            stream.start_time,
            stream.end_time,
            stream.cliff_time,
            env.ledger().timestamp(),
        );
        Ok(stream.total_amount - vested)
    }

    /// Vesting progress in basis points, from 0 (nothing vested) to 10000
    /// (100% vested). Useful for rendering a progress indicator without
    /// fetching the full stream.
    ///
    /// Progress calculations:
    /// - Returns `0` before `start_time` or `cliff_time`.
    /// - Scales linearly between `0` and `10000` from `start_time` to `end_time`.
    /// - Returns `10000` at or after `end_time`, or if `total_amount == 0`.
    /// - A stream with nothing left to vest, including a cancelled one, reports `10000`.
    ///
    /// Rejections and Error Behavior:
    /// - Returns [`StreamError::StreamNotFound`] if `id` does not exist in storage
    ///   (e.g. an unknown id or an id from a creation call rejected due to invalid participants).
    /// - `progress` is a read-only view function: it does not alter state or move tokens.
    pub fn progress(env: Env, id: u64) -> Result<u32, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        if stream.total_amount == 0 {
            return Ok(10_000);
        }
        let vested = vesting::vested_amount(
            stream.total_amount,
            stream.start_time,
            stream.end_time,
            stream.cliff_time,
            env.ledger().timestamp(),
        );
        let progress = vested * 10_000 / stream.total_amount;
        Ok(u32::try_from(progress.clamp(0, 10_000)).unwrap_or(0))
    }

    /// Lifecycle status of a stream at the current ledger time.
    ///
    /// Returns the derived [`StreamStatus`] for a valid stream:
    /// - [`StreamStatus::Cancelled`]: if the stream has been cancelled (takes precedence).
    /// - [`StreamStatus::Pending`]: if current ledger time `now < start_time`.
    /// - [`StreamStatus::Streaming`]: if `start_time <= now < end_time`.
    /// - [`StreamStatus::Completed`]: if `now >= end_time`.
    ///
    /// Rejections and Error Behavior:
    /// - Returns [`StreamError::StreamNotFound`] if `id` does not exist in storage
    ///   (e.g. an unknown id, or an id from a creation attempt rejected for invalid
    ///   participants like the contract's own address).
    /// - `status` is a read-only view function: it does not alter contract state or move tokens.
    pub fn status(env: Env, id: u64) -> Result<StreamStatus, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        if stream.cancelled {
            return Ok(StreamStatus::Cancelled);
        }
        let now = env.ledger().timestamp();
        let status = if now < stream.start_time {
            StreamStatus::Pending
        } else if now >= stream.end_time {
            StreamStatus::Completed
        } else {
            StreamStatus::Streaming
        };
        Ok(status)
    }

    /// Number of streams created so far. Ids run from zero up to this value
    /// minus one.
    pub fn stream_count(env: Env) -> u64 {
        storage::stream_count(&env)
    }
}
