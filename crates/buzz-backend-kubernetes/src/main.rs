//! Kubernetes backend provider for Buzz remote agents
//! (spec `docs/remote-agents.md`).
//!
//! One process per operation: read exactly one JSON request from stdin, write
//! exactly one JSON response to stdout, exit. The exit code carries exactly
//! one bit — 0 for a response that was produced, 1 for a failure to produce
//! one. Everything a caller needs to distinguish is *inside* the response's
//! `ok` field, because a provider that encoded outcomes in exit codes would
//! have a second, redundant error channel to keep in sync (§Provider Protocol).

mod classify;
mod client;
mod cluster;
mod config;
mod env;
mod gc;
mod image;
mod intent;
mod naming;
mod observe;
mod pod;
mod reconcile;
mod wire;

use std::io::Read;
use wire::{Request, Response};

/// The provider a shared-compute agent resolves to. Refused here as the
/// spec's backstop: a mesh agent runs on the relay's compute, so deploying it
/// as a pod would create a second, contending consumer of the same agent
/// identity (`:214-219`).
const RELAY_MESH_PROVIDER: &str = "relay-mesh";

fn main() {
    // rustls needs a process-level provider before the first TLS connection.
    // The release build compiles every sidecar in one cargo invocation, which
    // unifies the `ring` and `aws-lc-rs` features and leaves rustls unable to
    // auto-select — so this is an explicit install, not a default.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut input = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut input) {
        // No request means no request_id and no response contract to honor.
        // This is the one path that exits nonzero.
        eprintln!("could not read the request from stdin: {e}");
        std::process::exit(1);
    }

    let response = respond(&input);
    println!(
        "{}",
        serde_json::to_string(&response).unwrap_or_else(|e| {
            // The response types are plain data; this cannot fail in practice,
            // and a hand-built object is still a conforming response.
            format!(r#"{{"ok":false,"error":"could not serialize a response: {e}"}}"#)
        })
    );
}

/// Produce the single response for one request. Separated from `main` so the
/// whole dispatch is testable without a process.
fn respond(input: &str) -> Response {
    // Parsed as raw JSON first: the relay-mesh refusal below MUST see the wire
    // value, and `AgentPayload` deliberately does not carry `provider`.
    let raw: serde_json::Value = match serde_json::from_str(input) {
        Ok(value) => value,
        Err(e) => return Response::error(format!("request is not valid JSON: {e}")),
    };

    if let Some(refusal) = refuse_relay_mesh(&raw) {
        return Response::error(refusal);
    }

    let request: Request = match serde_json::from_value(raw) {
        Ok(request) => request,
        Err(e) => return Response::error(format!("could not understand the request: {e}")),
    };

    match request {
        Request::Info => Response::info(),
        Request::Deploy(deploy) => {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => return Response::error(format!("could not start the runtime: {e}")),
            };
            match runtime.block_on(deploy_agent(&deploy)) {
                Ok(agent_id) => Response::deployed(agent_id),
                Err(e) => Response::error(e),
            }
        }
    }
}

/// Refuse a shared-compute agent, reading the **raw wire value**.
///
/// Trimmed before comparing: the desktop's own layers disagree about padding
/// (`relay_mesh.rs:17` and `effective_config/mod.rs:46` trim; the deploy guard
/// at `agents_deploy.rs:116` did not), and `non_blank` preserves surrounding
/// whitespace on a non-blank value. A backstop that shares its bypass with the
/// layer it backs is not a backstop.
fn refuse_relay_mesh(raw: &serde_json::Value) -> Option<String> {
    let provider = raw.get("agent")?.get("provider")?.as_str()?;
    (provider.trim() == RELAY_MESH_PROVIDER).then(|| {
        "deploy refused: this agent is configured for shared compute \
         (relay-mesh), which runs on the relay rather than in a pod. \
         Switch the agent to a local runtime before deploying it to \
         Kubernetes."
            .to_string()
    })
}

/// Run one deploy to a terminal outcome.
async fn deploy_agent(request: &wire::DeployRequest) -> Result<String, String> {
    let cfg = config::parse(&request.provider_config)?;
    // Identity before any cluster contact: a malformed nsec is a refusal, not
    // a failed connection (§Deploy State Machine step 0).
    let identity = naming::AgentIdentity::from_nsec(&request.agent.private_key_nsec)?;

    // One generation for this operation's first attempt; the reconciler mints
    // its own per attempt and restamps the correlator to match.
    let env = env::build_env(
        &request.agent,
        env::AuthoritativeInputs {
            generation: &naming::new_generation(),
            inactivity_seconds: cfg.inactivity_seconds,
        },
    )?;

    let client = client::connect(cfg.context.as_deref()).await?;
    let substrate = cluster::Cluster::new(client, &cfg.namespace);
    reconcile::deploy(&substrate, &identity, &cfg, env).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error_of(response: &Response) -> String {
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["ok"], false, "expected a refusal: {json}");
        json["error"].as_str().unwrap().to_string()
    }

    /// The spec's backstop for the relay-mesh MUST. The desktop refuses first
    /// (`agents_deploy.rs:116`); this is the layer that owes the obligation.
    #[test]
    fn refuses_a_relay_mesh_agent() {
        let request = r#"{"op":"deploy","agent":{
            "relay_url":"wss://r","private_key_nsec":"nsec1x","provider":"relay-mesh"},
            "provider_config":{"namespace":"ns"}}"#;
        assert!(error_of(&respond(request)).contains("relay-mesh"));
    }

    /// Padding must not bypass the backstop. Reachable by construction:
    /// `GlobalConfig.provider` is a bare `Option<String>` with no trim on
    /// write, and `non_blank` rejects whitespace-only while preserving
    /// surrounding whitespace on everything else.
    #[test]
    fn refuses_a_padded_relay_mesh_agent() {
        let request = r#"{"op":"deploy","agent":{
            "relay_url":"wss://r","private_key_nsec":"nsec1x","provider":"  relay-mesh  "},
            "provider_config":{"namespace":"ns"}}"#;
        assert!(error_of(&respond(request)).contains("relay-mesh"));
    }

    /// The refusal must not fire on a normal agent — a guard that refuses
    /// everything passes its own test and ships a provider that deploys
    /// nothing.
    #[test]
    fn does_not_refuse_a_normal_provider() {
        let raw: serde_json::Value =
            serde_json::from_str(r#"{"agent":{"provider":"openai"}}"#).unwrap();
        assert!(refuse_relay_mesh(&raw).is_none());
        // …nor when the field is absent entirely, which is the common case:
        // `AgentPayload` does not carry `provider`.
        let bare: serde_json::Value = serde_json::from_str(r#"{"agent":{}}"#).unwrap();
        assert!(refuse_relay_mesh(&bare).is_none());
    }

    /// Malformed input still produces exactly one conforming response.
    #[test]
    fn malformed_input_is_an_in_band_error() {
        assert!(error_of(&respond("not json")).contains("valid JSON"));
        assert!(error_of(&respond(r#"{"op":"undeploy"}"#)).contains("understand"));
    }

    /// `info` answers without touching a cluster — it is what the desktop
    /// calls to render the config form, before any kubeconfig exists.
    #[test]
    fn info_answers_with_the_protocol_version_and_schema() {
        let json = serde_json::to_value(respond(r#"{"op":"info"}"#)).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["protocol_version"], wire::PROTOCOL_VERSION);
        assert!(json["config_schema"]["properties"]["namespace"].is_object());
    }
}
