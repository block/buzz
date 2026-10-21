//! Thin `buzz-admin user-deletions` adapter.

pub use buzz_deletion::user::Command as UserDeletionsCommand;

/// Delegate to the shared deletion engine's PostgreSQL-only demonstration path.
pub async fn run(command: UserDeletionsCommand) -> anyhow::Result<i32> {
    buzz_deletion::user::run(command).await
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[test]
    fn user_commands_are_registered_without_a_continuous_worker() {
        for args in [
            vec![
                "buzz-admin",
                "user-deletions",
                "submit",
                "--target",
                "demo:alice",
                "--requested-by",
                "operator-a",
            ],
            vec!["buzz-admin", "user-deletions", "list"],
            vec!["buzz-admin", "user-deletions", "drain"],
        ] {
            assert!(crate::Cli::try_parse_from(args).is_ok());
        }
        assert!(crate::Cli::try_parse_from(["buzz-admin", "user-deletions", "worker"]).is_err());
    }
}
