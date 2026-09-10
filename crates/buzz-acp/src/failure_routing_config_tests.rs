//! Configuration-only regressions for opt-in failure routing.
//!
//! These tests exercise the real clap -> Config::from_args path without
//! touching process environment or invoking a provider.

use clap::Parser;
use nostr::{Keys, PublicKey};

use crate::config::{CliArgs, Config};

fn parsed_args(extra: &[&str]) -> CliArgs {
    let private_key = format!("{}1", "0".repeat(63));
    let mut argv = vec![
        "buzz-acp".to_string(),
        "--private-key".to_string(),
        private_key,
    ];
    argv.extend(extra.iter().map(|value| value.to_string()));
    CliArgs::try_parse_from(argv).expect("test CLI arguments should parse")
}

fn config(extra: &[&str]) -> Result<Config, crate::config::ConfigError> {
    Config::from_args(parsed_args(extra))
}

fn own_public_key() -> PublicKey {
    Keys::parse(&format!("{}1", "0".repeat(63)))
        .expect("test private key should parse")
        .public_key()
}

#[test]
fn failure_routing_defaults_to_no_handlers() {
    let config = config(&[]).expect("default configuration should be valid");

    assert!(config.failure_handlers.ordinary.is_none());
    assert!(config.failure_handlers.recovery.is_none());
    assert!(!config.failure_handlers.enabled());
}

#[test]
fn invalid_failure_or_recovery_handler_key_fails_configuration() {
    for flag in ["--failure-handler", "--recovery-handler"] {
        let result = config(&[flag, "not-a-pubkey"]);
        assert!(result.is_err(), "{flag} must reject malformed pubkeys");
    }
}

#[test]
fn handler_cannot_be_this_agents_own_public_key() {
    let own = own_public_key().to_hex();

    for flag in ["--failure-handler", "--recovery-handler"] {
        let result = config(&[flag, &own]);
        assert!(result.is_err(), "{flag} must reject the agent's own pubkey");
    }
}

#[test]
fn valid_handler_keys_are_trimmed_and_normalized() {
    let ordinary = Keys::generate().public_key().to_hex().to_uppercase();
    let recovery = Keys::generate().public_key().to_hex().to_uppercase();
    let ordinary_with_space = format!("  {ordinary}  ");
    let recovery_with_space = format!("\t{recovery}\n");

    let parsed = config(&[
        "--failure-handler",
        &ordinary_with_space,
        "--recovery-handler",
        &recovery_with_space,
    ])
    .expect("valid handler pubkeys should be accepted");

    assert_eq!(
        parsed.failure_handlers.ordinary,
        Some(PublicKey::from_hex(&ordinary.to_lowercase()).unwrap())
    );
    assert_eq!(
        parsed.failure_handlers.recovery,
        Some(PublicKey::from_hex(&recovery.to_lowercase()).unwrap())
    );
    assert!(parsed.failure_handlers.enabled());
}
