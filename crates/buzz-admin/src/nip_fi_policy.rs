//! General stored community policy management with immutable saved intent.
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use clap::{Args as ClapArgs, Subcommand};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
use reqwest::{redirect::Policy, Method};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

const PATH: &str = "/operator/communities/nip-fi-policy";
const MAX_BYTES: usize = 64 * 1024;

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
        ensure!(
            !self.operation_id.is_nil() && !self.community_id.is_nil(),
            "UUIDs must not be nil"
        );
        let host = crate::communities::normalize_host_authority(&self.expected_host)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            host == self.expected_host && !host.contains(':'),
            "expected_host must be canonical lowercase without a port or trailing dot"
        );
        for key in [&self.actor_pubkey, &self.expected_owner_pubkey] {
            ensure!(
                key.len() == 64
                    && key
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
                "owner public keys must be 64 lowercase hex characters"
            );
        }
        Ok(())
    }
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .with_context(|| format!("cannot open {}", path.display()))?
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read {}", path.display()))?;
    ensure!(bytes.len() <= limit, "input file exceeds size limit");
    Ok(bytes)
}

fn endpoint(origin: &str) -> Result<Url> {
    let base = Url::parse(origin).context("invalid operator origin")?;
    ensure!(
        matches!(base.scheme(), "http" | "https")
            && base.host_str().is_some()
            && base.username().is_empty()
            && base.password().is_none()
            && base.path() == "/"
            && base.query().is_none()
            && base.fragment().is_none(),
        "operator origin must be an HTTP(S) origin without credentials, path, query or fragment"
    );
    Ok(base.join(PATH)?)
}

fn authorization(keys: &Keys, method: &Method, url: &Url, body: Option<&[u8]>) -> Result<String> {
    let mut tags = vec![
        Tag::parse(["u", url.as_str()])?,
        Tag::parse(["method", method.as_str()])?,
        Tag::parse(["nonce", &Uuid::new_v4().to_string()])?,
    ];
    if let Some(body) = body {
        tags.push(Tag::parse(["payload", &hex::encode(Sha256::digest(body))])?);
    }
    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags(tags)
        .sign_with_keys(keys)?;
    Ok(format!("Nostr {}", STANDARD.encode(event.as_json())))
}

/// Bounded, signed single call shared by the operator policy commands.
async fn signed_call(keys: &Keys, url: Url, body: Option<Vec<u8>>) -> Result<Value> {
    let method = if body.is_some() {
        Method::POST
    } else {
        Method::GET
    };
    let auth = authorization(keys, &method, &url, body.as_deref())?;
    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .timeout(Duration::from_secs(30))
        .build()?;
    let mut call = client.request(method, url).header("Authorization", auth);
    if let Some(body) = body {
        call = call.header("Content-Type", "application/json").body(body);
    }
    let mut response = call
        .send()
        .await
        .context("unconfirmed: request failed; recover the same operation_id")?;
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("unconfirmed: response interrupted; recover the same operation_id")?
    {
        ensure!(
            bytes.len() + chunk.len() <= MAX_BYTES,
            "unconfirmed: response exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        // Preserve known machine-readable codes, without echoing arbitrary response bodies.
        let error = serde_json::from_slice::<Value>(&bytes).ok();
        let code = error.as_ref().and_then(|value| value["code"].as_str());
        let detail = match code {
            Some(
                code @ ("invalid_request"
                | "community_not_found"
                | "operation_not_found"
                | "idempotency_conflict"
                | "policy_conflict"
                | "policy_generation_exhausted"
                | "host_conflict"
                | "owner_conflict"
                | "lifecycle_conflict"
                | "issuer_conflict"
                | "upgrades_disabled"
                | "authorization_unavailable"),
            ) => code,
            _ if error.is_none() => "non-JSON error response",
            _ => "unrecognized error code",
        };
        anyhow::bail!("operator API returned HTTP {} ({detail}); on policy_conflict re-read policy and use a new UUID; after response loss recover the same saved request and UUID", status.as_u16());
    }
    // Print only structured relay output, never transport headers or signing material.
    let receipt: Value = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "unconfirmed: invalid JSON response (HTTP {})",
            status.as_u16()
        )
    })?;
    Ok(receipt)
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
    let mut url = endpoint(&args.origin)?;
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
        .map_or(&value["expected_requires_nip_fi"], |r| {
            &r["current_requires_nip_fi"]
        });
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
    use axum::{routing::any, Json, Router};

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

    #[derive(Clone)]
    struct TestPolicy {
        origin: String,
        response: Value,
        expected_body: Value,
        host: String,
        community_id: Uuid,
        operation_id: Uuid,
        desired: bool,
    }

    async fn policy_test_handler(
        axum::extract::State(policy): axum::extract::State<TestPolicy>,
        method: Method,
        uri: axum::http::Uri,
        headers: axum::http::HeaderMap,
        body: axum::body::Bytes,
    ) -> Json<Value> {
        let encoded = headers["authorization"]
            .to_str()
            .unwrap()
            .strip_prefix("Nostr ")
            .unwrap();
        let event = String::from_utf8(STANDARD.decode(encoded).unwrap()).unwrap();
        let url = format!("{}{uri}", policy.origin);
        buzz_auth::verify_nip98_event(
            &event,
            &url,
            method.as_str(),
            (method == Method::POST).then_some(body.as_ref()),
        )
        .unwrap();
        if method == Method::POST {
            assert_eq!(
                serde_json::from_slice::<Value>(&body).unwrap(),
                policy.expected_body
            );
            assert!(buzz_auth::verify_nip98_event(&event, &url, "POST", Some(b"changed")).is_err());
        } else if uri.query().unwrap().starts_with("community_id=") {
            assert_eq!(
                uri.query(),
                Some(
                    format!(
                        "community_id={}&expected_host={}",
                        policy.community_id, policy.host
                    )
                    .as_str()
                )
            );
            return Json(serde_json::json!({
                "community_id": policy.community_id, "expected_host": policy.host,
                "expected_policy_generation": 1, "expected_requires_nip_fi": policy.desired,
                "mode": "shadow", "effective_required": policy.desired,
            }));
        } else {
            assert_eq!(
                uri.query(),
                Some(format!("operation_id={}", policy.operation_id).as_str())
            );
            assert!(buzz_auth::verify_nip98_event(
                &event,
                &format!("{}{PATH}", policy.origin),
                "GET",
                None
            )
            .is_err());
        }
        Json(policy.response)
    }

    #[tokio::test]
    async fn policy_commands_send_saved_action_and_recover_current_and_history() {
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
            let app = Router::new()
                .route(PATH, any(policy_test_handler))
                .with_state(TestPolicy {
                    origin: origin.clone(),
                    response,
                    expected_body,
                    host,
                    community_id,
                    operation_id: id,
                    desired,
                });
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
    #[tokio::test]
    async fn refuses_noncanonical_inputs_before_reading_key_or_sending_post() {
        for (field, value) in [
            ("expected_host", "TEST.communities.example.org"),
            ("expected_host", "test.communities.example.org."),
            ("expected_host", "test.communities.example.org:443"),
            ("expected_host", "https://test.communities.example.org"),
            ("actor_pubkey", &"A".repeat(64)),
            ("expected_owner_pubkey", &"A".repeat(64)),
            ("actor_pubkey", &"a".repeat(63)),
            ("expected_owner_pubkey", &"g".repeat(64)),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("request.json");
            let mut input = serde_json::to_value(saved(true)).unwrap();
            input[field] = value.into();
            std::fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();
            let error = run(Args {
                origin: "http://127.0.0.1:1".into(),
                key_file: dir.path().join("absent-key"),
                command: Command::Enable { request: path },
            })
            .await
            .unwrap_err();
            let message = format!("{error:#}");
            assert!(
                message.contains("host") || message.contains("public keys"),
                "{message}"
            );
            assert!(
                !message.contains("absent-key"),
                "key read preceded validation: {message}"
            );
        }
    }

    #[tokio::test]
    async fn error_responses_preserve_status_and_known_codes_without_echoing_bodies() {
        use axum::http::StatusCode;

        for (status, body, expected) in [
            (
                409,
                r#"{"code":"owner_conflict","error":"private detail"}"#,
                "owner_conflict",
            ),
            (409, r#"{"code":"issuer_conflict"}"#, "issuer_conflict"),
            (409, r#"{"code":"host_conflict"}"#, "host_conflict"),
            (
                409,
                r#"{"code":"idempotency_conflict"}"#,
                "idempotency_conflict",
            ),
            (503, r#"{"code":"upgrades_disabled"}"#, "upgrades_disabled"),
            (
                502,
                "<html>private detail</html>",
                "non-JSON error response",
            ),
            (
                409,
                r#"{"code":"private detail"}"#,
                "unrecognized error code",
            ),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let app = Router::new().route(
                PATH,
                any(move || async move { (StatusCode::from_u16(status).unwrap(), body) }),
            );
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let dir = tempfile::tempdir().unwrap();
            let key_file = dir.path().join("key");
            let request_file = dir.path().join("request.json");
            let keys = Keys::generate();
            std::fs::write(&key_file, keys.secret_key().display_secret().to_string()).unwrap();
            std::fs::write(&request_file, serde_json::to_vec(&saved(true)).unwrap()).unwrap();
            let error = run(Args {
                origin,
                key_file,
                command: Command::Enable {
                    request: request_file,
                },
            })
            .await
            .unwrap_err();
            let message = format!("{error:#}");
            assert!(message.contains(&format!("HTTP {status}")), "{message}");
            assert!(message.contains(expected), "{message}");
            assert!(!message.contains("private detail"), "{message}");
            assert!(!message.contains(&keys.secret_key().display_secret().to_string()));
            server.abort();
        }
    }

    #[test]
    fn origin_cannot_select_a_different_path_or_embed_credentials() {
        for origin in [
            "https://user:password@example.org",
            "https://example.org/wrong",
            "https://example.org/?q=x",
            "https://example.org/#fragment",
            "file:///tmp/key",
        ] {
            assert!(endpoint(origin).is_err(), "accepted {origin}");
        }
    }
}
