//! Events emitted by the stream contract.
//!
//! These types form part of the contract's public interface. Indexers and
//! off-chain consumers depend on the topic layout and payload fields to filter
//! streams by participant and to reconstruct stream state without follow-up
//! contract calls. The shapes of these events are therefore effectively an API
//! contract: any change to a topic, a field name, or a field type is a
//! breaking change for downstream consumers and should be treated with the
//! same care as a change to a contract entry point.

use soroban_sdk::{contractevent, Address};

/// Emitted when a new stream is opened. Indexers can filter on the `sender`
/// and `recipient` topics to find streams for either party. The schedule fields
/// are included so an indexer can record the full stream without a follow-up
/// `get_stream` call.
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

/// Emitted when a sender cancels a stream. Carries both sides of the split so
/// the recipient's accrued amount and the sender's refund are visible without
/// a follow-up query.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cancelled {
    #[topic]
    pub sender: Address,
    pub id: u64,
    pub recipient_amount: i128,
    pub sender_refund: i128,
}
