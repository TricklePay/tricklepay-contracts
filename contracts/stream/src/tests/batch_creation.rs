#![cfg(test)]

use soroban_sdk::{testutils::Address as _, vec, Address, Vec};

use crate::{StreamError, StreamInput, MAX_AMOUNT, MAX_BATCH_SIZE};

use super::helpers::{StreamTest, END, START};

fn entry(
    recipient: Address,
    total_amount: i128,
    start_time: u64,
    end_time: u64,
    cliff_time: u64,
) -> StreamInput {
    StreamInput {
        recipient,
        total_amount,
        start_time,
        end_time,
        cliff_time,
    }
}

fn single(env: &soroban_sdk::Env, input: StreamInput) -> Vec<StreamInput> {
    let mut entries = Vec::new(env);
    entries.push_back(input);
    entries
}

/// `create_streams` stores one stream per entry, returns their ids in input
/// order, and funds the whole batch with a single transfer.
#[test]
fn create_streams_stores_each_entry_and_moves_the_total_once() {
    let t = StreamTest::setup(3_000);
    t.set_time(START);

    let second = Address::generate(&t.env);
    let third = Address::generate(&t.env);
    let mut entries = Vec::new(&t.env);
    entries.push_back(entry(t.recipient.clone(), 1_000, START, END, START));
    entries.push_back(entry(second.clone(), 1_500, START, END, 400));
    entries.push_back(entry(third.clone(), 500, 200, 700, 200));

    let ids = t
        .contract
        .create_streams(&t.sender, &t.token_address, &entries);

    assert_eq!(ids.len(), 3);
    assert_eq!(ids.get(0), Some(0));
    assert_eq!(ids.get(1), Some(1));
    assert_eq!(ids.get(2), Some(2));

    // One token transfer, then one Created event per stream in input order.
    // This is asserted before any further contract call, because events only
    // report the most recent invocation.
    assert_eq!(
        t.event_publishers(),
        vec![
            &t.env,
            t.token_address.clone(),
            t.contract.address.clone(),
            t.contract.address.clone(),
            t.contract.address.clone(),
        ]
    );

    assert_eq!(t.contract.stream_count(), 3);

    // One transfer carried the combined 3,000 into the contract.
    assert_eq!(t.token.balance(&t.sender), 0);
    assert_eq!(t.token.balance(&t.contract.address), 3_000);

    let first = t.contract.get_stream(&0);
    assert_eq!(first.sender, t.sender);
    assert_eq!(first.recipient, t.recipient);
    assert_eq!(first.token, t.token_address);
    assert_eq!(first.total_amount, 1_000);
    assert_eq!(first.start_time, START);
    assert_eq!(first.end_time, END);
    assert_eq!(first.cliff_time, START);

    let second_stream = t.contract.get_stream(&1);
    assert_eq!(second_stream.recipient, second);
    assert_eq!(second_stream.total_amount, 1_500);
    assert_eq!(second_stream.cliff_time, 400);

    let third_stream = t.contract.get_stream(&2);
    assert_eq!(third_stream.recipient, third);
    assert_eq!(third_stream.total_amount, 500);
    assert_eq!(third_stream.start_time, 200);
    assert_eq!(third_stream.end_time, 700);
}

/// The batch works at exactly `MAX_BATCH_SIZE` entries, allocating ids
/// consecutively and creating each stream.
#[test]
fn create_streams_accepts_the_maximum_batch_size() {
    let t = StreamTest::setup(MAX_BATCH_SIZE as i128);
    t.set_time(START);

    let mut entries = Vec::new(&t.env);
    for _ in 0..MAX_BATCH_SIZE {
        entries.push_back(entry(Address::generate(&t.env), 1, START, END, START));
    }

    let ids = t
        .contract
        .create_streams(&t.sender, &t.token_address, &entries);

    assert_eq!(ids.len(), MAX_BATCH_SIZE);
    assert_eq!(t.contract.stream_count(), u64::from(MAX_BATCH_SIZE));
    for index in 0..MAX_BATCH_SIZE {
        assert_eq!(ids.get(index), Some(u64::from(index)));
        assert!(t.contract.stream_exists(&u64::from(index)));
    }
    assert_eq!(t.token.balance(&t.contract.address), MAX_BATCH_SIZE as i128);
}

/// An empty batch is rejected with `EmptyBatch` and changes nothing.
#[test]
fn create_streams_rejects_an_empty_batch() {
    let t = StreamTest::setup(1_000);
    t.set_time(START);

    let entries = Vec::new(&t.env);
    let result = t
        .contract
        .try_create_streams(&t.sender, &t.token_address, &entries);

    assert_eq!(result, Err(Ok(StreamError::EmptyBatch)));
    t.assert_nothing_happened(1_000);
}

/// A batch above `MAX_BATCH_SIZE` is rejected with `BatchTooLarge` before any
/// entry is examined.
#[test]
fn create_streams_rejects_an_oversized_batch() {
    let t = StreamTest::setup(MAX_BATCH_SIZE as i128 + 1);
    t.set_time(START);

    let mut entries = Vec::new(&t.env);
    for _ in 0..=MAX_BATCH_SIZE {
        entries.push_back(entry(Address::generate(&t.env), 1, START, END, START));
    }

    let result = t
        .contract
        .try_create_streams(&t.sender, &t.token_address, &entries);

    assert_eq!(result, Err(Ok(StreamError::BatchTooLarge)));
    t.assert_nothing_happened(MAX_BATCH_SIZE as i128 + 1);
}

/// A single invalid entry rejects the entire batch: no stream is stored, no id
/// is consumed, and no tokens move.
#[test]
fn create_streams_rejects_the_whole_batch_on_one_invalid_entry() {
    let t = StreamTest::setup(3_000);
    t.set_time(START);

    let mut entries = Vec::new(&t.env);
    entries.push_back(entry(t.recipient.clone(), 1_000, START, END, START));
    entries.push_back(entry(Address::generate(&t.env), 0, START, END, START));
    entries.push_back(entry(Address::generate(&t.env), 1_000, START, END, START));

    let result = t
        .contract
        .try_create_streams(&t.sender, &t.token_address, &entries);

    assert_eq!(result, Err(Ok(StreamError::InvalidAmount)));
    t.assert_nothing_happened(3_000);
    assert!(!t.contract.stream_exists(&0));
}

/// Per-entry participant rules: the recipient must differ from the sender and
/// be neither this contract's address nor the token.
#[test]
fn create_streams_rejects_invalid_entry_participants() {
    let t = StreamTest::setup(1_000);
    t.set_time(START);

    let contract_address = t.contract.address.clone();
    let token_address = t.token_address.clone();
    let sender = t.sender.clone();

    for recipient in [contract_address, token_address, sender] {
        let entries = single(&t.env, entry(recipient, 1_000, START, END, START));
        let result = t
            .contract
            .try_create_streams(&t.sender, &t.token_address, &entries);
        assert_eq!(result, Err(Ok(StreamError::InvalidParticipant)));
    }

    t.assert_nothing_happened(1_000);
}

/// Shared participant rules: neither the sender nor the token may be this
/// contract's address, and the token may not be the sender.
#[test]
fn create_streams_rejects_invalid_shared_participants() {
    let t = StreamTest::setup(1_000);
    t.set_time(START);
    let contract_address = t.contract.address.clone();

    let entries = single(&t.env, entry(t.recipient.clone(), 1_000, START, END, START));

    let result = t
        .contract
        .try_create_streams(&contract_address, &t.token_address, &entries);
    assert_eq!(result, Err(Ok(StreamError::InvalidParticipant)));

    let result = t
        .contract
        .try_create_streams(&t.sender, &contract_address, &entries);
    assert_eq!(result, Err(Ok(StreamError::InvalidParticipant)));

    let result = t
        .contract
        .try_create_streams(&t.sender, &t.sender, &entries);
    assert_eq!(result, Err(Ok(StreamError::InvalidParticipant)));

    t.assert_nothing_happened(1_000);
}

/// Per-entry amount, schedule, and cliff rules match `create_stream`.
#[test]
fn create_streams_rejects_invalid_entry_amounts_and_schedules() {
    let t = StreamTest::setup(1_000);
    t.set_time(START);

    let amount_too_large = single(
        &t.env,
        entry(t.recipient.clone(), MAX_AMOUNT + 1, START, END, START),
    );
    assert_eq!(
        t.contract
            .try_create_streams(&t.sender, &t.token_address, &amount_too_large),
        Err(Ok(StreamError::AmountTooLarge))
    );

    let bad_range = single(&t.env, entry(t.recipient.clone(), 1_000, END, START, START));
    assert_eq!(
        t.contract
            .try_create_streams(&t.sender, &t.token_address, &bad_range),
        Err(Ok(StreamError::InvalidTimeRange))
    );

    let bad_cliff = single(
        &t.env,
        entry(t.recipient.clone(), 1_000, START, END, END + 1),
    );
    assert_eq!(
        t.contract
            .try_create_streams(&t.sender, &t.token_address, &bad_cliff),
        Err(Ok(StreamError::InvalidCliff))
    );

    let past_window = single(&t.env, entry(t.recipient.clone(), 1_000, 10, 50, 10));
    assert_eq!(
        t.contract
            .try_create_streams(&t.sender, &t.token_address, &past_window),
        Err(Ok(StreamError::StreamWindowInPast))
    );

    t.assert_nothing_happened(1_000);
}

/// The combined amount must respect `MAX_AMOUNT` even when every entry is
/// individually within the ceiling.
#[test]
fn create_streams_rejects_a_combined_amount_above_max() {
    let t = StreamTest::setup(1);
    t.set_time(START);

    let mut entries = Vec::new(&t.env);
    entries.push_back(entry(
        Address::generate(&t.env),
        MAX_AMOUNT,
        START,
        END,
        START,
    ));
    entries.push_back(entry(
        Address::generate(&t.env),
        MAX_AMOUNT,
        START,
        END,
        START,
    ));

    let result = t
        .contract
        .try_create_streams(&t.sender, &t.token_address, &entries);

    assert_eq!(result, Err(Ok(StreamError::AmountTooLarge)));
    t.assert_nothing_happened(1);
}

/// The id counter must be able to allocate every entry without wrapping.
#[test]
fn create_streams_rejects_an_exhausted_counter() {
    let t = StreamTest::setup(2_000);
    t.set_time(START);
    t.set_stream_count(u64::MAX - 1);

    let mut entries = Vec::new(&t.env);
    entries.push_back(entry(Address::generate(&t.env), 1_000, START, END, START));
    entries.push_back(entry(Address::generate(&t.env), 1_000, START, END, START));

    let result = t
        .contract
        .try_create_streams(&t.sender, &t.token_address, &entries);
    assert_eq!(result, Err(Ok(StreamError::StreamCountExhausted)));
    assert_eq!(t.contract.stream_count(), u64::MAX - 1);
    assert_eq!(t.token.balance(&t.sender), 2_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// The last batch that exactly fills the id space is accepted.
#[test]
fn create_streams_accepts_the_final_ids_then_refuses_the_next() {
    let t = StreamTest::setup(2_000);
    t.set_time(START);
    t.set_stream_count(u64::MAX - 2);

    let mut entries = Vec::new(&t.env);
    entries.push_back(entry(Address::generate(&t.env), 1_000, START, END, START));
    entries.push_back(entry(Address::generate(&t.env), 1_000, START, END, START));

    let ids = t
        .contract
        .create_streams(&t.sender, &t.token_address, &entries);
    assert_eq!(ids.get(0), Some(u64::MAX - 2));
    assert_eq!(ids.get(1), Some(u64::MAX - 1));
    assert_eq!(t.contract.stream_count(), u64::MAX);

    let result = t
        .contract
        .try_create_streams(&t.sender, &t.token_address, &entries);
    assert_eq!(result, Err(Ok(StreamError::StreamCountExhausted)));
    assert_eq!(t.token.balance(&t.contract.address), 2_000);
}

/// Validation follows input order: the first invalid entry determines the
/// error.
#[test]
fn create_streams_reports_the_first_failing_entry_in_input_order() {
    let t = StreamTest::setup(2_000);
    t.set_time(START);

    let mut entries = Vec::new(&t.env);
    entries.push_back(entry(t.recipient.clone(), 1_000, END, START, START));
    entries.push_back(entry(Address::generate(&t.env), 0, START, END, START));

    let result = t
        .contract
        .try_create_streams(&t.sender, &t.token_address, &entries);
    assert_eq!(result, Err(Ok(StreamError::InvalidTimeRange)));
    t.assert_nothing_happened(2_000);
}
