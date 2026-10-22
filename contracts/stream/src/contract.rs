use soroban_sdk::{contract, contractimpl, token::TokenClient, Address, Env};

use crate::error::StreamError;
use crate::events;
use crate::status;
use crate::storage;
use crate::types::{Stream, StreamStatus};
use crate::vesting;

/// Maximum `total_amount` accepted when creating a stream.
pub const MAX_AMOUNT: i128 = i64::MAX as i128;

/// Basis points in 100%, the scale [`StreamContract::progress`] reports on.
pub const BPS_SCALE: u32 = 10_000;

fn vested_amount_at(stream: &Stream, timestamp: u64) -> i128 {
    stream.vested_amount(timestamp)
}

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

    /// Opens a linearly vesting stream, escrows the full amount, and returns its id.
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

    /// Cancels an active stream, refunds its unvested amount, and leaves vested tokens claimable.
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

        events::Cancelled {
            sender: stream.sender.clone(),
            id,
            recipient_amount: settlement.recipient_remaining,
            sender_refund: settlement.refund,
        }
        .publish(&env);

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

    /// Returns whether a stream record exists, including cancelled and completed streams.
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
        Ok(vested_amount_at(&stream, env.ledger().timestamp()))
    }

    /// Returns the total amount vested at `timestamp`, including withdrawals.
    pub fn vested_at(env: Env, id: u64, timestamp: u64) -> Result<i128, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        Ok(vested_amount_at(&stream, timestamp))
    }

    /// Returns the amount not yet vested, or `StreamNotFound`.
    pub fn locked(env: Env, id: u64) -> Result<i128, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        let vested = stream.vested_amount(env.ledger().timestamp());
        Ok((stream.total_amount - vested).max(0))
    }

    /// Returns vesting progress in basis points, from `0` to `10000`.
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

    events::publish_withdrawn(env, &stream.recipient, id, amount);

    Ok(amount)
}

/// Move `amount` of `token` from `from` to `to`.
fn transfer(env: &Env, token: &Address, from: &Address, to: &Address, amount: i128) {
    TokenClient::new(env, token).transfer(from, to, &amount);
}
