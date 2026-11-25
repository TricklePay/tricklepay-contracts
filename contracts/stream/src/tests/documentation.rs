#![cfg(test)]

use crate::StreamError;
use std::string::{String, ToString};

const README: &str = include_str!("../../../../README.md");

#[test]
fn readme_error_table_matches_stream_error_variants_and_codes() {
    let error_heading = README
        .lines()
        .position(|line| line == "## Error code reference")
        .expect("README must contain the Error code reference section");
    let actual: std::vec::Vec<(String, u32)> = README
        .lines()
        .skip(error_heading + 1)
        .skip_while(|line| {
            let mut columns = line.split('|').map(str::trim);
            columns.next() != Some("")
                || columns.next() != Some("Code")
                || columns.next() != Some("Variant")
        })
        .skip(1)
        .take_while(|line| line.trim_start().starts_with('|'))
        .filter(|line| !line.contains("---"))
        .map(|line| {
            let columns: std::vec::Vec<_> = line.split('|').map(str::trim).collect();
            let code = columns[1]
                .parse()
                .expect("README error code must be an integer");
            let variant = columns[2].trim_matches('`').to_string();
            (variant, code)
        })
        .collect();
    let expected: std::vec::Vec<(&'static str, u32)> = StreamError::ALL
        .iter()
        .map(|&err| (err.name(), err as u32))
        .collect();

    for (variant, expected_code) in &expected {
        let Some((_, actual_code)) = actual.iter().find(|(name, _)| name.as_str() == *variant)
        else {
            panic!("README error table is missing StreamError::{variant}");
        };
        assert_eq!(
            *actual_code, *expected_code,
            "README error table has the wrong code for StreamError::{variant}"
        );
    }
    for (variant, _) in &actual {
        assert!(
            expected.iter().any(|(name, _)| *name == variant.as_str()),
            "README error table has an unknown variant: {variant}"
        );
    }
    assert_eq!(
        actual.len(),
        expected.len(),
        "README error table must list every StreamError variant exactly once"
    );
}

#[test]
fn stream_error_all_matches_exhaustive_match() {
    fn check_in_all(err: StreamError) {
        assert!(
            StreamError::ALL.contains(&err),
            "StreamError::ALL is missing variant {:?}",
            err
        );
    }

    // Exhaustive match: if a variant is added to StreamError, this match
    // will fail to compile unless updated, ensuring StreamError::ALL is
    // kept complete.
    let check_exhaustive = |err: StreamError| match err {
        StreamError::StreamNotFound => check_in_all(err),
        StreamError::InvalidTimeRange => check_in_all(err),
        StreamError::InvalidAmount => check_in_all(err),
        StreamError::InvalidCliff => check_in_all(err),
        StreamError::AlreadyCancelled => check_in_all(err),
        StreamError::NothingToWithdraw => check_in_all(err),
        StreamError::InsufficientBalance => check_in_all(err),
        StreamError::StreamAlreadyCompleted => check_in_all(err),
        StreamError::AmountTooLarge => check_in_all(err),
        StreamError::StreamWindowInPast => check_in_all(err),
        StreamError::StreamCountExhausted => check_in_all(err),
        StreamError::InvalidParticipant => check_in_all(err),
        StreamError::BatchTooLarge => check_in_all(err),
        StreamError::EmptyBatch => check_in_all(err),
    };

    for &err in StreamError::ALL {
        check_exhaustive(err);
    }
    assert_eq!(
        StreamError::ALL.len(),
        14,
        "StreamError::ALL must have one entry per enum variant"
    );
}

const INTERFACE_SNAPSHOT: &str = include_str!("../../../../docs/interface.txt");

#[test]
fn interface_snapshot_lists_every_public_entry_point() {
    let mut parsed_functions: std::collections::BTreeMap<String, std::vec::Vec<String>> =
        std::collections::BTreeMap::new();

    let mut cursor = INTERFACE_SNAPSHOT;
    while let Some(fn_idx) = cursor.find("fn ") {
        let after_fn = &cursor[fn_idx + 3..];
        if let Some(open_paren) = after_fn.find('(') {
            let fn_name = after_fn[..open_paren].trim();
            if !fn_name.is_empty() && fn_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                let after_open = &after_fn[open_paren + 1..];
                if let Some(close_paren) = after_open.find(')') {
                    let params_str = &after_open[..close_paren];
                    let mut params = std::vec::Vec::new();
                    for param in params_str.split(',') {
                        let param = param.trim();
                        if !param.is_empty() {
                            if let Some(colon_idx) = param.find(':') {
                                params.push(param[..colon_idx].trim().to_string());
                            }
                        }
                    }
                    parsed_functions.insert(fn_name.to_string(), params);
                    cursor = &after_open[close_paren + 1..];
                    continue;
                }
            }
        }
        cursor = after_fn;
    }

    let expected_entry_points: &[(&str, &[&str])] = &[
        (
            "create_stream",
            &[
                "env",
                "sender",
                "recipient",
                "token",
                "total_amount",
                "start_time",
                "end_time",
                "cliff_time",
            ],
        ),
        ("withdraw", &["env", "id"]),
        ("withdraw_many", &["env", "ids"]),
        ("withdraw_amount", &["env", "id", "amount"]),
        ("cancel", &["env", "id"]),
        ("get_stream", &["env", "id"]),
        ("withdrawable", &["env", "id"]),
        ("withdrawable_many", &["env", "ids"]),
        ("rate_per_second", &["env", "id"]),
        ("vested", &["env", "id"]),
        ("locked", &["env", "id"]),
        ("progress", &["env", "id"]),
        ("status", &["env", "id"]),
        ("stream_count", &["env"]),
    ];

    for (fn_name, expected_params) in expected_entry_points {
        let Some(actual_params) = parsed_functions.get(*fn_name) else {
            panic!(
                "docs/interface.txt is missing public entry point '{}'; run 'make snapshot' to update",
                fn_name
            );
        };
        assert_eq!(
            actual_params.as_slice(),
            *expected_params,
            "docs/interface.txt parameters for '{}' differ; run 'make snapshot' to update",
            fn_name
        );
    }
}

#[test]
fn readme_resource_table_lists_required_entry_points() {
    let resource_heading = README
        .lines()
        .position(|line| line == "## Resource costs")
        .expect("README must contain '## Resource costs' section");

    let readme_section = README
        .lines()
        .skip(resource_heading)
        .collect::<std::vec::Vec<_>>();

    // Verify SDK version statement
    assert!(
        readme_section
            .iter()
            .take(30)
            .any(|line| line.contains("25.3.2")),
        "README resource cost section must document the SDK version (25.3.2)"
    );

    // Parse the table
    let table_rows: std::vec::Vec<String> = readme_section
        .iter()
        .skip_while(|line| !line.starts_with("| Entry point"))
        .skip(2) // header + separator
        .take_while(|line| line.starts_with('|'))
        .map(|line| {
            let cols: std::vec::Vec<_> = line.split('|').map(str::trim).collect();
            cols[1].replace('`', "").trim().to_string()
        })
        .collect();

    let expected_points = &[
        "create_stream",
        "withdraw",
        "withdraw_many (20 ids)",
        "cancel",
        "get_streams (1 id)",
        "get_streams (20 ids)",
    ];

    for point in expected_points {
        assert!(
            table_rows.iter().any(|row| row.starts_with(point)),
            "README resource table is missing entry point '{}'",
            point
        );
    }
}
