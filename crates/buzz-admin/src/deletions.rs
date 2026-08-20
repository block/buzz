//! Thin `buzz-admin deletions` adapter.

pub use buzz_deletion::Command as DeletionsCommand;

/// Delegate to the shared durable deletion engine.
pub async fn run(command: DeletionsCommand) -> anyhow::Result<i32> {
    buzz_deletion::run(command).await
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[test]
    fn preview_command_parses_as_a_distinct_read_only_operation() {
        let command = crate::Cli::try_parse_from([
            "buzz-admin",
            "deletions",
            "preview",
            "--host",
            "community.example",
            "--include-keys",
        ]);
        assert!(command.is_ok());
    }

    #[test]
    fn continuous_worker_command_is_not_exposed() {
        let command = crate::Cli::try_parse_from(["buzz-admin", "deletions", "worker"]);
        assert!(command.is_err());
    }
}
