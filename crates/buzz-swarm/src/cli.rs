use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::config::{expand_tilde, SwarmFile, DEFAULT_CONFIG_FILE};
use crate::env::Env;
use crate::plan::Plan;
use crate::supervisor::{self, Options};

#[derive(Parser)]
#[command(version, about = "Run Buzz agent identities on any number of relays")]
struct Cli {
    /// YAML swarm file. Paths inside it are relative to its directory.
    #[arg(short, long, default_value = DEFAULT_CONFIG_FILE, global = true)]
    config: PathBuf,
    /// Run only these agent names.
    #[arg(long, value_delimiter = ',', global = true)]
    only: Vec<String>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Start and supervise the swarm (default).
    Start {
        /// Grace period in seconds before forcibly stopping harness groups.
        #[arg(long, default_value_t = 15)]
        grace: u64,
    },
    /// Check swarm wiring and identities; write no keys and start no processes.
    /// Runtime-specific settings are validated by the runtime when it starts.
    Validate,
}

#[tokio::main]
pub async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(failed) if !failed => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("buzz-swarm: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<bool> {
    let env = Env::from_process();
    let file = SwarmFile::load(&expand_tilde(&cli.config, &env))?;
    let plan = Plan::build(&file, &cli.only, &env)?;
    match cli.command.unwrap_or(Command::Start { grace: 15 }) {
        Command::Validate => {
            println!("owner {}", plan.owner_pubkey);
            for agent in &plan.agents {
                println!(
                    "{} · relay {} · identity {} · {} · workdir {}",
                    agent.name,
                    agent.relay_url,
                    agent.pubkey,
                    agent.program.display(),
                    agent.workdir.display()
                );
            }
            for key in &plan.generated_keys {
                println!(
                    "{}: created on start (preview identity is temporary)",
                    key.path.display()
                );
            }
            for workdir in &plan.new_workdirs {
                println!("{}: created on start", workdir.display());
            }
            Ok(false)
        }
        Command::Start { grace } => {
            // Validate the whole file before creating keys or starting anything.
            for key in &plan.generated_keys {
                key.persist()?;
            }
            for workdir in &plan.new_workdirs {
                std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(workdir)
                    .with_context(|| format!("creating workdir {}", workdir.display()))?;
            }
            let summary = supervisor::run(
                plan,
                Options {
                    grace: Duration::from_secs(grace),
                },
            )
            .await?;
            for (name, outcome) in &summary.outcomes {
                eprintln!("{name}: {outcome:?}");
            }
            Ok(summary.failed())
        }
    }
}
