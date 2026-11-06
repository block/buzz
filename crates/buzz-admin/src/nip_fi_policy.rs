//! General stored community policy management with immutable saved intent.
use std::path::PathBuf;

use anyhow::{ensure, Context, Result};
use clap::{Args as ClapArgs, Subcommand};
use nostr::Keys;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::nip_fi_upgrade::{endpoint, read_bounded, signed_call, UpgradeRequest};

/// Explicit operator destination and signing key for community policy management.
#[derive(Debug, ClapArgs)]
pub struct Args {
    #[arg(long)]
    origin: String,
    #[arg(long)]
    key_file: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Apply a saved requires_nip_fi=true intent, or recover its acceptance.
    Enable {
        #[arg(long)]
        request: PathBuf,
    },
    /// Apply a saved requires_nip_fi=false intent, or recover its acceptance.
    Disable {
        #[arg(long)]
        request: PathBuf,
    },
    /// Read current preconditions by community/host, or a historical operation.
    Status {
        #[arg(long, conflicts_with_all = ["community_id", "host"])]
        operation_id: Option<Uuid>,
        #[arg(long, requires = "host")]
        community_id: Option<Uuid>,
        #[arg(long, requires = "community_id")]
        host: Option<String>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    operation_id: Uuid,
    community_id: Uuid,
    expected_host: String,
    actor_pubkey: String,
    expected_owner_pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expected_issuer: Option<String>,
    requires_nip_fi: bool,
    expected_policy_generation: i64,
    expected_requires_nip_fi: bool,
}

impl Request {
    fn validate(&self, desired: bool) -> Result<()> {
        ensure!(
            self.requires_nip_fi == desired,
            "saved requires_nip_fi does not match the command"
        );
        ensure!(
            self.expected_policy_generation >= 0,
            "generation must be nonnegative"
        );
        ensure!(
            if desired {
                self.expected_issuer.as_ref().is_some_and(|s| !s.is_empty())
            } else {
                self.expected_issuer.is_none()
            },
            "enable requires expected_issuer; disable must omit it"
        );
        UpgradeRequest {
            operation_id: self.operation_id,
            community_id: self.community_id,
            expected_host: self.expected_host.clone(),
            actor_pubkey: self.actor_pubkey.clone(),
            expected_owner_pubkey: self.expected_owner_pubkey.clone(),
            expected_issuer: self.expected_issuer.clone().unwrap_or_default(),
        }
        .validate()
    }
}

fn receipt_exit_code(
    value: &Value,
    id: Uuid,
    operator: &str,
    request: Option<&Request>,
) -> Result<i32> {
    let receipt = &value["receipt"];
    ensure!(
        receipt["operation_id"].as_str() == Some(id.to_string().as_str())
            && receipt["operator_pubkey"].as_str() == Some(operator)
            && receipt["policy_generation"].as_i64().is_some_and(|g| g > 0)
            && receipt["requires_nip_fi"].is_boolean(),
        "unconfirmed: invalid policy receipt"
    );
    if let Some(request) = request {
        let expected = serde_json::to_value(request)?;
        ensure!(
            expected
                .as_object()
                .context("invalid saved request")?
                .iter()
                .all(|(key, v)| receipt[key] == *v),
            "unconfirmed: receipt differs from saved intent"
        );
    }
    match (
        receipt["state"].as_str(),
        receipt["requires_nip_fi"].as_bool(),
    ) {
        (Some("applied"), Some(false)) => Ok(0),
        (Some("pending"), Some(true)) => Ok(10),
        (Some("superseded"), _) => Ok(11),
        (Some("unavailable"), _) => Ok(12),
        _ => anyhow::bail!("unconfirmed: invalid policy state"),
    }
}

/// Execute one signed policy call; never change or automatically retry saved intent.
pub async fn run(args: Args) -> Result<i32> {
    let request = match &args.command {
        Command::Enable { request } | Command::Disable { request } => {
            let saved: Request = serde_json::from_slice(&read_bounded(request, 64 * 1024)?)?;
            saved.validate(matches!(args.command, Command::Enable { .. }))?;
            Some(saved)
        }
        Command::Status { .. } => None,
    };
    let mut url = endpoint(&args.origin, None)?;
    url.set_path("/operator/communities/nip-fi-policy");
    if let Command::Status {
        operation_id,
        community_id,
        host,
    } = &args.command
    {
        match (operation_id, community_id, host) {
            (Some(id), None, None) if !id.is_nil() => {
                url.query_pairs_mut()
                    .append_pair("operation_id", &id.to_string());
            }
            (None, Some(id), Some(host)) if !id.is_nil() => {
                let canonical = crate::communities::normalize_host_authority(host)
                    .map_err(anyhow::Error::msg)?;
                ensure!(
                    canonical == *host && !host.contains(':'),
                    "host must be canonical lowercase without a port"
                );
                url.query_pairs_mut()
                    .append_pair("community_id", &id.to_string())
                    .append_pair("expected_host", host);
            }
            _ => anyhow::bail!("status requires operation-id or community-id and host"),
        }
    }
    // Validate the entire saved intent, including action, before touching the key.
    let secret = read_bounded(&args.key_file, 1024)?;
    let keys = std::str::from_utf8(&secret)
        .ok()
        .and_then(|s| Keys::parse(s.trim()).ok())
        .context("invalid operator key file")?;
    let value = signed_call(
        &keys,
        url,
        request.as_ref().map(serde_json::to_vec).transpose()?,
    )
    .await?;
    let code = if let Some(saved) = request.as_ref() {
        receipt_exit_code(
            &value,
            saved.operation_id,
            &keys.public_key().to_hex(),
            Some(saved),
        )?
    } else if let Command::Status {
        operation_id: Some(id),
        ..
    } = args.command
    {
        receipt_exit_code(&value, id, &keys.public_key().to_hex(), None)?
    } else {
        if let Command::Status {
            community_id: Some(id),
            host: Some(host),
            ..
        } = &args.command
        {
            ensure!(
                value["community_id"].as_str() == Some(id.to_string().as_str())
                    && value["expected_host"].as_str() == Some(host.as_str())
                    && value["expected_policy_generation"]
                        .as_i64()
                        .is_some_and(|g| g >= 0)
                    && value["expected_requires_nip_fi"].is_boolean(),
                "unconfirmed: invalid current policy"
            );
        }
        0
    };
    let stored = value
        .get("receipt")
        .map_or(&value["requires_nip_fi"], |r| &r["current_requires_nip_fi"]);
    if stored.is_boolean()
        && value["effective_required"].is_boolean()
        && *stored != value["effective_required"]
    {
        eprintln!("Global mode makes effective_required differ from the stored requirement; inspect mode in the response.");
    }
    println!("{}", serde_json::to_string(&value)?);
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(desired: bool) -> Request {
        Request {
            operation_id: Uuid::new_v4(),
            community_id: Uuid::new_v4(),
            expected_host: "team.communities.example.org".into(),
            actor_pubkey: "a".repeat(64),
            expected_owner_pubkey: "a".repeat(64),
            expected_issuer: desired.then(|| "https://issuer.example.org".into()),
            requires_nip_fi: desired,
            expected_policy_generation: 0,
            expected_requires_nip_fi: false,
        }
    }

    #[tokio::test]
    async fn immutable_action_and_preconditions_are_checked_before_key_read() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("request.json");
        for (mut request, change, expected) in [
            (saved(true), 0, "does not match"),
            (saved(false), 1, "generation"),
            (saved(false), 2, "omit"),
            (saved(false), 3, "canonical"),
        ] {
            match change {
                1 => request.expected_policy_generation = -1,
                2 => request.expected_issuer = Some("https://issuer.example.org".into()),
                3 => request.expected_host = "TEAM.communities.example.org".into(),
                _ => (),
            }
            std::fs::write(&file, serde_json::to_vec(&request).unwrap()).unwrap();
            let error = run(Args {
                origin: "http://127.0.0.1:1".into(),
                key_file: dir.path().join("absent-key"),
                command: Command::Disable {
                    request: file.clone(),
                },
            })
            .await
            .unwrap_err();
            assert!(error.to_string().contains(expected), "{error:#}");
        }
    }

    #[tokio::test]
    async fn policy_commands_send_saved_action_and_recover_current_and_history() {
        use axum::{
            body::Bytes,
            extract::RawQuery,
            http::{HeaderMap, Method},
            routing::any,
            Json, Router,
        };
        let keys = Keys::generate();
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join("key");
        let file = dir.path().join("request.json");
        std::fs::write(&key_file, keys.secret_key().display_secret().to_string()).unwrap();
        for desired in [true, false] {
            let saved = saved(desired);
            let id = saved.operation_id;
            let community_id = saved.community_id;
            let host = saved.expected_host.clone();
            let mut receipt = serde_json::to_value(&saved).unwrap();
            receipt["operator_pubkey"] = keys.public_key().to_hex().into();
            receipt["policy_generation"] = 1.into();
            receipt["current_requires_nip_fi"] = desired.into();
            receipt["state"] = if desired { "pending" } else { "applied" }.into();
            let response = serde_json::json!({"receipt": receipt, "mode": "shadow", "effective_required": desired});
            std::fs::write(&file, serde_json::to_vec(&saved).unwrap()).unwrap();
            let expected_body = serde_json::to_value(&saved).unwrap();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let app = Router::new().route("/operator/communities/nip-fi-policy", any(
                move |method: Method, RawQuery(query): RawQuery, headers: HeaderMap, body: Bytes| {
                    let response = response.clone(); let expected_body = expected_body.clone(); let host = host.clone();
                    async move {
                        assert!(headers["authorization"].to_str().unwrap().starts_with("Nostr "));
                        if method == Method::POST {
                            assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), expected_body);
                        } else if query.as_ref().unwrap().starts_with("community_id=") {
                            return Json(serde_json::json!({"community_id":community_id,"expected_host":host,
                                "expected_policy_generation":1,"expected_requires_nip_fi":desired,
                                "requires_nip_fi":desired,"mode":"shadow","effective_required":desired}));
                        } else { assert_eq!(query, Some(format!("operation_id={id}"))); }
                        Json(response)
                    }
                }));
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let command = if desired {
                Command::Enable {
                    request: file.clone(),
                }
            } else {
                Command::Disable {
                    request: file.clone(),
                }
            };
            for command in [
                command,
                Command::Status {
                    operation_id: Some(id),
                    community_id: None,
                    host: None,
                },
            ] {
                assert_eq!(
                    run(Args {
                        origin: origin.clone(),
                        key_file: key_file.clone(),
                        command
                    })
                    .await
                    .unwrap(),
                    if desired { 10 } else { 0 }
                );
            }
            assert_eq!(
                run(Args {
                    origin,
                    key_file: key_file.clone(),
                    command: Command::Status {
                        operation_id: None,
                        community_id: Some(community_id),
                        host: Some(saved.expected_host.clone()),
                    }
                })
                .await
                .unwrap(),
                0
            );
            server.abort();
            for (state, code) in [("superseded", 11), ("unavailable", 12)] {
                let value = serde_json::json!({"receipt": {"operation_id":id,"operator_pubkey":keys.public_key().to_hex(),"policy_generation":1,"requires_nip_fi":desired,"state":state}});
                assert_eq!(
                    receipt_exit_code(&value, id, &keys.public_key().to_hex(), None).unwrap(),
                    code
                );
            }
        }
    }
}
