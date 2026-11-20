//! The `buzz-acceptance` probe against a running relay: every check passes.
//!
//! Requires a running relay, so it is `#[ignore]`d. Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3000 cargo test -p buzz-test-client --test e2e_acceptance -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::acceptance::{self, Config, Outcome, CHECKS};
use nostr::Keys;

#[tokio::test]
#[ignore]
async fn every_acceptance_check_passes_against_a_live_relay() {
    let relay_url = std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".into());
    let report = acceptance::run(&Config {
        relay_url,
        keys: Keys::generate(),
        assertion_file: None,
        stage_timeout: Duration::from_secs(10),
    })
    .await;

    let summary = serde_json::to_string_pretty(&report).unwrap();
    assert!(report.passed, "{summary}");
    let names: Vec<&str> = report.checks.iter().map(|c| c.check).collect();
    assert_eq!(names, CHECKS, "{summary}");
    for check in &report.checks {
        assert_eq!(check.result, Outcome::Success, "{summary}");
    }
    assert!(report.relay_version.is_some(), "{summary}");
}
