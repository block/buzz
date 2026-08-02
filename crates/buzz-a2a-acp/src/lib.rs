#![forbid(unsafe_code)]

//! A small, protocol-faithful bridge from an AGNTCY/OASF Agent Record to ACP.
//!
//! The bridge is intentionally a subprocess. Buzz owns the ACP session and UI;
//! the source runtime owns its agent identity, context, execution, and keys.

use clap::Parser;
use reqwest::StatusCode;
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

mod a2a;
use a2a::parse_extensions_json;
#[cfg(test)]
use a2a::{
    extract_text, protocol_request, request_payload, task_outcome, validate_endpoint_binding,
    REQUEST_ID,
};
pub use a2a::{
    select_protocol_mode, AgentCapabilities, AgentCard, AgentExtension, ProtocolMode,
    ResolvedAgent, SupportedInterface,
};
mod acp_loop;
pub use acp_loop::run;
#[cfg(test)]
use acp_loop::{
    handle_acp_message, prompt_success, read_bounded_line, spawn_line_reader, AcpAction,
};
mod net;
#[cfg(test)]
use net::{is_private_ip, validate_resolved_addresses};
mod oasf;
#[cfg(test)]
use oasf::{descriptor_bytes, AgentRecord, Descriptor};

const MAX_RECORD_BYTES: usize = 2 * 1024 * 1024;
const MAX_ARTIFACT_BYTES: usize = 2 * 1024 * 1024;
const MAX_ACP_LINE_BYTES: usize = 1024 * 1024;
const MAX_EXTENSIONS_JSON_BYTES: usize = 64 * 1024;
const MAX_EXTENSIONS: usize = 32;
const MAX_EXTENSION_URI_BYTES: usize = 2 * 1024;
const DEFAULT_TASK_POLL_SECS: u64 = 7_200;
const TASK_POLL_BACKOFF_SECS: [u64; 4] = [1, 5, 15, 30];

/// Configuration supplied by Buzz's BYOH subprocess definition.
#[derive(Debug, Clone)]
pub struct AdapterConfig {
    /// Local path or HTTP(S) URL for an OASF Agent Record.
    pub record: String,
    /// Optional operator-supplied token for the A2A endpoint.
    pub bearer_token: Option<String>,
    /// Exact endpoint where the operator permits the bearer token to be sent.
    pub bearer_token_endpoint: Option<String>,
    /// Optional caller-supplied A2A conversation context identifier.
    pub context_id: Option<String>,
    /// Extension metadata keyed by an exact URI advertised in the Agent Card.
    pub extensions: BTreeMap<String, Value>,
    /// Maximum time to wait for an asynchronous A2A task.
    pub task_poll_secs: u64,
}

#[derive(Debug, Parser)]
#[command(
    name = "buzz-a2a-acp",
    about = "Expose an OASF Agent Record as an ACP subprocess"
)]
struct Cli {
    /// Local path or HTTP(S) URL for an OASF 1.0 Agent Record.
    #[arg(long, env = "BUZZ_A2A_AGENT_RECORD")]
    record: String,

    /// Exact A2A endpoint where the operator permits the bearer token to be sent.
    #[arg(long, env = "BUZZ_A2A_BEARER_ENDPOINT")]
    bearer_token_endpoint: Option<String>,

    /// Optional stable A2A conversation context identifier.
    #[arg(long, env = "BUZZ_A2A_CONTEXT_ID")]
    context_id: Option<String>,

    /// JSON object keyed by A2A extension URI.
    #[arg(long, env = "BUZZ_A2A_EXTENSIONS_JSON")]
    extensions_json: Option<String>,

    /// Maximum time to wait for an asynchronous A2A task.
    #[arg(
        long,
        env = "BUZZ_A2A_TASK_POLL_SECS",
        default_value_t = DEFAULT_TASK_POLL_SECS
    )]
    task_poll_secs: u64,
}

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("record source is empty")]
    EmptyRecord,
    #[error("record source is not a local path or HTTP(S) URL: {0}")]
    InvalidSource(String),
    #[error("fetch {what} failed with HTTP {status}")]
    HttpStatus {
        what: &'static str,
        status: StatusCode,
    },
    #[error("{what} exceeds the {limit} byte limit")]
    TooLarge { what: &'static str, limit: usize },
    #[error("read {what}: {source}")]
    Read {
        what: &'static str,
        source: std::io::Error,
    },
    #[error("decode {what}: {source}")]
    Decode {
        what: &'static str,
        source: serde_json::Error,
    },
    #[error("invalid OASF Agent Record: {0}")]
    InvalidRecord(String),
    #[error("invalid OASF A2A artifact: {0}")]
    InvalidArtifact(String),
    #[error("A2A endpoint is not advertised by the Agent Card")]
    MissingEndpoint,
    #[error("invalid A2A extension configuration: {0}")]
    InvalidExtensionConfig(String),
    #[error("A2A extension is not advertised by the Agent Card: {0}")]
    UnsupportedExtension(String),
    #[error("Agent Card requires an A2A extension that is not configured: {0}")]
    RequiredExtension(String),
    #[error("unsafe endpoint URL: {0}")]
    UnsafeEndpoint(String),
    #[error("bearer token is not authorized for A2A endpoint {0}")]
    UnauthorizedTokenEndpoint(String),
    #[error("remote A2A task did not complete before the {0} second timeout")]
    TaskTimeout(u64),
    #[error("A2A request failed: {0}")]
    Request(String),
    #[error("A2A response was invalid: {0}")]
    InvalidResponse(String),
    #[error("ACP protocol error: {0}")]
    Acp(String),
}

/// Run the adapter as a normal CLI process. Sprig uses this entry point for
/// the `buzz-a2a-acp` multicall personality.
pub fn run_cli() -> Result<(), String> {
    let args = Cli::parse();
    let bearer_token = std::env::var("BUZZ_A2A_BEARER_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let extensions = parse_extensions_json(args.extensions_json.as_deref())
        .map_err(|error| error.to_string())?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("build runtime: {error}"))?
        .block_on(run(AdapterConfig {
            record: args.record,
            bearer_token,
            bearer_token_endpoint: args.bearer_token_endpoint,
            context_id: args.context_id,
            extensions,
            task_poll_secs: args.task_poll_secs,
        }))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a2a::negotiate_extensions;
    use crate::net::pinned_http_client;
    use crate::oasf::{load_record, resolve_card, CardSource};
    use reqwest::Client;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::collections::HashSet;
    use std::fs;
    use std::net::{IpAddr, SocketAddr};
    use std::sync::atomic::Ordering;
    use tokio::io::{AsyncWriteExt, BufReader};
    use url::Url;

    fn card(endpoint: &str) -> Value {
        json!({ "id": "example-agent", "name": "Example Agent", "serviceEndpoint": endpoint })
    }

    #[tokio::test]
    async fn resolves_oasf_artifact_and_validates_descriptor() {
        let card = card("http://127.0.0.1:1337/a2a");
        let bytes = serde_json::to_vec(&card).expect("test card serializes");
        let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
        let record = json!({ "name": "Example Agent", "schema_version": "1.0.0", "modules": [{ "name": "integration/a2a", "id": 203, "artifact": { "json": card, "digest": digest, "media_type": "application/a2a-agent-card+json", "size": bytes.len() } }] });
        let path = std::env::temp_dir().join(format!(
            "buzz-a2a-record-{}-{}.json",
            std::process::id(),
            REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(
            &path,
            serde_json::to_vec(&record).expect("test record serializes"),
        )
        .expect("write record");
        let loaded = load_record(path.to_str().expect("temp path is utf8"))
            .await
            .expect("load record");
        let (resolved, source) = resolve_card(loaded.record, loaded.base.as_ref())
            .await
            .expect("resolve card");
        assert_eq!(source, CardSource::Artifact);
        assert_eq!(
            resolved.mode,
            ProtocolMode::VendorServiceEndpoint {
                endpoint: "http://127.0.0.1:1337/a2a".into()
            }
        );
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn accepts_one_record_collection_and_preserves_embedded_artifact_bytes() {
        let raw_card = r#"{"name":"Example Agent","version":"1.0.0","supportedInterfaces":[{"url":"http://127.0.0.1:1337/a2a/agent","protocolBinding":"JSONRPC","protocolVersion":"1.0"}]}"#;
        let digest = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(raw_card.as_bytes()))
        );
        let record = format!(
            r#"[{{"name":"Example Agent","schema_version":"1.0.0","modules":[{{"name":"integration/a2a","id":203,"artifact":{{"json":{raw_card},"digest":"{digest}","media_type":"application/a2a-agent-card+json","size":{}}}}}]}}]"#,
            raw_card.len()
        );
        let path = std::env::temp_dir().join(format!(
            "buzz-a2a-record-collection-{}-{}.json",
            std::process::id(),
            REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, record).expect("write record collection");
        let record = load_record(path.to_str().expect("temp path is utf8"))
            .await
            .expect("load one-record collection");
        let (resolved, source) = resolve_card(record.record, record.base.as_ref())
            .await
            .expect("resolve exact embedded artifact bytes");
        assert_eq!(source, CardSource::Artifact);
        assert_eq!(resolved.mode.endpoint(), "http://127.0.0.1:1337/a2a/agent");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn prefers_declared_jsonrpc_interface() {
        let card: AgentCard = serde_json::from_value(json!({ "supportedInterfaces": [{ "url": "https://agent.example/rpc", "protocolBinding": "JSONRPC", "protocolVersion": "1.0" }], "serviceEndpoint": "https://legacy.example/a2a" })).expect("card");
        assert_eq!(
            select_protocol_mode(&card).expect("mode"),
            ProtocolMode::JsonRpc {
                endpoint: "https://agent.example/rpc".into(),
                protocol_version: Some("1.0".into()),
            }
        );
    }

    #[test]
    fn builds_current_and_vendor_requests() {
        let current = request_payload(
            &ProtocolMode::JsonRpc {
                endpoint: "https://agent.example/rpc".into(),
                protocol_version: Some("1.0".into()),
            },
            Some("remote"),
            1,
            "session",
            "hello",
            &BTreeMap::new(),
        );
        assert_eq!(current["method"], "SendMessage");
        assert_eq!(current["params"]["message"]["role"], "ROLE_USER");
        assert!(current["params"]["message"]["parts"][0]["kind"].is_null());
        assert_eq!(current["params"]["message"]["parts"][0]["text"], "hello");
        let vendor = request_payload(
            &ProtocolMode::VendorServiceEndpoint {
                endpoint: "http://127.0.0.1:1337/a2a".into(),
            },
            Some("remote"),
            2,
            "session",
            "hello",
            &BTreeMap::new(),
        );
        assert_eq!(vendor["method"], "agent/sendMessage");
        assert_eq!(vendor["params"]["agentId"], "remote");
    }

    #[test]
    fn sends_a2a_version_header_for_standard_modes_only() {
        let client = Client::new();
        for (mode, expected) in [
            (
                ProtocolMode::JsonRpc {
                    endpoint: "https://agent.example/rpc".into(),
                    protocol_version: Some("1.0".into()),
                },
                Some("1.0"),
            ),
            (
                ProtocolMode::JsonRpc {
                    endpoint: "https://agent.example/rpc".into(),
                    protocol_version: Some("0.3".into()),
                },
                Some("0.3"),
            ),
            (
                ProtocolMode::VendorServiceEndpoint {
                    endpoint: "https://agent.example/rpc".into(),
                },
                None,
            ),
        ] {
            let request = protocol_request(&client, &mode, mode.endpoint(), &BTreeMap::new())
                .build()
                .expect("request builds");
            assert_eq!(
                request
                    .headers()
                    .get("A2A-Version")
                    .and_then(|value| value.to_str().ok()),
                expected
            );
        }
    }

    #[test]
    fn builds_a2a_0_3_request_and_preserves_context() {
        let request = request_payload(
            &ProtocolMode::JsonRpc {
                endpoint: "https://agent.example/rpc".into(),
                protocol_version: Some("0.3".into()),
            },
            Some("remote"),
            3,
            "buzz-session",
            "hello",
            &BTreeMap::new(),
        );
        assert_eq!(request["method"], "message/send");
        assert!(request["params"]["contextId"].is_null());
        assert_eq!(request["params"]["message"]["contextId"], "buzz-session");
        assert_eq!(request["params"]["message"]["parts"][0]["kind"], "text");
    }

    #[test]
    fn negotiates_and_projects_advertised_extensions() {
        let extension_uri = "https://example.com/a2a/extensions/work-context/v1";
        let card: AgentCard = serde_json::from_value(json!({
            "capabilities": {
                "extensions": [{ "uri": extension_uri }]
            },
            "supportedInterfaces": [{
                "url": "https://agent.example/rpc",
                "protocolBinding": "JSONRPC",
                "protocolVersion": "1.0"
            }]
        }))
        .expect("card");
        let mode = select_protocol_mode(&card).expect("mode");
        let configured = BTreeMap::from([(
            extension_uri.to_string(),
            json!({ "organizationRef": "https://example.com/organizations/acme" }),
        )]);
        let active =
            negotiate_extensions(&card, &configured, &mode).expect("extension is advertised");
        let request = request_payload(&mode, None, 4, "session", "hello", &active);
        assert_eq!(
            request["params"]["message"]["extensions"],
            json!([extension_uri])
        );
        assert_eq!(
            request["params"]["message"]["metadata"][extension_uri]["organizationRef"],
            "https://example.com/organizations/acme"
        );

        let http_request = protocol_request(&Client::new(), &mode, mode.endpoint(), &active)
            .build()
            .expect("request builds");
        assert_eq!(
            http_request
                .headers()
                .get("A2A-Extensions")
                .and_then(|value| value.to_str().ok()),
            Some(extension_uri)
        );
    }

    #[test]
    fn omits_extension_headers_and_metadata_when_unconfigured() {
        let mode = ProtocolMode::JsonRpc {
            endpoint: "https://agent.example/rpc".into(),
            protocol_version: Some("1.0".into()),
        };
        let extensions = BTreeMap::new();
        let payload = request_payload(&mode, None, 5, "session", "hello", &extensions);
        let message = &payload["params"]["message"];

        assert!(message.get("extensions").is_none());
        assert!(message.get("metadata").is_none());

        let request = protocol_request(&Client::new(), &mode, mode.endpoint(), &extensions)
            .build()
            .expect("request builds");
        assert!(request.headers().get("A2A-Extensions").is_none());
    }

    #[test]
    fn rejects_unadvertised_and_missing_required_extensions() {
        let required_uri = "https://example.com/a2a/extensions/required/v1";
        let card: AgentCard = serde_json::from_value(json!({
            "capabilities": {
                "extensions": [{ "uri": required_uri, "required": true }]
            },
            "supportedInterfaces": [{
                "url": "https://agent.example/rpc",
                "protocolBinding": "JSONRPC",
                "protocolVersion": "1.0"
            }]
        }))
        .expect("card");
        let mode = select_protocol_mode(&card).expect("mode");
        assert!(matches!(
            negotiate_extensions(&card, &BTreeMap::new(), &mode),
            Err(AdapterError::RequiredExtension(uri)) if uri == required_uri
        ));

        let configured = BTreeMap::from([(
            "https://example.com/a2a/extensions/other/v1".to_string(),
            json!({}),
        )]);
        assert!(matches!(
            negotiate_extensions(&card, &configured, &mode),
            Err(AdapterError::RequiredExtension(uri)) if uri == required_uri
        ));

        let optional_card: AgentCard = serde_json::from_value(json!({
            "capabilities": { "extensions": [] },
            "supportedInterfaces": [{
                "url": "https://agent.example/rpc",
                "protocolBinding": "JSONRPC",
                "protocolVersion": "1.0"
            }]
        }))
        .expect("card");
        let optional_mode = select_protocol_mode(&optional_card).expect("mode");
        assert!(matches!(
            negotiate_extensions(&optional_card, &configured, &optional_mode),
            Err(AdapterError::UnsupportedExtension(uri))
                if uri == "https://example.com/a2a/extensions/other/v1"
        ));
    }

    #[test]
    fn parses_bounded_extension_configuration() {
        let parsed =
            parse_extensions_json(Some(r#"{"urn:example:extension":{"scope":"project-1"}}"#))
                .expect("valid extension map");
        assert_eq!(parsed["urn:example:extension"]["scope"], "project-1");
        assert!(parse_extensions_json(Some("[]")).is_err());
        assert!(parse_extensions_json(Some(r#"{"not a uri":{}}"#)).is_err());
    }

    #[test]
    fn rejects_non_loopback_http_endpoints() {
        let card: AgentCard = serde_json::from_value(json!({
            "supportedInterfaces": [{
                "url": "http://remote.example/a2a",
                "protocolBinding": "JSONRPC",
                "protocolVersion": "1.0"
            }]
        }))
        .expect("card");
        assert!(matches!(
            select_protocol_mode(&card),
            Err(AdapterError::UnsafeEndpoint(_))
        ));
        let private_card: AgentCard = serde_json::from_value(json!({
            "supportedInterfaces": [{
                "url": "https://127.0.0.1/a2a",
                "protocolBinding": "JSONRPC",
                "protocolVersion": "1.0"
            }]
        }))
        .expect("card");
        assert!(matches!(
            select_protocol_mode(&private_card),
            Err(AdapterError::UnsafeEndpoint(_))
        ));
    }

    #[tokio::test]
    async fn pinned_localhost_client_falls_back_across_checked_addresses() {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind IPv4 listener");
        let port = listener.local_addr().expect("listener address").port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept request");
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .expect("write response");
        });
        let url = Url::parse(&format!("http://localhost:{port}/a2a")).expect("url");
        let addresses = [
            SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), port),
            SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port),
        ];
        validate_resolved_addresses(&url, &addresses).expect("loopback addresses");

        let response = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            pinned_http_client(&url, &addresses)
                .expect("pinned client")
                .get(url)
                .send(),
        )
        .await
        .expect("connect using validated fallback")
        .expect("HTTP response");
        assert_eq!(response.status(), StatusCode::OK);
        server.await.expect("server task");
    }

    #[test]
    fn resolver_policy_rejects_mixed_or_private_addresses() {
        let localhost = Url::parse("http://localhost:1337/a2a").expect("url");
        assert!(validate_resolved_addresses(
            &localhost,
            &[
                SocketAddr::from(([127, 0, 0, 1], 1337)),
                SocketAddr::from(([8, 8, 8, 8], 1337))
            ]
        )
        .is_err());
        let public = Url::parse("https://agent.example/a2a").expect("url");
        assert!(validate_resolved_addresses(
            &public,
            &[
                SocketAddr::from(([8, 8, 8, 8], 443)),
                SocketAddr::from(([10, 0, 0, 1], 443))
            ]
        )
        .is_err());
        assert!(
            validate_resolved_addresses(&public, &[SocketAddr::from(([8, 8, 8, 8], 443))]).is_ok()
        );
    }

    #[test]
    fn reviewed_endpoint_is_enforced_even_without_a_token() {
        assert!(validate_endpoint_binding(
            None,
            Some("https://reviewed.example/a2a"),
            "https://reviewed.example/a2a"
        )
        .is_ok());
        assert!(matches!(
            validate_endpoint_binding(
                None,
                Some("https://reviewed.example/a2a"),
                "https://other.example/a2a"
            ),
            Err(AdapterError::UnauthorizedTokenEndpoint(_))
        ));
        assert!(matches!(
            validate_endpoint_binding(Some("secret"), None, "https://agent.example/a2a"),
            Err(AdapterError::UnauthorizedTokenEndpoint(_))
        ));
    }

    #[tokio::test]
    async fn deprecated_card_data_is_explicit_compatibility_path() {
        let record: AgentRecord = serde_json::from_value(json!({ "modules": [{ "name": "integration/a2a", "data": { "card_data": card("http://127.0.0.1:1337/a2a"), "card_schema_version": "0.3" } }] })).expect("record");
        let (_, source) = resolve_card(record, None).await.expect("resolve card");
        assert_eq!(source, CardSource::DeprecatedCardData);
    }

    #[tokio::test]
    async fn digest_mismatch_is_rejected() {
        let artifact = json!({ "name": "wrong" });
        let artifact_bytes = serde_json::to_vec(&artifact).expect("artifact serializes");
        let descriptor: Descriptor = serde_json::from_value(json!({
            "json": artifact,
            "digest": "sha256:00",
            "media_type": "application/json",
            "size": artifact_bytes.len()
        }))
        .expect("descriptor");
        let err = descriptor_bytes(&descriptor, None)
            .await
            .expect_err("mismatch");
        assert!(err.to_string().contains("digest mismatch"));
    }

    #[tokio::test]
    async fn missing_oasf_descriptor_media_type_is_rejected() {
        let artifact = json!({ "name": "missing media type" });
        let artifact_bytes = serde_json::to_vec(&artifact).expect("artifact serializes");
        let descriptor: Descriptor = serde_json::from_value(json!({
            "json": artifact,
            "digest": format!("sha256:{}", hex::encode(Sha256::digest(&artifact_bytes))),
            "size": artifact_bytes.len()
        }))
        .expect("descriptor");
        let err = descriptor_bytes(&descriptor, None)
            .await
            .expect_err("missing media_type");
        assert!(err.to_string().contains("requires media_type"));
    }

    #[tokio::test]
    async fn acp_transcript_handles_initialize_new_and_prompt() {
        let mut sessions = HashSet::new();
        let initialize = handle_acp_message(
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": 1 } }),
            &mut sessions,
            "remote-agent",
            None,
        )
        .expect("initialize action")
        .expect("initialize response");
        let AcpAction::Response(initialize) = initialize else {
            panic!("initialize must return a response");
        };
        assert_eq!(initialize["result"]["protocolVersion"], 1);
        assert_eq!(initialize["result"]["agentInfo"]["name"], "remote-agent");

        let new = handle_acp_message(
            &json!({ "jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {} }),
            &mut sessions,
            "remote-agent",
            None,
        )
        .expect("session/new action")
        .expect("session/new response");
        let AcpAction::Response(new) = new else {
            panic!("session/new must return a response");
        };
        let session_id = new["result"]["sessionId"]
            .as_str()
            .expect("session id")
            .to_owned();
        assert!(sessions.contains(&session_id));

        let prompt = handle_acp_message(
            &json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "session/prompt",
                "params": { "sessionId": session_id, "prompt": [{ "type": "text", "text": "ship it" }] }
            }),
            &mut sessions,
            "remote-agent",
            None,
        )
        .expect("session/prompt action")
        .expect("session/prompt request");
        let AcpAction::Prompt {
            id,
            session_id,
            text,
        } = prompt
        else {
            panic!("session/prompt must invoke the remote runtime");
        };
        assert_eq!(id, 3);
        assert_eq!(text, "ship it");
        let [update, result] = prompt_success(id, &session_id, "done");
        assert_eq!(update["method"], "session/update");
        assert_eq!(update["params"]["sessionId"], session_id);
        assert_eq!(update["params"]["update"]["content"]["text"], "done");
        assert_eq!(result["result"]["stopReason"], "end_turn");
    }

    #[test]
    fn terminal_task_polling_distinguishes_working_and_terminal_states() {
        assert_eq!(
            task_outcome(&json!({ "status": { "state": "working" } }), "task-1")
                .expect("working is nonterminal"),
            None,
        );
        assert_eq!(
            task_outcome(
                &json!({ "status": { "state": "TASK_STATE_SUBMITTED" } }),
                "task-1"
            )
            .expect("protobuf-style submitted is nonterminal"),
            None,
        );
        assert_eq!(
            task_outcome(&json!({ "status": { "state": "completed" } }), "task-1")
                .expect("completed is successful"),
            Some("A2A task task-1 completed".into()),
        );
        assert_eq!(
            task_outcome(
                &json!({ "status": { "state": "TASK_STATE_COMPLETED" } }),
                "task-1"
            )
            .expect("protobuf-style completed is successful"),
            Some("A2A task task-1 completed".into()),
        );
        let failed = task_outcome(
            &json!({
                "status": { "state": "TASK_STATE_FAILED" },
                "parts": [{ "text": "remote failure" }]
            }),
            "task-1",
        );
        assert!(failed
            .expect_err("failed tasks are ACP errors")
            .to_string()
            .contains("remote failure"));
    }

    #[test]
    fn unwraps_current_a2a_task_and_message_results() {
        let task = json!({ "task": { "id": "task-2", "status": { "state": "completed" }, "artifacts": [{ "parts": [{ "text": "complete" }] }] } });
        let task_result = task.get("task").expect("task result");
        assert_eq!(task_result["id"], "task-2");
        assert_eq!(
            task_outcome(task_result, "task-2").expect("completed task"),
            Some("complete".into()),
        );

        let message = json!({ "message": { "messageId": "m-1", "role": "ROLE_AGENT", "parts": [{ "text": "direct response" }] } });
        let message_result = message.get("message").expect("message result");
        assert_eq!(extract_text(message_result), Some("direct response".into()));
    }

    #[tokio::test]
    async fn acp_reader_accepts_multiple_bounded_lines() {
        let input = b"{\"method\":\"initialize\"}\r\n{\"method\":\"session/new\"}\n";
        let mut reader = BufReader::new(&input[..]);
        assert_eq!(
            read_bounded_line(&mut reader).await.expect("first line"),
            Some("{\"method\":\"initialize\"}".into())
        );
        assert_eq!(
            read_bounded_line(&mut reader).await.expect("second line"),
            Some("{\"method\":\"session/new\"}".into())
        );
        assert_eq!(read_bounded_line(&mut reader).await.expect("eof"), None);
    }

    #[tokio::test]
    async fn spawned_reader_preserves_partial_lines_while_other_work_completes() {
        use tokio::io::AsyncWriteExt;

        let (mut writer, reader) = tokio::io::duplex(32 * 1024);
        let mut lines = spawn_line_reader(BufReader::with_capacity(8 * 1024, reader));
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"session/prompt\",\"padding\":\"{}\"}}",
            "P".repeat(20 * 1024)
        );
        writer
            .write_all(&line.as_bytes()[..12 * 1024])
            .await
            .expect("write first fragment");
        tokio::task::yield_now().await;
        writer
            .write_all(&line.as_bytes()[12 * 1024..])
            .await
            .expect("write second fragment");
        writer.write_all(b"\n").await.expect("finish line");

        assert_eq!(
            lines
                .recv()
                .await
                .expect("reader remains available")
                .expect("line is valid"),
            Some(line),
        );
    }

    #[tokio::test]
    async fn spawned_reader_stops_after_a_transport_error() {
        use std::{
            pin::Pin,
            sync::{
                atomic::{AtomicUsize, Ordering},
                Arc,
            },
            task::{Context, Poll},
        };
        use tokio::io::{AsyncBufRead, AsyncRead, ReadBuf};

        struct FailingReader {
            reads: Arc<AtomicUsize>,
        }

        impl AsyncRead for FailingReader {
            fn poll_read(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                _buf: &mut ReadBuf<'_>,
            ) -> Poll<std::io::Result<()>> {
                Poll::Ready(Err(std::io::Error::other("transport failed")))
            }
        }

        impl AsyncBufRead for FailingReader {
            fn poll_fill_buf(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::io::Result<&[u8]>> {
                self.reads.fetch_add(1, Ordering::Relaxed);
                Poll::Ready(Err(std::io::Error::other("transport failed")))
            }

            fn consume(self: Pin<&mut Self>, _amount: usize) {}
        }

        let reads = Arc::new(AtomicUsize::new(0));
        let mut lines = spawn_line_reader(FailingReader {
            reads: Arc::clone(&reads),
        });
        assert!(matches!(
            lines.recv().await,
            Some(Err(AdapterError::Read {
                what: "ACP request",
                ..
            }))
        ));
        assert!(lines.recv().await.is_none());
        assert_eq!(reads.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn idless_notifications_do_not_produce_json_rpc_responses() {
        let mut sessions = HashSet::new();
        let action = handle_acp_message(
            &json!({ "jsonrpc": "2.0", "method": "unknown/notification" }),
            &mut sessions,
            "remote-agent",
            None,
        )
        .expect("notification is valid");
        assert!(action.is_none());
    }

    #[test]
    fn cancel_notification_remains_actionable_without_an_id() {
        let mut sessions = HashSet::new();
        let action = handle_acp_message(
            &json!({
                "jsonrpc": "2.0",
                "method": "session/cancel",
                "params": { "sessionId": "session-1" }
            }),
            &mut sessions,
            "remote-agent",
            None,
        )
        .expect("cancel is valid")
        .expect("cancel action");
        let AcpAction::Cancel { id, session_id } = action else {
            panic!("cancel notification must produce a local action");
        };
        assert!(id.is_none());
        assert_eq!(session_id, "session-1");
    }

    #[test]
    fn private_ipv4_transitional_ipv6_addresses_are_rejected() {
        for address in [
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::0a00:0001",
            "2002:0a00:0001::",
            "2001:0000:4136:e378:8000:63bf:3fff:fdd2",
        ] {
            assert!(
                is_private_ip(address.parse().expect("test IP")),
                "{address} must not bypass the private-address policy"
            );
        }
    }

    #[tokio::test]
    async fn remote_artifact_requires_a_digest_before_fetch() {
        let descriptor: Descriptor = serde_json::from_value(json!({
            "urls": ["https://agent.example/card.json"],
            "media_type": "application/a2a-agent-card+json",
            "size": 1
        }))
        .expect("descriptor");
        let error = descriptor_bytes(&descriptor, None)
            .await
            .expect_err("unsigned remote artifact");
        assert!(error.to_string().contains("require a sha256 digest"));
    }
}
