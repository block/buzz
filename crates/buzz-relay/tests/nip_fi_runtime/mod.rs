//! Cross-module invariants for the provider-free relay runtime.

use crate::authorization_runtime::{
    InstalledAuthorizationRuntime, ProtectedEffect, ProtectedIngress, ProtectedResourceKind,
    ProviderFreeRuntimeConfig, ProviderFreeRuntimeMode, RouteAuthority, RouteRule,
};
use buzz_auth::{ProofTransport, RouteCapability};

const RUNTIME_SOURCES: &[(&str, &str)] = &[
    (
        "authority",
        include_str!("../../src/authorization_runtime/authority.rs"),
    ),
    (
        "config",
        include_str!("../../src/authorization_runtime/config.rs"),
    ),
    (
        "invalidation",
        include_str!("../../src/authorization_runtime/invalidation.rs"),
    ),
    (
        "jwks",
        include_str!("../../src/authorization_runtime/jwks.rs"),
    ),
    (
        "restore",
        include_str!("../../src/authorization_runtime/restore.rs"),
    ),
    (
        "routes",
        include_str!("../../src/authorization_runtime/routes.rs"),
    ),
    (
        "canonical_admission",
        include_str!("../../src/authorization_runtime/canonical_admission.rs"),
    ),
    (
        "startup",
        include_str!("../../src/authorization_runtime/startup.rs"),
    ),
    (
        "status",
        include_str!("../../src/authorization_runtime/status.rs"),
    ),
];

#[test]
fn current_status_keeps_bootstrap_scope_ephemeral_and_retired_ledger_absent() {
    let bootstrap_kind = concat!("242", "45");
    let forbidden_runtime = [
        bootstrap_kind,
        concat!("client_", "binding_epoch"),
        concat!("client_status_", "revisions"),
        concat!("status_", "delivery_history"),
        concat!("status_", "receipt"),
    ];
    for (name, source) in RUNTIME_SOURCES {
        for needle in forbidden_runtime {
            assert!(
                !source.contains(needle),
                "{name} contains forbidden current-status state: {needle}"
            );
        }
    }

    // A connection epoch is a sealed in-memory scope coordinate. It must not
    // become a durable database field or restore the retired status ledger.
    let storage_sources = [
        (
            "status delivery outbox migration",
            include_str!("../../../../migrations/0038_client_status_delivery_outbox.sql"),
        ),
        (
            "connection scope migration",
            include_str!("../../../../migrations/0039_client_status_connection_scope.sql"),
        ),
        (
            "desired schema",
            include_str!("../../../../schema/schema.sql"),
        ),
    ];
    for (name, source) in storage_sources {
        for needle in [
            concat!("connection_", "epoch"),
            concat!("client_status_", "revisions"),
        ] {
            assert!(
                !source.contains(needle),
                "{name} contains forbidden durable current-status state: {needle}"
            );
        }
    }
}

#[test]
fn protected_ingress_inventory_is_closed_and_nonempty() {
    assert_eq!(ProtectedIngress::ALL.len(), 23);
    let unique: std::collections::BTreeSet<_> = ProtectedIngress::ALL.into_iter().collect();
    assert_eq!(unique.len(), ProtectedIngress::ALL.len());
}

#[test]
fn sole_config_absence_and_emergency_denial_are_exact() {
    assert_eq!(
        ProviderFreeRuntimeConfig::from_optional_json(None)
            .unwrap()
            .mode(),
        ProviderFreeRuntimeMode::Off
    );
    assert_eq!(
        ProviderFreeRuntimeConfig::from_optional_json(Some(r#"{"deny_protected":true}"#))
            .unwrap()
            .mode(),
        ProviderFreeRuntimeMode::DenyProtected
    );
}

#[test]
fn relay_invite_is_typed_and_admission_loss_fails_closed() {
    let rules = ProtectedIngress::ALL.into_iter().map(|ingress| {
        RouteRule::protected(
            ingress,
            ingress.required_capability(),
            ingress.required_resource(),
            ingress.required_effect(),
            ingress.required_transport(),
        )
    });
    let routes = RouteAuthority::new(rules).unwrap();
    let invite = routes
        .resolve(
            ProtectedIngress::InviteClaim,
            ProtectedIngress::InviteClaim.required_transport(),
        )
        .unwrap();
    assert_eq!(invite.capability(), RouteCapability::InviteClaim);
    assert_eq!(invite.resource(), ProtectedResourceKind::Invitation);
    assert_eq!(invite.effect(), ProtectedEffect::Mutate);
    assert_eq!(invite.transport(), ProofTransport::Nip98);

    let config = ProviderFreeRuntimeConfig::from_optional_json(Some(
        r#"{
            "issuer":"https://issuer.example",
            "audience":"buzz",
            "maximum_token_lifetime_seconds":300,
            "jwks":{"jwks_uri":"https://issuer.example/keys"},
            "lease":{"maximum_seconds":120},
            "policy_revision":1,
            "audit":{"max_events_per_domain":100,"max_bytes_per_domain":65536,"max_envelope_bytes":4096},
            "client_status_admission":{"max_presentations_per_domain":100,"max_presentations_per_actor":5,"max_presentations_per_peer":20},
            "transport":{"kind":"sealed_nostr_proof"},
            "enrollment":{"kind":"canonical_admission"},
            "restore":{"kind":"operation_manifest"}
        }"#,
    ))
    .unwrap();
    let held = InstalledAuthorizationRuntime::fail_closed(&config);
    assert!(held.denies_protected());
    assert!(held.routes().is_err());
}
