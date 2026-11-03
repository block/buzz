//! Thin `buzz-admin user-deletions` adapter.

pub use buzz_deletion::user::Command as UserDeletionsCommand;

/// Delegate request intake to the shared deletion crate.
pub async fn run(command: UserDeletionsCommand) -> anyhow::Result<i32> {
    buzz_deletion::user::run(command).await
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[test]
    fn user_workflow_commands_are_registered() {
        for args in [
            vec![
                "buzz-admin",
                "user-deletions",
                "inventory",
                "--pubkey",
                "5e4083a7a793d02575876eaf47932b4835999f5af2653d26250a909b450da7c9",
            ],
            vec![
                "buzz-admin",
                "user-deletions",
                "submit",
                "--target",
                "demo:alice",
                "--requested-by",
                "operator-a",
            ],
            vec![
                "buzz-admin",
                "user-deletions",
                "approve",
                "00000000-0000-0000-0000-000000000001",
                "--approved-by",
                "operator-b",
                "--inventory-digest",
                "digest-from-inspect",
            ],
            vec![
                "buzz-admin",
                "user-deletions",
                "run",
                "00000000-0000-0000-0000-000000000001",
                "--next-step",
                "--executor-id",
                "demo-worker",
                "--fail-stage",
                "purge_db",
                "--stage-delay-ms",
                "1000",
                "--demo-no-agents-in",
                "00000000-0000-0000-0000-00000000000c",
            ],
            vec!["buzz-admin", "user-deletions", "list"],
            vec![
                "buzz-admin",
                "user-deletions",
                "inspect",
                "00000000-0000-0000-0000-000000000001",
            ],
        ] {
            assert!(crate::Cli::try_parse_from(args).is_ok());
        }
        assert!(crate::Cli::try_parse_from([
            "buzz-admin",
            "user-deletions",
            "approve",
            "00000000-0000-0000-0000-000000000001",
            "--approved-by",
            "operator-b",
        ])
        .is_err());
        for (flag, value) in [
            ("--max-stages", "1"),
            ("--stage-delay-ms", "60001"),
            ("--fail-stage", "real_delete"),
        ] {
            assert!(crate::Cli::try_parse_from([
                "buzz-admin",
                "user-deletions",
                "run",
                "00000000-0000-0000-0000-000000000001",
                flag,
                value
            ])
            .is_err());
        }
        assert!(crate::Cli::try_parse_from([
            "buzz-admin",
            "user-deletions",
            "run",
            "00000000-0000-0000-0000-000000000001",
            "--pause-after-handler"
        ])
        .is_err());
        for command in ["run", "drain", "worker"] {
            assert!(crate::Cli::try_parse_from(["buzz-admin", "user-deletions", command]).is_err());
        }
        assert!(crate::Cli::try_parse_from([
            "buzz-admin",
            "user-deletions",
            "inventory",
            "--pubkey",
            "not-a-key",
        ])
        .is_err());
    }
}
