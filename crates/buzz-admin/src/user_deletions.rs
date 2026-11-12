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
    fn batch_preview_defaults_to_print_only_and_requires_scope_and_valid_keys() {
        let args = [
            "buzz-admin",
            "user-deletions",
            "preview-batches",
            "--pubkey",
            "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "--community",
            "00000000-0000-0000-0000-000000000001",
            "--print-only",
        ];
        for (mode, expected_delete) in [
            (None, false),
            (Some("--print-only"), false),
            (Some("--read-only"), false),
            (Some("--delete"), true),
        ] {
            let parsed = crate::Cli::try_parse_from(args[..7].iter().copied().chain(mode))
                .expect("valid batch mode");
            let crate::Command::UserDeletions {
                command: super::UserDeletionsCommand::PreviewBatches { delete, .. },
            } = parsed.command
            else {
                panic!("expected batch command");
            };
            assert_eq!(delete, expected_delete);
        }
        assert!(crate::Cli::try_parse_from(&args[..5]).is_err());
        assert!(crate::Cli::try_parse_from(args.into_iter().chain(["--delete"])).is_err());
        assert!(crate::Cli::try_parse_from(
            args[..7].iter().copied().chain(["--read-only", "--delete"])
        )
        .is_err());
        for extra in [
            ["--batch-size", "0"],
            ["--batch-size", "101"],
            ["--pubkey", "demo"],
            ["--max-items", "0"],
            ["--max-items", "1000001"],
            ["--relay-pubkey", "invalid"],
        ] {
            assert!(crate::Cli::try_parse_from(args.into_iter().chain(extra)).is_err());
        }
        assert!(crate::Cli::try_parse_from(args.into_iter().chain(["--execute"])).is_err());
    }

    #[test]
    fn real_workflow_requires_explicit_delete_and_rejects_conflicting_preview_flags() {
        let base = [
            "buzz-admin",
            "user-deletions",
            "run",
            "00000000-0000-0000-0000-000000000001",
        ];
        for flags in [
            vec![],
            vec!["--read-only"],
            vec!["--print-only"],
            vec!["--delete", "--offline"],
            vec!["--rehearse", "--next-step"],
        ] {
            let parsed = crate::Cli::try_parse_from(base.into_iter().chain(flags.clone())).unwrap();
            let crate::Command::UserDeletions {
                command: super::UserDeletionsCommand::Run { execution, .. },
            } = parsed.command
            else {
                panic!("expected run command");
            };
            assert_eq!(execution.delete, flags.contains(&"--delete"));
            assert_eq!(
                execution.rehearse,
                flags.contains(&"--rehearse") || flags.contains(&"--read-only")
            );
        }
        for flags in [
            vec!["--read-only", "--delete"],
            vec!["--offline"],
            vec!["--community", "00000000-0000-0000-0000-000000000001"],
            vec!["--delete", "--stage", "purge_media"],
            vec!["--rehearse", "--delete"],
            vec!["--read-only", "--print-only"],
            vec!["--rehearse", "--offline"],
            vec!["--rehearse", "--stage", "purge_content"],
        ] {
            assert!(crate::Cli::try_parse_from(base.into_iter().chain(flags)).is_err());
        }
    }

    #[test]
    fn user_workflow_commands_are_registered() {
        for args in [
            vec![
                "buzz-admin",
                "user-deletions",
                "submit",
                "--rehearse",
                "--pubkey",
                "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
                "--requested-by",
                "operator-a",
            ],
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
