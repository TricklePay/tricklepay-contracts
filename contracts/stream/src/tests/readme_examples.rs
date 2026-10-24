#![cfg(test)]

use super::helpers::{StreamTest, AMOUNT, END, MIDPOINT, START};
use crate::StreamStatus;

/// README: "Cancel at the midpoint" worked example.
#[test]
fn readme_cancel_midpoint_refunds_and_leaves_claimable_balance() {
    let t = StreamTest::setup(AMOUNT);
    t.set_time(START);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &AMOUNT,
        &START,
        &END,
        &START,
    );
    t.set_time(MIDPOINT);
    assert_eq!(t.contract.cancel(&id), 500);
    assert_eq!(t.contract.withdrawable(&id), 500);
    assert_eq!(t.contract.withdraw(&id), 500);
}

/// README: "Cancel after an earlier withdrawal" worked example.
#[test]
fn readme_cancel_after_withdrawal_leaves_three_hundred_claimable() {
    let t = StreamTest::setup(AMOUNT);
    t.set_time(START);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &AMOUNT,
        &START,
        &END,
        &START,
    );
    t.set_time(MIDPOINT);
    assert_eq!(t.contract.withdraw_amount(&id, &200), 200);
    // Sender refund is the unvested half (500), not the recipient's remaining claim.
    assert_eq!(t.contract.cancel(&id), 500);
    assert_eq!(t.contract.withdrawable(&id), 300);
}

/// README: "Fully withdrawn stream: what each view reports" table.
#[test]
fn readme_fully_withdrawn_completed_stream_views() {
    let t = StreamTest::setup(AMOUNT);
    t.set_time(START);
    let id = t.contract.create_stream(
        &t.sender,
        &t.recipient,
        &t.token_address,
        &AMOUNT,
        &START,
        &END,
        &START,
    );
    t.set_time(END);
    assert_eq!(t.contract.withdraw(&id), AMOUNT);
    assert_eq!(t.contract.withdrawable(&id), 0);
    assert_eq!(t.contract.get_stream(&id).withdrawn, AMOUNT);
    assert_eq!(t.contract.status(&id), StreamStatus::Completed);
    assert_eq!(t.token.balance(&t.contract.address), 0);
}
