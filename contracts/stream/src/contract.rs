use soroban_sdk::{contract, contractimpl, token::TokenClient, Address, Env};

use crate::error::StreamError;
use crate::events;
use crate::status;
use crate::storage;
use crate::types::{Stream, StreamStatus};
use crate::vesting;

/// Maximum value accepted for `total_amount` at stream creation.
///
/// The vesting arithmetic computes `total_amount * elapsed / duration` where
/// `elapsed` can be at most `u64::MAX` seconds (the full range of the ledger
/// clock). To guarantee that intermediate `i128` multiplication never
/// overflows — regardless of stream duration — amounts are capped at
/// `i64::MAX` (≈ 9.2 × 10¹⁸ stroops). The bound is well above the total
/// supply of any realistic token and satisfies:
///
///   `i64::MAX as i128 * u64::MAX as i128 < i128::MAX`
pub const MAX_AMOUNT: i128 = i64::MAX as i128;

/// Check every rule `create_stream` enforces, before any token moves and
/// before any storage is written.
///
/// The rules are evaluated in the order documented on
/// [`StreamContract::create_stream`] and the first one that matches decides
/// the error, so a rejected call is indistinguishable from the equivalent
/// inline checks. Nothing here mutates state: the only storage touched is the
/// stream counter, which is read to hand back the id for the new stream.
///
/// Returns the id to use for the new stream together with the counter value
/// to persist once the transfer has succeeded.
fn validate_stream_creation(
    env: &Env,
    sender: &Address,
    recipient: &Address,
    token: &Address,
    total_amount: i128,
    start_time: u64,
    end_time: u64,
    cliff_time: u64,
) -> Result<(u64, u64), StreamError> {
    // 1. Participants. Identity is the most fundamental precondition and
    //    these are pure comparisons, so they run first.
    //
    //    A stream from an address to itself has no effect other than
    //    locking the sender's own tokens and handing them back over time.
    //    It is almost always a mistake — a swapped argument or an unset
    //    field — so it is refused rather than silently accepted.
    if sender == recipient {
        return Err(StreamError::InvalidParticipant);
    }
    //    A token contract cannot act as a stream participant, and attempting
    //    to stream a token to or from its own address is refused.
    if token == sender || token == recipient {
        return Err(StreamError::InvalidParticipant);
    }
    //    This contract's own address is not valid in any role. Each case
    //    fails differently — an unclaimable recipient, a token with no
    //    `transfer` entry point, a sender drawing on the holdings that
    //    back every other stream — so all three are refused here.
    let this = env.current_contract_address();
    if sender == &this || recipient == &this || token == &this {
        return Err(StreamError::InvalidParticipant);
    }

    // 2. Amount.
    if total_amount <= 0 {
        return Err(StreamError::InvalidAmount);
    }
    if total_amount > MAX_AMOUNT {
        return Err(StreamError::AmountTooLarge);
    }

    // 3. Schedule.
    if start_time >= end_time {
        return Err(StreamError::InvalidTimeRange);
    }
    if cliff_time < start_time || cliff_time > end_time {
        return Err(StreamError::InvalidCliff);
    }
    // Reject a window that is entirely in the past. A stream whose
    // end_time has already passed would be 100 % vested on creation —
    // effectively an immediate transfer with extra ceremony. Callers who
    // genuinely need that should use a token transfer directly.
    if end_time <= env.ledger().timestamp() {
        return Err(StreamError::StreamWindowInPast);
    }

    // 4. Capacity. Reserve the id before any tokens move. The counter is
    //    the source of every id and never reuses one, so if it were
    //    allowed to wrap the next stream would be written over a record
    //    that already exists. Checking here means an exhausted counter
    //    costs the caller nothing.
    let id = storage::stream_count(env);
    let next_id = id.checked_add(1).ok_or(StreamError::StreamCountExhausted)?;

    Ok((id, next_id))
}

#[contract]
pub struct StreamContract;

#[contractimpl]
impl StreamContract {
    // =====================================================================
    // Mutating entry points
    //
    // These require authorization, write contract state, and may move tokens.
    // Everything below the views section is read-only and moves nothing.
    // =====================================================================

    /// Open a new stream from `sender` to `recipient`.
    ///
    /// The full `total_amount` is pulled from the sender into the contract at
    /// creation, so the recipient is guaranteed the funds exist for the life
    /// of the stream. Vesting runs linearly from `start_time` to `end_time`;
    /// pass `cliff_time == start_time` for a stream with no cliff.
    ///
    /// Returns the id assigned to the new stream.
    ///
    /// # Validation order
    ///
    /// Arguments are checked in a fixed order, and **all of it happens before
    /// any tokens move or any storage is written**. A call that is rejected
    /// leaves no trace: no transfer, no stream record, no id consumed. When an
    /// argument list violates more than one rule, the first matching rule below
    /// determines the error, so the result is deterministic rather than an
    /// artefact of how the checks happen to be ordered in the body:
    ///
    /// 1. **Authorization** — `sender` must authorize the call.
    /// 2. **Participants** — [`StreamError::InvalidParticipant`] if `sender`
    ///    equals `recipient`, or if `token` equals `sender` or `recipient`,
    ///    or if any of `sender`, `recipient`, or `token` is this contract's
    ///    own address.
    /// 3. **Amount** — [`StreamError::InvalidAmount`] if `total_amount` is not
    ///    positive, then [`StreamError::AmountTooLarge`] if it exceeds
    ///    [`MAX_AMOUNT`].
    /// 4. **Schedule** — [`StreamError::InvalidTimeRange`] if `start_time` is
    ///    not strictly before `end_time`, then [`StreamError::InvalidCliff`]
    ///    if `cliff_time` falls outside `[start_time, end_time]`, then
    ///    [`StreamError::StreamWindowInPast`] if `end_time` is not in the
    ///    future.
    /// 5. **Capacity** — [`StreamError::StreamCountExhausted`] if the id
    ///    counter has reached `u64::MAX`. The operation fails closed without
    ///    wrapping to zero or reusing any previously assigned stream id.
    ///
    /// Only once all five pass are tokens transferred and the stream stored.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let id = client.create_stream(
    ///     &sender,
    ///     &recipient,
    ///     &token,
    ///     &1_000_000_000, // 1000 tokens (7 decimals)
    ///     &1_700_000_000, // start_time (unix timestamp)
    ///     &1_700_086_400, // end_time (start + 1 day)
    ///     &1_700_000_000, // cliff_time (no cliff)
    /// );
    /// ```
    // A contract entry point: every field is part of the public call shape,
    // so bundling them into a struct would only obscure the interface.
    // The too-many-arguments threshold is raised to 8 in clippy.toml to
    // accommodate this function without an inline allow attribute.
    pub fn create_stream(
        env: Env,
        sender: Address,
        recipient: Address,
        token: Address,
        total_amount: i128,
        start_time: u64,
        end_time: u64,
        cliff_time: u64,
    ) -> Result<u64, StreamError> {
        sender.require_auth();

        // All five rules are checked up front, before a single token moves or a
        // single storage key is written, so a rejected call leaves no trace.
        let (id, next_id) = validate_stream_creation(
            &env,
            &sender,
            &recipient,
            &token,
            total_amount,
            start_time,
            end_time,
            cliff_time,
        )?;

        // Effects. Every rejection above returns before this point, so a
        // failed creation never moves tokens or touches storage.
        TokenClient::new(&env, &token).transfer(
            &sender,
            env.current_contract_address(),
            &total_amount,
        );

        let stream = Stream {
            sender: sender.clone(),
            recipient: recipient.clone(),
            token: token.clone(),
            total_amount,
            withdrawn: 0,
            start_time,
            end_time,
            cliff_time,
            cancelled: false,
        };
        storage::set_stream(&env, id, &stream);
        storage::set_stream_count(&env, next_id);
        storage::extend_instance_ttl(&env);

        events::Created {
            sender: sender.clone(),
            recipient: recipient.clone(),
            id,
            token: token.clone(),
            total_amount,
            start_time,
            end_time,
            cliff_time,
        }
        .publish(&env);

        Ok(id)
    }

    /// Withdraw everything that has vested but not yet been taken.
    ///
    /// Only the recipient may call this. The amount sent is whatever has
    /// vested up to the current ledger time minus what was withdrawn before.
    /// Returns the amount transferred.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let amount_withdrawn = client.withdraw(&stream_id);
    /// ```
    pub fn withdraw(env: Env, id: u64) -> Result<i128, StreamError> {
        let mut stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        stream.recipient.require_auth();

        let now = env.ledger().timestamp();
        let vested = vesting::vested_amount(
            stream.total_amount,
            stream.start_time,
            stream.end_time,
            stream.cliff_time,
            now,
        );
        let available = vesting::withdrawable_amount(vested, stream.withdrawn);
        if available <= 0 {
            return Err(StreamError::NothingToWithdraw);
        }

        stream.withdrawn += available;
        storage::set_stream(&env, id, &stream);

        TokenClient::new(&env, &stream.token).transfer(
            &env.current_contract_address(),
            &stream.recipient,
            &available,
        );

        events::Withdrawn {
            recipient: stream.recipient.clone(),
            id,
            amount: available,
        }
        .publish(&env);

        Ok(available)
    }

    /// Withdraw a specific amount, up to what has vested.
    ///
    /// Only the recipient may call this. It behaves like [`Self::withdraw`] but
    /// lets the caller take less than the full available balance, which is
    /// useful for drawing a fixed sum or leaving a buffer in the stream. Fails
    /// if the requested amount exceeds the currently withdrawable balance.
    /// Returns the amount transferred.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let amount_withdrawn = client.withdraw_amount(&stream_id, &250_000_000);
    /// ```
    pub fn withdraw_amount(env: Env, id: u64, amount: i128) -> Result<i128, StreamError> {
        let mut stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        stream.recipient.require_auth();

        if amount <= 0 {
            return Err(StreamError::InvalidAmount);
        }

        let now = env.ledger().timestamp();
        let vested = vesting::vested_amount(
            stream.total_amount,
            stream.start_time,
            stream.end_time,
            stream.cliff_time,
            now,
        );
        let available = vesting::withdrawable_amount(vested, stream.withdrawn);
        if amount > available {
            return Err(StreamError::InsufficientBalance);
        }

        stream.withdrawn += amount;
        storage::set_stream(&env, id, &stream);

        TokenClient::new(&env, &stream.token).transfer(
            &env.current_contract_address(),
            &stream.recipient,
            &amount,
        );

        events::Withdrawn {
            recipient: stream.recipient.clone(),
            id,
            amount,
        }
        .publish(&env);

        Ok(amount)
    }

    /// Cancel a stream and refund the unvested remainder to the sender.
    ///
    /// Only the sender may call this. Whatever has vested up to the current
    /// ledger time stays claimable by the recipient through [`Self::withdraw`];
    /// the rest is returned to the sender. Once cancelled, no further tokens
    /// vest. Returns the amount refunded to the sender.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let refund_amount = client.cancel(&stream_id);
    /// ```
    pub fn cancel(env: Env, id: u64) -> Result<i128, StreamError> {
        let mut stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        stream.sender.require_auth();

        if stream.cancelled {
            return Err(StreamError::AlreadyCancelled);
        }

        let now = env.ledger().timestamp();

        if now >= stream.end_time {
            return Err(StreamError::StreamAlreadyCompleted);
        }
        let vested = vesting::vested_amount(
            stream.total_amount,
            stream.start_time,
            stream.end_time,
            stream.cliff_time,
            now,
        );
        let refund = stream.total_amount - vested;
        let recipient_remaining = vested - stream.withdrawn;

        // Freeze the stream at the vested amount. With the total reduced to
        // what has vested and the window closed at `now`, no further tokens
        // vest, but the recipient can still withdraw their accrued share.
        stream.total_amount = vested;
        stream.start_time = stream.start_time.min(now);
        stream.cliff_time = stream.cliff_time.min(now);
        stream.end_time = now;
        stream.cancelled = true;
        storage::set_stream(&env, id, &stream);

        if refund > 0 {
            TokenClient::new(&env, &stream.token).transfer(
                &env.current_contract_address(),
                &stream.sender,
                &refund,
            );
        }

        events::Cancelled {
            sender: stream.sender.clone(),
            id,
            recipient_amount: recipient_remaining,
            sender_refund: refund,
        }
        .publish(&env);

        Ok(refund)
    }

    // =====================================================================
    // Views
    //
    // Read-only. None of these authorize, write storage, or move tokens, so
    // they are safe to call from anywhere and return the same value for the
    // same ledger state.
    // =====================================================================

    /// Fetch a stream by id.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let stream = client.get_stream(&stream_id);
    /// assert_eq!(stream.total_amount, 1_000_000_000);
    /// ```
    pub fn get_stream(env: Env, id: u64) -> Result<Stream, StreamError> {
        storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)
    }

    /// Amount the recipient can withdraw right now.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let available = client.withdrawable(&stream_id);
    /// ```
    pub fn withdrawable(env: Env, id: u64) -> Result<i128, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        let vested = vesting::vested_amount(
            stream.total_amount,
            stream.start_time,
            stream.end_time,
            stream.cliff_time,
            env.ledger().timestamp(),
        );
        Ok(vesting::withdrawable_amount(vested, stream.withdrawn))
    }

    /// Total amount vested so far, including anything already withdrawn.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let total_vested = client.vested(&stream_id);
    /// ```
    pub fn vested(env: Env, id: u64) -> Result<i128, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        Ok(vesting::vested_amount(
            stream.total_amount,
            stream.start_time,
            stream.end_time,
            stream.cliff_time,
            env.ledger().timestamp(),
        ))
    }

    /// Amount not yet vested: the portion still locked in the contract that the
    /// recipient cannot withdraw yet.
    ///
    /// Locked behavior across stream lifecycle:
    /// - Before `start_time` or `cliff_time`: returns `total_amount` (entire amount locked).
    /// - Between `start_time` and `end_time`: decreases linearly as tokens vest (`total_amount - vested`).
    /// - At or after `end_time`: returns `0` (0% locked).
    /// - A cancelled stream returns `0` because cancellation freezes `total_amount` at `vested`.
    ///
    /// Rejections and Error Behavior:
    /// - Returns [`StreamError::StreamNotFound`] if `id` does not exist in storage
    ///   (e.g. an unknown id or an id from a creation call rejected for invalid participants).
    /// - `locked` is a read-only view function: it does not alter state or move tokens.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let remaining_locked = client.locked(&stream_id);
    /// ```
    pub fn locked(env: Env, id: u64) -> Result<i128, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        let vested = vesting::vested_amount(
            stream.total_amount,
            stream.start_time,
            stream.end_time,
            stream.cliff_time,
            env.ledger().timestamp(),
        );
        Ok(stream.total_amount - vested)
    }

    /// Vesting progress in basis points, from 0 (nothing vested) to 10000
    /// (100% vested). Useful for rendering a progress indicator without
    /// fetching the full stream.
    ///
    /// Progress calculations:
    /// - Returns `0` before `start_time` or `cliff_time`.
    /// - Scales linearly between `0` and `10000` from `start_time` to `end_time`.
    /// - Returns `10000` at or after `end_time`, or if `total_amount == 0`.
    /// - A stream with nothing left to vest, including a cancelled one, reports `10000`.
    ///
    /// Rejections and Error Behavior:
    /// - Returns [`StreamError::StreamNotFound`] if `id` does not exist in storage
    ///   (e.g. an unknown id or an id from a creation call rejected due to invalid participants).
    /// - `progress` is a read-only view function: it does not alter state or move tokens.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let bps = client.progress(&stream_id); // e.g. 5000 for 50%
    /// ```
    pub fn progress(env: Env, id: u64) -> Result<u32, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        if stream.total_amount == 0 {
            return Ok(10_000);
        }
        let vested = vesting::vested_amount(
            stream.total_amount,
            stream.start_time,
            stream.end_time,
            stream.cliff_time,
            env.ledger().timestamp(),
        );
        let progress = vested * 10_000 / stream.total_amount;
        Ok(u32::try_from(progress.clamp(0, 10_000)).unwrap_or(0))
    }

    /// Lifecycle status of a stream at the current ledger time.
    ///
    /// Returns the derived [`StreamStatus`] for a valid stream:
    /// - [`StreamStatus::Cancelled`]: if the stream has been cancelled (takes precedence).
    /// - [`StreamStatus::Pending`]: if current ledger time `now < start_time`.
    /// - [`StreamStatus::Streaming`]: if `start_time <= now < end_time`.
    /// - [`StreamStatus::Completed`]: if `now >= end_time`.
    ///
    /// Rejections and Error Behavior:
    /// - Returns [`StreamError::StreamNotFound`] if `id` does not exist in storage
    ///   (e.g. an unknown id, or an id from a creation attempt rejected for invalid
    ///   participants like the contract's own address).
    /// - `status` is a read-only view function: it does not alter contract state or move tokens.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let current_status = client.status(&stream_id);
    /// assert_eq!(current_status, StreamStatus::Streaming);
    /// ```
    pub fn status(env: Env, id: u64) -> Result<StreamStatus, StreamError> {
        let stream = storage::get_stream(&env, id).ok_or(StreamError::StreamNotFound)?;
        Ok(status::stream_status(&stream, env.ledger().timestamp()))
    }

    /// Number of streams created so far. Ids run from zero up to this value
    /// minus one.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let count = client.stream_count();
    /// ```
    pub fn stream_count(env: Env) -> u64 {
        storage::stream_count(&env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::Env;

    /// The ledger clock every case below is anchored to.
    const NOW: u64 = 1_000;
    /// A window entirely in the future, so rule 3 does not reject it.
    const START: u64 = 1_100;
    const END: u64 = 2_100;
    const AMOUNT: i128 = 1_000;

    /// A valid `create_stream` call that individual tests perturb one field at
    /// a time. Validation reads the ledger clock and the stream counter, so it
    /// is exercised from inside a contract context.
    struct Case {
        env: Env,
        contract: Address,
        sender: Address,
        recipient: Address,
        token: Address,
    }

    impl Case {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            env.ledger().set_timestamp(NOW);
            // Registering creates the contract instance, which is where the id
            // counter lives. Validation reads that counter, so the instance has
            // to exist before it can be exercised outside a real call.
            let contract = env.register(StreamContract, ());
            Self {
                sender: Address::generate(&env),
                recipient: Address::generate(&env),
                token: Address::generate(&env),
                contract,
                env,
            }
        }

        /// Validate the well-formed call, with `amount` substituted in.
        fn amount(&self, amount: i128) -> Result<(u64, u64), StreamError> {
            self.validate(amount, START, END, START)
        }

        /// Validate a fully specified call.
        fn validate(
            &self,
            amount: i128,
            start: u64,
            end: u64,
            cliff: u64,
        ) -> Result<(u64, u64), StreamError> {
            self.env.as_contract(&self.contract, || {
                validate_stream_creation(
                    &self.env,
                    &self.sender,
                    &self.recipient,
                    &self.token,
                    amount,
                    start,
                    end,
                    cliff,
                )
            })
        }

        /// Validate with explicit participants and token, for the identity
        /// rules.
        fn with_participants(
            &self,
            sender: &Address,
            recipient: &Address,
            token: &Address,
        ) -> Result<(u64, u64), StreamError> {
            self.env.as_contract(&self.contract, || {
                validate_stream_creation(
                    &self.env, sender, recipient, token, AMOUNT, START, END, START,
                )
            })
        }

        fn set_stream_count(&self, count: u64) {
            self.env.as_contract(&self.contract, || {
                storage::set_stream_count(&self.env, count)
            });
        }
    }

    // -- The amount ceiling ------------------------------------------------

    /// The ceiling is the bound the validation compares against, pinned here
    /// so a change to the constant cannot silently move the range of amounts
    /// `create_stream` accepts.
    #[test]
    fn the_amount_ceiling_is_i64_max() {
        assert_eq!(MAX_AMOUNT, i64::MAX as i128);
    }

    /// The ceiling exists for one reason: the vesting arithmetic multiplies
    /// `total_amount` by an elapsed time as large as `u64::MAX`, and that
    /// product has to stay inside `i128`. This asserts the property the
    /// constant's comment claims, so the comment cannot drift away from the
    /// value it documents.
    #[test]
    fn the_amount_ceiling_keeps_the_vesting_product_inside_i128() {
        let largest_possible_product = MAX_AMOUNT.saturating_mul(u64::MAX as i128);
        assert!(
            largest_possible_product < i128::MAX,
            "MAX_AMOUNT * u64::MAX must stay below i128::MAX, got {}",
            largest_possible_product
        );
    }

    // -- Accepted ---------------------------------------------------------

    #[test]
    fn accepts_a_well_formed_stream_and_reserves_the_first_id() {
        assert_eq!(Case::new().amount(AMOUNT), Ok((0, 1)));
    }

    #[test]
    fn returns_the_current_counter_as_the_new_id() {
        let c = Case::new();
        c.set_stream_count(7);
        assert_eq!(c.amount(AMOUNT), Ok((7, 8)));
    }

    /// The ceiling itself is inside the accepted range; only values above it
    /// are refused.
    #[test]
    fn accepts_an_amount_exactly_at_the_ceiling() {
        assert_eq!(Case::new().amount(MAX_AMOUNT), Ok((0, 1)));
    }

    /// `end_time` must be strictly in the future, so one second past the
    /// ledger clock is enough.
    #[test]
    fn accepts_a_window_ending_one_second_ahead() {
        let c = Case::new();
        assert_eq!(c.validate(AMOUNT, NOW, NOW + 1, NOW), Ok((0, 1)));
    }

    /// A cliff may sit on either edge of the window, including both.
    #[test]
    fn accepts_a_cliff_on_either_edge_of_the_window() {
        let c = Case::new();
        assert_eq!(c.validate(AMOUNT, START, END, START), Ok((0, 1)));
        assert_eq!(c.validate(AMOUNT, START, END, END), Ok((0, 1)));
    }

    /// A start time already in the past is allowed: the elapsed portion just
    /// vests immediately.
    #[test]
    fn accepts_a_start_time_in_the_past() {
        let c = Case::new();
        assert_eq!(c.validate(AMOUNT, 0, END, 0), Ok((0, 1)));
    }

    // -- Rejected: participants -------------------------------------------

    #[test]
    fn rejects_sender_equal_to_recipient() {
        let c = Case::new();
        assert_eq!(
            c.with_participants(&c.sender, &c.sender, &c.token),
            Err(StreamError::InvalidParticipant)
        );
    }

    #[test]
    fn rejects_token_equal_to_sender() {
        let c = Case::new();
        assert_eq!(
            c.with_participants(&c.sender, &c.recipient, &c.sender),
            Err(StreamError::InvalidParticipant)
        );
    }

    #[test]
    fn rejects_token_equal_to_recipient() {
        let c = Case::new();
        assert_eq!(
            c.with_participants(&c.sender, &c.recipient, &c.recipient),
            Err(StreamError::InvalidParticipant)
        );
    }

    #[test]
    fn rejects_the_contract_as_sender() {
        let c = Case::new();
        assert_eq!(
            c.with_participants(&c.contract, &c.recipient, &c.token),
            Err(StreamError::InvalidParticipant)
        );
    }

    #[test]
    fn rejects_the_contract_as_recipient() {
        let c = Case::new();
        assert_eq!(
            c.with_participants(&c.sender, &c.contract, &c.token),
            Err(StreamError::InvalidParticipant)
        );
    }

    #[test]
    fn rejects_the_contract_as_token() {
        let c = Case::new();
        assert_eq!(
            c.with_participants(&c.sender, &c.recipient, &c.contract),
            Err(StreamError::InvalidParticipant)
        );
    }

    // -- Rejected: amount -------------------------------------------------

    #[test]
    fn rejects_a_zero_amount() {
        assert_eq!(Case::new().amount(0), Err(StreamError::InvalidAmount));
    }

    #[test]
    fn rejects_a_negative_amount() {
        assert_eq!(Case::new().amount(-1), Err(StreamError::InvalidAmount));
    }

    #[test]
    fn rejects_an_amount_one_above_the_ceiling() {
        assert_eq!(
            Case::new().amount(MAX_AMOUNT + 1),
            Err(StreamError::AmountTooLarge)
        );
    }

    // -- Rejected: schedule ------------------------------------------------

    #[test]
    fn rejects_start_equal_to_end() {
        let c = Case::new();
        assert_eq!(
            c.validate(AMOUNT, START, START, START),
            Err(StreamError::InvalidTimeRange)
        );
    }

    #[test]
    fn rejects_start_after_end() {
        let c = Case::new();
        assert_eq!(
            c.validate(AMOUNT, END, START, START),
            Err(StreamError::InvalidTimeRange)
        );
    }

    #[test]
    fn rejects_a_cliff_before_the_window() {
        let c = Case::new();
        assert_eq!(
            c.validate(AMOUNT, START, END, START - 1),
            Err(StreamError::InvalidCliff)
        );
    }

    #[test]
    fn rejects_a_cliff_after_the_window() {
        let c = Case::new();
        assert_eq!(
            c.validate(AMOUNT, START, END, END + 1),
            Err(StreamError::InvalidCliff)
        );
    }

    /// The window is rejected only once it has fully closed, so `end_time`
    /// equal to the ledger clock is still too late.
    #[test]
    fn rejects_a_window_that_has_already_closed() {
        let c = Case::new();
        assert_eq!(
            c.validate(AMOUNT, NOW - 10, NOW, NOW - 10),
            Err(StreamError::StreamWindowInPast)
        );
    }

    // -- Rejected: capacity ------------------------------------------------

    #[test]
    fn rejects_an_exhausted_counter() {
        let c = Case::new();
        c.set_stream_count(u64::MAX);
        assert_eq!(c.amount(AMOUNT), Err(StreamError::StreamCountExhausted));
    }

    /// The last id before the counter runs out is still handed out.
    #[test]
    fn accepts_the_last_id_before_exhaustion() {
        let c = Case::new();
        c.set_stream_count(u64::MAX - 1);
        assert_eq!(c.amount(AMOUNT), Ok((u64::MAX - 1, u64::MAX)));
    }

    // -- Error precedence --------------------------------------------------

    /// The rules are ordered, and the first match wins, so a call that breaks
    /// several at once reports the earliest. These pin that order: participants,
    /// then amount, then schedule, then capacity.
    #[test]
    fn a_bad_participant_outranks_a_bad_amount_and_schedule() {
        let c = Case::new();
        c.set_stream_count(u64::MAX);
        c.env.as_contract(&c.contract, || {
            assert_eq!(
                validate_stream_creation(
                    &c.env,
                    &c.sender,
                    &c.sender,
                    &c.token,
                    0,
                    END,
                    START,
                    END + 1,
                ),
                Err(StreamError::InvalidParticipant)
            );
        });
    }

    #[test]
    fn a_bad_amount_outranks_a_bad_schedule_and_an_exhausted_counter() {
        let c = Case::new();
        c.set_stream_count(u64::MAX);
        assert_eq!(
            c.validate(0, END, START, END + 1),
            Err(StreamError::InvalidAmount)
        );
    }

    #[test]
    fn a_bad_time_range_outranks_a_bad_cliff_and_an_exhausted_counter() {
        let c = Case::new();
        c.set_stream_count(u64::MAX);
        assert_eq!(
            c.validate(AMOUNT, END, START, END + 1),
            Err(StreamError::InvalidTimeRange)
        );
    }

    #[test]
    fn a_bad_cliff_outranks_a_past_window_and_an_exhausted_counter() {
        let c = Case::new();
        c.set_stream_count(u64::MAX);
        assert_eq!(
            c.validate(AMOUNT, START, END, END + 1),
            Err(StreamError::InvalidCliff)
        );
    }

    #[test]
    fn a_past_window_outranks_an_exhausted_counter() {
        let c = Case::new();
        c.set_stream_count(u64::MAX);
        assert_eq!(
            c.validate(AMOUNT, NOW - 10, NOW, NOW - 10),
            Err(StreamError::StreamWindowInPast)
        );
    }

    // -- No side effects ---------------------------------------------------

    /// Rejected calls must not consume an id or write anything. The counter is
    /// the only storage this function can reach, and it must come back
    /// unchanged after a rejection.
    #[test]
    fn a_rejected_call_leaves_the_counter_untouched() {
        let c = Case::new();
        c.set_stream_count(3);
        for outcome in [
            c.amount(0),
            c.amount(MAX_AMOUNT + 1),
            c.validate(AMOUNT, END, START, START),
            c.validate(AMOUNT, START, END, END + 1),
            c.validate(AMOUNT, NOW - 10, NOW, NOW - 10),
            c.with_participants(&c.sender, &c.sender, &c.token),
        ] {
            assert!(outcome.is_err(), "expected a rejection, got {:?}", outcome);
        }
        assert_eq!(c.amount(AMOUNT), Ok((3, 4)));
    }
}
