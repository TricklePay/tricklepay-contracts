#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::contract::{StreamContract, StreamContractClient};

/// A fully wired test environment: a registered stream contract, a token to
/// stream, and helpers to fund accounts and move the ledger clock.
pub struct StreamTest<'a> {
    pub env: Env,
    pub contract: StreamContractClient<'a>,
    pub token: token::TokenClient<'a>,
    pub token_admin: token::StellarAssetClient<'a>,
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
            token_admin,
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
