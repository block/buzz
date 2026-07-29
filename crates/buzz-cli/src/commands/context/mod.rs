mod doctor;
mod migrate;
pub(crate) mod profile;

use crate::error::CliError;
use crate::{ContextCmd, ContextSubcommand};

pub async fn dispatch(command: &ContextCmd) -> Result<(), CliError> {
    match &command.command {
        ContextSubcommand::Version { json } => {
            doctor::print_version(&doctor::version_report(), *json)
        }
        ContextSubcommand::Doctor { json, offline } => {
            let environment = profile::ProfileEnvironment::from_process()?;
            let profile = profile::resolve_profile(&command.profile, &environment)?;
            let report = doctor::diagnose(&profile, &environment, *offline).await;
            doctor::print_doctor(&report, *json)
        }
        ContextSubcommand::Migrate {
            apply,
            legacy_home,
            local_relay,
            rendezvous,
            context,
            json,
        } => {
            let environment = profile::ProfileEnvironment::from_process()?;
            let report = migrate::migrate(
                migrate::MigrationRequest {
                    profile: &command.profile,
                    legacy_root: legacy_home.as_deref(),
                    local_relay,
                    rendezvous: rendezvous.as_deref(),
                    default_context: context.as_deref(),
                    apply: *apply,
                },
                &environment,
            )?;
            migrate::print_report(&report, *json)
        }
    }
}
