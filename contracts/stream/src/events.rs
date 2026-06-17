use soroban_sdk::{Address, Env, Symbol};

/// Publish a `created` event when a new stream is opened.
///
/// Topics: `("created", sender, recipient)` so an indexer can filter by
/// either party. Data: `(id, token, total_amount)`.
pub fn created(
    env: &Env,
    id: u64,
    sender: &Address,
    recipient: &Address,
    token: &Address,
    total_amount: i128,
) {
    let topics = (Symbol::new(env, "created"), sender.clone(), recipient.clone());
    env.events().publish(topics, (id, token.clone(), total_amount));
}

/// Publish a `withdrawn` event when a recipient pulls vested tokens.
///
/// Topics: `("withdrawn", recipient)`. Data: `(id, amount)`.
pub fn withdrawn(env: &Env, id: u64, recipient: &Address, amount: i128) {
    let topics = (Symbol::new(env, "withdrawn"), recipient.clone());
    env.events().publish(topics, (id, amount));
}

/// Publish a `cancelled` event when a sender stops a stream early.
///
/// Topics: `("cancelled", sender)`. Data: `(id, recipient_amount,
/// sender_refund)` so both sides of the split are visible without a
/// follow-up query.
pub fn cancelled(
    env: &Env,
    id: u64,
    sender: &Address,
    recipient_amount: i128,
    sender_refund: i128,
) {
    let topics = (Symbol::new(env, "cancelled"), sender.clone());
    env.events().publish(topics, (id, recipient_amount, sender_refund));
}
