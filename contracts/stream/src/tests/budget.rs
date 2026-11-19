//! Resource budget regression tests.
//!
//! Integrators pay Soroban resource fees based on instructions, memory, and
//! ledger entries touched. These tests record the measured resource costs in the
//! test environment for each primary entry point (`create_stream`, `withdraw`,
//! `cancel`, and `get_streams` at 1 and 20 ids) and assert documented ceilings.
//!
//! If an assertion fails, the error message reports the measured value and ceiling
//! so that any resource growth can be analyzed and the ceiling raised deliberately.

#![cfg(test)]

use soroban_sdk::{vec, Vec};

use super::helpers::StreamTest;

/// Ceiling limits for `create_stream`:
/// Measured: ~329,231 CPU instructions, ~128,436 memory bytes, 9 read entries, 5 write entries.
const CREATE_STREAM_CPU_CEILING: u64 = 400_000;
const CREATE_STREAM_MEM_CEILING: u64 = 160_000;
const CREATE_STREAM_READ_ENTRIES_CEILING: u32 = 12;
const CREATE_STREAM_WRITE_ENTRIES_CEILING: u32 = 7;

/// Ceiling limits for `withdraw`:
/// Measured: ~327,290 CPU instructions, ~125,723 memory bytes, 8 read entries, 4 write entries.
const WITHDRAW_CPU_CEILING: u64 = 400_000;
const WITHDRAW_MEM_CEILING: u64 = 160_000;
const WITHDRAW_READ_ENTRIES_CEILING: u32 = 10;
const WITHDRAW_WRITE_ENTRIES_CEILING: u32 = 6;

/// Ceiling limits for `cancel`:
/// Measured: ~325,373 CPU instructions, ~119,236 memory bytes, 7 read entries, 4 write entries.
const CANCEL_CPU_CEILING: u64 = 400_000;
const CANCEL_MEM_CEILING: u64 = 150_000;
const CANCEL_READ_ENTRIES_CEILING: u32 = 10;
const CANCEL_WRITE_ENTRIES_CEILING: u32 = 6;

/// Ceiling limits for `get_streams` at 1 id:
/// Measured: ~65,457 CPU instructions, ~35,165 memory bytes, 2 read entries, 0 write entries.
const GET_STREAMS_1_CPU_CEILING: u64 = 85_000;
const GET_STREAMS_1_MEM_CEILING: u64 = 45_000;
const GET_STREAMS_1_READ_ENTRIES_CEILING: u32 = 3;

/// Ceiling limits for `get_streams` at 20 ids:
/// Measured: ~976,203 CPU instructions, ~529,963 memory bytes, 21 read entries, 0 write entries.
const GET_STREAMS_20_CPU_CEILING: u64 = 1_200_000;
const GET_STREAMS_20_MEM_CEILING: u64 = 650_000;
const GET_STREAMS_20_READ_ENTRIES_CEILING: u32 = 25;

#[test]
fn create_stream_resource_budget_stays_within_ceiling() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    t.env.cost_estimate().budget().reset_unlimited();

    let _id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    let res = t.env.cost_estimate().resources();
    let cpu = t.env.cost_estimate().budget().cpu_instruction_cost();
    let mem = t.env.cost_estimate().budget().memory_bytes_cost();
    let total_reads = res.disk_read_entries + res.memory_read_entries;

    assert!(
        cpu <= CREATE_STREAM_CPU_CEILING,
        "create_stream CPU budget exceeded ceiling: measured {}, ceiling {}",
        cpu,
        CREATE_STREAM_CPU_CEILING
    );
    assert!(
        mem <= CREATE_STREAM_MEM_CEILING,
        "create_stream memory budget exceeded ceiling: measured {}, ceiling {}",
        mem,
        CREATE_STREAM_MEM_CEILING
    );
    assert!(
        total_reads <= CREATE_STREAM_READ_ENTRIES_CEILING,
        "create_stream read entries exceeded ceiling: measured {}, ceiling {}",
        total_reads,
        CREATE_STREAM_READ_ENTRIES_CEILING
    );
    assert!(
        res.write_entries <= CREATE_STREAM_WRITE_ENTRIES_CEILING,
        "create_stream write entries exceeded ceiling: measured {}, ceiling {}",
        res.write_entries,
        CREATE_STREAM_WRITE_ENTRIES_CEILING
    );
}

#[test]
fn withdraw_resource_budget_stays_within_ceiling() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.set_time(600);
    t.env.cost_estimate().budget().reset_unlimited();

    let withdrawn = t.contract.withdraw(&id);
    assert_eq!(withdrawn, 500);

    let res = t.env.cost_estimate().resources();
    let cpu = t.env.cost_estimate().budget().cpu_instruction_cost();
    let mem = t.env.cost_estimate().budget().memory_bytes_cost();
    let total_reads = res.disk_read_entries + res.memory_read_entries;

    assert!(
        cpu <= WITHDRAW_CPU_CEILING,
        "withdraw CPU budget exceeded ceiling: measured {}, ceiling {}",
        cpu,
        WITHDRAW_CPU_CEILING
    );
    assert!(
        mem <= WITHDRAW_MEM_CEILING,
        "withdraw memory budget exceeded ceiling: measured {}, ceiling {}",
        mem,
        WITHDRAW_MEM_CEILING
    );
    assert!(
        total_reads <= WITHDRAW_READ_ENTRIES_CEILING,
        "withdraw read entries exceeded ceiling: measured {}, ceiling {}",
        total_reads,
        WITHDRAW_READ_ENTRIES_CEILING
    );
    assert!(
        res.write_entries <= WITHDRAW_WRITE_ENTRIES_CEILING,
        "withdraw write entries exceeded ceiling: measured {}, ceiling {}",
        res.write_entries,
        WITHDRAW_WRITE_ENTRIES_CEILING
    );
}

#[test]
fn cancel_resource_budget_stays_within_ceiling() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.set_time(600);
    t.env.cost_estimate().budget().reset_unlimited();

    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 500);

    let res = t.env.cost_estimate().resources();
    let cpu = t.env.cost_estimate().budget().cpu_instruction_cost();
    let mem = t.env.cost_estimate().budget().memory_bytes_cost();
    let total_reads = res.disk_read_entries + res.memory_read_entries;

    assert!(
        cpu <= CANCEL_CPU_CEILING,
        "cancel CPU budget exceeded ceiling: measured {}, ceiling {}",
        cpu,
        CANCEL_CPU_CEILING
    );
    assert!(
        mem <= CANCEL_MEM_CEILING,
        "cancel memory budget exceeded ceiling: measured {}, ceiling {}",
        mem,
        CANCEL_MEM_CEILING
    );
    assert!(
        total_reads <= CANCEL_READ_ENTRIES_CEILING,
        "cancel read entries exceeded ceiling: measured {}, ceiling {}",
        total_reads,
        CANCEL_READ_ENTRIES_CEILING
    );
    assert!(
        res.write_entries <= CANCEL_WRITE_ENTRIES_CEILING,
        "cancel write entries exceeded ceiling: measured {}, ceiling {}",
        res.write_entries,
        CANCEL_WRITE_ENTRIES_CEILING
    );
}

#[test]
fn get_streams_single_id_resource_budget_stays_within_ceiling() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.env.cost_estimate().budget().reset_unlimited();

    let streams = t.contract.get_streams(&vec![&t.env, id]);
    assert_eq!(streams.len(), 1);

    let res = t.env.cost_estimate().resources();
    let cpu = t.env.cost_estimate().budget().cpu_instruction_cost();
    let mem = t.env.cost_estimate().budget().memory_bytes_cost();
    let total_reads = res.disk_read_entries + res.memory_read_entries;

    assert!(
        cpu <= GET_STREAMS_1_CPU_CEILING,
        "get_streams(1) CPU budget exceeded ceiling: measured {}, ceiling {}",
        cpu,
        GET_STREAMS_1_CPU_CEILING
    );
    assert!(
        mem <= GET_STREAMS_1_MEM_CEILING,
        "get_streams(1) memory budget exceeded ceiling: measured {}, ceiling {}",
        mem,
        GET_STREAMS_1_MEM_CEILING
    );
    assert!(
        total_reads <= GET_STREAMS_1_READ_ENTRIES_CEILING,
        "get_streams(1) read entries exceeded ceiling: measured {}, ceiling {}",
        total_reads,
        GET_STREAMS_1_READ_ENTRIES_CEILING
    );
    assert_eq!(
        res.write_entries, 0,
        "get_streams is read-only and must never write ledger entries"
    );
}

#[test]
fn get_streams_batch_20_ids_resource_budget_stays_within_ceiling() {
    let t = StreamTest::setup(25_000);
    t.set_time(100);

    let mut ids = Vec::new(&t.env);
    for _ in 0..20 {
        ids.push_back(t.open_default_stream(1_000));
    }

    t.env.cost_estimate().budget().reset_unlimited();

    let streams = t.contract.get_streams(&ids);
    assert_eq!(streams.len(), 20);

    let res = t.env.cost_estimate().resources();
    let cpu = t.env.cost_estimate().budget().cpu_instruction_cost();
    let mem = t.env.cost_estimate().budget().memory_bytes_cost();
    let total_reads = res.disk_read_entries + res.memory_read_entries;

    assert!(
        cpu <= GET_STREAMS_20_CPU_CEILING,
        "get_streams(20) CPU budget exceeded ceiling: measured {}, ceiling {}",
        cpu,
        GET_STREAMS_20_CPU_CEILING
    );
    assert!(
        mem <= GET_STREAMS_20_MEM_CEILING,
        "get_streams(20) memory budget exceeded ceiling: measured {}, ceiling {}",
        mem,
        GET_STREAMS_20_MEM_CEILING
    );
    assert!(
        total_reads <= GET_STREAMS_20_READ_ENTRIES_CEILING,
        "get_streams(20) read entries exceeded ceiling: measured {}, ceiling {}",
        total_reads,
        GET_STREAMS_20_READ_ENTRIES_CEILING
    );
    assert_eq!(
        res.write_entries, 0,
        "get_streams is read-only and must never write ledger entries"
    );
}
