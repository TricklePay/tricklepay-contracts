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
    pub created_at: u64,
}

/// Emitted when a stream's recipient transfers their receiving rights.
///
/// Both the previous and the new recipient are indexed as topics so indexers
/// can filter transfers by either side. The stream id stays in the data
/// payload, matching the convention used by the other stream events.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipientTransferred {
    #[topic]
    pub old_recipient: Address,
    #[topic]
    pub new_recipient: Address,
    pub stream_id: u64,
}

/// Emitted when a recipient withdraws vested tokens.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Withdrawn {
    #[topic]
    pub recipient: Address,
    pub id: u64,
    pub amount: i128,
    pub destination: Address,
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

/// Emitted when a stream is topped up by the sender.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToppedUp {
    #[topic]
    pub sender: Address,
    pub stream_id: u64,
    pub added_amount: i128,
    pub new_total_amount: i128,
    pub new_end_time: u64,
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
        created_at: stream.created_at,
    }
    .publish(env);
}

/// Publishes [`RecipientTransferred`] after the stream's recipient changes.
pub(crate) fn publish_recipient_transferred(
    env: &Env,
    old_recipient: &Address,
    new_recipient: &Address,
    stream_id: u64,
) {
    RecipientTransferred {
        old_recipient: old_recipient.clone(),
        new_recipient: new_recipient.clone(),
        stream_id,
    }
    .publish(env);
}

/// Publishes [`Withdrawn`] after transferring tokens to the recipient or destination.
pub(crate) fn publish_withdrawn(
    env: &Env,
    recipient: &Address,
    id: u64,
    amount: i128,
    destination: &Address,
) {
    Withdrawn {
        recipient: recipient.clone(),
        id,
        amount,
        destination: destination.clone(),
    }
    .publish(env);
}

/// Publishes [`Cancelled`] after settling an early termination.
pub(crate) fn publish_cancelled(
    env: &Env,
    sender: &Address,
    id: u64,
    recipient_amount: i128,
    sender_refund: i128,
) {
    Cancelled {
        sender: sender.clone(),
        id,
        recipient_amount,
        sender_refund,
    }
    .publish(env);
}

/// Publishes [`ToppedUp`] after depositing additional tokens into an active stream.
pub(crate) fn publish_topped_up(
    env: &Env,
    sender: &Address,
    stream_id: u64,
    added_amount: i128,
    new_total_amount: i128,
    new_end_time: u64,
) {
    ToppedUp {
        sender: sender.clone(),
        stream_id,
        added_amount,
        new_total_amount,
        new_end_time,
    }
    .publish(env);
}
