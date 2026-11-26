# TTL Restore Walkthrough for Archived Entries

When a stream entry or the contract instance is not accessed for longer than `ENTRY_TTL` (≈ 30 days at 5-second ledger close), it is archived by the Stellar network and must be restored off-chain before the contract can use it again.

This guide shows how to restore archived entries using the Stellar CLI for both testnet and mainnet deployments.

## Background

The stream contract uses two types of storage entries:

1. **Instance storage** (`StreamCount`) — holds the monotonic stream ID counter
2. **Persistent storage** (`Stream(id)`) — holds individual stream records

Both expire after `ENTRY_TTL` ledgers (≈ 30 days) without access. Individual stream views like `get_stream(id)` extend TTL automatically when accessed within `BUMP_THRESHOLD` (≈ 6 days before expiry), but batch reads (`get_streams`) and preview functions (`cancel_preview`) do not extend TTL by design (see [storage.rs](../contracts/stream/src/storage.rs) for the rationale).

## Prerequisites

- Stellar CLI version `25.2.0` (aligned with Soroban SDK `25.3.2`):
  ```bash
  cargo install --locked stellar-cli --version 25.2.0
  ```
- The contract address of your deployed stream contract
- The stream ID you want to restore (for stream entries)
- Network RPC endpoint URL

## Ledger Key Forms

To restore an entry, you need its ledger key in XDR format. The stream contract uses these key structures:

### Instance Storage Key (StreamCount)

The instance entry holds the `StreamCount` counter. Its ledger key is:

```text
LedgerKey::ContractData {
    contract: <contract_address>,
    key: ScVal::LedgerKeyContractInstance,
    durability: ContractDataDurability::Persistent
}
```

### Stream Entry Key (Stream(id))

Each stream record is stored with a persistent key:

```text
LedgerKey::ContractData {
    contract: <contract_address>,
    key: ScVal::Vec([
        ScVal::Symbol("Stream"),
        ScVal::U64(stream_id)
    ]),
    durability: ContractDataDurability::Persistent
}
```

## Restoring the Contract Instance

The instance entry must be alive for `create_stream` to work. If it has been archived, restore it:

### Testnet

```bash
stellar contract restore \
  --id <CONTRACT_ID> \
  --network testnet \
  --source <YOUR_SECRET_KEY_OR_IDENTITY> \
  --rpc-url https://soroban-testnet.stellar.org \
  --ledger-key-instance
```

### Mainnet

```bash
stellar contract restore \
  --id <CONTRACT_ID> \
  --network mainnet \
  --source <YOUR_SECRET_KEY_OR_IDENTITY> \
  --rpc-url https://soroban-mainnet.stellar.org \
  --ledger-key-instance
```

## Restoring a Stream Entry

If a specific stream has been archived (e.g., an old completed or cancelled stream), restore it by its ID:

### Testnet

```bash
stellar contract restore \
  --id <CONTRACT_ID> \
  --network testnet \
  --source <YOUR_SECRET_KEY_OR_IDENTITY> \
  --rpc-url https://soroban-testnet.stellar.org \
  --key '[{"symbol":"Stream"},{"u64":<STREAM_ID>}]'
```

### Mainnet

```bash
stellar contract restore \
  --id <CONTRACT_ID> \
  --network mainnet \
  --source <YOUR_SECRET_KEY_OR_IDENTITY> \
  --rpc-url https://soroban-mainnet.stellar.org \
  --key '[{"symbol":"Stream"},{"u64":<STREAM_ID>}]'
```

Replace `<STREAM_ID>` with the numeric stream ID (e.g., `42` for stream 42).

## Example: Restore Stream 100 on Testnet

```bash
stellar contract restore \
  --id CDXYZ...ABC \
  --network testnet \
  --source alice \
  --rpc-url https://soroban-testnet.stellar.org \
  --key '[{"symbol":"Stream"},{"u64":100}]'
```

## Confirming the Restore Worked

After restoring, verify the entry is accessible:

### For Instance Storage

Try creating a new stream or querying `stream_count()`:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network testnet \
  --source <YOUR_IDENTITY> \
  --rpc-url https://soroban-testnet.stellar.org \
  -- stream_count
```

If it returns a number without error, the instance is alive.

### For Stream Entry

Query the restored stream:

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network testnet \
  --source <YOUR_IDENTITY> \
  --rpc-url https://soroban-testnet.stellar.org \
  -- get_stream --id <STREAM_ID>
```

If it returns the stream record, the entry has been successfully restored.

## Extending TTL After Restore

Once restored, an entry starts with a fresh TTL of `ENTRY_TTL` ledgers. To prevent it from expiring again:

- **For streams**: Call any mutating function (`withdraw`, `cancel`) or individual view (`get_stream`, `withdrawable`, `vested`, etc.) that extends TTL.
- **For instance**: Call `create_stream` — it extends the instance TTL automatically.

Use the `stellar contract extend` command to manually extend TTL without invoking the contract:

### Extend Stream Entry TTL

```bash
stellar contract extend \
  --id <CONTRACT_ID> \
  --network testnet \
  --source <YOUR_IDENTITY> \
  --rpc-url https://soroban-testnet.stellar.org \
  --key '[{"symbol":"Stream"},{"u64":<STREAM_ID>}]' \
  --ledgers-to-extend 518400
```

### Extend Instance TTL

```bash
stellar contract extend \
  --id <CONTRACT_ID> \
  --network testnet \
  --source <YOUR_IDENTITY> \
  --rpc-url https://soroban-testnet.stellar.org \
  --ledger-key-instance \
  --ledgers-to-extend 518400
```

The `--ledgers-to-extend` value should match or exceed `ENTRY_TTL` (518400 ledgers ≈ 30 days).

## Common Errors

### `ContractDataNotFound`

The entry is archived or never existed. Use `stellar contract restore` with the correct key before querying.

### `Invalid XDR key format`

Check the JSON structure passed to `--key`. Stream keys must be `[{"symbol":"Stream"},{"u64":<ID>}]`.

### `Transaction failed: insufficient balance`

Restore and extend operations consume XLM for transaction fees and rent. Ensure the source account has sufficient balance.

## References

- [storage.rs](../contracts/stream/src/storage.rs) — TTL constants and rationale
- [THREAT_MODEL.md § Storage TTL](../THREAT_MODEL.md) — archival risks and mitigation
- [INTEGRATOR_OPERATIONS.md](INTEGRATOR_OPERATIONS.md) — operational policies for integrators
- [Stellar CLI documentation](https://developers.stellar.org/docs/tools/developer-tools/cli/stellar-cli)
