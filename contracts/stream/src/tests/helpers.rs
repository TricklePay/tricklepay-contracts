#![cfg(test)]

use soroban_sdk::{
    auth::{Context, CustomAccountInterface},
    contract, contracterror, contractimpl,
    crypto::Hash,
    testutils::{
        storage::{Instance as _, Persistent as _},
        Address as _, Events as _, Ledger as _, MockAuth, MockAuthInvoke,
    },
    token, vec, xdr, Address, Env, TryFromVal, Val, Vec,
};

use crate::contract::{StreamContract, StreamContractClient};
use crate::storage::{self, DataKey};
use crate::{Stream, StreamError};

pub const START: u64 = 100;
pub const MIDPOINT: u64 = 600;
pub const END: u64 = 1_100;
pub const AMOUNT: i128 = 1_000;

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

    /// Move the ledger sequence to `seq`, simulating elapsed ledgers rather
    /// than elapsed wall-clock time. Entry lifetimes are counted in ledgers,
    /// so this is the clock that time to live is measured against.
    pub fn set_sequence(&self, seq: u32) {
        self.env.ledger().set_sequence_number(seq);
    }

    /// Ledgers of life remaining on the contract instance, which is where the
    /// stream id counter lives.
    pub fn instance_ttl(&self) -> u32 {
        let address = self.contract.address.clone();
        self.env
            .as_contract(&address, || self.env.storage().instance().get_ttl())
    }

    /// Force the id counter to `count`, so boundary behaviour can be reached
    /// without actually opening `u64::MAX` streams.
    pub fn set_stream_count(&self, count: u64) {
        let address = self.contract.address.clone();
        self.env
            .as_contract(&address, || storage::set_stream_count(&self.env, count));
    }

    /// Assert a rejected `create_stream` left nothing behind: no stream, no
    /// id consumed, and every token still with the sender.
    pub fn assert_nothing_happened(&self, sender_balance: i128) {
        assert_eq!(self.contract.stream_count(), 0);
        assert_eq!(self.token.balance(&self.sender), sender_balance);
        assert_eq!(self.token.balance(&self.contract.address), 0);
    }

    /// Open a stream over `[100, 1100]` with no cliff, the shape most of these
    /// tests use.
    pub fn open_default_stream(&self, amount: i128) -> u64 {
        self.contract.create_stream(
            &self.sender,
            &self.recipient,
            &self.token_address,
            &amount,
            &100,
            &1_100,
            &100,
        )
    }

    /// Exercise a raw `create_stream` call and confirm the contract rejects it
    /// without panicking. The helper is intentionally written to return a bool
    /// so tests can assert on the failed invocation as a smoke test before
    /// checking the exact contract error separately.
    pub fn try_create_stream_for_raw(
        &self,
        sender: &Address,
        recipient: &Address,
        token: &Address,
        amount: i128,
    ) -> bool {
        self.contract
            .try_create_stream(sender, recipient, token, &amount, &100, &1_100, &100)
            .is_err()
    }

    /// Assert that creating a stream with explicit participant and token
    /// overrides, using the standard schedule `[100, 1100]` with no cliff and
    /// `amount`, fails with the expected contract error.
    pub fn assert_create_stream_error(
        &self,
        sender: &Address,
        recipient: &Address,
        token: &Address,
        amount: i128,
        expected: StreamError,
    ) {
        let res = self
            .contract
            .try_create_stream(sender, recipient, token, &amount, &100, &1_100, &100);
        assert_eq!(res, Err(Ok(expected)));
    }

    /// The addresses that published the events of the latest invocation, in
    /// publication order.
    ///
    /// In soroban-sdk 25 `events().all()` reports only the most recent
    /// invocation, so running this right after an operation yields exactly the
    /// events that operation published rather than everything the fixture
    /// emitted while being built.
    ///
    /// Ordering is asserted through the publisher rather than the payload
    /// because that is exactly what distinguishes "the tokens moved, then the
    /// contract announced it" from "the contract announced it, then the tokens
    /// moved": a token transfer is published by the token contract, and the
    /// stream's own `Created` / `Withdrawn` / `Cancelled` events are published
    /// by the stream contract.
    pub fn event_publishers(&self) -> Vec<Address> {
        let mut publishers = Vec::new(&self.env);
        for event in self.env.events().all().events() {
            let Some(contract_id) = event.contract_id.clone() else {
                continue;
            };
            let publisher =
                Address::try_from_val(&self.env, &xdr::ScAddress::Contract(contract_id)).unwrap();
            publishers.push_back(publisher);
        }
        publishers
    }

    /// The publisher sequence a single stream operation is expected to leave
    /// behind: the token contract's transfer event, then the stream
    /// contract's own event describing it.
    pub fn transfer_then_announce(&self) -> Vec<Address> {
        vec![
            &self.env,
            self.token_address.clone(),
            self.contract.address.clone(),
        ]
    }

    /// Assert the latest stream event carries the expected topic list.
    pub fn assert_latest_stream_event_topics(&self, expected: xdr::ContractEvent) {
        let all_events = self.env.events().all();
        let latest = all_events.events().last().unwrap();
        let xdr::ContractEventBody::V0(latest_body) = &latest.body;
        let xdr::ContractEventBody::V0(expected_body) = &expected.body;
        assert_eq!(latest.ext, expected.ext);
        assert_eq!(latest.type_, expected.type_);
        assert_eq!(latest_body.topics, expected_body.topics);
        assert_eq!(latest_body.data, expected_body.data);
    }

    /// Assert the latest invocation did not publish a stream event with the
    /// given topic list.
    pub fn assert_no_stream_event_topics(&self, unexpected: xdr::ContractEvent) {
        let all_events = self.env.events().all();
        let xdr::ContractEventBody::V0(unexpected_body) = &unexpected.body;

        for event in all_events.events() {
            let xdr::ContractEventBody::V0(event_body) = &event.body;
            assert!(
                !(event.ext == unexpected.ext
                    && event.contract_id == unexpected.contract_id
                    && event.type_ == unexpected.type_
                    && event_body.topics == unexpected_body.topics),
                "unexpected stream event topics were published"
            );
        }
    }

    /// Whether a persistent entry exists under `key`, read straight out of the
    /// contract's storage rather than through an entry point. Entry points
    /// answer `StreamNotFound` for both "no such key" and "key holds
    /// something unexpected", so key-level questions have to be asked here.
    pub fn persistent_has(&self, key: &DataKey) -> bool {
        let address = self.contract.address.clone();
        self.env
            .as_contract(&address, || self.env.storage().persistent().has(key))
    }

    /// The stream stored under `key`, if the key holds one.
    pub fn persistent_stream(&self, key: &DataKey) -> Option<Stream> {
        let address = self.contract.address.clone();
        self.env
            .as_contract(&address, || self.env.storage().persistent().get(key))
    }

    /// Whether an instance entry exists under `key`.
    pub fn instance_has(&self, key: &DataKey) -> bool {
        let address = self.contract.address.clone();
        self.env
            .as_contract(&address, || self.env.storage().instance().has(key))
    }

    /// Ledgers of life remaining on the persistent entry holding stream `id`.
    pub fn stream_ttl(&self, id: u64) -> u32 {
        let address = self.contract.address.clone();
        self.env.as_contract(&address, || {
            self.env
                .storage()
                .persistent()
                .get_ttl(&DataKey::Stream(id))
        })
    }

    /// Construct a [`xdr::SorobanAuthorizationEntry`] for the given address and invocation
    /// without registering [`soroban_sdk::testutils::MockAuthContract`] at the address,
    /// allowing custom account contracts to retain their implementation of `__check_auth`.
    pub fn auth_entry(
        &self,
        address: &Address,
        invoke: &MockAuthInvoke,
    ) -> xdr::SorobanAuthorizationEntry {
        let mock_auth = MockAuth { address, invoke };
        (&mock_auth).into()
    }
}

/// Error returned when a custom account contract refuses authorization.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CustomAccountError {
    Denied = 1,
}

/// A minimal smart-wallet / custom account contract that implements [`CustomAccountInterface`].
///
/// It can be dynamically toggled to approve or deny authorization checks via `set_allowed`.
#[contract]
pub struct CustomAccountContract;

#[contractimpl]
impl CustomAccountContract {
    pub fn set_allowed(env: Env, allowed: bool) {
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("allow"), &allowed);
    }
}

#[contractimpl]
impl CustomAccountInterface for CustomAccountContract {
    type Signature = Val;
    type Error = CustomAccountError;

    fn __check_auth(
        env: Env,
        _signature_payload: Hash<32>,
        _signatures: Val,
        _auth_contexts: Vec<Context>,
    ) -> Result<(), CustomAccountError> {
        let allowed: bool = env
            .storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("allow"))
            .unwrap_or(true);
        if allowed {
            Ok(())
        } else {
            Err(CustomAccountError::Denied)
        }
    }
}

/// A mock token contract that allows configuring custom `decimals()` and records whether
/// `decimals()` was ever called during contract execution.
#[contract]
pub struct MockTokenWithDecimals;

#[contractimpl]
impl MockTokenWithDecimals {
    pub fn initialize(env: Env, decimals: u32) {
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("dec"), &decimals);
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("read"), &false);
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("read"), &true);
        env.storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("dec"))
            .unwrap_or(7)
    }

    pub fn decimals_were_read(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("read"))
            .unwrap_or(false)
    }

    pub fn reset_decimals_read(env: Env) {
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("read"), &false);
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage().persistent().get(&id).unwrap_or(0)
    }

    pub fn mint(env: Env, to: Address, amount: i128) {
        let bal: i128 = env.storage().persistent().get(&to).unwrap_or(0);
        env.storage().persistent().set(&to, &(bal + amount));
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        let from_bal: i128 = env.storage().persistent().get(&from).unwrap_or(0);
        assert!(from_bal >= amount, "insufficient balance");
        env.storage().persistent().set(&from, &(from_bal - amount));

        let to_bal: i128 = env.storage().persistent().get(&to).unwrap_or(0);
        env.storage().persistent().set(&to, &(to_bal + amount));
    }
}

/// A mock token contract that withholds a fee on `transfer`, delivering less to the
/// recipient than was deducted from the sender.
///
/// This models a non-conforming "fee-on-transfer" token to test and pin the stream
/// contract's behavior when deposits deliver less than `total_amount`.
#[contract]
pub struct FeeOnTransferToken;

#[contractimpl]
impl FeeOnTransferToken {
    /// Initialize the token with a fee in basis points (e.g. 1000 = 10%) and an optional fee collector address.
    pub fn initialize(env: Env, fee_bps: u32, fee_collector: Option<Address>) {
        env.storage()
            .instance()
            .set(&soroban_sdk::symbol_short!("fee_bps"), &fee_bps);
        if let Some(ref collector) = fee_collector {
            env.storage()
                .instance()
                .set(&soroban_sdk::symbol_short!("fee_rec"), collector);
        }
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage().persistent().get(&id).unwrap_or(0)
    }

    pub fn mint(env: Env, to: Address, amount: i128) {
        let bal: i128 = env.storage().persistent().get(&to).unwrap_or(0);
        env.storage().persistent().set(&to, &(bal + amount));
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        let from_bal: i128 = env.storage().persistent().get(&from).unwrap_or(0);
        assert!(from_bal >= amount, "insufficient balance");
        env.storage().persistent().set(&from, &(from_bal - amount));

        let fee_bps: u32 = env
            .storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("fee_bps"))
            .unwrap_or(0);
        let fee_collector: Option<Address> = env
            .storage()
            .instance()
            .get(&soroban_sdk::symbol_short!("fee_rec"));

        let fee = (amount * fee_bps as i128) / 10_000;
        let delivered = amount - fee;

        if fee > 0 {
            if let Some(collector) = fee_collector {
                let col_bal: i128 = env.storage().persistent().get(&collector).unwrap_or(0);
                env.storage().persistent().set(&collector, &(col_bal + fee));
            }
        }

        let to_bal: i128 = env.storage().persistent().get(&to).unwrap_or(0);
        env.storage().persistent().set(&to, &(to_bal + delivered));
    }
}
