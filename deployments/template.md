# Deployment Record: <contract-name> (<network>)

Use this template to record each contract deployment. Copy this file to
`deployments/<network>-<date>-<contract-id-prefix>.md` and fill in the details.

## Deployment Metadata

| Field | Value | Notes |
|---|---|---|
| **Contract Name** | `tricklepay-stream` | Crate name |
| **Contract ID / Address** | `C...` | 56-character Bech32 Soroban contract address |
| **Target Network** | `testnet` \| `futurenet` \| `mainnet` | Target Stellar network |
| **Deployment Date (UTC)** | `YYYY-MM-DD HH:MM:SS UTC` | Timestamp of deployment transaction |
| **Deployer Identity** | `<identity-name>` (`G...`) | Stellar CLI identity name and public key |
| **Transaction Hash** | `<tx-hash-hex>` | Transaction ID on Stellar network |

## Build Reference & Provenance

| Field | Value | Notes |
|---|---|---|
| **Git Commit SHA** | `<commit-sha-40-hex>` | Exact git commit deployed from |
| **Git Branch / Tag** | `vX.Y.Z` or `main` | Tagged release or branch reference |
| **WASM Hash (Local Build)** | `<sha256-hex>` | Output of `sha256sum` on release WASM |
| **WASM Hash (On-Chain)** | `<sha256-hex>` | Hash from `stellar contract inspect --id <CONTRACT_ID>` |
| **Toolchain Version** | `rustc ...` | Pinned compiler version from `rust-toolchain.toml` |
| **Build Profile** | `release` | Build profile and flags used |

## Verification Checklist

- [ ] On-chain WASM bytecode SHA-256 matches local release build (`make verify-deploy`).
- [ ] Contract interface verified against specification.
- [ ] Deployment record committed to version control.
