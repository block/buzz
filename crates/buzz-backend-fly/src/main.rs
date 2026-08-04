mod config;
mod env;
mod fly;
mod wire;

use std::io::Read;
use wire::{Request, Response};

const RELAY_MESH_PROVIDER: &str = "relay-mesh";

fn main() {
    let mut input = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("could not read provider request: {error}");
        std::process::exit(1);
    }
    let response = respond(&input);
    println!(
        "{}",
        serde_json::to_string(&response).unwrap_or_else(|error| format!(
            r#"{{"ok":false,"error":"could not serialize provider response: {error}"}}"#
        ))
    );
}

fn respond(input: &str) -> Response {
    let raw: serde_json::Value = match serde_json::from_str(input) {
        Ok(value) => value,
        Err(error) => return Response::error(format!("request is not valid JSON: {error}")),
    };
    if let Some(provider) = raw
        .get("agent")
        .and_then(|agent| agent.get("provider"))
        .and_then(serde_json::Value::as_str)
    {
        if provider.trim() == RELAY_MESH_PROVIDER {
            return Response::error(
                "deploy refused: relay-mesh uses a desktop-local transport and cannot run on Fly.io",
            );
        }
    }
    let request: Request = match serde_json::from_value(raw) {
        Ok(request) => request,
        Err(error) => {
            return Response::error(format!("could not understand provider request: {error}"))
        }
    };
    match request {
        Request::Info => Response::info(),
        Request::Deploy(request) => match deploy(&request) {
            Ok(agent_id) => Response::deployed(agent_id),
            Err(error) => Response::error(error),
        },
    }
}

fn deploy(request: &wire::DeployRequest) -> Result<String, String> {
    let config = config::parse(&request.provider_config)?;
    let keys = nostr::Keys::parse(&request.agent.private_key_nsec).map_err(|_| {
        "deploy refused: private_key_nsec is not a valid Nostr secret key".to_string()
    })?;
    let pubkey = keys.public_key().to_hex();
    let app = format!("{}-{}", config.app_prefix, &pubkey[..16]);

    // Validate and resolve the full environment before creating paid Fly resources.
    let env = env::build_env(&request.agent, &config)?;

    let fly = fly::FlyCli::discover()?;
    fly.require_auth()?;
    fly.ensure_app(&app, &config.organization)?;
    let volume_id = fly.ensure_volume(&app, &config)?;
    fly.remove_legacy_mcp_secrets(&app)?;
    fly.import_secrets(&app, &env)?;
    let machine_id = fly.reconcile_machine(&app, &pubkey, &volume_id, &config)?;
    Ok(format!("{app}/{machine_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(response: Response) -> String {
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["ok"], false);
        value["error"].as_str().unwrap().to_string()
    }

    #[test]
    fn info_is_pure_and_has_a_schema() {
        let value = serde_json::to_value(respond(r#"{"op":"info"}"#)).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["name"], "fly");
        assert!(value["config_schema"]["properties"]["region"].is_object());
    }

    #[test]
    fn malformed_input_is_an_in_band_error() {
        assert!(error(respond("not-json")).contains("valid JSON"));
    }

    #[test]
    fn relay_mesh_is_refused_before_identity_or_fly_access() {
        let request = r#"{"op":"deploy","agent":{
            "relay_url":"wss://relay","private_key_nsec":"invalid","provider":" relay-mesh "
        }}"#;
        assert!(error(respond(request)).contains("relay-mesh"));
    }

    #[test]
    fn invalid_identity_is_refused_before_fly_access() {
        let request = r#"{"op":"deploy","agent":{
            "relay_url":"wss://relay","private_key_nsec":"invalid","auth_tag":"owner"
        }}"#;
        assert!(error(respond(request)).contains("valid Nostr secret key"));
    }
}
