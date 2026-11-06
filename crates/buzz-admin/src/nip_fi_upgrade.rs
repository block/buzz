//! One-shot calls to the existing operator upgrade API; no database or worker startup.

use std::{io::Read, path::Path, path::PathBuf, time::Duration};

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

const PATH: &str = "/operator/communities/nip-fi-upgrade";
const MAX_BYTES: usize = 64 * 1024;

/// Explicit operator destination and protected signing material.
#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Must match the relay's RELAY_OPERATOR_API_ORIGIN (HTTP for a trusted mesh, or HTTPS).
    #[arg(long)]
    origin: String,
    /// File containing the authorized deployment operator's hex secret key or nsec.
    #[arg(long)]
    key_file: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Accept or recover an upgrade using the exact saved request tuple.
    Enable {
        /// JSON with operation_id, community_id, expected_host, actor_pubkey,
        /// expected_owner_pubkey and expected_issuer. Reuse after response loss.
        #[arg(long)]
        request: PathBuf,
    },
    /// Read the original operator's durable receipt without sending a mutation.
    Status {
        #[arg(long)]
        operation_id: Uuid,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct UpgradeRequest {
    operation_id: Uuid,
    community_id: Uuid,
    expected_host: String,
    actor_pubkey: String,
    expected_owner_pubkey: String,
    expected_issuer: String,
}

impl UpgradeRequest {
    fn validate(&self) -> Result<()> {
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

fn endpoint(origin: &str, operation: Option<Uuid>) -> Result<Url> {
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
    let mut url = base.join(PATH)?;
    if let Some(id) = operation {
        ensure!(!id.is_nil(), "operation_id must not be nil");
        url.query_pairs_mut()
            .append_pair("operation_id", &id.to_string());
    }
    Ok(url)
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

fn receipt_exit_code(
    receipt: &Value,
    operation_id: Uuid,
    operator: &str,
    request: Option<&UpgradeRequest>,
) -> Result<i32> {
    ensure!(
        receipt["operation_id"].as_str() == Some(operation_id.to_string().as_str())
            && receipt["operator_pubkey"].as_str() == Some(operator)
            && receipt["requires_nip_fi"].as_bool() == Some(true)
            && receipt["policy_generation"].as_i64().is_some_and(|g| g > 0),
        "unconfirmed: response is not the expected durable acceptance receipt"
    );
    if let Some(request) = request {
        let expected = serde_json::to_value(request)?;
        ensure!(
            expected
                .as_object()
                .context("request must be an object")?
                .iter()
                .all(|(k, v)| receipt[k] == *v),
            "unconfirmed: receipt tuple does not match the saved request"
        );
    }
    let reasons = receipt["pending_reasons"]
        .as_array()
        .context("unconfirmed: missing pending reasons")?;
    ensure!(
        reasons.iter().all(Value::is_string),
        "unconfirmed: invalid pending reasons"
    );
    match receipt["state"].as_str() {
        Some("active") if reasons.is_empty() => Ok(0),
        Some("pending") => Ok(10),
        Some("unavailable") => Ok(12),
        _ => anyhow::bail!("unconfirmed: invalid upgrade state"),
    }
}

/// Send exactly one signed request. A timeout never means the upgrade was rejected.
pub async fn run(args: Args) -> Result<i32> {
    let request = match &args.command {
        Command::Enable { request } => {
            let request: UpgradeRequest =
                serde_json::from_slice(&read_bounded(request, MAX_BYTES)?)?;
            request.validate()?;
            Some(request)
        }
        Command::Status { .. } => None,
    };
    let operation_id = match &args.command {
        Command::Enable { .. } => request.as_ref().context("missing request")?.operation_id,
        Command::Status { operation_id } => *operation_id,
    };
    let url = endpoint(&args.origin, request.is_none().then_some(operation_id))?;
    // Do not include the secret or its parse error in diagnostics.
    let secret = read_bounded(&args.key_file, 1024)?;
    let keys = std::str::from_utf8(&secret)
        .ok()
        .and_then(|s| Keys::parse(s.trim()).ok())
        .context("invalid operator key file")?;
    let body = request.as_ref().map(serde_json::to_vec).transpose()?;
    let method = if body.is_some() {
        Method::POST
    } else {
        Method::GET
    };
    let auth = authorization(&keys, &method, &url, body.as_deref())?;
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
        anyhow::bail!("operator API returned HTTP {} ({detail}); recover the same operation_id after an uncertain enable", status.as_u16());
    }
    // Print only structured relay output, never transport headers or signing material.
    let receipt: Value = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "unconfirmed: invalid JSON response (HTTP {})",
            status.as_u16()
        )
    })?;
    let code = receipt_exit_code(
        &receipt,
        operation_id,
        &keys.public_key().to_hex(),
        request.as_ref(),
    )?;
    println!("{}", serde_json::to_string(&receipt)?);
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Bytes, http::HeaderMap, routing::any, Json, Router};
    use tokio::sync::mpsc;

    fn tuple() -> UpgradeRequest {
        let owner = Keys::generate().public_key().to_hex();
        UpgradeRequest {
            operation_id: Uuid::new_v4(),
            community_id: Uuid::new_v4(),
            expected_host: "test.communities.example.org".into(),
            actor_pubkey: owner.clone(),
            expected_owner_pubkey: owner,
            expected_issuer: "https://issuer.example.org".into(),
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
            let mut input = serde_json::to_value(tuple()).unwrap();
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
            std::fs::write(&request_file, serde_json::to_vec(&tuple()).unwrap()).unwrap();
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

    fn receipt(request: &UpgradeRequest, keys: &Keys, state: &str) -> Value {
        let mut value = serde_json::to_value(request).unwrap();
        value["operator_pubkey"] = keys.public_key().to_hex().into();
        value["policy_generation"] = 1.into();
        value["requires_nip_fi"] = true.into();
        value["state"] = state.into();
        value["pending_reasons"] = serde_json::json!([]);
        value
    }

    #[tokio::test]
    async fn command_signs_exact_post_body_and_get_query_and_preserves_operation() {
        let keys = Keys::generate();
        let tuple = tuple();
        let response = receipt(&tuple, &keys, "pending");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (sender, mut requests) = mpsc::channel(4);
        let app = Router::new().route(
            PATH,
            any(
                move |method: Method, uri: axum::http::Uri, headers: HeaderMap, body: Bytes| {
                    let response = response.clone();
                    let sender = sender.clone();
                    async move {
                        sender.send((method, uri, headers, body)).await.unwrap();
                        Json(response)
                    }
                },
            ),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join("key");
        let request_file = dir.path().join("request.json");
        std::fs::write(&key_file, keys.secret_key().display_secret().to_string()).unwrap();
        std::fs::write(&request_file, serde_json::to_vec(&tuple).unwrap()).unwrap();

        for command in [
            Command::Enable {
                request: request_file,
            },
            Command::Status {
                operation_id: tuple.operation_id,
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
                10
            );
            let (method, uri, headers, body) = requests.recv().await.unwrap();
            let encoded = headers["Authorization"]
                .to_str()
                .unwrap()
                .strip_prefix("Nostr ")
                .unwrap();
            let event = String::from_utf8(STANDARD.decode(encoded).unwrap()).unwrap();
            let url = format!("{origin}{uri}");
            let pubkey = buzz_auth::verify_nip98_event(
                &event,
                &url,
                method.as_str(),
                (method == Method::POST).then_some(body.as_ref()),
            )
            .unwrap();
            assert_eq!(pubkey, keys.public_key());
            if method == Method::POST {
                assert_eq!(
                    serde_json::from_slice::<Value>(&body).unwrap(),
                    serde_json::to_value(&tuple).unwrap()
                );
                assert!(
                    buzz_auth::verify_nip98_event(&event, &url, "POST", Some(b"changed")).is_err()
                );
            } else {
                assert_eq!(
                    uri.query(),
                    Some(format!("operation_id={}", tuple.operation_id).as_str())
                );
                assert!(buzz_auth::verify_nip98_event(
                    &event,
                    &format!("{origin}{PATH}"),
                    "GET",
                    None
                )
                .is_err());
            }
        }
        assert!(requests.try_recv().is_err());
        server.abort();
    }

    #[test]
    fn only_matching_required_receipts_can_report_success() {
        let keys = Keys::generate();
        let request = tuple();
        let operator = keys.public_key().to_hex();
        for (state, code) in [("active", 0), ("pending", 10), ("unavailable", 12)] {
            assert_eq!(
                receipt_exit_code(
                    &receipt(&request, &keys, state),
                    request.operation_id,
                    &operator,
                    Some(&request)
                )
                .unwrap(),
                code
            );
        }
        for (field, bad) in [
            ("operation_id", Value::from(Uuid::new_v4().to_string())),
            (
                "operator_pubkey",
                Value::from(Keys::generate().public_key().to_hex()),
            ),
            ("community_id", Value::from(Uuid::new_v4().to_string())),
            ("requires_nip_fi", Value::from(false)),
            ("policy_generation", Value::from(0)),
            ("state", Value::from("unknown")),
            ("pending_reasons", serde_json::json!(["not_drained"])),
        ] {
            let mut value = receipt(&request, &keys, "active");
            value[field] = bad;
            assert!(
                receipt_exit_code(&value, request.operation_id, &operator, Some(&request)).is_err(),
                "accepted invalid {field}"
            );
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
            assert!(endpoint(origin, None).is_err(), "accepted {origin}");
        }
    }
}
