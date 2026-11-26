#![cfg(test)]

use std::vec;
use std::vec::Vec;

use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token, Address, Env,
};

use crate::contract::{StreamContract, StreamContractClient};

/// Configuration for an initial stream created across tokens and participants.
#[derive(Debug, Clone)]
pub struct StreamSpec {
    pub token_idx: usize,
    pub sender_idx: usize,
    pub recipient_idx: usize,
    pub amount: i128,
    pub duration: u64,
    pub cliff_offset: u64,
}

/// Operations that can be applied to active streams in the multi-token environment.
#[derive(Debug, Clone)]
pub enum StreamOp {
    Withdraw {
        stream_idx: usize,
    },
    WithdrawAmount {
        stream_idx: usize,
        amount: i128,
    },
    WithdrawTo {
        stream_idx: usize,
        destination_idx: usize,
        amount: i128,
    },
    WithdrawMany {
        stream_indices: Vec<usize>,
    },
    Cancel {
        stream_idx: usize,
    },
    TopUp {
        stream_idx: usize,
        amount: i128,
    },
    AdvanceTime {
        delta_seconds: u64,
    },
    CreateStream {
        token_idx: usize,
        sender_idx: usize,
        recipient_idx: usize,
        amount: i128,
        duration: u64,
        cliff_offset: u64,
    },
}

/// Strategy for generating initial streams spanning at least 3 tokens, several senders and recipients.
fn initial_streams_strategy() -> impl Strategy<Value = Vec<StreamSpec>> {
    (
        // Guarantee at least one stream for token 0, token 1, and token 2 with different senders and recipients
        (
            0usize..3,
            0usize..3,
            500i128..=30_000,
            200u64..=10_000,
            0u64..=100,
        ),
        (
            0usize..3,
            0usize..3,
            500i128..=30_000,
            200u64..=10_000,
            0u64..=100,
        ),
        (
            0usize..3,
            0usize..3,
            500i128..=30_000,
            200u64..=10_000,
            0u64..=100,
        ),
        prop::collection::vec(
            (
                0usize..3,
                0usize..3,
                0usize..3,
                500i128..=30_000,
                200u64..=10_000,
                0u64..=100,
            ),
            0..4,
        ),
    )
        .prop_map(|(s0, s1, s2, extra)| {
            let mut streams = vec![
                StreamSpec {
                    token_idx: 0,
                    sender_idx: s0.0,
                    recipient_idx: s0.1,
                    amount: s0.2,
                    duration: s0.3,
                    cliff_offset: s0.4.min(s0.3),
                },
                StreamSpec {
                    token_idx: 1,
                    sender_idx: s1.0,
                    recipient_idx: s1.1,
                    amount: s1.2,
                    duration: s1.3,
                    cliff_offset: s1.4.min(s1.3),
                },
                StreamSpec {
                    token_idx: 2,
                    sender_idx: s2.0,
                    recipient_idx: s2.1,
                    amount: s2.2,
                    duration: s2.3,
                    cliff_offset: s2.4.min(s2.3),
                },
            ];
            for (token_idx, sender_idx, recipient_idx, amount, duration, cliff_offset) in extra {
                streams.push(StreamSpec {
                    token_idx,
                    sender_idx,
                    recipient_idx,
                    amount,
                    duration,
                    cliff_offset: cliff_offset.min(duration),
                });
            }
            streams
        })
}

/// Strategy for generating sequential operations.
fn stream_ops_strategy() -> impl Strategy<Value = Vec<StreamOp>> {
    prop::collection::vec(
        prop_oneof![
            (0usize..10).prop_map(|stream_idx| StreamOp::Withdraw { stream_idx }),
            (0usize..10, 1i128..=5_000)
                .prop_map(|(stream_idx, amount)| StreamOp::WithdrawAmount { stream_idx, amount }),
            (0usize..10, 0usize..3, 1i128..=5_000).prop_map(
                |(stream_idx, destination_idx, amount)| StreamOp::WithdrawTo {
                    stream_idx,
                    destination_idx,
                    amount,
                },
            ),
            prop::collection::vec(0usize..10, 1..=5)
                .prop_map(|stream_indices| StreamOp::WithdrawMany { stream_indices }),
            (0usize..10).prop_map(|stream_idx| StreamOp::Cancel { stream_idx }),
            (0usize..10, -100i128..=10_000)
                .prop_map(|(stream_idx, amount)| StreamOp::TopUp { stream_idx, amount }),
            (1u64..=1_000).prop_map(|delta_seconds| StreamOp::AdvanceTime { delta_seconds }),
            (
                0usize..3,
                0usize..3,
                0usize..3,
                500i128..=20_000,
                200u64..=5_000,
                0u64..=50,
            )
                .prop_map(
                    |(token_idx, sender_idx, recipient_idx, amount, duration, cliff_offset)| {
                        StreamOp::CreateStream {
                            token_idx,
                            sender_idx,
                            recipient_idx,
                            amount,
                            duration,
                            cliff_offset: cliff_offset.min(duration),
                        }
                    }
                ),
        ],
        10..40,
    )
}

/// The multi-token invariant: each token's contract balance must equal the sum
/// of all unwithdrawn escrow (`total_amount - withdrawn`) across all streams of that token,
/// and `total_paid_out + contract_balance == total_deposited`.
fn assert_multi_token_escrow_invariants(
    contract: &StreamContractClient,
    tokens: &[token::TokenClient; 3],
    stream_records: &[(u64, usize)],
    total_deposited: &[i128; 3],
    total_paid_out: &[i128; 3],
) {
    let mut expected_escrow = [0i128; 3];

    for &(id, token_idx) in stream_records {
        let stream = contract.get_stream(&id);
        let unpaid = stream.total_amount - stream.withdrawn;
        expected_escrow[token_idx] += unpaid;
    }

    for (token_idx, token) in tokens.iter().enumerate() {
        let actual_balance = token.balance(&contract.address);
        let expected = expected_escrow[token_idx];
        assert_eq!(
            actual_balance, expected,
            "escrow balance invariant violated for token {}: contract holds {} but unwithdrawn escrow is {}",
            token_idx, actual_balance, expected
        );
        assert_eq!(
            total_paid_out[token_idx] + actual_balance,
            total_deposited[token_idx],
            "conservation invariant violated for token {}: paid_out ({}) + contract_balance ({}) != deposited ({})",
            token_idx,
            total_paid_out[token_idx],
            actual_balance,
            total_deposited[token_idx]
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// Property test checking that random sequences of stream creation, top-up,
    /// withdrawal, cancellation, and time advancement across multiple tokens and
    /// participants strictly preserve each token's escrow balance invariant at every step.
    #[test]
    fn multi_token_multi_sender_escrow_conservation(
        initial_specs in initial_streams_strategy(),
        ops in stream_ops_strategy()
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);

        let contract_id = env.register(StreamContract, ());
        let contract = StreamContractClient::new(&env, &contract_id);

        // Set up 3 distinct SAC tokens.
        let mut raw_token_addrs = Vec::new();
        for _ in 0..3 {
            let issuer = Address::generate(&env);
            let sac = env.register_stellar_asset_contract_v2(issuer);
            raw_token_addrs.push(sac.address());
        }

        let token_addrs: [Address; 3] = [
            raw_token_addrs[0].clone(),
            raw_token_addrs[1].clone(),
            raw_token_addrs[2].clone(),
        ];
        let tokens: [token::TokenClient; 3] = [
            token::TokenClient::new(&env, &token_addrs[0]),
            token::TokenClient::new(&env, &token_addrs[1]),
            token::TokenClient::new(&env, &token_addrs[2]),
        ];

        // Set up 3 distinct senders and 3 distinct recipients.
        let senders = [
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ];
        let recipients = [
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ];
        let destinations = [
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ];

        // Fund each sender with a generous balance for every token.
        for sender in &senders {
            for addr in &token_addrs {
                let admin = token::StellarAssetClient::new(&env, addr);
                admin.mint(sender, &5_000_000);
            }
        }

        // Create the initial set of streams (at least 3 tokens, multiple senders and recipients).
        let mut stream_records: Vec<(u64, usize)> = Vec::new();
        let mut total_deposited = [0i128; 3];
        let mut total_paid_out = [0i128; 3];

        let start_time = env.ledger().timestamp();
        for spec in initial_specs {
            let end_time = start_time + spec.duration;
            let cliff_time = start_time + spec.cliff_offset;
            let id = contract.create_stream(
                &senders[spec.sender_idx],
                &recipients[spec.recipient_idx],
                &token_addrs[spec.token_idx],
                &spec.amount,
                &start_time,
                &end_time,
                &cliff_time,
            );
            stream_records.push((id, spec.token_idx));
            total_deposited[spec.token_idx] += spec.amount;
        }

        // Verify invariant right after initial multi-token stream generation.
        assert_multi_token_escrow_invariants(
            &contract,
            &tokens,
            &stream_records,
            &total_deposited,
            &total_paid_out,
        );

        // Execute sequential operations and verify invariant after every step.
        for op in ops {
            match op {
                StreamOp::Withdraw { stream_idx } => {
                    if !stream_records.is_empty() {
                        let (id, token_idx) = stream_records[stream_idx % stream_records.len()];
                        if let Ok(Ok(paid)) = contract.try_withdraw(&id) {
                            total_paid_out[token_idx] += paid;
                        }
                    }
                }
                StreamOp::WithdrawAmount { stream_idx, amount } => {
                    if !stream_records.is_empty() {
                        let (id, token_idx) = stream_records[stream_idx % stream_records.len()];
                        if let Ok(Ok(paid)) = contract.try_withdraw_amount(&id, &amount) {
                            total_paid_out[token_idx] += paid;
                        }
                    }
                }
                StreamOp::WithdrawTo {
                    stream_idx,
                    destination_idx,
                    amount,
                } => {
                    if !stream_records.is_empty() {
                        let (id, token_idx) = stream_records[stream_idx % stream_records.len()];
                        if let Ok(Ok(paid)) = contract.try_withdraw_to(
                            &id,
                            &destinations[destination_idx],
                            &amount,
                        ) {
                            total_paid_out[token_idx] += paid;
                        }
                    }
                }
                StreamOp::WithdrawMany { stream_indices } => {
                    if !stream_records.is_empty() {
                        let mut ids = soroban_sdk::Vec::new(&env);
                        let mut batch_tokens = Vec::new();
                        for idx in stream_indices {
                            let (id, token_idx) = stream_records[idx % stream_records.len()];
                            ids.push_back(id);
                            batch_tokens.push(token_idx);
                        }
                        if let Ok(Ok(amounts)) = contract.try_withdraw_many(&ids) {
                            for (paid, token_idx) in amounts.iter().zip(batch_tokens.iter()) {
                                total_paid_out[*token_idx] += paid;
                            }
                        }
                    }
                }
                StreamOp::Cancel { stream_idx } => {
                    if !stream_records.is_empty() {
                        let (id, token_idx) = stream_records[stream_idx % stream_records.len()];
                        if let Ok(Ok(refund)) = contract.try_cancel(&id) {
                            total_paid_out[token_idx] += refund;
                        }
                    }
                }
                StreamOp::TopUp { stream_idx, amount } => {
                    if !stream_records.is_empty() {
                        let (id, token_idx) = stream_records[stream_idx % stream_records.len()];
                        if let Ok(Ok(())) = contract.try_topup_stream(&id, &amount) {
                            total_deposited[token_idx] += amount;
                        }
                    }
                }
                StreamOp::AdvanceTime { delta_seconds } => {
                    let now = env.ledger().timestamp();
                    env.ledger().set_timestamp(now.saturating_add(delta_seconds));
                }
                StreamOp::CreateStream {
                    token_idx,
                    sender_idx,
                    recipient_idx,
                    amount,
                    duration,
                    cliff_offset,
                } => {
                    let now = env.ledger().timestamp();
                    let end_time = now + duration;
                    let cliff_time = now + cliff_offset;
                    if let Ok(Ok(id)) = contract.try_create_stream(
                        &senders[sender_idx],
                        &recipients[recipient_idx],
                        &token_addrs[token_idx],
                        &amount,
                        &now,
                        &end_time,
                        &cliff_time,
                    ) {
                        stream_records.push((id, token_idx));
                        total_deposited[token_idx] += amount;
                    }
                }
            }

            // Invariant must hold for all tokens after EVERY operation.
            assert_multi_token_escrow_invariants(
                &contract,
                &tokens,
                &stream_records,
                &total_deposited,
                &total_paid_out,
            );
        }
    }
}
