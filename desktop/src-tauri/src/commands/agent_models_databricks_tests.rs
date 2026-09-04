//! Cooldown and interactive-auth policy tests for Databricks discovery.
//!
//! Housed as a child of `agent_models_databricks` (not the shared
//! `agent_models_tests`) so the async timeout/cooldown cases sit next to the
//! code they exercise and reach its `pub(super)` items directly via
//! `use super::*` — and so the shared test file stays under its size ratchet.

use super::*;

#[test]
fn databricks_cooldown_suppresses_passive_relaunch_but_never_the_picker() {
    let cooldowns = AuthCooldown::default();
    let host = "https://example.cloud.databricks.com";
    let now = Instant::now();

    // A fresh host permits either surface to launch.
    assert!(cooldowns.permits_launch(DatabricksAuthIntent::PassiveDraftDiscovery, host, now));
    assert!(cooldowns.permits_launch(DatabricksAuthIntent::InteractiveModelPicker, host, now));

    // After a failed/cancelled attempt, passive discovery must NOT re-pop the
    // browser while the window is active...
    cooldowns.record(host, now);
    assert!(!cooldowns.permits_launch(DatabricksAuthIntent::PassiveDraftDiscovery, host, now));

    // ...but an explicit picker click always launches, and clears the window so
    // a later passive read is unblocked too.
    assert!(cooldowns.permits_launch(DatabricksAuthIntent::InteractiveModelPicker, host, now));
    assert!(cooldowns.permits_launch(DatabricksAuthIntent::PassiveDraftDiscovery, host, now));
}

#[test]
fn databricks_cooldown_expires_after_its_window_and_is_host_scoped() {
    let cooldowns = AuthCooldown::default();
    let host = "https://a.cloud.databricks.com";
    let other = "https://b.cloud.databricks.com";
    let now = Instant::now();

    cooldowns.record(host, now);
    // A cooldown on one host never suppresses another.
    assert!(!cooldowns.is_active(other, now));
    assert!(cooldowns.is_active(host, now));

    // The window is closed the instant it elapses, so a genuine later retry
    // launches again.
    let after = now + AUTH_COOLDOWN;
    assert!(!cooldowns.is_active(host, after));
}

#[tokio::test]
async fn databricks_interactive_auth_success_clears_a_prior_cooldown() {
    let cooldowns = AuthCooldown::default();
    let host = "https://example.cloud.databricks.com";
    let redaction = BTreeMap::new();
    cooldowns.record(host, Instant::now());

    let result = run_interactive_databricks_auth(
        async { Ok(()) },
        Duration::from_secs(150),
        &cooldowns,
        host,
        &redaction,
    )
    .await;

    assert!(result.is_ok());
    assert!(!cooldowns.is_active(host, Instant::now()));
}

#[tokio::test]
async fn databricks_interactive_auth_failure_records_a_cooldown() {
    let cooldowns = AuthCooldown::default();
    let host = "https://example.cloud.databricks.com";
    let redaction = BTreeMap::new();

    let result = run_interactive_databricks_auth(
        async { Err(buzz_agent_pkg::AgentError::LlmAuth("closed the tab".into())) },
        Duration::from_secs(150),
        &cooldowns,
        host,
        &redaction,
    )
    .await;

    let error = result.expect_err("a failed sign-in must surface an error");
    assert!(error.contains("Databricks sign-in failed"));
    assert!(cooldowns.is_active(host, Instant::now()));
}

#[tokio::test(start_paused = true)]
async fn databricks_interactive_auth_timeout_records_cooldown_and_returns_timeout_copy() {
    let cooldowns = AuthCooldown::default();
    let host = "https://example.cloud.databricks.com";
    let redaction = BTreeMap::new();

    // An abandoned SSO tab: the flow never resolves. Under the paused clock the
    // injected timeout fires deterministically without real waiting.
    let result = run_interactive_databricks_auth(
        std::future::pending::<Result<(), buzz_agent_pkg::AgentError>>(),
        Duration::from_secs(150),
        &cooldowns,
        host,
        &redaction,
    )
    .await;

    let error = result.expect_err("a timed-out sign-in must surface an error");
    assert_eq!(error, databricks_sign_in_timed_out_error());
    assert!(cooldowns.is_active(host, Instant::now()));
}
