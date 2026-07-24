#[cfg(test)]
#[derive(Clone)]
struct FakeMcpProbe {
    expected_token: String,
    server_identity: String,
    tools: Vec<String>,
    snapshot_status: Value,
}

#[cfg(test)]
impl McpProbe for FakeMcpProbe {
    fn attest(
        &self,
        _config: &RagConfig,
        bearer_token: &str,
        _attestation_secret: &str,
    ) -> Result<McpAttestation, RagError> {
        if bearer_token != self.expected_token {
            return Err(RagError::AuthenticationFailed);
        }
        Ok(McpAttestation {
            server_identity: self.server_identity.clone(),
            tools: self.tools.clone(),
            snapshot_status: self.snapshot_status.clone(),
        })
    }
}

use super::*;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;

const SNAPSHOT_TIME: &str = "2026-07-24T03:30:00Z";
const ACTIVATED_AT: &str = "2026-07-24T04:00:00Z";

#[test]
fn rag_canonicalization_matches_upstream_rfc8785_golden_bytes() {
    assert_eq!(
        canonical_json_bytes(&json!({"a": 1, "z": [3, {"é": true}]})).expect("canonical object"),
        r#"{"a":1,"z":[3,{"é":true}]}"#.as_bytes(),
    );
    assert_eq!(
        canonical_json_bytes(&json!({"n": 0.000001})).expect("canonical decimal"),
        br#"{"n":0.000001}"#,
    );
    let rfc_sample: Value = serde_json::from_str(
        r#"{"numbers":[333333333.33333329,1E30,4.50,2e-3,0.000000000000000000000000001],"string":"€$\u000f\nA'B\"\\\\\"/"}"#,
    )
    .expect("parse RFC sample");
    assert_eq!(
        canonical_json_bytes(&rfc_sample).expect("canonical RFC sample"),
        b"{\"numbers\":[333333333.3333333,1e+30,4.5,0.002,1e-27],\"string\":\"\xE2\x82\xAC$\\u000f\\nA'B\\\"\\\\\\\\\\\"/\"}",
    );
}

fn hex(value: &Value) -> String {
    let bytes = canonical_json_bytes(value).expect("canonical fixture");
    crate::command_services::policy::sha256_hex(&bytes)
}

fn fixture() -> (RagConfig, Value, Value, Value, Value, FakeMcpProbe) {
    let signer = "b".repeat(64);
    let collection_schema = json!({"vectors":{"dense":{"distance":"Cosine","size":1024}}});
    let catalogue = json!({
        "collections": [{
            "name": "documents",
            "point_count": 2,
            "schema": collection_schema.clone(),
        }],
        "documents": [{
            "doc_id": "doc-1",
            "collection": "navy-publications",
        }, {
            "doc_id": "doc-2",
            "collection": "navy-publications",
        }],
    });
    let catalogue_hash = hex(&catalogue);
    let manifest = json!({
        "format": "rag-snapshot-v1",
        "snapshot_time": SNAPSHOT_TIME,
        "service": {
            "qdrant_version": "1.17.1",
            "rag_commit": "0123456789abcdef0123456789abcdef01234567",
        },
        "signer": {
            "algorithm": "ed25519",
            "public_key_sha256": signer,
        },
        "retrieval_models": {
            "dense": {"implementation":"bge-m3","version":"BAAI/bge-m3@v1"},
            "sparse": {"implementation":"bge-m3-sparse","version":"1.0.0"},
            "reranker": {
                "implementation":"bge-reranker-v2-m3",
                "version":"BAAI/bge-reranker-v2-m3@v1",
            },
        },
        "collections": [{
            "name":"documents",
            "point_count":2,
            "schema":collection_schema,
            "snapshot_path":"objects/qdrant/documents.snapshot",
        }],
        "catalogue": {
            "document_count":2,
            "path":"objects/catalogue.json",
            "sha256":catalogue_hash,
        },
        "golden_queries": {
            "path":"objects/golden.json",
            "sha256":"d".repeat(64),
        },
        "objects": [{
            "media_type":"application/vnd.qdrant.snapshot",
            "path":"objects/qdrant/documents.snapshot",
            "sha256":"e".repeat(64),
            "size":128,
        },{
            "media_type":"application/json",
            "path":"objects/catalogue.json",
            "sha256":catalogue_hash,
            "size":64,
        },{
            "media_type":"application/json",
            "path":"objects/golden.json",
            "sha256":"d".repeat(64),
            "size":64,
        }],
    });
    let snapshot_id = hex(&manifest);
    let runtime_collection = format!("staging-{}-documents", &snapshot_id[..12]);
    let activation = json!({
        "format":"rag-activation-v2",
        "snapshot_id":snapshot_id,
        "manifest_sha256":snapshot_id,
        "signer_fingerprint":signer,
        "snapshot_time":SNAPSHOT_TIME,
        "service":manifest["service"].clone(),
        "retrieval_models":manifest["retrieval_models"].clone(),
        "collections":[{
            "name":"documents",
            "runtime_name":runtime_collection,
            "point_count":2,
            "schema":manifest["collections"][0]["schema"].clone(),
        }],
        "golden_object_sha256":"d".repeat(64),
        "golden_queries":{"passed":true,"case_count":2,"passed_count":2},
        "activated_at":ACTIVATED_AT,
    });
    let activation_id = hex(&activation);
    let readiness = json!({
        "format":"rag-readiness-v2",
        "active_activation_id":activation_id,
        "active_snapshot_id":snapshot_id,
        "signature_fingerprint":signer,
        "snapshot_time":SNAPSHOT_TIME,
        "service":manifest["service"].clone(),
        "retrieval_models":manifest["retrieval_models"].clone(),
        "collections":activation["collections"].clone(),
        "golden_queries":activation["golden_queries"].clone(),
        "last_successful_activation":ACTIVATED_AT,
    });
    let config = RagConfig {
        schema_version: 1,
        endpoint: "http://127.0.0.1:8005/mcp/".to_string(),
        state_root: "/var/lib/command-rag".into(),
        expected_server_identity: "rag".to_string(),
        expected_active_snapshot_id: snapshot_id.clone(),
        trusted_signer_fingerprint: signer,
        credential_key: "rag.local.read".to_string(),
        attestation_credential_key: "rag.local.attestation".to_string(),
        tool_allowlist: RAG_CATALOG_TOOLS
            .iter()
            .map(|tool| (*tool).to_string())
            .collect(),
        maximum_snapshot_age_hours: 48,
    };
    let probe = FakeMcpProbe {
        expected_token: "rag-read-token-123456".to_string(),
        server_identity: "rag".to_string(),
        tools: RAG_CATALOG_TOOLS
            .iter()
            .map(|tool| (*tool).to_string())
            .collect(),
        snapshot_status: readiness.clone(),
    };
    (config, manifest, catalogue, activation, readiness, probe)
}

fn signed_documents<'a>(manifest: &'a Value, catalogue: &'a Value) -> SignedSnapshotDocuments<'a> {
    SignedSnapshotDocuments {
        manifest,
        catalogue,
    }
}

#[test]
fn recomputes_manifest_and_activation_hashes_before_ready() {
    let (config, manifest, catalogue, activation, _readiness, probe) = fixture();
    let result = verify_rag_service(
        &config,
        "rag-read-token-123456",
        "rag-attestation-secret-123456",
        signed_documents(&manifest, &catalogue),
        &activation,
        &probe,
        "2026-07-24T04:30:00Z",
    )
    .expect("verified RAG");
    assert_eq!(result.status, RagServiceStatus::Ready);
    assert_eq!(result.validation, RagValidationState::Verified);
    assert_eq!(result.freshness, RagFreshness::Fresh);
    assert_eq!(
        result.server_identity.as_deref(),
        Some(config.expected_server_identity.as_str()),
    );
    assert_eq!(
        result.active_snapshot_id.as_deref(),
        Some(config.expected_active_snapshot_id.as_str()),
    );
    assert_eq!(
        result.tool_allowlist.iter().collect::<BTreeSet<_>>(),
        config.tool_allowlist.iter().collect::<BTreeSet<_>>(),
    );
    let binding = verified_snapshot_from_readiness(&result).expect("frozen snapshot binding");
    assert_eq!(binding.snapshot_id(), config.expected_active_snapshot_id);
    assert_eq!(binding.physical_collections(), &["documents".to_string()]);
    assert_eq!(
        binding.logical_collections(),
        &["navy-publications".to_string()],
    );
    assert_eq!(
        result.admitted.expect("admission candidate").bearer_token,
        "rag-read-token-123456",
    );
}

#[test]
fn rejects_catalogue_content_that_does_not_match_the_signed_manifest_hash() {
    let (config, manifest, mut catalogue, activation, _readiness, probe) = fixture();
    catalogue["documents"][0]["collection"] = json!("untrusted-logical-collection");

    assert_eq!(
        verify_rag_service(
            &config,
            "rag-read-token-123456",
            "rag-attestation-secret-123456",
            signed_documents(&manifest, &catalogue),
            &activation,
            &probe,
            "2026-07-24T04:30:00Z",
        ),
        Err(RagError::SnapshotHashMismatch),
    );
}

#[test]
fn verifies_manifest_signature_and_pinned_public_key_fingerprint() {
    let (_config, mut manifest, _catalogue, _activation, _readiness, _probe) = fixture();
    let signing_key = SigningKey::from_bytes(&[7; 32]);
    let public_key = signing_key.verifying_key().to_bytes();
    manifest["signer"]["public_key_sha256"] =
        Value::String(::hex::encode(Sha256::digest(public_key)));
    let manifest_bytes = canonical_json_bytes(&manifest).expect("canonical manifest");
    let signature = signing_key.sign(&manifest_bytes).to_bytes();
    let working_directory = std::env::current_dir().expect("current test directory");
    let directory = tempfile::Builder::new()
        .prefix(".rag-signature-test-")
        .tempdir_in(working_directory)
        .expect("protected temporary snapshot directory");
    for (name, bytes) in [
        ("manifest.pub", public_key.as_slice()),
        ("manifest.sig", signature.as_slice()),
    ] {
        let path = directory.path().join(name);
        std::fs::write(&path, bytes).expect("write signature fixture");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("protect signature fixture");
    }

    assert_eq!(
        verify_manifest_signature(directory.path(), &manifest, &manifest_bytes),
        Ok(()),
        "the exact canonical bytes and pinned key must verify",
    );
    let mut tampered = manifest_bytes;
    tampered.push(b' ');
    assert_eq!(
        verify_manifest_signature(directory.path(), &manifest, &tampered),
        Err(RagError::ValidationFailed),
    );
}

#[test]
fn rejects_tampered_manifest_activation_and_stale_expected_snapshot() {
    let (config, manifest, catalogue, activation, _readiness, probe) = fixture();

    let mut tampered_manifest = manifest.clone();
    tampered_manifest["collections"][0]["point_count"] = json!(3);
    assert_eq!(
        verify_rag_service(
            &config,
            "rag-read-token-123456",
            "rag-attestation-secret-123456",
            signed_documents(&tampered_manifest, &catalogue),
            &activation,
            &probe,
            "2026-07-24T04:30:00Z",
        ),
        Err(RagError::SnapshotHashMismatch),
    );

    let mut tampered_activation = activation.clone();
    tampered_activation["golden_queries"]["passed"] = json!(false);
    assert_eq!(
        verify_rag_service(
            &config,
            "rag-read-token-123456",
            "rag-attestation-secret-123456",
            signed_documents(&manifest, &catalogue),
            &tampered_activation,
            &probe,
            "2026-07-24T04:30:00Z",
        ),
        Err(RagError::ValidationFailed),
    );

    let mut stale = config;
    stale.expected_active_snapshot_id = "a".repeat(64);
    assert_eq!(
        verify_rag_service(
            &stale,
            "rag-read-token-123456",
            "rag-attestation-secret-123456",
            signed_documents(&manifest, &catalogue),
            &activation,
            &probe,
            "2026-07-24T04:30:00Z",
        ),
        Err(RagError::SnapshotHashMismatch),
    );
}

#[test]
fn rejects_wrong_server_tool_catalog_missing_auth_and_stale_data() {
    let (config, manifest, catalogue, activation, _readiness, probe) = fixture();

    let mut wrong_server = probe.clone();
    wrong_server.server_identity = "memory".to_string();
    assert_eq!(
        verify_rag_service(
            &config,
            "rag-read-token-123456",
            "rag-attestation-secret-123456",
            signed_documents(&manifest, &catalogue),
            &activation,
            &wrong_server,
            "2026-07-24T04:30:00Z",
        ),
        Err(RagError::ServerIdentityMismatch),
    );

    let mut missing_tool = probe.clone();
    missing_tool
        .tools
        .retain(|tool| tool != "get_snapshot_status");
    assert_eq!(
        verify_rag_service(
            &config,
            "rag-read-token-123456",
            "rag-attestation-secret-123456",
            signed_documents(&manifest, &catalogue),
            &activation,
            &missing_tool,
            "2026-07-24T04:30:00Z",
        ),
        Err(RagError::ToolCatalogMismatch),
    );

    assert_eq!(
        verify_rag_service(
            &config,
            "",
            "rag-attestation-secret-123456",
            signed_documents(&manifest, &catalogue),
            &activation,
            &probe,
            "2026-07-24T04:30:00Z",
        ),
        Err(RagError::AuthenticationFailed),
    );

    assert_eq!(
        verify_rag_service(
            &config,
            "rag-read-token-123456",
            "rag-attestation-secret-123456",
            signed_documents(&manifest, &catalogue),
            &activation,
            &probe,
            "2026-07-27T04:30:01Z",
        ),
        Err(RagError::SnapshotStale),
    );

    let mut invalid_credential_key = config;
    invalid_credential_key.credential_key = "rag.\nunsafe".to_string();
    assert_eq!(
        verify_rag_service(
            &invalid_credential_key,
            "rag-read-token-123456",
            "rag-attestation-secret-123456",
            signed_documents(&manifest, &catalogue),
            &activation,
            &probe,
            "2026-07-24T04:30:00Z",
        ),
        Err(RagError::InvalidConfig),
    );
}

#[test]
fn rejects_reused_rag_admission_secret_values() {
    let (config, manifest, catalogue, activation, _readiness, mut probe) = fixture();
    let shared = "r".repeat(256);
    probe.expected_token = shared.clone();
    assert_eq!(
        verify_rag_service(
            &config,
            &shared,
            &shared,
            signed_documents(&manifest, &catalogue),
            &activation,
            &probe,
            "2026-07-24T04:30:00Z",
        ),
        Err(RagError::AuthenticationFailed),
    );

    let bearer = format!("{}a", "r".repeat(255));
    let attestation = format!("{}b", "r".repeat(255));
    probe.expected_token = bearer.clone();
    assert!(
        verify_rag_service(
            &config,
            &bearer,
            &attestation,
            signed_documents(&manifest, &catalogue),
            &activation,
            &probe,
            "2026-07-24T04:30:00Z",
        )
        .is_ok(),
        "maximum-length secrets that differ only in the final byte remain independent",
    );
}

#[test]
fn fail_soft_status_is_redacted_and_never_carries_admission() {
    let status = fail_soft_readiness(RagError::AuthenticationFailed);
    assert_eq!(status.status, RagServiceStatus::Unavailable);
    assert_eq!(status.error.as_deref(), Some("authentication_failed"));
    assert_eq!(status.validation, RagValidationState::Failed);
    assert!(status.endpoint.is_none());
    assert!(status.admitted.is_none());
}
