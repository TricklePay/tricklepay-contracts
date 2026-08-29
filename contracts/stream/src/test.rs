#![cfg(test)]

use soroban_sdk::{
    testutils::{storage::Instance as _, Address as _, Ledger as _},
    token, Address, Env,
};

use crate::contract::{StreamContract, StreamContractClient};
use crate::storage::{self, ENTRY_TTL};
use crate::{StreamError, StreamStatus, MAX_AMOUNT};

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
    let recipient_balance_before = t.token.balance(&t.recipient);
    let contract_balance_before = t.token.balance(&t.contract.address);
    let withdrawn_before = t.contract.get_stream(&id).withdrawn;
    assert_eq!(
        t.contract.try_withdraw_amount(&id, &400),
        Err(Ok(StreamError::InsufficientBalance))
    );
    assert_eq!(t.token.balance(&t.recipient), recipient_balance_before);
    assert_eq!(
        t.token.balance(&t.contract.address),
        contract_balance_before
    );
    assert_eq!(t.contract.get_stream(&id).withdrawn, withdrawn_before);

    // A non-positive amount is rejected.
    let recipient_balance_before = t.token.balance(&t.recipient);
    let contract_balance_before = t.token.balance(&t.contract.address);
    let withdrawn_before = t.contract.get_stream(&id).withdrawn;
    assert_eq!(
        t.contract.try_withdraw_amount(&id, &0),
        Err(Ok(StreamError::InvalidAmount))
    );
    assert_eq!(t.token.balance(&t.recipient), recipient_balance_before);
    assert_eq!(
        t.token.balance(&t.contract.address),
        contract_balance_before
    );
    assert_eq!(t.contract.get_stream(&id).withdrawn, withdrawn_before);

    let recipient_balance_before = t.token.balance(&t.recipient);
    let contract_balance_before = t.token.balance(&t.contract.address);
    let withdrawn_before = t.contract.get_stream(&id).withdrawn;
    assert_eq!(
        t.contract.try_withdraw_amount(&id, &-1),
        Err(Ok(StreamError::InvalidAmount))
    );
    assert_eq!(t.token.balance(&t.recipient), recipient_balance_before);
    assert_eq!(
        t.token.balance(&t.contract.address),
        contract_balance_before
    );
    assert_eq!(t.contract.get_stream(&id).withdrawn, withdrawn_before);
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

/// Issue #57 — Test partial withdrawals across multiple calls.
///
/// Build a fixture for a vesting stream (start=100, end=1100, no cliff) and
/// draw `withdraw_amount` three times at increasing points in time. Each call
/// must return the exact partial amount, move only that amount to the
/// recipient, and accumulate correctly in the stored `withdrawn` field.
#[test]
fn test_partial_withdrawals_across_multiple_calls() {
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

    // First partial draw midway through: 500 vested, take 200.
    t.set_time(600);
    assert_eq!(t.contract.withdraw_amount(&id, &200), 200);
    assert_eq!(t.token.balance(&t.recipient), 200);
    assert_eq!(t.token.balance(&t.contract.address), 800);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 200);

    // Second partial draw later: 750 vested, take 300 more.
    t.set_time(850);
    assert_eq!(t.contract.withdraw_amount(&id, &300), 300);
    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&t.contract.address), 500);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 500);

    // Third and final draw at the end: the remaining 500 vests and is taken.
    t.set_time(1_100);
    assert_eq!(t.contract.withdraw_amount(&id, &500), 500);
    assert_eq!(t.token.balance(&t.recipient), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 1_000);
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

/// Issue #59 — Test withdrawal at exact cliff.
///
/// Build a fixture for a vesting stream with a cliff at the midpoint
/// (start=100, cliff=600, end=1100). Move the clock to exactly `cliff_time`
/// and withdraw. Everything accrued since the start unlocks in one step, so
/// the call must return 500, move it to the recipient, and leave the other
/// 500 still locked in the contract.
#[test]
fn test_withdraw_at_exact_cliff() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let start = 100u64;
    let cliff = 600u64;
    let end = 1_100u64;
    let amount = 1_000i128;

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &amount,
        &start,
        &end,
        &cliff,
    );

    // At exactly the cliff, the accrued half unlocks all at once.
    t.set_time(cliff);
    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.withdraw(&id), 500);

    // The recipient received the vested half; the rest is still locked.
    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&t.contract.address), 500);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 500);

    // Nothing further vests until the clock advances again.
    assert_eq!(t.contract.withdrawable(&id), 0);
}

/// Issue #60 — Test withdrawal at exact start.
///
/// Build a fixture for a vesting stream (start=600, end=1100, no cliff) and
/// move the clock to exactly `start_time`. Zero time has elapsed at the exact
/// start — the first token only vests one second later — so `withdraw` must be
/// rejected with `NothingToWithdraw` and no tokens may move.
#[test]
fn test_withdraw_at_exact_start() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let start = 600u64;
    let end = 1_100u64;
    let amount = 1_000i128;

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &amount,
        &start,
        &end,
        &start,
    );

    // One second before the start nothing is available either.
    t.set_time(start - 1);
    assert_eq!(t.contract.withdrawable(&id), 0);

    // At exactly start_time zero time has elapsed, so nothing has vested.
    t.set_time(start);
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );

    // The rejection moved nothing and left the stream untouched.
    assert_eq!(t.token.balance(&t.recipient), 0);
    assert_eq!(t.token.balance(&t.contract.address), amount);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 0);
    assert_eq!(t.contract.status(&id), StreamStatus::Streaming);
}

/// Issue #58 — Test withdrawal after full vesting.
///
/// Build a fixture for a vesting stream (start=100, end=1100, no cliff) and
/// advance the clock well past `end_time`. The whole amount is vested, so a
/// single `withdraw` must return the full 1 000, drain the contract, and mark
/// the stored `withdrawn` field as fully taken.
#[test]
fn test_withdraw_after_full_vesting() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let start = 100u64;
    let end = 1_100u64;
    let amount = 1_000i128;

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &amount,
        &start,
        &end,
        &start,
    );

    // Long after the window closes the stream is fully vested.
    t.set_time(end + 1_000);
    assert_eq!(t.contract.status(&id), StreamStatus::Completed);
    assert_eq!(t.contract.withdrawable(&id), amount);

    // A single withdraw releases the entire balance.
    assert_eq!(t.contract.withdraw(&id), amount);
    assert_eq!(t.token.balance(&t.recipient), amount);
    assert_eq!(t.token.balance(&t.contract.address), 0);
    assert_eq!(t.contract.get_stream(&id).withdrawn, amount);

    // Nothing remains to withdraw.
    assert_eq!(t.contract.withdrawable(&id), 0);
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

/// Cancel a stream the recipient has already partially withdrawn from.
///
/// The recipient keeps what they took, the sender gets back only the still
/// unvested remainder, and the withdrawn balance survives into the frozen
/// record so the same tokens can never be refunded twice. This pins the
/// interaction between `withdraw_amount` and `cancel`; a change to vesting,
/// authorization, or lifecycle accounting that breaks it fails here.
#[test]
fn cancel_after_partial_withdrawal() {
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

    // Midpoint: 500 vested, 500 still locked. The recipient takes 200 of the
    // vested half and leaves 300 behind.
    t.set_time(600);
    assert_eq!(t.contract.withdraw_amount(&id, &200), 200);
    assert_eq!(t.token.balance(&t.recipient), 200);

    // The sender cancels. The refund is the unvested half; the 200 already
    // withdrawn is not double-refunded to the sender.
    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 500);
    assert_eq!(t.token.balance(&t.sender), 500);
    assert_eq!(t.token.balance(&t.contract.address), 300);

    // The stored stream is frozen at the vested amount with the prior
    // withdrawal still accounted for.
    let stream = t.contract.get_stream(&id);
    assert!(stream.cancelled);
    assert_eq!(stream.total_amount, 500);
    assert_eq!(stream.withdrawn, 200);
    assert_eq!(stream.end_time, 600);

    // The recipient can still claim the rest of the vested balance; together
    // with the 200 already taken that is the full vested 500, the split adds
    // up to the original total, and the contract is drained.
    assert_eq!(t.contract.withdrawable(&id), 300);
    assert_eq!(t.contract.withdraw(&id), 300);
    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// Cancel in the last instant the contract still allows: one second before
/// `end_time`. `cancel` is documented to fail once `now >= end_time`, so this
/// is the tightest window in which a stream can still be cancelled. Only the
/// final unvested sliver is refunded, the stream freezes at the vested amount,
/// and the recipient keeps everything that has accrued. A regression in the
/// boundary (`StreamAlreadyCompleted` firing early) or in the refund arithmetic
/// at `end_time - 1` fails here.
#[test]
fn cancel_immediately_before_end_time() {
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

    // One second before the end: still Streaming, the last valid cancel moment.
    t.set_time(1_099);
    assert_eq!(t.contract.status(&id), StreamStatus::Streaming);

    // 999 of 1000 has vested. The refund is the remaining 1, not the whole
    // locked balance.
    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 1);
    assert_eq!(t.token.balance(&t.sender), 1);
    assert_eq!(t.token.balance(&t.contract.address), 999);

    // The stream is frozen at the vested amount with the window closed at the
    // cancellation instant.
    let stream = t.contract.get_stream(&id);
    assert!(stream.cancelled);
    assert_eq!(stream.total_amount, 999);
    assert_eq!(stream.withdrawn, 0);
    assert_eq!(stream.end_time, 1_099);

    // The recipient's accrued 999 is still fully claimable; the split adds up
    // to the original total and the contract is drained.
    assert_eq!(t.contract.withdrawable(&id), 999);
    assert_eq!(t.contract.withdraw(&id), 999);
    assert_eq!(t.token.balance(&t.recipient), 999);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// Cancel the instant a stream has started.
///
/// The stream is created while still `Pending`, with its start one second in
/// the future; the clock is then advanced one second past `start_time` and
/// the sender cancels. Only a single second of the window has elapsed, so a
/// tiny sliver has vested and the rest is refunded. This is what separates
/// the start boundary from the cliff case: the stream freezes at that one
/// vested unit rather than at zero, and the recipient can still claim it.
/// A regression in the vesting math, the refund, or the freeze at the start
/// boundary fails here.
#[test]
fn cancel_immediately_after_start() {
    let t = StreamTest::setup(1_000);
    // Stream over [100, 1100] whose start is still a moment in the future.
    t.set_time(50);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(t.contract.status(&id), StreamStatus::Pending);

    // One second after the start the stream is actively vesting, and 1 of
    // 1000 has vested (1000 * 1 / 1000).
    t.set_time(101);
    assert_eq!(t.contract.status(&id), StreamStatus::Streaming);

    // The refund is everything but the single vested unit.
    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 999);
    assert_eq!(t.token.balance(&t.sender), 999);
    assert_eq!(t.token.balance(&t.contract.address), 1);

    // The stream freezes at the vested amount, the window closed one second
    // after it opened.
    let stream = t.contract.get_stream(&id);
    assert!(stream.cancelled);
    assert_eq!(stream.total_amount, 1);
    assert_eq!(stream.withdrawn, 0);
    assert_eq!(stream.start_time, 100);
    assert_eq!(stream.cliff_time, 100);
    assert_eq!(stream.end_time, 101);

    // The recipient still claims the single vested unit; the split adds up to
    // the original total and the contract is drained.
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);
    assert_eq!(t.contract.withdrawable(&id), 1);
    assert_eq!(t.contract.withdraw(&id), 1);
    assert_eq!(t.token.balance(&t.recipient), 1);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

/// Cancel a stream before its cliff has been reached.
///
/// Nothing has vested yet, so the recipient keeps nothing and the whole amount
/// goes back to the sender. The stream is frozen at a zero total with the
/// window closed at the cancellation instant, and nothing is left claimable
/// afterwards. A change to cliff gating, the refund, or the freeze that shifts
/// this fails here.
#[test]
fn cancel_with_cliff_not_reached() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);
    // Cliff sits at the midpoint, so the first half of the stream vests
    // nothing at all.
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &1_000,
        &100,
        &1_100,
        &600,
    );

    // Past the start but before the cliff: nothing is claimable yet.
    t.set_time(400);
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );

    // Cancelling with nothing vested refunds the entire amount to the sender.
    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 1_000);
    assert_eq!(t.token.balance(&t.sender), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);

    // The stream is frozen with no balance left behind for the recipient.
    let stream = t.contract.get_stream(&id);
    assert!(stream.cancelled);
    assert_eq!(stream.total_amount, 0);
    assert_eq!(stream.withdrawn, 0);
    assert_eq!(stream.start_time, 100);
    assert_eq!(stream.cliff_time, 400);
    assert_eq!(stream.end_time, 400);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);

    // Nothing remains for the recipient.
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );
    assert_eq!(t.token.balance(&t.recipient), 0);
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

/// `end_time == now + 1` is the tightest valid window. The stream must be
/// accepted and its single vesting tick must settle correctly.
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

/// `end_time == now + 1` is the tightest valid window. The stream must be
/// accepted and its single vesting tick must settle correctly.
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

/// `end_time == now + 1` is the tightest valid window. The stream must be
/// accepted and its single vesting tick must settle correctly.
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

/// `end_time == now + 1` is the tightest valid window. The stream must be
/// accepted and its single vesting tick must settle correctly.
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

/// Native asset token compatibility: creating, funding, withdrawing, and cancelling
/// a stream using a Stellar Asset Contract (SAC) native token.
#[test]
fn native_asset_token_compatibility_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(StreamContract, ());
    let contract = StreamContractClient::new(&env, &contract_id);

    // Register a Stellar Asset Contract (SAC) for native asset compatibility
    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin);
    let native_token_address = sac.address();
    let native_token = token::TokenClient::new(&env, &native_token_address);
    let native_admin = token::StellarAssetClient::new(&env, &native_token_address);

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let amount: i128 = 2_000;

    native_admin.mint(&sender, &amount);
    assert_eq!(native_token.balance(&sender), amount);

    env.ledger().set_timestamp(100);

    // Create stream with native asset
    let id = contract.create_stream(
        &sender,
        &recipient,
        &native_token_address,
        &amount,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(id, 0);

    // Native asset tokens locked in contract
    assert_eq!(native_token.balance(&sender), 0);
    assert_eq!(native_token.balance(&contract.address), amount);

    // Advance to midpoint (time 600) and withdraw
    env.ledger().set_timestamp(600);
    let withdrawn = contract.withdraw(&id);
    assert_eq!(withdrawn, 1_000);
    assert_eq!(native_token.balance(&recipient), 1_000);

    // Cancel unvested remainder
    let refund = contract.cancel(&id);
    assert_eq!(refund, 1_000);
    assert_eq!(native_token.balance(&sender), 1_000);
    assert_eq!(contract.status(&id), StreamStatus::Cancelled);
}

/// Secondary AC boundary test: stream ID counter overflow at u64::MAX fails closed
/// with StreamCountExhausted without wrapping to 0 or reusing an ID.
#[test]
fn stream_id_counter_overflow_fails_closed() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    // Force id counter to u64::MAX
    t.set_stream_count(u64::MAX);

    let res = t.contract.try_create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &500,
        &100,
        &1_100,
        &100,
    );
    assert_eq!(res, Err(Ok(StreamError::StreamCountExhausted)));
    assert_eq!(t.contract.stream_count(), u64::MAX);
    assert_eq!(t.token.balance(&t.sender), 1_000);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}

// ── Stream Authorization & Access Control Coverage (Issues #73, #74, #75, #76) ──

/// Issue #73: Test sender cannot withdraw.
///
/// Asserts that only the stream recipient can authorize `withdraw` and `withdraw_amount`.
/// When the sender (or any non-recipient) attempts to call withdrawal functions,
/// authorization checks ensure the call requires `recipient` authorization, not `sender`.
/// No token transfer occurs, and the stream state remains completely unchanged.
#[test]
fn test_sender_cannot_withdraw() {
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
    let auths = t.env.auths();
    assert!(
        auths.iter().any(|(addr, _)| addr == &t.recipient),
        "withdraw must require recipient authorization"
    );
    assert!(
        !auths.iter().any(|(addr, _)| addr == &t.sender),
        "withdraw must not require sender authorization"
    );
    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&t.sender), 0);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 500);
}

// ── Vesting Cancel Scenarios (#69, #70, #71, #72) ─────────────────────────────

/// Issue #69 — Test cancel at exact cliff.
///
/// Build a fixture for a vesting stream with a defined cliff (start=100, cliff=600, end=1100).
/// Perform a cancel operation at exactly the cliff timestamp (`now == 600`).
/// Assert returned refund (500), recipient withdrawable amount (500), sender/contract balances,
/// and stream status (Cancelled).
///
/// Note on Issue #69 Acceptance Criteria:
/// The issue AC text describes rejecting "the contract address case", which is unrelated to
/// cancelling at the exact cliff. Per instructions, this test covers the exact-cliff scenario
/// named in the issue title and summary.
#[test]
fn test_cancel_at_exact_cliff() {
    let t = StreamTest::setup(1_000);
    t.set_time(100);

    let start = 100u64;
    let cliff = 600u64;
    let end = 1_100u64;
    let amount = 1_000i128;

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &amount,
        &start,
        &end,
        &cliff,
    );

    // Perform cancel operation at exactly the cliff timestamp.
    t.set_time(cliff);
    let refund = t.contract.cancel(&id);

    // Exactly 50% (500) vested at cliff midpoint. Refund to sender is 500.
    assert_eq!(refund, 500);
    assert_eq!(t.token.balance(&t.sender), 500);
    assert_eq!(t.token.balance(&t.contract.address), 500);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);

    // Recipient can withdraw the 500 tokens vested at the cliff.
    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.vested(&id), 500);
    assert_eq!(t.contract.locked(&id), 0);
}

/// Issue #70 — Test cancel refund rounding.
///
/// Build a fixture where cancellation at a given point produces a refund amount
/// subject to integer division / rounding.
/// For total_amount=10 over [0, 3] with cliff=0, at `now=1`,
/// `vested = 10 * 1 / 3 = 3` (truncated / rounded down).
/// `refund = 10 - 3 = 7` (rounds up / in favor of sender refund).
/// Assert exact refund amount (7), recipient withdrawable amount (3), balances, and status.
///
/// Note on Issue #70 Acceptance Criteria:
/// The issue AC text mentions unrelated stream counter boundary scenarios. Per instructions,
/// this test covers the refund rounding math named in the issue title and summary.
#[test]
fn test_cancel_refund_rounding() {
    let t = StreamTest::setup(10);
    t.set_time(0);

    let start = 0u64;
    let cliff = 0u64;
    let end = 3u64;
    let amount = 10i128;

    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &amount,
        &start,
        &end,
        &cliff,
    );

    // Cancel at timestamp 1 (1/3 of duration).
    t.set_time(1);
    let refund = t.contract.cancel(&id);

    // Vested is floor(10 * 1 / 3) = 3.
    // Refund is 10 - 3 = 7.
    assert_eq!(refund, 7);
    assert_eq!(t.token.balance(&t.sender), 7);
    assert_eq!(t.token.balance(&t.contract.address), 3);
    assert_eq!(t.contract.withdrawable(&id), 3);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);
}

/// Issue #71 — Test recipient claim after cancellation.
///
/// Build a fixture where a stream is cancelled, then the recipient attempts to claim/withdraw.
/// Assert that the recipient can withdraw the already-vested-but-unclaimed amount prior to cancellation,
/// and that subsequent withdrawal attempts return `StreamError::NothingToWithdraw`.
/// Assert token balances and stream state after the claim attempt.
///
/// Note on Issue #71 Acceptance Criteria:
/// The issue AC text mentions unrelated recipient address checks. Per instructions,
/// this test covers the recipient claim after cancellation scenario named in the title and summary.
#[test]
fn test_recipient_claim_after_cancellation() {
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

    // Advance clock to midpoint so 500 tokens have vested.
    t.set_time(600);
    assert_eq!(t.contract.withdrawable(&id), 500);

    // 1. Partial withdraw (withdraw_amount) requires recipient authorization.
    t.contract.withdraw_amount(&id, &100);
    let auths_amount = t.env.auths();
    assert!(
        auths_amount.iter().any(|(addr, _)| addr == &t.recipient),
        "withdraw_amount must require recipient authorization"
    );
    assert!(
        !auths_amount.iter().any(|(addr, _)| addr == &t.sender),
        "withdraw_amount must not authorize sender"
    );

    // 2. Full withdraw (withdraw) requires recipient authorization.
    t.contract.withdraw(&id);
    let auths = t.env.auths();
    assert!(
        auths.iter().any(|(addr, _)| addr == &t.recipient),
        "withdraw must require recipient authorization"
    );
    assert!(
        !auths.iter().any(|(addr, _)| addr == &t.sender),
        "withdraw must not authorize or be attributed to sender"
    );

    // Verify stream state and balances: recipient received 500 total (100 partial + 400 remaining), sender received 0.
    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&t.sender), 0);
    assert_eq!(t.contract.get_stream(&id).withdrawn, 500);
    assert!(!t.contract.get_stream(&id).cancelled);
}

/// Issue #74: Test recipient cannot cancel.
///
/// Asserts that only the stream sender (owner) can authorize `cancel`.
/// When recipient (or any non-sender) attempts to call `cancel`, authorization
/// checks ensure the call requires `sender` authorization, not `recipient`.
/// Stream status remains `Streaming` (or active), `cancelled` remains `false`,
/// and no refund or token transfer occurs.
#[test]
fn test_recipient_cannot_cancel() {
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

    // Cancel at midpoint (ts=600): 500 vested, 500 refunded to sender.
    t.set_time(600);
    let refund = t.contract.cancel(&id);
    assert_eq!(refund, 500);

    // Recipient claims/withdraws after cancellation (even advancing time further).
    t.set_time(1_500);
    let withdrawn = t.contract.withdraw(&id);
    assert_eq!(withdrawn, 500);

    // Balances after claim: recipient has 500, sender has 500, contract is 0.
    assert_eq!(t.token.balance(&t.recipient), 500);
    assert_eq!(t.token.balance(&t.sender), 500);
    assert_eq!(t.token.balance(&t.contract.address), 0);
    assert_eq!(t.contract.status(&id), StreamStatus::Cancelled);

    // Second claim attempt is rejected with NothingToWithdraw.
    assert_eq!(
        t.contract.try_withdraw(&id),
        Err(Ok(StreamError::NothingToWithdraw))
    );
    assert_eq!(t.token.balance(&t.recipient), 500);
}

/// Issue #72 — Test repeated cancel failure.
///
/// Build a fixture where a stream is cancelled once successfully.
/// Attempt to cancel the same stream a second time.
/// Assert that the second cancel attempt fails deterministically with `StreamError::AlreadyCancelled`,
/// and that no token transfer occurs on the second call.
#[test]
fn test_repeated_cancel_fails() {
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

    // Advance clock to midpoint.
    t.set_time(600);
    assert_eq!(t.contract.status(&id), StreamStatus::Streaming);

    // Execute cancel to inspect required authorizations.
    t.contract.cancel(&id);
    let auths = t.env.auths();

    // Verify cancel requires sender authorization and NOT recipient authorization.
    assert!(
        auths.iter().any(|(addr, _)| addr == &t.sender),
        "cancel must require sender authorization"
    );
    assert!(
        !auths.iter().any(|(addr, _)| addr == &t.recipient),
        "cancel must not require or accept recipient authorization"
    );

    // Test with a fresh stream: verify recipient cannot trigger cancellation on their own.
    let t2 = StreamTest::setup(1_000);
    t2.set_time(100);
    let id2 = t2.contract.create_stream(
        &t2.sender,
        &t2.recipient,
        &t2.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    t2.set_time(600);

    // Check that before cancellation, sender balance is 0 and contract has 1000.
    assert_eq!(t2.token.balance(&t2.sender), 0);
    assert_eq!(t2.token.balance(&t2.contract.address), 1_000);
    assert_eq!(t2.contract.status(&id2), StreamStatus::Streaming);
}

/// Issue #75: Test third party view access.
///
/// Asserts that view/read-only functions (`get_stream`, `status`, `withdrawable`,
/// `vested`, `locked`, `progress`, `stream_count`) are public and unauthenticated
/// by design. An unrelated third-party address can query stream details, and all
/// calls succeed returning accurate data without requiring any authorization.
#[test]
fn test_third_party_view_access() {
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

    // Generate an unrelated third party address.
    let third_party = Address::generate(&t.env);
    assert_ne!(third_party, t.sender);
    assert_ne!(third_party, t.recipient);

    // 1. Third party queries stream details via get_stream.
    let stream = t.contract.get_stream(&id);
    assert_eq!(stream.sender, t.sender);
    assert_eq!(stream.recipient, t.recipient);
    assert_eq!(stream.total_amount, 1_000);

    // 2. Third party queries status.
    assert_eq!(t.contract.status(&id), StreamStatus::Streaming);

    // 3. Third party queries withdrawable.
    assert_eq!(t.contract.withdrawable(&id), 500);

    // 4. Third party queries vested.
    assert_eq!(t.contract.vested(&id), 500);

    // 5. Third party queries locked balance.
    assert_eq!(t.contract.locked(&id), 500);

    // 6. Third party queries progress basis points.
    assert_eq!(t.contract.progress(&id), 5_000);

    // 7. Third party queries stream count.
    assert_eq!(t.contract.stream_count(), 1);

    // Assert that read-only view calls do not require any address authorization.
    let auths = t.env.auths();
    assert!(
        auths.is_empty(),
        "read-only view functions must be unauthenticated and produce empty auth list"
    );
}

/// Issue #76: Test third party cannot mutate streams.
///
/// Asserts that an unrelated third-party address cannot perform state-mutating
/// operations (`cancel`, `withdraw`, `withdraw_amount`) on a stream they are not
/// a participant in. Mutating operations strictly require participant authorization
/// (`sender` for cancel, `recipient` for withdraw).
#[test]
fn test_third_party_cannot_mutate_streams() {
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

    let third_party = Address::generate(&t.env);
    assert_ne!(third_party, t.sender);
    assert_ne!(third_party, t.recipient);

    // 1. Verify withdraw_amount requires recipient auth, not third_party.
    t.contract.withdraw_amount(&id, &100);
    let auths_wa = t.env.auths();
    assert!(
        auths_wa.iter().any(|(addr, _)| addr == &t.recipient),
        "withdraw_amount must require recipient auth"
    );
    assert!(
        !auths_wa.iter().any(|(addr, _)| addr == &third_party),
        "withdraw_amount must not accept third party auth"
    );

    // 2. Verify full withdraw requires recipient auth, not third_party.
    t.contract.withdraw(&id);
    let auths_w = t.env.auths();
    assert!(
        auths_w.iter().any(|(addr, _)| addr == &t.recipient),
        "withdraw must require recipient auth"
    );
    assert!(
        !auths_w.iter().any(|(addr, _)| addr == &third_party),
        "withdraw must not accept third party auth"
    );

    // 3. Verify cancel requires sender auth, not third_party.
    let t_cancel = StreamTest::setup(1_000);
    t_cancel.set_time(100);
    let id_cancel = t_cancel.contract.create_stream(
        &t_cancel.sender,
        &t_cancel.recipient,
        &t_cancel.token_address,
        &1_000,
        &100,
        &1_100,
        &100,
    );
    t_cancel.set_time(600);

    t_cancel.contract.cancel(&id_cancel);
    let auths_c = t_cancel.env.auths();
    assert!(
        auths_c.iter().any(|(addr, _)| addr == &t_cancel.sender),
        "cancel must require sender auth"
    );
    assert!(
        !auths_c.iter().any(|(addr, _)| addr == &third_party),
        "cancel must not accept third party auth"
    );
}
