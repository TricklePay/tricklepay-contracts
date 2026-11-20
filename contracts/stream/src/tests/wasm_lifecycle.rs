#![cfg(test)]

extern crate std;

use std::eprintln;
use std::fs;
use std::path::{Path, PathBuf};
use std::vec::Vec;

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::contract::StreamContractClient;
use crate::error::StreamError;
use crate::types::StreamStatus;

const RELATIVE_WASM_PATH: &str = "target/wasm32v1-none/release/tricklepay_stream.wasm";

fn resolve_wasm_path(relative_path: &str) -> Option<PathBuf> {
    // 1. Check relative to CARGO_MANIFEST_DIR (crate root)
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .join(relative_path);
    if manifest_path.exists() {
        return Some(manifest_path);
    }

    // 2. Check from current working directory
    let root_path = PathBuf::from(relative_path);
    if root_path.exists() {
        return Some(root_path);
    }

    // 3. Check two levels up (if running cargo test inside contracts/stream)
    let nested_path = PathBuf::from("../../").join(relative_path);
    if nested_path.exists() {
        return Some(nested_path);
    }

    None
}

fn is_ci_environment() -> bool {
    std::env::var("CI")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false)
        || std::env::var("GITHUB_ACTIONS")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false)
}

fn load_wasm_bytes_or_skip_internal(relative_path: &str, is_ci: bool) -> Option<Vec<u8>> {
    match resolve_wasm_path(relative_path) {
        Some(path) => match fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(err) => {
                panic!(
                    "Failed to read compiled WASM artifact at {}: {}",
                    path.display(),
                    err
                );
            }
        },
        None => {
            if is_ci {
                panic!(
                    "Compiled WASM binary `{}` was not found in CI environment. Ensure the WASM is built before running cargo test (run `cargo build --locked --release --target wasm32v1-none`).",
                    relative_path
                );
            } else {
                eprintln!(
                    "SKIPPED: `{}` was not found. Build the WASM first (`make wasm`) to run compiled WASM lifecycle tests.",
                    relative_path
                );
                None
            }
        }
    }
}

fn load_wasm_bytes_or_skip_from(relative_path: &str) -> Option<Vec<u8>> {
    load_wasm_bytes_or_skip_internal(relative_path, is_ci_environment())
}

fn load_wasm_bytes_or_skip() -> Option<Vec<u8>> {
    load_wasm_bytes_or_skip_from(RELATIVE_WASM_PATH)
}

#[test]
fn missing_wasm_skips_outside_ci() {
    let result = load_wasm_bytes_or_skip_internal("target/non_existent.wasm", false);
    assert!(result.is_none());
}

#[test]
#[should_panic(
    expected = "Compiled WASM binary `target/non_existent.wasm` was not found in CI environment"
)]
fn missing_wasm_panics_in_ci() {
    let _ = load_wasm_bytes_or_skip_internal("target/non_existent.wasm", true);
}

#[test]
fn compiled_wasm_stream_cancellation_lifecycle() {
    let wasm_bytes = match load_wasm_bytes_or_skip() {
        Some(bytes) => bytes,
        None => return,
    };

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(100);

    // Register compiled release WASM bytecode instead of native code
    let contract_id = env.register(wasm_bytes.as_slice(), ());
    let client = StreamContractClient::new(&env, &contract_id);

    // Deploy a Stellar Asset Contract token for streaming
    let issuer = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(issuer);
    let token_address = sac.address();
    let token = token::TokenClient::new(&env, &token_address);
    let token_admin = token::StellarAssetClient::new(&env, &token_address);

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    token_admin.mint(&sender, &10_000);
    assert_eq!(token.balance(&sender), 10_000);
    assert_eq!(token.balance(&contract_id), 0);

    // 1. Create Stream (start = 100, end = 1100, cliff = 100, amount = 1000)
    let stream_id = client.create_stream(
        &sender,
        &recipient,
        &token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(stream_id, 0);

    // Verify post-creation state
    assert_eq!(token.balance(&sender), 9_000);
    assert_eq!(token.balance(&contract_id), 1_000);
    assert_eq!(client.status(&stream_id), StreamStatus::Streaming);
    assert_eq!(client.vested(&stream_id), 0);
    assert_eq!(client.withdrawable(&stream_id), 0);
    assert_eq!(client.locked(&stream_id), 1_000);
    assert_eq!(client.stream_count(), 1);

    // 2. Advance time to midpoint (t = 600)
    env.ledger().set_timestamp(600);
    assert_eq!(client.vested(&stream_id), 500);
    assert_eq!(client.withdrawable(&stream_id), 500);
    assert_eq!(client.locked(&stream_id), 500);
    assert_eq!(client.progress(&stream_id), 5_000);

    // 3. Partial Withdrawal
    let withdrawn_amt = client.withdraw_amount(&stream_id, &200);
    assert_eq!(withdrawn_amt, 200);
    assert_eq!(token.balance(&recipient), 200);
    assert_eq!(client.withdrawable(&stream_id), 300);

    // Full withdrawal of currently vested balance
    let withdrawn_full = client.withdraw(&stream_id);
    assert_eq!(withdrawn_full, 300);
    assert_eq!(token.balance(&recipient), 500);
    assert_eq!(client.withdrawable(&stream_id), 0);
    assert_eq!(token.balance(&contract_id), 500);

    // 4. Advance time to t = 700 and check cancel preview
    env.ledger().set_timestamp(700);
    // At t = 700: 600 vested, 500 withdrawn -> 100 recipient claimable, 400 sender refund
    let preview = client.cancel_preview(&stream_id);
    assert_eq!(preview.recipient_remaining, 100);
    assert_eq!(preview.refund, 400);

    // 5. Cancel stream at t = 700
    let refund = client.cancel(&stream_id);
    assert_eq!(refund, 400);
    assert_eq!(token.balance(&sender), 9_400);
    assert_eq!(client.status(&stream_id), StreamStatus::Cancelled);
    assert_eq!(client.locked(&stream_id), 0);

    // 6. Recipient claims remaining unwithdrawn vested balance after cancellation
    let final_withdraw = client.withdraw(&stream_id);
    assert_eq!(final_withdraw, 100);
    assert_eq!(token.balance(&recipient), 600);
    assert_eq!(token.balance(&contract_id), 0);
    assert_eq!(client.withdrawable(&stream_id), 0);
}

#[test]
fn compiled_wasm_stream_completion_lifecycle() {
    let wasm_bytes = match load_wasm_bytes_or_skip() {
        Some(bytes) => bytes,
        None => return,
    };

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(100);

    let contract_id = env.register(wasm_bytes.as_slice(), ());
    let client = StreamContractClient::new(&env, &contract_id);

    let issuer = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(issuer);
    let token_address = sac.address();
    let token = token::TokenClient::new(&env, &token_address);
    let token_admin = token::StellarAssetClient::new(&env, &token_address);

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    token_admin.mint(&sender, &5_000);

    // Create stream with cliff: start = 100, end = 500, cliff = 300, total = 2000
    let stream_id = client.create_stream(
        &sender,
        &recipient,
        &token_address,
        &2_000,
        &100,
        &500,
        &300,
    );
    assert_eq!(stream_id, 0);
    assert_eq!(client.status(&stream_id), StreamStatus::Streaming);

    // Before start (t = 50): status is Pending
    env.ledger().set_timestamp(50);
    assert_eq!(client.status(&stream_id), StreamStatus::Pending);
    assert_eq!(client.vested(&stream_id), 0);
    assert_eq!(client.withdrawable(&stream_id), 0);

    // Before cliff (t = 200): 0 vested, 0 withdrawable, status is Streaming
    env.ledger().set_timestamp(200);
    assert_eq!(client.status(&stream_id), StreamStatus::Streaming);
    assert_eq!(client.vested(&stream_id), 0);
    assert_eq!(client.withdrawable(&stream_id), 0);
    assert_eq!(client.progress(&stream_id), 0);

    // Exactly at cliff (t = 300): 50% vested (1000)
    env.ledger().set_timestamp(300);
    assert_eq!(client.status(&stream_id), StreamStatus::Streaming);
    assert_eq!(client.vested(&stream_id), 1_000);
    assert_eq!(client.withdrawable(&stream_id), 1_000);
    assert_eq!(client.progress(&stream_id), 5_000);

    // Advance to stream end (t = 500): fully vested (2000), status becomes Completed
    env.ledger().set_timestamp(500);
    assert_eq!(client.vested(&stream_id), 2_000);
    assert_eq!(client.withdrawable(&stream_id), 2_000);
    assert_eq!(client.progress(&stream_id), 10_000);
    assert_eq!(client.status(&stream_id), StreamStatus::Completed);

    // Attempting to cancel a completed stream fails with StreamAlreadyCompleted
    assert_eq!(
        client.try_cancel(&stream_id),
        Err(Ok(StreamError::StreamAlreadyCompleted))
    );

    // Withdraw entire stream amount
    let withdrawn = client.withdraw(&stream_id);
    assert_eq!(withdrawn, 2_000);
    assert_eq!(token.balance(&recipient), 2_000);
    assert_eq!(token.balance(&contract_id), 0);
    assert_eq!(client.withdrawable(&stream_id), 0);
    assert_eq!(client.locked(&stream_id), 0);
    assert_eq!(client.status(&stream_id), StreamStatus::Completed);

    // Subsequent withdrawal returns NothingToWithdraw
    assert_eq!(
        client.try_withdraw(&stream_id),
        Err(Ok(StreamError::NothingToWithdraw))
    );
}
