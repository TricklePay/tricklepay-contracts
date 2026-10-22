//! Public events emitted by the stream contract for off-chain consumers.

use soroban_sdk::{contractevent, Address, Env};

use crate::types::Stream;

/// Emitted when a stream is opened; sender and recipient are indexed as topics.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Created {
    #[topic]
    pub sender: Address,
    #[topic]
    pub recipient: Address,
    pub id: u64,
    pub token: Address,
    pub total_amount: i128,
    pub start_time: u64,
    pub end_time: u64,
    pub cliff_time: u64,
}

/// Emitted when a recipient withdraws vested tokens.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Withdrawn {
    #[topic]
    pub recipient: Address,
    pub id: u64,
    pub amount: i128,
}

/// Emitted when a stream is cancelled, reporting the recipient's remaining
/// vested balance and sender's refund.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cancelled {
    #[topic]
    pub sender: Address,
    pub id: u64,
    pub recipient_amount: i128,
    pub sender_refund: i128,
}

/// Publishes [`Created`] for a newly stored stream.
pub(crate) fn publish_created(env: &Env, id: u64, stream: &Stream) {
    Created {
        sender: stream.sender.clone(),
        recipient: stream.recipient.clone(),
        id,
        token: stream.token.clone(),
        total_amount: stream.total_amount,
        start_time: stream.start_time,
        end_time: stream.end_time,
        cliff_time: stream.cliff_time,
    }
    .publish(env);
}

/// Publishes [`Withdrawn`] after transferring tokens to the recipient.
pub(crate) fn publish_withdrawn(env: &Env, recipient: &Address, id: u64, amount: i128) {
    Withdrawn {
        recipient: recipient.clone(),
        id,
        amount,
    }
    .publish(env);
}
