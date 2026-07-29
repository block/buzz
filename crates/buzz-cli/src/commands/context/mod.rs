mod agreement;
mod artifact_sync;
mod doctor;
mod handoff;
mod journal;
mod migrate;
mod observation;
pub(crate) mod profile;
mod runtime;

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
        ContextSubcommand::Save { scope, file } => {
            let environment = profile::ProfileEnvironment::from_process()?;
            let profile = profile::resolve_profile(&command.profile, &environment)?;
            journal::save(&profile, &environment, scope, file.as_deref()).await
        }
        ContextSubcommand::Load { scope } => {
            let environment = profile::ProfileEnvironment::from_process()?;
            let profile = profile::resolve_profile(&command.profile, &environment)?;
            journal::load(&profile, &environment, scope).await
        }
        ContextSubcommand::Ls { json } => {
            let environment = profile::ProfileEnvironment::from_process()?;
            let profile = profile::resolve_profile(&command.profile, &environment)?;
            journal::list(&profile, &environment, *json).await
        }
        ContextSubcommand::Log { project, message } => {
            let environment = profile::ProfileEnvironment::from_process()?;
            let profile = profile::resolve_profile(&command.profile, &environment)?;
            println!(
                "{}",
                journal::log(&profile, &environment, project, message).await?
            );
            Ok(())
        }
        ContextSubcommand::Sessions {
            project,
            limit,
            json,
        } => {
            let environment = profile::ProfileEnvironment::from_process()?;
            let profile = profile::resolve_profile(&command.profile, &environment)?;
            journal::sessions(&profile, &environment, project, *limit, *json).await
        }
        ContextSubcommand::Sync { dry_run } => {
            let environment = profile::ProfileEnvironment::from_process()?;
            let profile = profile::resolve_profile(&command.profile, &environment)?;
            journal::sync(&profile, &environment, *dry_run)
        }
        ContextSubcommand::Share { file, message } => {
            let environment = profile::ProfileEnvironment::from_process()?;
            let profile = profile::resolve_profile(&command.profile, &environment)?;
            println!(
                "{}",
                journal::share(&profile, &environment, file.as_deref(), message.as_deref()).await?
            );
            Ok(())
        }
        ContextSubcommand::Artifact(subcommand) => {
            let environment = profile::ProfileEnvironment::from_process()?;
            let profile = profile::resolve_profile(&command.profile, &environment)?;
            match subcommand {
                crate::ContextArtifactSubcommand::Put { file } => {
                    artifact_sync::put(&profile, &environment, file).await
                }
                crate::ContextArtifactSubcommand::Get { sha256, out } => {
                    artifact_sync::get(&profile, &environment, sha256, out.as_deref()).await
                }
                crate::ContextArtifactSubcommand::Head { sha256 } => {
                    artifact_sync::head(&profile, &environment, sha256).await
                }
                crate::ContextArtifactSubcommand::Announce { file, message } => {
                    artifact_sync::announce(&profile, &environment, file, message.as_deref())
                        .await?;
                    Ok(())
                }
                crate::ContextArtifactSubcommand::Sync { dry_run, json } => {
                    let report = artifact_sync::sync(&profile, &environment, *dry_run).await?;
                    artifact_sync::print_report(&report, *json)
                }
            }
        }
        ContextSubcommand::Handoff(subcommand) => {
            let environment = profile::ProfileEnvironment::from_process()?;
            let profile = profile::resolve_profile(&command.profile, &environment)?;
            match subcommand {
                crate::ContextHandoffSubcommand::Open { spec } => {
                    handoff::open(&profile, &environment, spec).await?;
                    Ok(())
                }
                crate::ContextHandoffSubcommand::Claim { open_id, note } => {
                    handoff::claim(&profile, &environment, open_id, note.as_deref()).await?;
                    Ok(())
                }
                crate::ContextHandoffSubcommand::Return { open_id, spec } => {
                    handoff::return_handoff(&profile, &environment, open_id, spec).await?;
                    Ok(())
                }
                crate::ContextHandoffSubcommand::Close {
                    open_id,
                    return_id,
                    note,
                } => {
                    handoff::close(&profile, &environment, open_id, return_id, note.as_deref())
                        .await?;
                    Ok(())
                }
                crate::ContextHandoffSubcommand::VerifyArtifacts { return_id } => {
                    handoff::verify_artifacts(&profile, &environment, return_id).await
                }
                crate::ContextHandoffSubcommand::List { json } => {
                    handoff::list(&profile, &environment, *json).await
                }
            }
        }
        ContextSubcommand::Agreement(subcommand) => {
            let environment = profile::ProfileEnvironment::from_process()?;
            let profile = profile::resolve_profile(&command.profile, &environment)?;
            match subcommand {
                crate::ContextAgreementSubcommand::Export { stream, readers } => {
                    agreement::export(&profile, &environment, stream, readers).await?;
                }
                crate::ContextAgreementSubcommand::Admit {
                    source,
                    principal,
                    verification_keys,
                } => {
                    agreement::admit(&profile, &environment, source, principal, verification_keys)
                        .await?;
                }
                crate::ContextAgreementSubcommand::Read {
                    stream,
                    principal,
                    reader_keys,
                } => {
                    agreement::read(&profile, &environment, stream, principal, reader_keys).await?;
                }
                crate::ContextAgreementSubcommand::Steward {
                    principal,
                    steward_pubkey,
                } => {
                    agreement::steward(&profile, &environment, principal, steward_pubkey).await?;
                }
                crate::ContextAgreementSubcommand::Match { stream } => {
                    agreement::match_stream(&profile, &environment, stream).await?;
                }
                crate::ContextAgreementSubcommand::List { json } => {
                    agreement::list(&profile, &environment, *json).await?;
                }
            }
            Ok(())
        }
        ContextSubcommand::Status { node, json } => {
            let environment = profile::ProfileEnvironment::from_process()?;
            let profile = profile::resolve_profile(&command.profile, &environment)?;
            agreement::status_view(&profile, &environment, node.as_deref(), *json).await
        }
        ContextSubcommand::Pulse { relay, json } => {
            let environment = profile::ProfileEnvironment::from_process()?;
            let profile = profile::resolve_profile(&command.profile, &environment)?;
            observation::pulse(&profile, &environment, relay.as_deref(), *json).await
        }
        ContextSubcommand::Graph { format } => {
            let environment = profile::ProfileEnvironment::from_process()?;
            let profile = profile::resolve_profile(&command.profile, &environment)?;
            observation::graph(&profile, &environment, *format).await
        }
    }
}
