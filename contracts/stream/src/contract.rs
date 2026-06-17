use soroban_sdk::{contract, contractimpl, token::Client as TokenClient, Address, Env};

use crate::error::StreamError;
use crate::events;
use crate::storage;
use crate::types::Stream;
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

        events::created(&env, id, &sender, &recipient, &token, total_amount);

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

        events::withdrawn(&env, id, &stream.recipient, available);

        Ok(available)
    }
}
