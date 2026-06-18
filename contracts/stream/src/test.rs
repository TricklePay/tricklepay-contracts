#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::contract::{StreamContract, StreamContractClient};
use crate::{StreamError, StreamStatus};

/// A fully wired test environment: a registered stream contract, a token to
/// stream, and helpers to fund accounts and move the ledger clock.
pub struct StreamTest<'a> {
    pub env: Env,
    pub contract: StreamContractClient<'a>,
    pub token: token::TokenClient<'a>,
    pub token_address: Address,
    pub sender: Address,
    pub recipient: Address,
}

impl<'a> StreamTest<'a> {
    /// Build a test with a fresh contract, a fresh token, and a sender funded
    /// with `sender_balance`. All authorization is mocked so calls can be made
    /// without constructing signatures.
    pub fn setup(sender_balance: i128) -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(StreamContract, ());
        let contract = StreamContractClient::new(&env, &contract_id);

        let issuer = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(issuer);
        let token_address = sac.address();
        let token = token::TokenClient::new(&env, &token_address);
        let token_admin = token::StellarAssetClient::new(&env, &token_address);

        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        token_admin.mint(&sender, &sender_balance);

        StreamTest {
            env,
            contract,
            token,
            token_address,
            sender,
            recipient,
        }
    }

    /// Set the ledger timestamp, in Unix seconds.
    pub fn set_time(&self, ts: u64) {
        self.env.ledger().set_timestamp(ts);
    }
}

#[test]
fn create_stream_locks_funds_and_assigns_id() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    // start == cliff means the stream has no cliff.
    let id = t
        .contract
        .create_stream(&t.sender, &t.recipient, &t.token_address, &1_000, &100, &1_100, &100);

    assert_eq!(id, 0);
    assert_eq!(t.contract.stream_count(), 1);

    // The full amount has moved from the sender into the contract.
    assert_eq!(t.token.balance(&t.sender), 0);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);

    let stream = t.contract.get_stream(&id);
    assert_eq!(stream.sender, t.sender);
    assert_eq!(stream.recipient, t.recipient);
    assert_eq!(stream.token, t.token_address);
    assert_eq!(stream.total_amount, 1_000);
    assert_eq!(stream.withdrawn, 0);
    assert!(!stream.cancelled);
}

#[test]
fn withdraw_releases_vested_in_steps() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t
        .contract
        .create_stream(&t.sender, &t.recipient, &t.token_address, &1_000, &100, &1_100, &100);

    // Midpoint: half has vested.
    t.set_time(600);
    assert_eq!(t.contract.withdraw(&id), 500);
    assert_eq!(t.token.balance(&t.recipient), 500);
    // Nothing more is available until the clock advances again.
    assert_eq!(t.contract.withdrawable(&id), 0);

    // Three-quarter point: another 250 has vested.
    t.set_time(850);
    assert_eq!(t.contract.withdraw(&id), 250);
    assert_eq!(t.token.balance(&t.recipient), 750);

    // End: the final 250.
    t.set_time(1_100);
    assert_eq!(t.contract.withdraw(&id), 250);
    assert_eq!(t.token.balance(&t.recipient), 1_000);

    // The contract is drained and the stream is fully settled.
    assert_eq!(t.token.balance(&t.contract.address), 0);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 1_000);
}

#[test]
fn cliff_blocks_withdrawal_until_reached() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    // Cliff sits at the midpoint of the stream.
    let id = t
        .contract
        .create_stream(&t.sender, &t.recipient, &t.token_address, &1_000, &100, &1_100, &600);

    // Before the cliff, time has passed but nothing is available.
    t.set_time(400);
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );

    // At the cliff, everything accrued since the start unlocks at once.
    t.set_time(600);
    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.withdraw(&id), 500);
    assert_eq!(t.token.balance(&t.recipient), 500);
}

#[test]
fn cancel_refunds_unvested_and_preserves_vested() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t
        .contract
        .create_stream(&t.sender, &t.recipient, &t.token_address, &1_000, &100, &1_100, &100);

    // Halfway through: 500 vested, 500 still locked.
    t.set_time(600);
    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 500);

    // The sender gets the unvested half back immediately.
    assert_eq!(t.token.balance(&t.sender), 500);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);

    // The recipient's vested half stays claimable, even much later.
    t.set_time(2_000);
    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.withdraw(&id), 500);
    assert_eq!(t.token.balance(&t.recipient), 500);

    // The split adds up to the original total and the contract is drained.
    assert_eq!(t.token.balance(&t.contract.address), 0);

    // A stream cannot be cancelled twice.
    assert_eq!(
        t.contract.try_cancel(&id),
        Err(Ok(StreamError::AlreadyCancelled))
    );
}

#[test]
fn withdraw_requires_recipient_authorization() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t
        .contract
        .create_stream(&t.sender, &t.recipient, &t.token_address, &1_000, &100, &1_100, &100);

    t.set_time(600);
    t.contract.withdraw(&id);

    // The withdraw required the recipient to authorize; no one else could
    // have pulled these funds.
    let auths = t.env.auths();
    assert!(auths.iter().any(|(addr, _)| addr == &t.recipient));
}

#[test]
fn cancel_requires_sender_authorization() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t
        .contract
        .create_stream(&t.sender, &t.recipient, &t.token_address, &1_000, &100, &1_100, &100);

    t.set_time(600);
    t.contract.cancel(&id);

    // Only the sender can cancel and reclaim the unvested remainder.
    let auths = t.env.auths();
    assert!(auths.iter().any(|(addr, _)| addr == &t.sender));
}
