//! Thin client for the relay's CAS-backed default-branch operation.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::{client::BuzzClient, error::CliError, ReposDefaultBranchCmd};

#[derive(Deserialize)]
struct DefaultBranch {
    head: String,
    manifest: String,
}

fn parse_snapshot(raw: &str) -> Result<Value, CliError> {
    let value: Value = serde_json::from_str(raw).map_err(|_| {
        CliError::Other("relay did not return default-branch JSON; it may need updating".into())
    })?;
    let snapshot: DefaultBranch = serde_json::from_value(value.clone()).map_err(|_| {
        CliError::Other(
            "relay response is missing default-branch state; it may need updating".into(),
        )
    })?;
    crate::validate::validate_hex64(&snapshot.manifest)?;
    if !snapshot.head.starts_with("refs/") {
        return Err(CliError::Other("relay returned an invalid HEAD".into()));
    }
    Ok(value)
}

fn classify(error: CliError) -> CliError {
    match error {
        CliError::Relay { status: 409, body } => CliError::Conflict(body),
        CliError::Relay { status: 401 | 403, body } => CliError::Auth(body),
        CliError::Relay { status: 400, body } => CliError::Usage(body),
        CliError::Relay { status: 404, body } => CliError::NotFound(format!("{body}; check repository access and that this relay supports default-branch management")),
        other => other,
    }
}

pub(super) async fn dispatch(
    command: ReposDefaultBranchCmd,
    client: &BuzzClient,
) -> Result<(), CliError> {
    let (id, owner, update) = match command {
        ReposDefaultBranchCmd::Get { id, owner } => (id, owner, None),
        ReposDefaultBranchCmd::Set {
            id,
            owner,
            branch,
            expected_manifest,
        } => (id, owner, Some((branch, expected_manifest))),
    };
    crate::validate::validate_repo_id(&id)?;
    let owner = owner.unwrap_or_else(|| client.keys().public_key().to_hex());
    crate::validate::validate_hex64(&owner)?;
    let path = format!("/git/{owner}/{id}/default-branch");
    let result = match update {
        None => parse_snapshot(&client.get_authed(&path).await.map_err(classify)?)?,
        Some((branch, expected)) => {
            let expected = match expected {
                Some(digest) => {
                    crate::validate::validate_hex64(&digest)?;
                    digest
                }
                None => {
                    let raw = client.get_authed(&path).await.map_err(classify)?;
                    let snapshot = parse_snapshot(&raw)?;
                    snapshot["manifest"]
                        .as_str()
                        .ok_or_else(|| CliError::Other("missing manifest".into()))?
                        .to_string()
                }
            };
            let uncertain = |detail: String| {
                CliError::DeliveryUnknown(format!(
                "{detail}; attempted branch {branch:?} against manifest {expected}. Read the current default branch before deciding what to do; do not blindly re-run set without --expected-manifest {expected}"
            ))
            };
            let raw = client
                .post_json_once_authed(
                    &path,
                    &json!({"branch": branch, "expected_manifest": expected}),
                )
                .await
                .map_err(|e| match e {
                    CliError::DeliveryUnknown(detail) => uncertain(detail),
                    other => classify(other),
                })?;
            let result = parse_snapshot(&raw).map_err(|e| uncertain(e.to_string()))?;
            if !result["changed"].is_boolean() {
                return Err(uncertain(
                    "relay did not confirm the default-branch update".into(),
                ));
            }
            result
        }
    };
    println!("{result}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, Bytes},
        http::{HeaderMap, Response, StatusCode},
        routing::get,
        Router,
    };
    use base64::Engine;
    use clap::Parser;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn default_branch_cli_parses_get_and_set() {
        for operation in [
            vec!["get", "--id", "demo"],
            vec![
                "set",
                "--id",
                "demo",
                "--branch",
                "release/v1",
                "--expected-manifest",
                &"a".repeat(64),
            ],
        ] {
            let mut args = vec!["buzz", "repos", "default-branch"];
            args.extend(operation);
            assert!(crate::Cli::try_parse_from(args).is_ok());
        }
        assert!(crate::Cli::try_parse_from([
            "buzz",
            "repos",
            "default-branch",
            "set",
            "--id",
            "demo"
        ])
        .is_err());
    }

    #[tokio::test]
    async fn default_branch_command_binds_observed_digest_and_does_not_retry_or_follow_redirects() {
        for status in [200u16, 307, 308, 409, 500, 502, 503, 504] {
            let posts = Arc::new(AtomicUsize::new(0));
            let gets = Arc::new(AtomicUsize::new(0));
            let captured = Arc::new(Mutex::new(None));
            let keys = nostr::Keys::generate();
            let owner = keys.public_key().to_hex();
            let path = format!("/git/{owner}/demo/default-branch");
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let expected_url = format!("{url}{path}");
            let get_count = gets.clone();
            let post_count = posts.clone();
            let capture = captured.clone();
            let route = get(move || {
                get_count.fetch_add(1, Ordering::SeqCst);
                async { axum::Json(json!({"head":"refs/heads/legacy", "branch":"legacy", "manifest":"a".repeat(64)})) }
            }).post(move |headers: HeaderMap, body: Bytes| {
                let post_count = post_count.clone();
                let capture = capture.clone();
                let expected_url = expected_url.clone();
                async move {
                    post_count.fetch_add(1, Ordering::SeqCst);
                    let auth = headers["authorization"].to_str().unwrap().strip_prefix("Nostr ").unwrap();
                    let event = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(auth).unwrap()).unwrap();
                    let event: nostr::Event = serde_json::from_str(&event).unwrap();
                    event.verify().unwrap();
                    assert!(event.tags.iter().any(|t| t.as_slice() == ["u", &expected_url]));
                    assert!(event.tags.iter().any(|t| t.as_slice() == ["method", "POST"]));
                    use sha2::Digest;
                    let digest = hex::encode(sha2::Sha256::digest(&body));
                    assert!(event.tags.iter().any(|t| t.as_slice() == ["payload", &digest]));
                    *capture.lock().unwrap() = Some(serde_json::from_slice::<Value>(&body).unwrap());
                    Response::builder().status(status).header("location", "/redirect-target")
                        .body(Body::from(if status == 200 { json!({"head":"refs/heads/main", "branch":"main", "manifest":"b".repeat(64), "changed":true}).to_string() } else { json!({"error":"test outcome"}).to_string() })).unwrap()
                }
            });
            let redirected = posts.clone();
            let app = Router::new().route(&path, route).route(
                "/redirect-target",
                axum::routing::post(move || {
                    redirected.fetch_add(1, Ordering::SeqCst);
                    async { StatusCode::OK }
                }),
            );
            let server = tokio::spawn(async { axum::serve(listener, app).await.unwrap() });
            let client = BuzzClient::new(url, keys, None, None).unwrap();
            let result = crate::commands::repos::dispatch(
                crate::ReposCmd::DefaultBranch(ReposDefaultBranchCmd::Set {
                    id: "demo".into(),
                    owner: None,
                    branch: "main".into(),
                    expected_manifest: None,
                }),
                &client,
            )
            .await;
            assert_eq!(gets.load(Ordering::SeqCst), 1);
            assert_eq!(
                posts.load(Ordering::SeqCst),
                1,
                "HTTP {status} must not cause another POST"
            );
            assert_eq!(
                *captured.lock().unwrap(),
                Some(json!({"branch":"main", "expected_manifest":"a".repeat(64)}))
            );
            match status {
                200 => assert!(result.is_ok(), "{result:?}"),
                409 => assert!(matches!(result, Err(CliError::Conflict(_)))),
                _ => {
                    let error = result.unwrap_err();
                    assert!(matches!(error, CliError::DeliveryUnknown(_)), "{error}");
                    assert!(!crate::error::is_retryable_error(&error));
                    assert!(error.to_string().contains(&"a".repeat(64)));
                }
            }
            server.abort();
        }
    }
}
