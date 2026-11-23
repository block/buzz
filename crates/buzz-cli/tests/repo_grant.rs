use nostr::{Keys, Timestamp};
use std::process::Command;

#[test]
fn grant_command_prints_a_verifiable_tag_without_a_relay() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let repo = format!("30617:{}:example", owner.public_key().to_hex());
    for (scope, value) in [("--all", None), ("--repo", Some(repo.as_str()))] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_buzz"));
        command
            .env_clear()
            .env("BUZZ_PRIVATE_KEY", owner.secret_key().to_secret_hex())
            .args([
                "--relay",
                "http://127.0.0.1:1",
                "repos",
                "grant",
                "--agent",
                &agent.public_key().to_hex(),
                "--git",
                "push",
                "--for",
                "7d",
                scope,
            ]);
        if let Some(value) = value {
            command.arg(value);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json = String::from_utf8(output.stdout).unwrap();
        let grant = buzz_sdk::nip_oa::verify_git_grant(
            &json,
            &agent.public_key(),
            Timestamp::now().as_secs(),
        )
        .unwrap();
        assert_eq!(grant.owner, owner.public_key());
        assert_eq!(grant.window(), Some(604800));
        assert_eq!(
            grant.repo.as_ref().map(ToString::to_string).as_deref(),
            value
        );
        assert!(buzz_sdk::nip_oa::verify_auth_tag(&json, &agent.public_key()).is_err());
    }
}
