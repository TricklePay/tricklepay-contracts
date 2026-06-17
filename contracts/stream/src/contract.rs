use soroban_sdk::{contract, contractimpl, token::Client as TokenClient, Address, Env};

use crate::error::StreamError;
use crate::events;
use crate::storage;
use crate::types::{Stream, StreamStatus};
use crate::vesting;

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

        if total_amount <= 0 {
            return Err(StreamError::InvalidAmount);
        }
        if start_time >= end_time {
            return Err(StreamError::InvalidTimeRange);
        }
        if cliff_time < start_time || cliff_time > end_time {
            return Err(StreamError::InvalidCliff);
        }

        TokenClient::new(&env, &token).transfer(
            &sender,
            &env.current_contract_address(),
            &total_amount,
        );

        let id = storage::stream_count(&env);
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
        storage::set_stream_count(&env, id + 1);

        events::Created {
            sender: sender.clone(),
            recipient: recipient.clone(),
            id,
            token: token.clone(),
            total_amount,
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

    /// Lifecycle status of a stream at the current ledger time.
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
