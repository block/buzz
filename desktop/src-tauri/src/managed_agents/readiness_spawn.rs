//! Spawn-time readiness snapshot.
//!
//! `buzz-acp/setup_mode.rs` snapshots the readiness payload at spawn time
//! (into `BUZZ_ACP_SETUP_PAYLOAD`) and explicitly never re-derives it, so a
//! single transient non-zero CLI-login probe on startup traps the child in
//! setup-listener mode for the process's lifetime. The Fizz Air incident
//! (2026-08-23) hit this: `claude auth status` returned `loggedIn=false`
//! for a sub-second window while the credential store was refreshing, the
//! desktop froze that payload, and the harness kept serving a config-nudge
//! for the rest of its life even though `auth status` was green seconds
//! later.
//!
//! [`agent_readiness_for_spawn`] runs the ordinary single-shot readiness
//! ([`agent_readiness`]) first, then post-passes any surviving
//! `Requirement::CliLogin { availability: Available, .. }` through
//! `cli_probe::login_probe_with_recheck` — bounded retry + one
//! authoritative final recheck. If the retry recovers, the requirement is
//! dropped from the returned readiness; if not, the requirement is
//! preserved unchanged.
//!
//! Only [`spawn_agent_child`](crate::managed_agents::runtime) calls this.
//! Every other `agent_readiness` caller (status polling, sidebar refresh,
//! config-change diff, background verification) stays on the single-shot
//! path so retry sleeps do not pile up ~1.75 s per logged-out row per
//! poll while holding transition/store locks.

use crate::managed_agents::{
    agent_readiness,
    discovery::resolve_command,
    readiness::{cli_probe, EffectiveAgentEnv},
    AcpAvailabilityStatus, AgentReadiness, Requirement,
};

/// Spawn-time readiness snapshot. Only [`spawn_agent_child`](crate::managed_agents::runtime)
/// calls this. See module docs for the load-bearing semantics.
pub(crate) fn agent_readiness_for_spawn(effective: &EffectiveAgentEnv) -> AgentReadiness {
    let single_shot = agent_readiness(effective);
    let AgentReadiness::NotReady { requirements } = single_shot else {
        return AgentReadiness::Ready;
    };
    let retried: Vec<Requirement> = requirements
        .into_iter()
        .filter_map(retry_cli_login_if_transient)
        .collect();
    if retried.is_empty() {
        AgentReadiness::Ready
    } else {
        AgentReadiness::NotReady {
            requirements: retried,
        }
    }
}

/// If `req` is a `CliLogin` requirement with `AcpAvailabilityStatus::Available`
/// (meaning the CLI is installed and the single-shot probe returned
/// `LoggedOut`), re-run the probe through
/// [`cli_probe::login_probe_with_recheck`]. If the retry sequence resolves
/// to `LoggedIn`, drop the requirement by returning `None`. Otherwise
/// preserve the original requirement.
///
/// Non-`CliLogin` requirements, and `CliLogin` requirements whose
/// `availability` is anything other than `Available` (meaning the probe
/// never ran because the adapter/CLI is missing), pass through unchanged
/// — retrying them would spend budget on cases the retry cannot fix.
fn retry_cli_login_if_transient(req: Requirement) -> Option<Requirement> {
    let Requirement::CliLogin {
        probe_args,
        setup_copy,
        availability,
    } = req
    else {
        return Some(req);
    };
    if availability != AcpAvailabilityStatus::Available {
        return Some(Requirement::CliLogin {
            probe_args,
            setup_copy,
            availability,
        });
    }
    let Some(binary_path) = probe_args.first().and_then(|arg| resolve_command(arg)) else {
        return Some(Requirement::CliLogin {
            probe_args,
            setup_copy,
            availability,
        });
    };
    let augmented_path = cli_probe::augmented_path();
    let args_refs: Vec<&str> = probe_args.iter().map(String::as_str).collect();
    match cli_probe::login_probe_with_recheck(&binary_path, &args_refs, augmented_path.as_deref()) {
        cli_probe::ProbeOutcome::LoggedIn => None,
        cli_probe::ProbeOutcome::LoggedOut | cli_probe::ProbeOutcome::ConfigInvalid { .. } => {
            Some(Requirement::CliLogin {
                probe_args,
                setup_copy,
                availability,
            })
        }
    }
}
