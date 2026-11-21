use soroban_sdk::{contract, contractimpl, token::TokenClient, Address, Env, Vec};

pub use crate::constants::{BPS_SCALE, MAX_AMOUNT, MAX_BATCH_SIZE};
use crate::error::StreamError;
use crate::events;
use crate::status;
use crate::storage;
use crate::types::{Stream, StreamStatus};
use crate::vesting;

#[contract]
pub struct StreamContract;

#[contractimpl]
impl StreamContract {
    // =====================================================================
    // Mutating entry points
    //
    // These require authorization, write contract state, and may move tokens.
    // Everything below the views section is read-only and moves nothing.
    // =====================================================================

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
    /// 2. **Participants** — [`StreamError::InvalidParticipant`] if any of
    ///    `sender`, `recipient`, or `token` is this contract's own address.
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
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use soroban_sdk::testutils::Address as _;
    /// # use soroban_sdk::{Address, Env};
    /// # use soroban_sdk::testutils::Address as _;
    /// # use tricklepay_stream::{StreamContract, StreamContractClient};
    /// # let env = Env::default();
    /// # let contract_id = env.register(StreamContract, ());
    /// # let client = StreamContractClient::new(&env, &contract_id);
    /// # let sender = Address::generate(&env);
    /// # let recipient = Address::generate(&env);
    /// # let token = Address::generate(&env);
    /// let id = client.create_stream(
    ///     &sender,
    ///     &recipient,
    ///     &token,
    ///     &1_000_000_000, // 1000 tokens (7 decimals)
    ///     &1_700_000_000, // start_time (unix timestamp)
    ///     &1_700_086_400, // end_time (start + 1 day)
    ///     &1_700_000_000, // cliff_time (no cliff)
    /// );
    /// ```
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

        // Effects. Every rejection above returns before this point, so a
        // failed creation never moves tokens or touches storage.
        TokenClient::new(&env, &token).transfer(
            &sender,
            env.current_contract_address(),
            &total_amount,
        );

        let stream = Stream {
            sender,
            recipient,
            token,
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

        events::publish_created(&env, id, &stream);

        Ok(id)
    }

    /// Transfers the recipient's full withdrawable balance and returns the amount.
    pub fn withdraw(env: Env, id: u64) -> Result<i128, StreamError> {
        withdraw_with(&env, id, |available| {
            if available <= 0 {
                return Err(StreamError::NothingToWithdraw);
            }
            Ok(available)
        })
    }

    /// Transfers a requested amount up to the recipient's withdrawable balance.
    pub fn withdraw_amount(env: Env, id: u64, amount: i128) -> Result<i128, StreamError> {
        withdraw_with(&env, id, |available| {
            if amount <= 0 {
                return Err(StreamError::InvalidAmount);
            }
            if amount > available {
                return Err(StreamError::InsufficientBalance);
            }
            Ok(amount)
        })
    }

    /// Withdraw vested tokens to a specified destination address.
    ///
    /// Only the recipient may call this. Behaves like `withdraw_amount` but sends
    /// the tokens to `destination` instead of the stream's stored `recipient`. The
    /// stream's `withdrawn` counter is incremented by the amount transferred.
    ///
    /// Returns the amount transferred.
    ///
    /// # Authorization
    ///
    /// - Requires authorization from the stream's `recipient` (not from `destination`).
    ///
    /// # Validation
    ///
    /// - `destination` cannot be the stream contract's own address.
    /// - `destination` can be the same as `stream.recipient`.
    ///
    /// # Errors
    ///
    /// - [`StreamError::StreamNotFound`] if `id` does not exist in storage.
    /// - [`StreamError::InvalidAmount`] if `amount <= 0`.
    /// - [`StreamError::InvalidParticipant`] if `destination` is the stream contract's own address.
    /// - [`StreamError::InsufficientBalance`] if `amount` exceeds the withdrawable balance.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use soroban_sdk::testutils::Address as _;
    /// # use soroban_sdk::{Address, Env};
    /// # use tricklepay_stream::{StreamContract, StreamContractClient};
    /// # let env = Env::default();
    /// # let contract_id = env.register(StreamContract, ());
    /// # let client = StreamContractClient::new(&env, &contract_id);
    /// # let destination = Address::generate(&env);
    /// # let stream_id = 0u64;
    /// let amount = client.withdraw_to(&stream_id, &destination, &500);
    /// ```
    pub fn withdraw_to(
        env: Env,
        id: u64,
        destination: Address,
        amount: i128,
    ) -> Result<i128, StreamError> {
        let mut stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        stream.recipient.require_auth();

        if amount <= 0 {
            return Err(StreamError::InvalidAmount);
        }
        if destination == env.current_contract_address() {
            return Err(StreamError::InvalidParticipant);
        }

        let vested = stream.vested_amount(env.ledger().timestamp());
        let available = vesting::withdrawable_amount(vested, stream.withdrawn);
        if amount > available {
            return Err(StreamError::InsufficientBalance);
        }

        stream.withdrawn += amount;
        storage::set_stream(&env, id, &stream);

        transfer(
            &env,
            &stream.token,
            &env.current_contract_address(),
            &destination,
            amount,
        );

        events::publish_withdrawn(&env, &stream.recipient, id, amount, &destination);

        Ok(amount)
    }

    /// Top up an existing stream by adding tokens.
    ///
    /// Only the sender may call this. The added amount is transferred from the
    /// sender into the contract, increasing the stream's total while keeping
    /// the end time and duration unchanged (Option B: increase rate).
    ///
    /// # Authorization
    ///
    /// - Requires authorization from the stream's `sender`.
    ///
    /// # Errors
    ///
    /// - [`StreamError::InvalidAmount`] if `added_amount <= 0`.
    /// - [`StreamError::StreamNotFound`] if `id` does not match an existing stream.
    /// - [`StreamError::AlreadyCancelled`] if the stream has already been cancelled.
    /// - [`StreamError::StreamAlreadyCompleted`] if the stream is already complete (`now >= end_time`).
    /// - [`StreamError::AmountTooLarge`] if `stream.total_amount + added_amount > MAX_AMOUNT`.
    pub fn topup_stream(env: Env, id: u64, added_amount: i128) -> Result<(), StreamError> {
        if added_amount <= 0 {
            return Err(StreamError::InvalidAmount);
        }

        let mut stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        stream.sender.require_auth();

        if stream.cancelled {
            return Err(StreamError::AlreadyCancelled);
        }

        let now = env.ledger().timestamp();
        if now >= stream.end_time {
            return Err(StreamError::StreamAlreadyCompleted);
        }

        let new_total = stream
            .total_amount
            .checked_add(added_amount)
            .ok_or(StreamError::AmountTooLarge)?;

        if new_total > MAX_AMOUNT {
            return Err(StreamError::AmountTooLarge);
        }

        TokenClient::new(&env, &stream.token).transfer(
            &stream.sender,
            env.current_contract_address(),
            &added_amount,
        );

        stream.total_amount = new_total;
        storage::set_stream(&env, id, &stream);
        storage::extend_instance_ttl(&env);

        events::publish_topped_up(
            &env,
            &stream.sender,
            id,
            added_amount,
            new_total,
            stream.end_time,
        );

        Ok(())
    }

    /// Cancel a stream and refund the unvested remainder to the sender.
    ///
    /// Only the sender may call this. Whatever has vested up to the current
    /// ledger time stays claimable by the recipient through [`Self::withdraw`];
    /// the rest is returned to the sender. Once cancelled, no further tokens
    /// vest. Returns the amount refunded to the sender.
    ///
    /// # Validation order
    ///
    /// The stream is loaded from storage first. If `id` does not exist, this
    /// returns [`StreamError::StreamNotFound`] before any authorization check,
    /// so an unknown id can be distinguished without sender authorization.
    /// For an existing stream, the checks then run in this order:
    ///
    /// 1. The stream's `sender` must authorize the call.
    /// 2. Returns [`StreamError::AlreadyCancelled`] if the stream has already
    ///    been cancelled.
    /// 3. Returns [`StreamError::StreamAlreadyCompleted`] if the current ledger
    ///    time is at or past `end_time` — a fully vested stream has nothing
    ///    unvested to refund.
    ///
    /// If a stream is both cancelled and past its end time,
    /// [`StreamError::AlreadyCancelled`] is returned because the cancellation
    /// check precedes the completion check. If all checks pass, the stream is
    /// frozen at the vested amount and the unvested remainder is refunded to
    /// the sender.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use soroban_sdk::Env;
    /// # use tricklepay_stream::{StreamContract, StreamContractClient};
    /// # let env = Env::default();
    /// # let contract_id = env.register(StreamContract, ());
    /// # let client = StreamContractClient::new(&env, &contract_id);
    /// # let stream_id = 0u64;
    /// let refund_amount = client.cancel(&stream_id);
    /// ```
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
        let vested = stream.vested_amount(now);
        let settlement = vesting::settlement(stream.total_amount, vested, stream.withdrawn);

        // Freeze the stream at the vested amount. With the total reduced to
        // what has vested and the window closed at `now`, no further tokens
        // vest, but the recipient can still withdraw their accrued share.
        stream.total_amount = vested;
        stream.start_time = stream.start_time.min(now);
        stream.cliff_time = stream.cliff_time.min(now);
        stream.end_time = now;
        stream.cancelled = true;
        storage::set_stream(&env, id, &stream);

        if settlement.refund > 0 {
            transfer(
                &env,
                &stream.token,
                &env.current_contract_address(),
                &stream.sender,
                settlement.refund,
            );
        }

        events::publish_cancelled(
            &env,
            &stream.sender,
            id,
            settlement.recipient_remaining,
            settlement.refund,
        );

        Ok(settlement.refund)
    }

    // =====================================================================
    // Views
    //
    // Read-only. None of these authorize, write storage, or move tokens, so
    // they are safe to call from anywhere and return the same value for the
    // same ledger state.
    // =====================================================================

    /// Returns the stream with the given id, or `StreamNotFound`.
    pub fn get_stream(env: Env, id: u64) -> Result<Stream, StreamError> {
        storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)
    }

    /// Preview the settlement for cancelling a stream at the current ledger time.
    ///
    /// Returns the same [`vesting::Settlement`] that [`Self::cancel`] would
    /// produce now, without requiring authorization, changing storage, or
    /// publishing events. Returns [`StreamError::StreamNotFound`] for an
    /// unknown id, [`StreamError::AlreadyCancelled`] for a cancelled stream,
    /// and [`StreamError::StreamAlreadyCompleted`] when the stream has ended.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use soroban_sdk::Env;
    /// # use tricklepay_stream::{StreamContract, StreamContractClient};
    /// # let env = Env::default();
    /// # let contract_id = env.register(StreamContract, ());
    /// # let client = StreamContractClient::new(&env, &contract_id);
    /// # let stream_id = 0u64;
    /// let settlement = client.cancel_preview(&stream_id);
    /// ```
    pub fn cancel_preview(env: Env, id: u64) -> Result<vesting::Settlement, StreamError> {
        let stream = storage::get_stream_readonly(&env, id).ok_or(StreamError::StreamNotFound)?;
        if stream.cancelled {
            return Err(StreamError::AlreadyCancelled);
        }

        let now = env.ledger().timestamp();
        if now >= stream.end_time {
            return Err(StreamError::StreamAlreadyCompleted);
        }

        let vested = stream.vested_amount(now);
        Ok(vesting::settlement(
            stream.total_amount,
            vested,
            stream.withdrawn,
        ))
    }

    /// Fetch several streams in request order, returning `None` for unknown ids.
    ///
    /// This view accepts at most [`MAX_BATCH_SIZE`] ids and returns
    /// [`StreamError::BatchTooLarge`] when the limit is exceeded. It does not
    /// change storage or publish events.
    pub fn get_streams(env: Env, ids: Vec<u64>) -> Result<Vec<Option<Stream>>, StreamError> {
        if ids.len() > MAX_BATCH_SIZE {
            return Err(StreamError::BatchTooLarge);
        }

        let mut streams = Vec::new(&env);
        for id in ids {
            streams.push_back(storage::get_stream_readonly(&env, id));
        }
        Ok(streams)
    }

    /// Whether a stream record exists, including cancelled or completed streams.
    ///
    /// This check does not modify storage or publish events.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use soroban_sdk::Env;
    /// # use tricklepay_stream::{StreamContract, StreamContractClient};
    /// # let env = Env::default();
    /// # let contract_id = env.register(StreamContract, ());
    /// # let client = StreamContractClient::new(&env, &contract_id);
    /// # let stream_id = 0u64;
    /// let exists = client.stream_exists(&stream_id);
    /// ```
    pub fn stream_exists(env: Env, id: u64) -> bool {
        storage::has_stream(&env, id)
    }

    /// Returns the vested balance not yet withdrawn, or `StreamNotFound`.
    pub fn withdrawable(env: Env, id: u64) -> Result<i128, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        let vested = stream.vested_amount(env.ledger().timestamp());
        Ok(vesting::withdrawable_amount(vested, stream.withdrawn))
    }

    /// Returns the total amount vested at the current ledger time, including withdrawals.
    pub fn vested(env: Env, id: u64) -> Result<i128, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        Ok(stream.vested_amount(env.ledger().timestamp()))
    }

    /// Returns the total amount vested at `timestamp`, including withdrawals.
    pub fn vested_at(env: Env, id: u64, timestamp: u64) -> Result<i128, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        Ok(stream.vested_amount(timestamp))
    }

    /// Returns the amount not yet vested, or `StreamNotFound`.
    pub fn locked(env: Env, id: u64) -> Result<i128, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        let vested = stream.vested_amount(env.ledger().timestamp());
        Ok((stream.total_amount - vested).max(0))
    }

    /// Returns seconds until the stream ends, or `StreamNotFound`.
    ///
    /// The current time is read from the ledger. Completed and cancelled
    /// streams return `0`.
    pub fn remaining_time(env: Env, id: u64) -> Result<u64, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        if stream.cancelled {
            return Ok(0);
        }
        Ok(stream.end_time.saturating_sub(env.ledger().timestamp()))
    }

    /// Vesting progress in basis points, from 0 (nothing vested) to
    /// [`crate::BPS_SCALE`] (100% vested). Useful for rendering a progress indicator without
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
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use soroban_sdk::Env;
    /// # use tricklepay_stream::{StreamContract, StreamContractClient};
    /// # let env = Env::default();
    /// # let contract_id = env.register(StreamContract, ());
    /// # let client = StreamContractClient::new(&env, &contract_id);
    /// # let stream_id = 0u64;
    /// let bps = client.progress(&stream_id); // e.g. 5000 for 50%
    /// ```
    pub fn progress(env: Env, id: u64) -> Result<u32, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        if stream.total_amount == 0 {
            return Ok(BPS_SCALE);
        }
        let vested = stream.vested_amount(env.ledger().timestamp());
        let scale = i128::from(BPS_SCALE);
        let progress = vested * scale / stream.total_amount;
        // The clamp keeps the value in 0..=BPS_SCALE, which always fits in u32.
        Ok(u32::try_from(progress.clamp(0, scale)).expect("clamped progress fits in u32"))
    }

    /// Returns the lifecycle status derived from the stream and current ledger time.
    pub fn status(env: Env, id: u64) -> Result<StreamStatus, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        Ok(status::stream_status(&stream, env.ledger().timestamp()))
    }

    /// Returns the number of streams created so far.
    pub fn stream_count(env: Env) -> u64 {
        storage::stream_count(&env)
    }

    /// Returns the maximum amount accepted by `create_stream`.
    pub fn max_amount(_env: Env) -> i128 {
        MAX_AMOUNT
    }
}

/// Shared body of [`StreamContract::withdraw`] and
/// [`StreamContract::withdraw_amount`]. `select` receives the withdrawable
/// balance and returns the amount to send, or the error to reject with.
fn withdraw_with(
    env: &Env,
    id: u64,
    select: impl FnOnce(i128) -> Result<i128, StreamError>,
) -> Result<i128, StreamError> {
    let mut stream = storage::get_stream(env, id).ok_or(StreamError::StreamNotFound)?;
    stream.recipient.require_auth();

    let vested = stream.vested_amount(env.ledger().timestamp());
    let amount = select(vesting::withdrawable_amount(vested, stream.withdrawn))?;

    stream.withdrawn += amount;
    storage::set_stream(env, id, &stream);

    transfer(
        env,
        &stream.token,
        &env.current_contract_address(),
        &stream.recipient,
        amount,
    );

    events::publish_withdrawn(env, &stream.recipient, id, amount, &stream.recipient);

    Ok(amount)
}

/// Move `amount` of `token` from `from` to `to`.
fn transfer(env: &Env, token: &Address, from: &Address, to: &Address, amount: i128) {
    TokenClient::new(env, token).transfer(from, to, &amount);
}
