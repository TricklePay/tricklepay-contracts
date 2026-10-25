# Design: batch stream creation

This document proposes `create_streams` for payroll and grant disbursements.
It is a design only; the existing `create_stream` entry point remains
unchanged.

## Proposed signature

Use one shared sender and token, with each entry carrying its own recipient,
amount, and schedule:

```rust
#[contracttype]
pub struct StreamInput {
    pub recipient: Address,
    pub total_amount: i128,
    pub start_time: u64,
    pub end_time: u64,
    pub cliff_time: u64,
}

pub fn create_streams(
    env: Env,
    sender: Address,
    token: Address,
    entries: Vec<StreamInput>,
) -> Result<Vec<u64>, StreamError>
```

The returned ids correspond to entries in input order. The batch is limited
to **20 entries**; this bounds validation work, storage writes, event
publication, and transaction resource use. An empty batch returns a proposed
`EmptyBatch` error (new code 15); an oversized batch returns
`BatchTooLarge` (code 14).

## Validation and effects

Perform checks in this order, before any token transfer or storage write:

1. Require authorization from `sender`.
2. Reject an empty batch with `EmptyBatch`, or a batch larger than 20 with
   `BatchTooLarge`.
3. Validate shared participants: `sender` and `token` must not be this
   contract's address, and the token cannot be the sender.
4. In input order, validate each entry's participant addresses (recipient
   differs from sender and neither address is this contract or the token),
   positive amount and `MAX_AMOUNT` ceiling, then time range, cliff range,
   and future end time, using the same rules as `create_stream`.
5. Sum amounts with checked arithmetic. Reject overflow or a combined amount
   above `MAX_AMOUNT`, which also keeps the single token transfer within the
   supported amount ceiling.
6. Check that the stream id counter can allocate all entries without
   overflowing `u64`.

The first failing check determines the returned error. **Any invalid entry
rejects the entire batch**: no ids are consumed, no streams are written, no
token transfer occurs, and no stream events are emitted. A token-transfer
failure likewise fails the transaction before stream writes; Soroban
transaction atomicity rolls back any effects if a later operation fails.

After validation, transfer the combined amount from `sender` to the contract
once. Then store each stream, advance the id counter by the number of entries,
and publish one existing `Created` event per stream in input order, after the
funding transfer. There is no separate batch event.

## Fees and resource use

Compared with **N separate `create_stream` calls**, a single batch requires
one contract invocation and one sender authorization rather than N, and makes
one token transfer invocation rather than N. These reduce repeated invocation,
authorization, and transfer overhead, so a batch should generally cost less
than N separate calls for the same streams.

The savings are not N-fold: the batch still validates and stores N streams,
consumes N ids, and emits N `Created` events. Those per-stream storage and
event costs remain O(N), and the single transaction must fit the resources for
the whole batch. The 20-entry limit bounds that cost; actual fee savings depend
on the token implementation and ledger resource pricing.
