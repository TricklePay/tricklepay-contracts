#![cfg(test)]

use soroban_sdk::{
    testutils::{
        storage::{Instance as _, Persistent as _},
        Address as _, Events as _, Ledger as _,
    },
    token, vec, xdr, Address, Env, TryFromVal, Vec,
};

use crate::contract::{StreamContract, StreamContractClient};
use crate::storage::{self, DataKey};
use crate::Stream;

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

    /// Attempt to create a stream with explicit participant and token
    /// overrides, using the standard schedule `[100, 1100]` with no cliff and
    /// `amount`. Returns `true` if the call was rejected, `false` if it
    /// succeeded.
    pub fn try_create_stream_for_raw(
        &self,
        sender: &Address,
        recipient: &Address,
        token: &Address,
        amount: i128,
    ) -> bool {
        let res = self
            .contract
            .try_create_stream(sender, recipient, token, &amount, &100, &1_100, &100);
        match res {
            Ok(Ok(_)) => false,
            Ok(Err(_)) => true,
            Err(_) => true,
        }
    }

    /// The addresses that published the events of the latest invocation, in
    /// publication order.
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
    }

    /// Whether a persistent entry exists under `key`.
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
}
