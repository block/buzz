use super::*;
use clap::Parser;

#[derive(Parser)]
struct TestCli {
    #[command(subcommand)]
    command: RemindersCmd,
}

#[test]
fn create_requires_one_unambiguous_due_time() {
    assert!(TestCli::try_parse_from(["buzz", "create", "--note", "inspect results"]).is_err());
    assert!(TestCli::try_parse_from([
        "buzz",
        "create",
        "--note",
        "inspect results",
        "--after",
        "7d",
        "--at",
        "2026-09-14T14:00:00Z"
    ])
    .is_err());
    assert!(TestCli::try_parse_from([
        "buzz",
        "create",
        "--note",
        "inspect results",
        "--after",
        "7d"
    ])
    .is_ok());
}

#[test]
fn due_time_handles_units_timezone_and_rejects_past_or_overflow() {
    for (raw, expected) in [("30s", 130), ("20m", 1300), ("4h", 14500), ("7d", 604900)] {
        assert_eq!(
            DueTime {
                at: None,
                after: Some(raw.into())
            }
            .resolve(100)
            .unwrap(),
            expected
        );
    }
    for raw in [
        "0s",
        "-1h",
        "3weeks",
        "1.5h",
        "18446744073709551615d",
        "é",
        " 5m",
    ] {
        assert!(DueTime {
            at: None,
            after: Some(raw.into())
        }
        .resolve(100)
        .is_err());
    }
    let first = DueTime {
        at: Some("2026-09-14T14:00:00Z".into()),
        after: None,
    }
    .resolve(100)
    .unwrap();
    assert_eq!(
        first,
        DueTime {
            at: Some("2026-09-14T10:00:00-04:00".into()),
            after: None
        }
        .resolve(100)
        .unwrap()
    );
    assert!(DueTime {
        at: Some("2026-09-14T14:00:00".into()),
        after: None
    }
    .resolve(100)
    .is_err());
    assert!(DueTime {
        at: Some("1970-01-01T00:00:00Z".into()),
        after: None
    }
    .resolve(100)
    .is_err());
}
