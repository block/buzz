//! Structural O4 conformance checks for the disabled client-status surface.

use std::fs;
use std::path::{Path, PathBuf};

const FIXTURE: &str = include_str!("fixtures/nip_fi_trusted_proxy.json");
const STATUS_MODULE: &str = include_str!("../src/authorization_runtime/status.rs");
const ASSERTION_VERIFIER: &str = include_str!("../src/corporate_identity.rs");
const INVALIDATION_RUNTIME: &str = include_str!("../src/authorization_runtime/invalidation.rs");
const TRANSPORT_RUNTIME: &str = include_str!("../src/authorization_runtime/transport.rs");
const FINALIZATION_RUNTIME: &str = include_str!("../src/authorization_runtime/finalization.rs");
const PRODUCTION_RUNTIME: &str = include_str!("../src/authorization_runtime/production.rs");
const AUTH_HANDLER: &str = include_str!("../src/handlers/auth.rs");
const KIND_REGISTRY: &str = include_str!("../../buzz-core/src/kind.rs");
const INGEST_HANDLER: &str = include_str!("../src/handlers/ingest.rs");
const READ_ONLY_RELAY_CLIENT: &str =
    include_str!("../../../desktop/src/shared/api/readOnlyRelayClient.ts");
const PRIMARY_RELAY_CLIENT: &str =
    include_str!("../../../desktop/src/shared/api/relayClientSession.ts");
const STATUS_RELAY_CLIENT: &str =
    include_str!("../../../desktop/src/shared/api/relayClientStatusConnection.ts");
const NATIVE_WEBSOCKET: &str = include_str!("../../../desktop/src-tauri/src/native_websocket.rs");
const DESKTOP_BUILD: &str = include_str!("../../../desktop/src-tauri/build.rs");
const WORKSPACE_COMMAND: &str =
    include_str!("../../../desktop/src-tauri/src/commands/workspace.rs");
const IDENTITY_COMMAND: &str = include_str!("../../../desktop/src-tauri/src/commands/identity.rs");

#[test]
fn mandatory_o4_security_contracts_are_present() {
    let cases = [
        (
            "protected-header-denial",
            ASSERTION_VERIFIER.contains("headers.get_all(config.jwt_header.as_str())")
                && ASSERTION_VERIFIER.contains("values.next().is_some()")
                && ASSERTION_VERIFIER.contains("raw.contains(',')"),
        ),
        (
            "bounded-known-key-jwks-degradation",
            ASSERTION_VERIFIER.contains("JWKS_CACHE_MAX_AGE")
                && ASSERTION_VERIFIER.contains("buzz_jwks_stale_key_uses_total")
                && ASSERTION_VERIFIER.contains("buzz_jwks_unknown_kid_total")
                && ASSERTION_VERIFIER.contains("buzz_jwt_verification_errors_total"),
        ),
        (
            "lease-expiry-and-invalidation",
            TRANSPORT_RUNTIME.contains("expiry_delay")
                && INVALIDATION_RUNTIME.contains("cancel_invalid(community_id)"),
        ),
        (
            "revocation-enforcement-timing",
            INVALIDATION_RUNTIME.contains("buzz_authorization_revocation_to_enforcement_seconds"),
        ),
        (
            "client-status-fail-closed-degradation",
            AUTH_HANDLER.contains("client_status_unavailable")
                && AUTH_HANDLER.contains("buzz_client_status_degradation_total"),
        ),
        (
            "deny-protected-request-renewal-and-session-eviction",
            FINALIZATION_RUNTIME.contains("DenyProtected")
                && PRODUCTION_RUNTIME
                    .contains("\"deny_protected\" => AuthorizationMode::DenyProtected")
                && TRANSPORT_RUNTIME.contains("AuthorizationMode::DenyProtected =>")
                && TRANSPORT_RUNTIME.contains("deny_protected_request(request)")
                && TRANSPORT_RUNTIME.contains("request.cancellation()")
                && TRANSPORT_RUNTIME.contains("cancellation.cancel()")
                && TRANSPORT_RUNTIME.contains("ProtectedTransportError::DenyProtected"),
        ),
    ];

    for (name, present) in cases {
        assert!(present, "missing mandatory O4 security contract: {name}");
    }

    for source in [ASSERTION_VERIFIER, INVALIDATION_RUNTIME, AUTH_HANDLER] {
        let observability = source
            .lines()
            .filter(|line| line.contains("metrics::"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "identity_token =",
            "access_token =",
            "refresh_token =",
            "uid =",
            "subject =",
            "kid =",
        ] {
            assert!(
                !observability.contains(forbidden),
                "security observability gained a private label: {forbidden}"
            );
        }
    }
}

#[test]
fn synthetic_fixture_names_required_nip_fi_and_j3c_rows() {
    let fixture: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture JSON parses");
    assert_eq!(fixture["fixture_classification"], "synthetic-only");
    assert_eq!(fixture["full_stack_conformance_claim"], false);
    assert_eq!(fixture["presentation_gate"], "disabled");

    for row in [
        "TR-1.direct-bypass",
        "TR-1.inbound-header-copy",
        "TR-1.complete-deployment-evidence",
        "AS-3.future-iat",
        "BD-1.cross-domain",
        "SE-4.invalidation",
        "DG-3.no-finite-bound",
        "OP-2.discovery",
        "OP-3.absent",
        "OP-3.implemented",
        "OP-4.privacy",
        "J3C-STATUS-RELAY-SIGNER",
        "J3C-STATUS-EXACT-SCOPE",
        "J3C-STATUS-FRESHNESS",
        "J3C-STATUS-REVISION-FOLD",
        "J3C-STATUS-WITHDRAWAL",
        "J3C-STATUS-PRIVACY",
        "J3C-STATUS-VERIFY-ONLY",
        "J3C-STATUS-DEDICATED-TRANSPORT",
        "J3C-STATUS-REAL-USER-HIDDEN",
    ] {
        assert!(FIXTURE.contains(row), "fixture omitted required row {row}");
    }

    let future_iat = fixture["nip_fi_rows"]
        .as_array()
        .expect("NIP-FI rows are an array")
        .iter()
        .find(|row| row["row"] == "AS-3.future-iat")
        .expect("future-iat allocation exists");
    assert_eq!(future_iat["owner"], "o4-client-status");
    assert_eq!(future_iat["status"], "covered");

    let projection = fixture["nip_fi_rows"]
        .as_array()
        .expect("NIP-FI rows are an array")
        .iter()
        .find(|row| row["row"] == "OP-3.implemented")
        .expect("implemented projection allocation exists");
    assert_eq!(
        projection["status"],
        "disabled-pending-approved-rfc-presentation-gate"
    );
}

#[test]
fn optional_iat_uses_the_shared_injected_authorization_clock() {
    assert!(ASSERTION_VERIFIER.contains("self.authorization_clock.now()?"));
    assert!(ASSERTION_VERIFIER.contains("validate_optional_iat("));
    assert!(
        !ASSERTION_VERIFIER
            .contains("validate_optional_iat(&decoded.claims.claims, Timestamp::now().as_secs())"),
        "optional iat must not bypass the injected authorization clock"
    );
}

#[test]
fn client_authored_status_is_rejected_by_the_central_relay_only_fence() {
    let relay_only_predicate = KIND_REGISTRY
        .split("pub const fn is_relay_only_kind")
        .nth(1)
        .and_then(|suffix| suffix.split("/// Extract the kind").next())
        .expect("central relay-only predicate exists");
    assert!(relay_only_predicate.contains("KIND_CLIENT_BINDING_STATUS"));
    assert!(INGEST_HANDLER.contains("buzz_core::kind::is_relay_only_kind(kind_u32)"));
    assert!(INGEST_HANDLER.contains("restricted: relay-only kind"));
}

#[test]
fn public_projection_retirement_is_durable_internal_and_not_operator_wired() {
    let production = ASSERTION_VERIFIER
        .split("#[cfg(test)]")
        .next()
        .expect("production assertion verifier exists");
    let migration =
        include_str!("../../../migrations/0044_identity_public_projection_retirement.sql");
    assert!(migration.contains("identity_public_projection_retirements"));
    assert!(migration.contains("source_binding_id"));
    assert!(migration.contains("source_binding_version"));
    for private in [
        "issuer TEXT",
        "uid",
        "display_name",
        "actor",
        "reason",
        "identity_token",
        "access_token",
        "refresh_token",
    ] {
        assert!(
            !migration.contains(private),
            "projection retirement state gained private field {private}"
        );
    }
    assert!(
        production.contains("begin_active_public_projection"),
        "active projection publication must revalidate the exact binding at its database boundary"
    );
    let production_runtime = include_str!("../src/authorization_runtime/production.rs");
    assert!(
        production_runtime.contains("reconcile_public_projection_retirements_startup"),
        "committed lifecycle retirement must be reconciled before protected runtime installation"
    );
    assert!(
        production_runtime.contains("run_public_projection_retirement_reconciliation"),
        "committed lifecycle retirement needs durable periodic/restart reconciliation"
    );
    for operator_surface in [
        include_str!("../src/api/operator.rs"),
        include_str!("../src/api/bridge.rs"),
    ] {
        assert!(!operator_surface.contains("PublicProjectionRetirement"));
    }
}

#[test]
fn trusted_proxy_fixture_fails_closed_without_both_deployment_controls() {
    let fixture: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture JSON parses");
    let cases = fixture["trusted_proxy_cases"]
        .as_array()
        .expect("trusted proxy cases are an array");
    assert_eq!(cases.len(), 3);

    for case in cases {
        let isolation = case["origin_isolation_enforced"]
            .as_bool()
            .expect("fixture isolation flag is boolean");
        let stripping = case["inbound_assertion_header_stripped"]
            .as_bool()
            .expect("fixture stripping flag is boolean");
        let expected = case["expected"]
            .as_str()
            .expect("fixture expectation is a string");
        if isolation && stripping {
            assert_eq!(expected, "eligible-for-provider-verification");
        } else {
            assert_eq!(expected, "deny-before-verification");
        }
    }
}

#[test]
fn verification_only_adapter_has_no_authority_storage_or_pubsub_dependency() {
    let production = STATUS_MODULE
        .split("#[cfg(test)]")
        .next()
        .expect("production status module exists");
    for forbidden in [
        "AuthContext",
        "AuthorizationLease",
        "CapabilitySet",
        "AuthState",
        "buzz_db",
        "buzz_pubsub",
        "publish_event",
        "store_event",
        "KIND_USER_TRUSTED_ASSERTION",
        "corporate_identity",
    ] {
        assert!(
            !production.contains(forbidden),
            "verification-only status gained forbidden authority path {forbidden}"
        );
    }

    assert_eq!(
        production
            .matches("pub struct ClientStatusPresentationPermit {")
            .count(),
        1,
        "the disabled presentation permit must have one opaque definition"
    );
    assert!(production.contains("impl ClientStatusPresentationPermit"));
    assert!(production.contains("pub fn from_complete_stack("));
    assert!(production.contains("reviewed_implementation_revision"));
    assert!(production.contains("presentation_gate_passed"));
    assert!(production.contains("dedicated_client_contract_passed"));
    assert!(
        !production.contains("std::env"),
        "presentation must not be enabled by an environment boolean"
    );

    let current_issuance = production
        .split("pub fn issue_verification_only")
        .nth(1)
        .and_then(|suffix| suffix.split("/// Sign a generic withdrawal").next())
        .expect("current-display issuance method exists");
    assert!(
        current_issuance.contains("&ClientStatusPresentationPermit"),
        "current-display signing must require the complete-stack permit"
    );
    assert_eq!(
        production
            .matches("issue_current(&evidence, label)")
            .count(),
        1,
        "no second production current-display signing path may bypass the permit"
    );
}

#[test]
fn status_uses_only_the_dedicated_authenticated_production_path() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo = manifest
        .parent()
        .and_then(Path::parent)
        .expect("relay crate is nested under repository crates directory");
    let roots = [
        manifest.join("src/handlers"),
        manifest.join("src/api"),
        manifest.join("src/main.rs"),
        manifest.join("src/router.rs"),
        manifest.join("src/subscription.rs"),
        manifest.join("src/connection.rs"),
        manifest.join("src/protocol.rs"),
        repo.join("desktop/src"),
        repo.join("desktop/src-tauri/src"),
        repo.join("mobile/lib"),
        repo.join("web/src"),
    ];

    for root in roots {
        for file in source_files(&root) {
            if file.ends_with("src/handlers/auth.rs") {
                continue;
            }
            let source = fs::read_to_string(&file).expect("source file is readable");
            let native_socket_tests =
                file == repo.join("desktop/src-tauri/src/native_websocket_tests.rs");
            if native_socket_tests {
                assert!(NATIVE_WEBSOCKET
                    .contains("#[cfg(test)]\n#[path = \"native_websocket_tests.rs\"]\nmod tests;"));
                continue;
            }
            let native_session =
                file == repo.join("desktop/src-tauri/src/client_binding_status_session.rs");
            let native_socket = file == repo.join("desktop/src-tauri/src/native_websocket.rs");
            let native_socket_status =
                file == repo.join("desktop/src-tauri/src/native_websocket_status.rs");
            let native_module_declaration = file == repo.join("desktop/src-tauri/src/lib.rs");
            if native_module_declaration {
                assert_eq!(
                    source.matches("mod client_binding_status_session;").count(),
                    1,
                    "native status module must have one private declaration"
                );
            }
            let source_to_scan = if native_module_declaration {
                source.replacen("mod client_binding_status_session;", "", 1)
            } else {
                source.clone()
            };
            for forbidden in [
                "KIND_CLIENT_BINDING_STATUS",
                "ClientBindingStatus",
                "client_binding_status",
                "24244",
                "deliver_verification_only",
            ] {
                let expected_native_count = match (
                    native_session,
                    native_socket,
                    native_socket_status,
                    forbidden,
                ) {
                    (true, false, false, "KIND_CLIENT_BINDING_STATUS") => Some(1),
                    (true, false, false, "ClientBindingStatus") => Some(33),
                    (true, false, false, "client_binding_status") => Some(2),
                    (false, true, false, "ClientBindingStatus") => Some(2),
                    (false, true, false, "client_binding_status") => Some(1),
                    (false, false, true, "ClientBindingStatus") => Some(3),
                    (false, false, true, "client_binding_status") => Some(1),
                    _ => None,
                };
                if let Some(expected) = expected_native_count {
                    assert_eq!(
                        source_to_scan.matches(forbidden).count(),
                        expected,
                        "{} changed the narrow native status allowance for {forbidden}",
                        file.display()
                    );
                    continue;
                }
                assert!(
                    !source_to_scan.contains(forbidden),
                    "{} exposes status through an ordinary route {forbidden}",
                    file.display()
                );
            }
        }
    }

    let handler = fs::read_to_string(manifest.join("src/handlers/auth.rs"))
        .expect("AUTH handler source is readable");
    let state = fs::read_to_string(manifest.join("src/state.rs"))
        .expect("application state source is readable");
    let status = fs::read_to_string(manifest.join("src/authorization_runtime/status.rs"))
        .expect("status runtime source is readable");
    let nip11 =
        fs::read_to_string(manifest.join("src/nip11.rs")).expect("NIP-11 source is readable");
    let router =
        fs::read_to_string(manifest.join("src/router.rs")).expect("router source is readable");
    assert!(handler.contains("present_after_auth"));
    assert!(state.contains("install_client_status_runtime"));
    assert!(state.contains("install_nip_fi_discovery"));
    assert!(status.contains("from_complete_stack"));
    assert!(status.contains("__buzz_client_binding_status_v1__"));
    assert!(nip11.contains("state.nip_fi_discovery()"));
    assert!(nip11.contains("with_conformant_federated_identity"));
    assert!(router.contains("nip11_document(&state, raw_host).await"));
    assert!(!status.contains("std::env"));
}

#[test]
fn scope_mutations_finalize_the_status_fence_before_propagating_failure() {
    let workspace = WORKSPACE_COMMAND
        .split("pub async fn apply_workspace")
        .nth(1)
        .and_then(|suffix| suffix.split("#[tauri::command]").next())
        .expect("workspace command body exists");
    let identity = IDENTITY_COMMAND
        .split("pub async fn import_identity")
        .nth(1)
        .and_then(|suffix| suffix.split("/// Commit an imported identity").next())
        .expect("identity command body exists");

    for (name, command, result_propagation) in [
        ("workspace", workspace, "mutation_result?"),
        ("identity", identity, "let identity = identity_result?"),
    ] {
        let begin = command
            .find(".begin_scope_mutation()")
            .unwrap_or_else(|| panic!("{name} must enter the status mutation fence"));
        let blocking = command
            .find("spawn_blocking")
            .unwrap_or_else(|| panic!("{name} blocking mutation exists"));
        let finish = command
            .find(".finish_scope_mutation()")
            .unwrap_or_else(|| panic!("{name} must exit the status mutation fence"));
        let propagate = command
            .find(result_propagation)
            .unwrap_or_else(|| panic!("{name} must propagate its stored result"));
        assert!(
            begin < blocking && blocking < finish && finish < propagate,
            "{name} must finalize the status fence on success, error, or join failure"
        );
        assert!(
            !command.contains(".invalidate_projection()"),
            "{name} must not leave a reconnect gap between independent invalidations"
        );
    }
}

#[test]
fn auth_bootstrap_precedes_status_and_success_ack() {
    let bootstrap = AUTH_HANDLER
        .find("conn.send(RelayMessage::event(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &event))")
        .expect("AUTH queues the exact-connection bootstrap");
    let presentation = AUTH_HANDLER
        .find(".present_after_auth(")
        .expect("AUTH invokes O4 presentation");
    let success = AUTH_HANDLER
        .find("conn.send(RelayMessage::ok(&event_id_hex, true, \"\"))")
        .expect("AUTH queues the successful NIP-42 acknowledgement");

    assert!(
        bootstrap < presentation && presentation < success,
        "bootstrap must be queued before O4 status and the NIP-42 OK"
    );
    assert!(AUTH_HANDLER.contains("ClientBindingScopeV1::from_verified_auth_event"));
    assert!(AUTH_HANDLER.contains("scope.relay_signer() == state.relay_keypair.public_key()"));
    assert!(AUTH_HANDLER.contains("if let (Some(runtime), Some(assertion), Some(scope)) ="));
    assert!(AUTH_HANDLER.contains("let presentation = if bootstrap_queued"));
    assert!(!include_str!("../src/router.rs").contains("CLIENT_BINDING_EPOCH_HEADER"));
    assert!(!READ_ONLY_RELAY_CLIENT.contains("onProjection"));
    assert!(!READ_ONLY_RELAY_CLIENT.contains("nativeWebsocketId"));
}

#[test]
fn native_status_connect_is_dedicated_and_primary_composition_is_bound() {
    let ordinary = NATIVE_WEBSOCKET
        .split("async fn connect(")
        .nth(1)
        .and_then(|suffix| suffix.split("#[tauri::command]").next())
        .expect("ordinary native connect command exists");
    let status = NATIVE_WEBSOCKET
        .split("async fn connect_with_status(")
        .nth(1)
        .and_then(|suffix| suffix.split("async fn connect_internal(").next())
        .expect("dedicated status connect command exists");

    assert!(!ordinary.contains("on_projection"));
    assert!(status.contains("on_projection: Channel<serde_json::Value>"));
    assert!(!status.contains("Option<Channel<serde_json::Value>>"));
    assert!(NATIVE_WEBSOCKET.contains("connect_with_status,"));
    assert!(DESKTOP_BUILD.contains("\"connect_with_status\""));

    assert!(PRIMARY_RELAY_CLIENT.contains("RelayClientStatusConnection"));
    assert!(PRIMARY_RELAY_CLIENT.contains("statusConnection.connect("));
    assert!(PRIMARY_RELAY_CLIENT.contains("statusConnection.bind(wsId, connectionRelayUrl);"));
    assert!(PRIMARY_RELAY_CLIENT.contains("statusConnection.handleAuthChallenge(rest[0])"));
    assert!(STATUS_RELAY_CLIENT.contains("\"plugin:websocket|connect_with_status\""));
    assert!(!STATUS_RELAY_CLIENT.contains("\"plugin:websocket|connect\""));
    assert!(STATUS_RELAY_CLIENT.contains("onProjection: this.projectionChannel"));
    assert!(STATUS_RELAY_CLIENT.contains("nativeWebsocketId: binding.id"));
}

#[test]
fn neutral_verified_evidence_is_exactly_one_and_reachable_in_production() {
    assert!(TRANSPORT_RUNTIME.contains("trait VerifiedProviderEvidenceResolver"));
    assert!(TRANSPORT_RUNTIME.contains("VerifiedProviderEvidenceResolution::Absent"));
    assert!(TRANSPORT_RUNTIME.contains("VerifiedProviderEvidenceResolution::One(evidence)"));
    assert!(TRANSPORT_RUNTIME.contains("VerifiedProviderEvidenceResolution::Ambiguous"));
    assert!(TRANSPORT_RUNTIME.contains("ConflictingProviderEvidence"));
    assert!(TRANSPORT_RUNTIME.contains("provider_evidence_resolver_is_installed"));
    assert!(PRODUCTION_RUNTIME.contains("install_from_environment_with_providers_and_evidence"));
    assert!(PRODUCTION_RUNTIME.contains("normalized_provider_evidence"));
    assert!(PRODUCTION_RUNTIME.contains("provider_evidence_missing"));
    assert!(PRODUCTION_RUNTIME.contains("provider_evidence_invalid"));

    for route in [
        include_str!("../src/api/bridge.rs"),
        include_str!("../src/api/invites.rs"),
        include_str!("../src/api/media.rs"),
        include_str!("../src/api/git/transport.rs"),
        include_str!("../src/audio/handler.rs"),
        AUTH_HANDLER,
    ] {
        assert!(
            route.contains("provider_evidence_resolver_is_installed"),
            "protected route omitted the neutral evidence seam"
        );
    }
}

fn source_files(root: &Path) -> Vec<PathBuf> {
    if root.is_file() {
        return vec![root.to_path_buf()];
    }
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("source directory is readable") {
            let path = entry.expect("source directory entry is readable").path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    matches!(extension, "rs" | "ts" | "tsx" | "js" | "jsx" | "dart")
                })
            {
                files.push(path);
            }
        }
    }
    files
}
