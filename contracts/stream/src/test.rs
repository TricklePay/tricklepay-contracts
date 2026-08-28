#![cfg(test)]

use soroban_sdk::{
    testutils::{
        storage::{Instance as _, Persistent as _},
        Address as _, Events as _, Ledger as _,
    },
    token, vec, Address, Env, Vec,
};

use crate::contract::{StreamContract, StreamContractClient};
use crate::storage::{self, DataKey, BUMP_THRESHOLD, ENTRY_TTL};
use crate::{Stream, StreamError, StreamStatus, MAX_AMOUNT};

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
    fn open_default_stream(&self, amount: i128) -> u64 {
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
    /// `amount`. The raw helper returns `true` if the call was rejected and
    /// `false` if it succeeded — this keeps tests resilient to SDK client
    /// return-shape changes.
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

    /// How many events have been published so far. Tests take this reading
    /// before an operation so they can look at only the events that operation
    /// added, rather than everything the fixture emitted while being built.
    pub fn event_count(&self) -> u32 {
        self.env.events().all().len()
    }

    /// The address that published each event after the first `from`, in
    /// publication order.
    ///
    /// Ordering is asserted through the publisher rather than the payload
    /// because that is exactly what distinguishes "the tokens moved, then the
    /// contract announced it" from "the contract announced it, then the tokens
    /// moved": a token transfer is published by the token contract, and the
    /// stream's own `Created` / `Withdrawn` / `Cancelled` events are published
    /// by the stream contract.
    pub fn event_publishers_since(&self, from: u32) -> Vec<Address> {
        let mut publishers = Vec::new(&self.env);
        for (publisher, _, _) in self.env.events().all().iter().skip(from as usize) {
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
            self.env.storage().persistent().get_ttl(&DataKey::Stream(id))
        })
    }
}

#[test]
fn create_stream_locks_funds_and_assigns_id() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    // start == cliff means the stream has no cliff.
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

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
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

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
fn withdraw_at_exact_end() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    // Move the clock to exactly end_time. The full amount must be
    // withdrawable and withdraw() must return the whole remaining balance.
    t.set_time(1_100);
    assert_eq!(t.contract.withdrawable(&id), 1_000);
    let withdrawn = t.contract.withdraw(&id);
    assert_eq!(withdrawn, 1_000);
    assert_eq!(t.token.balance(&t.recipient), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);
    // After draining, nothing more is withdrawable.
    assert_eq!(t.contract.withdrawable(&id), 0);
}

#[test]
fn progress_reports_basis_points() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    // Nothing vested at the start.
    assert_eq!(t.contract.progress(&id), 0);
    // Halfway is 50 percent, in basis points.
    t.set_time(600);
    assert_eq!(t.contract.progress(&id), 5_000);
    // Fully vested at the end.
    t.set_time(1_100);
    assert_eq!(t.contract.progress(&id), 10_000);
}

#[test]
fn locked_decreases_as_the_stream_vests() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    // At the start the whole amount is locked.
    assert_eq!(t.contract.locked(&id), 1_000);
    // Halfway, half is locked.
    t.set_time(600);
    assert_eq!(t.contract.locked(&id), 500);
    // At the end, nothing is locked.
    t.set_time(1_100);
    assert_eq!(t.contract.locked(&id), 0);
}

#[test]
fn withdraw_amount_takes_a_partial_balance() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    // Midpoint: 500 vested. Take only 200 of it.
    t.set_time(600);
    assert_eq!(t.contract.withdraw_amount(&id, &200), 200);
    assert_eq!(t.token.balance(&t.recipient), 200);
    // 300 of the vested 500 is still available.
    assert_eq!(t.contract.withdrawable(&id), 300);

    // Taking more than is available is rejected.
    assert_eq!(
        t.contract.try_withdraw_amount(&id, &400),
        Err(Ok(StreamError::InsufficientBalance))
    );

    // A non-positive amount is rejected.
    assert_eq!(
        t.contract.try_withdraw_amount(&id, &0),
        Err(Ok(StreamError::InvalidAmount))
    );
}

#[test]
fn withdraw_amount_exactly_available_balance_succeeds() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    t.set_time(600);
    let available = t.contract.withdrawable(&id);
    assert_eq!(available, 500);

    assert_eq!(t.contract.withdraw_amount(&id, &available), available);
    assert_eq!(t.token.balance(&t.recipient), available);
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(t.contract.get_stream(&id).withdrawn, available);
}

#[test]
fn withdraw_amount_available_plus_one_receives_insufficient_balance() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    t.set_time(600);
    let available = t.contract.withdrawable(&id);
    assert_eq!(available, 500);

    assert_eq!(
        t.contract.try_withdraw_amount(&id, &(available + 1)),
        Err(Ok(StreamError::InsufficientBalance))
    );
    assert_eq!(t.token.balance(&t.recipient), 0);
    assert_eq!(t.contract.withdrawable(&id), available);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 0);
}

#[test]
fn cliff_blocks_withdrawal_until_reached() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    // Cliff sits at the midpoint of the stream.
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &600,
    );

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
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

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

// ── Post-cancellation view correctness ──────────────────────────────────────

/// `cancel` rewrites `total_amount`, `start_time`, `cliff_time`, and
/// `end_time`. The doc comments on `locked` and `progress` make specific
/// claims about the values a cancelled stream should report (0 and 10 000
/// respectively). This test verifies those claims, along with `status` and
/// `withdrawable`, immediately after cancellation and before the recipient
/// has touched their remaining balance.
#[test]
fn views_are_correct_on_a_cancelled_stream() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    // Cancel at the exact midpoint: 500 has vested, 500 is still locked.
    t.set_time(600);
    t.contract.cancel(&id);

    // locked() must be 0 after cancellation — the doc comment guarantees it.
    // cancel() freezes total_amount at the vested amount, so total - vested == 0.
    assert_eq!(t.contract.locked(&id), 0);

    // progress() must be 10 000 after cancellation — the doc comment guarantees
    // it. The stream is considered fully vested relative to its frozen total.
    assert_eq!(t.contract.progress(&id), 10_000);

    // status() must report Cancelled.
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);

    // withdrawable() must equal the vested-but-not-yet-taken balance: the
    // recipient cancelled at the midpoint and has withdrawn nothing, so 500
    // is still available.
    assert_eq!(t.contract.withdrawable(&id), 500);
}

/// Same four view assertions, but run again after the recipient has drained
/// the remaining vested balance. Once the recipient withdraws, withdrawable
/// must fall to 0 and the other views must stay stable. This also confirms
/// the token balances add up and a second withdraw is rejected.
#[test]
fn views_remain_correct_after_recipient_drains_cancelled_stream() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    // Cancel at the midpoint and then advance time well past the original end
    // to confirm the frozen state does not change with the clock.
    t.set_time(600);
    t.contract.cancel(&id);
    t.set_time(2_000);

    // Recipient drains their share.
    let withdrawn = t.contract.withdraw(&id);
    assert_eq!(withdrawn, 500);

    // Token balances add up to the original total — nothing was lost.
    assert_eq!(t.token.balance(&t.sender), 500);
    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&t.contract.address), 0);

    // Views must remain consistent after the drain.
    assert_eq!(t.contract.locked(&id), 0);
    assert_eq!(t.contract.progress(&id), 10_000);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);

    // withdrawable() must now be 0 — the recipient took everything.
    assert_eq!(t.contract.withdrawable(&id), 0);

    // A second withdraw attempt must be rejected.
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );
}

#[test]
fn withdraw_requires_recipient_authorization() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

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
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    t.set_time(600);
    t.contract.cancel(&id);

    // Only the sender can cancel and reclaim the unvested remainder.
    let auths = t.env.auths();
    assert!(auths.iter().any(|(addr, _)| addr == &t.sender));
}

#[test]
fn create_stream_rejects_invalid_parameters() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let zero_amount = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &0,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(zero_amount, Err(Ok(StreamError::InvalidAmount)));

    let negative_amount = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &-5,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(negative_amount, Err(Ok(StreamError::InvalidAmount)));

    // Start is not strictly before end.
    let bad_range = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &1_100,
        &1_100,
        &1_100,
    );
    assert_eq!(bad_range, Err(Ok(StreamError::InvalidTimeRange)));

    // Cliff before the start.
    let cliff_early = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &50,
    );
    assert_eq!(cliff_early, Err(Ok(StreamError::InvalidCliff)));

    // Cliff after the end.
    let cliff_late = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &1_200,
    );
    assert_eq!(cliff_late, Err(Ok(StreamError::InvalidCliff)));

    // None of the rejected calls created state or moved funds.
    assert_eq!(t.contract.stream_count(), 0);
    assert_eq!(t.token.balance(&t.sender), 1_000);
}

#[test]
fn cancel_on_stream_at_end_time_is_rejected() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    // Move the clock to exactly end_time — the stream is now fully vested.
    t.set_time(1_100);
    assert_eq!(t.contract.status(&id), StreamStatus::Completed);

    // Cancel must be rejected.
    assert_eq!(
        t.contract.try_cancel(&id),
        Err(Ok(StreamError::StreamAlreadyCompleted))
    );

    // The stream still reports Completed, not Cancelled.
    assert_eq!(t.contract.status(&id), StreamStatus::Completed);

    // No tokens moved back to the sender — the contract still holds them.
    assert_eq!(t.token.balance(&t.sender), 0);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);
}

#[test]
fn cancel_past_end_time_is_rejected_and_status_stays_completed() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    // Move well past the end of the stream.
    t.set_time(5_000);
    assert_eq!(t.contract.status(&id), StreamStatus::Completed);

    // Cancel must be rejected regardless of how far past end_time we are.
    assert_eq!(
        t.contract.try_cancel(&id),
        Err(Ok(StreamError::StreamAlreadyCompleted))
    );

    // Status is unchanged — the stream is still Completed, not Cancelled.
    assert_eq!(t.contract.status(&id), StreamStatus::Completed);

    // The recipient can still withdraw the full amount.
    assert_eq!(t.contract.withdrawable(&id), 1_000);
    assert_eq!(t.contract.withdraw(&id), 1_000);
    assert_eq!(t.token.balance(&t.recipient), 1_000);
}

#[test]
fn second_withdraw_without_progress_is_rejected() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );

    t.set_time(600);
    assert_eq!(t.contract.withdraw(&id), 500);

    // Withdrawing again with no time elapsed releases nothing.
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );
    assert_eq!(t.token.balance(&t.recipient), 500);
}

#[test]
fn operations_on_unknown_stream_report_not_found() {
    let t = StreamTest::setup(1_000);

    assert_eq!(
        t.contract.try_get_stream(&99),
        Err(Ok(StreamError::StreamNotFound))
    );
    assert_eq!(
        t.contract.try_withdraw(&99),
        Err(Ok(StreamError::StreamNotFound))
    );
    assert_eq!(
        t.contract.try_cancel(&99),
        Err(Ok(StreamError::StreamNotFound))
    );
    assert_eq!(
        t.contract.try_withdrawable(&99),
        Err(Ok(StreamError::StreamNotFound))
    );
}

#[test]
fn create_stream_extends_the_instance_ttl() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    // A fresh instance only gets the ledger's minimum lifetime, which is far
    // shorter than the window stream entries are given.
    let default_ttl = t.instance_ttl();
    assert!(
        default_ttl < ENTRY_TTL,
        "expected the default instance TTL to be shorter than ENTRY_TTL"
    );

    t.open_default_stream(1_000);

    // Creating a stream lifts the instance to the same window its streams get,
    // so the counter cannot be archived out from under streams that outlive
    // the default lifetime.
    assert_eq!(t.instance_ttl(), ENTRY_TTL);
}

#[test]
fn stream_count_survives_a_ledger_advance_past_the_default_ttl() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    let first = t.open_default_stream(1_000);

    // Advance well past the lifetime the instance would have had without the
    // bump in `create_stream`.
    let default_ttl = t.env.ledger().get().min_persistent_entry_ttl;
    let advanced_to = default_ttl * 2;
    t.set_sequence(advanced_to);

    // The instance is still carrying the window `create_stream` granted it,
    // less the ledgers that have elapsed since.
    //
    // This has to be an exact check rather than "is there any life left". The
    // in-memory test host silently restores an expired persistent entry on
    // access instead of archiving it, so without the bump the counter would
    // still answer and the instance would still report a non-zero TTL — just
    // the bare minimum the restore grants, not the window streams get.
    assert_eq!(t.instance_ttl(), ENTRY_TTL - advanced_to);
    assert_eq!(t.contract.stream_count(), 1);

    // Ids keep marching from where they left off rather than restarting and
    // colliding with the stream already in storage.
    let second = t.open_default_stream(1_000);
    assert_eq!(second, first + 1);
    assert_eq!(t.contract.stream_count(), 2);
    assert_eq!(t.contract.get_stream(&first).total_amount, 1_000);
}

// ── Overflow-guard / MAX_AMOUNT boundary tests ──────────────────────────────

/// `create_stream` must reject `total_amount == MAX_AMOUNT + 1` with
/// `AmountTooLarge`. This is the boundary value: one above the cap.
#[test]
fn create_stream_rejects_amount_above_max() {
    // Mint enough so the token transfer is not the thing that fails.
    let t = StreamTest::setup(MAX_AMOUNT + 1);
    t.set_time(100);

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &(MAX_AMOUNT + 1),
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::AmountTooLarge)));

    // No stream was created and no funds left the sender.
    assert_eq!(t.contract.stream_count(), 0);
    assert_eq!(t.token.balance(&t.sender), MAX_AMOUNT + 1);
}

/// `create_stream` must accept exactly `MAX_AMOUNT` — the boundary is
/// inclusive and the stream must work end-to-end without overflow.
#[test]
fn create_stream_accepts_max_amount() {
    let t = StreamTest::setup(MAX_AMOUNT);
    t.set_time(100);

    // A one-second stream maximises elapsed/duration pressure.
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &MAX_AMOUNT,
        &100,
        &101,
        &100,
    );

    // At the midpoint (t == 100, i.e. 0 elapsed out of 1 second) nothing
    // has vested yet.
    assert_eq!(t.contract.withdrawable(&id), 0);

    // At end_time the full amount is vested and withdrawable without panic.
    t.set_time(101);
    assert_eq!(t.contract.withdrawable(&id), MAX_AMOUNT);
    assert_eq!(t.contract.withdraw(&id), MAX_AMOUNT);
    assert_eq!(t.token.balance(&t.recipient), MAX_AMOUNT);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// `i128::MAX` is well above `MAX_AMOUNT` and must be rejected.
#[test]
fn create_stream_rejects_i128_max() {
    // We cannot actually mint i128::MAX tokens (the token contract would
    // reject it), so we only check that *our* guard fires before the
    // transfer is attempted. Use try_create_stream to observe the error.
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &i128::MAX,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::AmountTooLarge)));
}

/// A long-lived stream (duration close to u64::MAX) with an amount at the
/// cap must compute vested amounts without overflow at any point in time.
/// We sample a handful of checkpoints to exercise the multiplication.
#[test]
fn vesting_with_max_amount_over_long_duration_does_not_overflow() {
    // Use a very long stream: 0 to u64::MAX/2 to keep timestamps representable.
    let duration: u64 = u64::MAX / 2;
    let start: u64 = 0;
    let end: u64 = duration;

    let t = StreamTest::setup(MAX_AMOUNT);
    t.set_time(start);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &MAX_AMOUNT,
        &start,
        &end,
        &start,
    );

    // Quarter-point
    t.set_time(duration / 4);
    let q = t.contract.vested(&id);
    assert!(
        q > 0 && q < MAX_AMOUNT,
        "quarter-point vested={q} out of range"
    );

    // Midpoint
    t.set_time(duration / 2);
    let half = t.contract.vested(&id);
    assert!(half > q, "midpoint must exceed quarter-point");

    // At end_time the full amount vests.
    t.set_time(end);
    assert_eq!(t.contract.vested(&id), MAX_AMOUNT);
}

// ── Past time-window rejection (issue #10) ──────────────────────────────────

/// A stream whose `end_time` is strictly before the current ledger time is
/// entirely in the past and would be 100 % vested on creation. That is
/// effectively an immediate transfer disguised as a stream, so it must be
/// rejected with `StreamWindowInPast`.
#[test]
fn create_stream_rejects_end_time_in_the_past() {
    let t = StreamTest::setup(1_000);
    t.set_time(1_000); // clock is now at t=1000

    // Window [100, 900] ended 100 seconds ago.
    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &900,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::StreamWindowInPast)));

    // No stream was created and no funds left the sender.
    assert_eq!(t.contract.stream_count(), 0);
    assert_eq!(t.token.balance(&t.sender), 1_000);
}

/// A stream whose `end_time` equals the current ledger timestamp is also
/// 100 % vested the instant it would be created, so it must be rejected too.
#[test]
fn create_stream_rejects_end_time_equal_to_now() {
    let t = StreamTest::setup(1_000);
    t.set_time(1_000); // clock is now at t=1000

    // Window [100, 1000] — end_time == now, fully vested immediately.
    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_000,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::StreamWindowInPast)));

    assert_eq!(t.contract.stream_count(), 0);
    assert_eq!(t.token.balance(&t.sender), 1_000);
}

/// A stream whose `start_time` is in the past but whose `end_time` is still
/// in the future is a valid backdated schedule and must be accepted. This
/// pattern is legitimate for, e.g., payroll that should have started last
/// month: the employee immediately accrues the already-elapsed portion.
#[test]
fn create_stream_accepts_past_start_time_with_future_end_time() {
    let t = StreamTest::setup(1_000);
    t.set_time(600); // clock is at t=600, midway through [100, 1100]

    // start_time is in the past, end_time is in the future — this is fine.
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,   // 500 seconds ago
        &1_100, // 500 seconds from now
        &100,
    );

    // Stream was created; funds are in the contract.
    assert_eq!(t.contract.stream_count(), 1);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);

    // The recipient can immediately withdraw the already-elapsed half.
    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.withdraw(&id), 500);
    assert_eq!(t.token.balance(&t.recipient), 500);
}

/// `end_time` one second in the future is the tightest valid window. The
/// stream must be accepted and its single vesting tick must settle correctly.
#[test]
fn create_stream_accepts_end_time_one_second_in_the_future() {
    let t = StreamTest::setup(1_000);
    t.set_time(1_000);

    // end_time == now + 1: just barely valid.
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &999,
        &1_001,
        &999,
    );

    assert_eq!(t.contract.stream_count(), 1);

    // Advance to end_time; the full amount must be withdrawable.
    t.set_time(1_001);
    assert_eq!(t.contract.withdrawable(&id), 1_000);
}

/// The id counter must never wrap. At `u64::MAX` there is no id left to hand
/// out, so creation is refused outright rather than rolling over to zero and
/// overwriting the stream that already holds id 0.
#[test]
fn create_stream_rejects_an_exhausted_counter() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    t.set_stream_count(u64::MAX);

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::StreamCountExhausted)));

    // The counter is untouched and the rejection cost the sender nothing:
    // the check runs before the token transfer.
    assert_eq!(t.contract.stream_count(), u64::MAX);
    assert_eq!(t.token.balance(&t.sender), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// The last id below the ceiling is still usable, and using it takes the
/// counter to exactly `u64::MAX` — the point at which the next call must fail.
#[test]
fn create_stream_accepts_the_final_id_then_refuses_the_next() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    t.set_stream_count(u64::MAX - 1);

    // The final id is handed out normally.
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(id, u64::MAX - 1);
    assert_eq!(t.contract.stream_count(), u64::MAX);
    assert_eq!(t.contract.get_stream(&id).total_amount, 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);

    // The very next creation has nowhere left to go.
    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::StreamCountExhausted)));

    // The stream that owns the final id is intact and the second amount never
    // left the sender.
    assert_eq!(t.contract.get_stream(&id).total_amount, 1_000);
    assert_eq!(t.token.balance(&t.sender), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);
}

/// The contract's own address as recipient would lock the tokens forever:
/// `withdraw` demands the recipient's authorization and the contract cannot
/// sign for itself, so nothing could ever claim them.
#[test]
fn create_stream_rejects_the_contract_as_recipient() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let contract_address = t.contract.address.clone();

    let result = t.contract.try_create_stream(
        &t.sender,
        &contract_address,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::InvalidParticipant)));
    t.assert_nothing_happened(1_000);
}

/// The contract as sender would let a caller draw on the pooled holdings that
/// back every other stream.
#[test]
fn create_stream_rejects_the_contract_as_sender() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let contract_address = t.contract.address.clone();

    let result = t.contract.try_create_stream(
        &contract_address,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::InvalidParticipant)));
    t.assert_nothing_happened(1_000);
}

/// The contract as the token would mean calling `transfer` on this contract,
/// which exposes no such entry point. Rejecting it turns an obscure host-level
/// failure into a documented error.
#[test]
fn create_stream_rejects_the_contract_as_token() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let contract_address = t.contract.address.clone();

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &contract_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::InvalidParticipant)));
    t.assert_nothing_happened(1_000);
}

/// A token contract cannot also act as a stream participant. Using the sender or
/// recipient address as the token input creates a nonsensical stream that is
/// rejected before any fund transfer.
#[test]
fn create_stream_rejects_token_equal_to_sender_or_recipient() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.sender,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::InvalidParticipant)));
    t.assert_nothing_happened(1_000);

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.recipient,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::InvalidParticipant)));
    t.assert_nothing_happened(1_000);
}

/// A stream from an address to itself only locks the sender's own tokens and
/// hands them back over time. It is almost always a swapped or unset argument,
/// so it is refused before any tokens move.
#[test]
fn create_stream_rejects_a_stream_to_self() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let result = t.contract.try_create_stream(
        &t.sender,
        &t.sender,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(result, Err(Ok(StreamError::InvalidParticipant)));
    t.assert_nothing_happened(1_000);
}

/// When an argument list breaks more than one rule, which error comes back is
/// fixed by the documented order on `create_stream` rather than by the
/// incidental arrangement of the checks. Each case below violates two rules
/// and must report the earlier one.
#[test]
fn create_stream_validation_order_is_deterministic() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    // Participants (2) beat amount (3): self-stream with a zero amount.
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &t.sender,
            &t.token_address,
            &0,
            &100,
            &1_100,
            &100
        ),
        Err(Ok(StreamError::InvalidParticipant))
    );

    // Participants (2) beat schedule (4): the contract as recipient, with a
    // window that is also inverted.
    let contract_address = t.contract.address.clone();
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &contract_address,
            &t.token_address,
            &1_000,
            &1_100,
            &100,
            &1_100
        ),
        Err(Ok(StreamError::InvalidParticipant))
    );

    // Amount (3) beats schedule (4): zero amount with an inverted window.
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &t.recipient,
            &t.token_address,
            &0,
            &1_100,
            &100,
            &1_100
        ),
        Err(Ok(StreamError::InvalidAmount))
    );

    // Amount (3) beats capacity (5): an exhausted counter is reported only
    // once the arguments themselves are sound.
    t.set_stream_count(u64::MAX);
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &t.recipient,
            &t.token_address,
            &0,
            &100,
            &1_100,
            &100
        ),
        Err(Ok(StreamError::InvalidAmount))
    );
    // With sound arguments the same counter now surfaces.
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &t.recipient,
            &t.token_address,
            &1_000,
            &100,
            &1_100,
            &100
        ),
        Err(Ok(StreamError::StreamCountExhausted))
    );

    // Nothing above moved a token or consumed an id.
    assert_eq!(t.token.balance(&t.sender), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// Within the schedule group the order is also fixed: range, then cliff, then
/// the past-window rule.
#[test]
fn create_stream_schedule_checks_run_in_order() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    // An inverted window whose cliff is also out of bounds reports the range.
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &t.recipient,
            &t.token_address,
            &1_000,
            &1_100,
            &100,
            &50
        ),
        Err(Ok(StreamError::InvalidTimeRange))
    );

    // A cliff past the end, on a window that has also already elapsed,
    // reports the cliff.
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &t.recipient,
            &t.token_address,
            &1_000,
            &10,
            &50,
            &60
        ),
        Err(Ok(StreamError::InvalidCliff))
    );

    // With a well-formed cliff, the elapsed window is what is reported.
    assert_eq!(
        t.contract.try_create_stream(
            &t.sender,
            &t.recipient,
            &t.token_address,
            &1_000,
            &10,
            &50,
            &10
        ),
        Err(Ok(StreamError::StreamWindowInPast))
    );

    t.assert_nothing_happened(1_000);
}

// ── Timestamp boundary tests ─────────────────────────────────────────────────

/// `start_time == 0` and a future `end_time` is a valid edge case: Unix epoch
/// zero is a legal timestamp. The stream should be created, and since the
/// current ledger time is well past epoch-zero, the elapsed portion should
/// vest immediately.
#[test]
fn create_stream_accepts_start_time_of_zero() {
    let t = StreamTest::setup(1_000);
    // Ledger is at t=500, which is inside the window [0, 1000].
    t.set_time(500);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &0,     // start_time = epoch zero
        &1_000, // end_time in the future
        &0,     // cliff == start (no cliff)
    );

    // A stream was created and tokens moved to the contract.
    assert_eq!(t.contract.stream_count(), 1);
    assert_eq!(t.token.balance(&t.contract.address), 1_000);

    // At t=500 exactly half the window [0,1000] has elapsed, so 500 is vested.
    assert_eq!(t.contract.vested(&id), 500);
    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.locked(&id), 500);
}

// --- Event ordering ---------------------------------------------------------
//
// Every state-changing entry point both moves tokens and publishes an event.
// The order of those two effects is part of the contract's observable
// behaviour: an indexer that reacts to a stream event must be able to assume
// the corresponding transfer has already settled. These tests pin that order
// down so a future reshuffle of an entry point body cannot quietly invert it.

/// `create_stream` announces a stream only once the funds are in the
/// contract. The token's transfer event therefore precedes the stream's
/// `Created` event, and those two are the only events a creation publishes.
#[test]
fn create_emits_created_after_the_funding_transfer() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let before = t.event_count();

    t.open_default_stream(1_000);

    assert_eq!(t.event_publishers_since(before), t.transfer_then_announce());
}

/// `withdraw` pays the recipient before it announces, so a `Withdrawn` event
/// never describes tokens that are still sitting in the contract.
#[test]
fn withdraw_emits_withdrawn_after_the_payout_transfer() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    // Halfway through [100, 1100]: 500 has vested and is paid out.
    t.set_time(600);
    let before = t.event_count();
    assert_eq!(t.contract.withdraw(&id), 500);

    assert_eq!(t.event_publishers_since(before), t.transfer_then_announce());
}

/// `withdraw_amount` follows the same order as `withdraw`: refund first,
/// event second.
#[test]
fn withdraw_amount_emits_withdrawn_after_the_payout_transfer() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    t.set_time(600);
    let before = t.event_count();
    assert_eq!(t.contract.withdraw_amount(&id, &200), 200);

    assert_eq!(t.event_publishers_since(before), t.transfer_then_announce());
}

/// `cancel` refunds the sender before announcing the cancellation, so the
/// `Cancelled` event's `sender_refund` is already reflected in balances by
/// the time anyone observes it.
#[test]
fn cancel_emits_cancelled_after_the_refund_transfer() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    // Cancelling at the midpoint leaves a 500 refund, so a transfer does occur.
    t.set_time(600);
    let before = t.event_count();
    assert_eq!(t.contract.cancel(&id), 500);

    assert_eq!(t.event_publishers_since(before), t.transfer_then_announce());
}

/// Over a full lifecycle the contract's events arrive in operation order —
/// create, withdraw, cancel — and each one trails the transfer it describes.
#[test]
fn lifecycle_events_follow_operation_order() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let before = t.event_count();

    let id = t.open_default_stream(1_000);
    t.set_time(600);
    t.contract.withdraw(&id);
    t.set_time(700);
    t.contract.cancel(&id);

    // Three operations, each contributing a transfer followed by the stream
    // contract's own event.
    assert_eq!(
        t.event_publishers_since(before),
        vec![
            &t.env,
            t.token_address.clone(),
            t.contract.address.clone(),
            t.token_address.clone(),
            t.contract.address.clone(),
            t.token_address.clone(),
            t.contract.address.clone(),
        ]
    );
}

/// A creation rejected for an invalid participant publishes nothing: the
/// participant checks run before any token moves, so there is neither a
/// transfer event nor a `Created` event for an indexer to unwind.
#[test]
fn rejected_create_publishes_no_events() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let before = t.event_count();

    // sender == recipient is refused by the first validation step.
    let sender = t.sender.clone();
    assert!(t.try_create_stream_for_raw(&sender, &sender, &t.token_address, 1_000));

    assert_eq!(t.event_publishers_since(before), vec![&t.env]);
    t.assert_nothing_happened(1_000);
}

/// The same holds for a rejection that happens later in the validation order:
/// an exhausted id counter is caught before the transfer, so the call is
/// silent on the event stream too.
#[test]
fn rejected_create_on_exhausted_counter_publishes_no_events() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    t.set_stream_count(u64::MAX);
    let before = t.event_count();

    assert!(t.try_create_stream_for_raw(&t.sender, &t.recipient, &t.token_address, 1_000));

    assert_eq!(t.event_publishers_since(before), vec![&t.env]);
}

/// A rejected `withdraw` publishes nothing either: `NothingToWithdraw` is
/// returned before the payout, so no event describes a transfer that did not
/// happen.
#[test]
fn rejected_withdraw_publishes_no_events() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    // Still at start_time, so nothing has vested.
    let before = t.event_count();
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );

    assert_eq!(t.event_publishers_since(before), vec![&t.env]);
}

// --- Storage key encoding ---------------------------------------------------
//
// `DataKey` is the only thing standing between a stream record and the wrong
// slot in storage. `StreamCount` and `Stream(id)` must encode to different
// keys, every id must encode to its own key across the whole `u64` range, and
// the two live in different storage types. None of that is checked by the
// entry points, which report `StreamNotFound` whether a key is missing or
// merely holds something unexpected — so these tests read the raw keys.

/// Each stream id encodes to its own persistent key. Three streams written in
/// a row occupy `Stream(0)`, `Stream(1)` and `Stream(2)` and do not overwrite
/// one another; the id one past the last is absent rather than aliasing.
#[test]
fn stream_ids_map_to_distinct_persistent_keys() {
    let t = StreamTest::setup(6_000);
    t.set_time(100);

    // Distinct amounts so an aliased key shows up as a wrong value, not just a
    // wrong count.
    let first = t.open_default_stream(1_000);
    let second = t.open_default_stream(2_000);
    let third = t.open_default_stream(3_000);
    assert_eq!((first, second, third), (0, 1, 2));

    assert!(t.persistent_has(&DataKey::Stream(0)));
    assert!(t.persistent_has(&DataKey::Stream(1)));
    assert!(t.persistent_has(&DataKey::Stream(2)));
    assert!(!t.persistent_has(&DataKey::Stream(3)));

    // Each key holds its own record.
    assert_eq!(
        t.persistent_stream(&DataKey::Stream(0)).unwrap().total_amount,
        1_000
    );
    assert_eq!(
        t.persistent_stream(&DataKey::Stream(1)).unwrap().total_amount,
        2_000
    );
    assert_eq!(
        t.persistent_stream(&DataKey::Stream(2)).unwrap().total_amount,
        3_000
    );
}

/// `StreamCount` and `Stream(0)` are different keys in different storage
/// types. The counter must not be reachable as a stream, and stream zero must
/// not be reachable as the counter — a collision would have the first stream
/// silently overwrite the id sequence.
#[test]
fn stream_count_and_stream_zero_do_not_share_a_key() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    t.open_default_stream(1_000);

    // The counter lives in instance storage only.
    assert!(t.instance_has(&DataKey::StreamCount));
    assert!(!t.persistent_has(&DataKey::StreamCount));

    // The stream lives in persistent storage only.
    assert!(t.persistent_has(&DataKey::Stream(0)));
    assert!(!t.instance_has(&DataKey::Stream(0)));

    // And the counter still reads back as a count, not as a stream record.
    assert_eq!(t.contract.stream_count(), 1);
}

/// Key encoding has to hold at the top of the id range too, where a truncating
/// or wrapping encoding would be easiest to miss. A stream created at
/// `u64::MAX - 1` lands on exactly that key and nowhere near `Stream(0)`.
#[test]
fn boundary_stream_ids_encode_to_their_own_keys() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    t.set_stream_count(u64::MAX - 1);

    let id = t.open_default_stream(1_000);
    assert_eq!(id, u64::MAX - 1);

    assert!(t.persistent_has(&DataKey::Stream(u64::MAX - 1)));
    assert!(!t.persistent_has(&DataKey::Stream(u64::MAX)));
    assert!(!t.persistent_has(&DataKey::Stream(0)));

    // The record found under the boundary key is the one that was written.
    assert_eq!(
        t.persistent_stream(&DataKey::Stream(u64::MAX - 1))
            .unwrap()
            .total_amount,
        1_000
    );
}

/// A stream round-trips through its key unchanged: what `get_stream` returns
/// is byte-for-byte what sits under `Stream(id)`, so no field is lost or
/// reordered by serialization.
#[test]
fn stream_records_round_trip_under_their_key() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &200,
        &1_200,
        &400, // a real cliff, so cliff_time is not just a copy of start_time
    );

    let stored = t.persistent_stream(&DataKey::Stream(id)).unwrap();
    assert_eq!(stored, t.contract.get_stream(&id));

    // Spot-check the fields most likely to be confused with one another.
    assert_eq!(stored.start_time, 200);
    assert_eq!(stored.cliff_time, 400);
    assert_eq!(stored.end_time, 1_200);
    assert_eq!(stored.withdrawn, 0);
    assert!(!stored.cancelled);
}

/// A creation rejected because the contract's own address was passed as
/// `recipient` writes no key at all. The participant checks run before the
/// transfer and before storage, so `Stream(0)` stays empty and the id is not
/// consumed.
#[test]
fn rejected_create_writes_no_storage_key() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let contract_address = t.contract.address.clone();
    assert!(t.try_create_stream_for_raw(
        &t.sender,
        &contract_address,
        &t.token_address,
        1_000
    ));

    assert!(!t.persistent_has(&DataKey::Stream(0)));
    t.assert_nothing_happened(1_000);
}

// --- Persistent entry TTL ---------------------------------------------------
//
// Stream records live in persistent storage, which expires on a ledger clock
// rather than a wall clock. A stream that outlives its entry's time to live is
// archived and stops answering, so `storage::get_stream` and
// `storage::set_stream` bump every entry they touch back up to `ENTRY_TTL`
// once it drops below `BUMP_THRESHOLD`.
//
// These are regression tests for that bump. Note that they check the TTL
// directly rather than checking whether a stream still reads back: the
// in-memory test host silently restores an expired persistent entry on access
// instead of archiving it, so a stream with a dead TTL would still answer here
// while failing on a real network.

/// A freshly created stream starts with the full `ENTRY_TTL` window, not the
/// ledger's much shorter default for new persistent entries.
#[test]
fn create_stream_gives_the_record_the_full_entry_ttl() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let default_ttl = t.env.ledger().get().min_persistent_entry_ttl;
    assert!(
        default_ttl < ENTRY_TTL,
        "expected the default persistent entry TTL to be shorter than ENTRY_TTL"
    );

    let id = t.open_default_stream(1_000);
    assert_eq!(t.stream_ttl(id), ENTRY_TTL);
}

/// The entry's remaining life is measured in ledgers, so advancing the ledger
/// sequence draws it down one for one.
#[test]
fn a_stream_entry_ttl_decays_with_the_ledger_sequence() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    // Stay well above the bump threshold so nothing is refreshed yet.
    let elapsed = 1_000;
    t.set_sequence(elapsed);
    assert_eq!(t.stream_ttl(id), ENTRY_TTL - elapsed);
}

/// Reading a stream once its entry has fallen below `BUMP_THRESHOLD` restores
/// the full window. This is the bump that keeps a long-running stream from
/// being archived out from under its recipient.
#[test]
fn reading_a_stream_below_the_threshold_restores_the_full_ttl() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    // Advance far enough that fewer than BUMP_THRESHOLD ledgers remain.
    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);
    assert!(ENTRY_TTL - elapsed < BUMP_THRESHOLD);

    // A plain view call goes through `storage::get_stream`, which bumps.
    t.contract.get_stream(&id);
    assert_eq!(t.stream_ttl(id), ENTRY_TTL);
}

/// Above the threshold a read deliberately does nothing: an entry with plenty
/// of life left should not pay to be re-extended on every access.
#[test]
fn reading_a_stream_above_the_threshold_leaves_the_ttl_alone() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    // One ledger short of the point where the bump kicks in.
    let elapsed = ENTRY_TTL - BUMP_THRESHOLD - 1;
    t.set_sequence(elapsed);
    assert!(ENTRY_TTL - elapsed > BUMP_THRESHOLD);

    t.contract.get_stream(&id);
    assert_eq!(t.stream_ttl(id), ENTRY_TTL - elapsed);
}

/// Writing a stream refreshes it on the same schedule as reading one, so a
/// withdrawal late in a stream's life also renews the entry that records it.
#[test]
fn withdrawing_restores_a_decayed_stream_ttl() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);

    // Move the wall clock to the midpoint so there is something to withdraw.
    t.set_time(600);
    assert_eq!(t.contract.withdraw(&id), 500);

    assert_eq!(t.stream_ttl(id), ENTRY_TTL);
    // The write went to the same entry it refreshed.
    assert_eq!(t.contract.get_stream(&id).withdrawn, 500);
}

/// Cancelling refreshes the entry too. A cancelled stream still holds the
/// recipient's accrued balance, so its record has to stay alive for them to
/// come back and claim it.
#[test]
fn cancelling_restores_a_decayed_stream_ttl() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);

    t.set_time(600);
    assert_eq!(t.contract.cancel(&id), 500);

    assert_eq!(t.stream_ttl(id), ENTRY_TTL);
    assert_eq!(t.contract.withdrawable(&id), 500);
}

/// Each stream carries its own entry. Touching one does not extend another, so
/// an idle stream cannot be kept alive by traffic on a busy one — and, more to
/// the point, a bump never lands on the wrong key.
#[test]
fn bumping_one_stream_does_not_extend_another() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    let busy = t.open_default_stream(1_000);
    let idle = t.open_default_stream(1_000);

    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);

    t.contract.get_stream(&busy);

    assert_eq!(t.stream_ttl(busy), ENTRY_TTL);
    assert_eq!(t.stream_ttl(idle), ENTRY_TTL - elapsed);
}

/// A stream that is touched every time its entry nears expiry keeps rolling
/// forward indefinitely, which is what makes a stream longer than `ENTRY_TTL`
/// workable at all.
#[test]
fn repeated_bumps_keep_a_stream_entry_alive_indefinitely() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    // Three full cycles, each one advancing until the entry is nearly spent
    // and then touching it.
    let step = ENTRY_TTL - BUMP_THRESHOLD + 1;
    for cycle in 1..=3u32 {
        t.set_sequence(step * cycle);
        t.contract.get_stream(&id);
        assert_eq!(t.stream_ttl(id), ENTRY_TTL);
    }

    // Total ledgers elapsed exceed a single ENTRY_TTL window, yet the record
    // is intact.
    assert!(step * 3 > ENTRY_TTL);
    assert_eq!(t.contract.get_stream(&id).total_amount, 1_000);
}

// --- Instance TTL -----------------------------------------------------------
//
// The contract instance holds `DataKey::StreamCount`, the source of every
// stream id. Nothing bumps it as a side effect of being read, so
// `create_stream` extends it explicitly on the same schedule as stream
// entries. If that bump were dropped, an instance left untouched past its
// lifetime would be archived and take the id sequence with it — a fresh
// counter would restart at zero and the next stream would be written over
// stream 0, which still exists in persistent storage with its own longer TTL.
//
// As with the persistent-entry tests, these assert the TTL directly. The
// in-memory test host restores an archived entry on access rather than
// failing, so "the counter still answers" proves nothing on its own.

/// Creating a stream lifts the instance from the ledger's short default to the
/// same `ENTRY_TTL` window the stream record gets.
#[test]
fn create_stream_lifts_the_instance_to_the_full_entry_ttl() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let default_ttl = t.instance_ttl();
    assert!(
        default_ttl < ENTRY_TTL,
        "expected the default instance TTL to be shorter than ENTRY_TTL"
    );

    let id = t.open_default_stream(1_000);

    // The counter and the stream it numbered expire together, so neither can
    // outlive the other.
    assert_eq!(t.instance_ttl(), ENTRY_TTL);
    assert_eq!(t.stream_ttl(id), ENTRY_TTL);
}

/// The instance TTL is measured in ledgers and decays one for one, exactly
/// like a persistent entry.
#[test]
fn the_instance_ttl_decays_with_the_ledger_sequence() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    t.open_default_stream(1_000);

    let elapsed = 1_000;
    t.set_sequence(elapsed);
    assert_eq!(t.instance_ttl(), ENTRY_TTL - elapsed);
}

/// A later creation restores a decayed instance to the full window. This is
/// the bump that keeps a long-lived contract's id sequence alive.
#[test]
fn a_later_create_restores_a_decayed_instance_ttl() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    t.open_default_stream(1_000);

    // Advance until the instance is nearly spent.
    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);
    assert_eq!(t.instance_ttl(), ENTRY_TTL - elapsed);

    t.open_default_stream(1_000);
    assert_eq!(t.instance_ttl(), ENTRY_TTL);
}

/// Read-only traffic does not extend the instance. Only `create_stream` bumps
/// it, so a contract that is queried constantly but never written to still
/// runs its instance down — this pins the current behaviour so a change to it
/// is a deliberate one.
#[test]
fn view_calls_do_not_extend_the_instance_ttl() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    let id = t.open_default_stream(1_000);

    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);
    t.set_time(600);

    // Reading a stream bumps that stream's own entry...
    t.contract.get_stream(&id);
    t.contract.withdrawable(&id);
    t.contract.stream_count();
    assert_eq!(t.stream_ttl(id), ENTRY_TTL);

    // ...but leaves the instance where it was.
    assert_eq!(t.instance_ttl(), ENTRY_TTL - elapsed);
}

/// Withdrawing and cancelling do not extend the instance either. They write a
/// stream entry, not the counter, so the instance is untouched.
#[test]
fn withdraw_and_cancel_do_not_extend_the_instance_ttl() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    let first = t.open_default_stream(1_000);
    let second = t.open_default_stream(1_000);

    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);
    t.set_time(600);

    t.contract.withdraw(&first);
    t.contract.cancel(&second);

    assert_eq!(t.instance_ttl(), ENTRY_TTL - elapsed);
}

/// The instance keeps rolling forward as long as streams keep being created,
/// so a contract in continuous use never loses its id sequence no matter how
/// many `ENTRY_TTL` windows have gone by.
#[test]
fn repeated_creates_keep_the_instance_alive_indefinitely() {
    // Four streams inside the loop plus one after it.
    let t = StreamTest::setup(5_000);
    t.set_time(100);
    t.open_default_stream(1_000);

    let step = ENTRY_TTL - BUMP_THRESHOLD + 1;
    for cycle in 1..=3u32 {
        t.set_sequence(step * cycle);
        t.open_default_stream(1_000);
        assert_eq!(t.instance_ttl(), ENTRY_TTL);
    }

    // More ledgers have elapsed than a single window covers, and the counter
    // has still never restarted: ids are contiguous and none was reused.
    assert!(step * 3 > ENTRY_TTL);
    assert_eq!(t.contract.stream_count(), 4);
    assert_eq!(t.open_default_stream(1_000), 4);
}

/// The regression this bump exists to prevent: if the instance were archived
/// and its counter reset to zero, a new stream would take id 0 and overwrite
/// the record already sitting under `Stream(0)`. Ids must keep advancing past
/// a long idle gap, and the original record must survive intact.
#[test]
fn ids_never_restart_after_a_long_idle_gap() {
    let t = StreamTest::setup(2_000);
    t.set_time(100);
    let first = t.open_default_stream(1_000);
    assert_eq!(first, 0);

    // Idle for longer than the instance's default lifetime would have allowed.
    let elapsed = ENTRY_TTL - BUMP_THRESHOLD + 1;
    t.set_sequence(elapsed);

    let second = t.open_default_stream(1_000);
    assert_eq!(second, 1);
    assert_eq!(t.contract.stream_count(), 2);

    // Stream 0 was not overwritten by stream 1.
    assert!(t.persistent_has(&DataKey::Stream(0)));
    assert!(t.persistent_has(&DataKey::Stream(1)));
    assert_eq!(t.contract.get_stream(&first).total_amount, 1_000);
}
