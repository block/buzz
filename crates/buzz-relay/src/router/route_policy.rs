//! Central inventory of corporate-identity policy at the HTTP routing boundary.
//!
//! This module classifies axum's *matched route template* (for example,
//! `/media/{sha256_ext}`), not an untrusted literal request path. Keeping the
//! complete inventory here makes every authenticated surface and every
//! deliberate exemption reviewable in one place. The handlers remain the
//! enforcement point because they have the authenticated principal, resolved
//! tenant, and admission result needed to finalize an identity safely.

use axum::http::Method;

/// Why a route deliberately does not use tenant corporate-identity auth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CorporateIdentityExemption {
    /// Public relay metadata (NIP-05, NIP-11-adjacent information).
    PublicMetadata,
    /// Kubernetes/service health endpoint.
    HealthProbe,
    /// Public pre-membership policy and policy-acceptance bootstrap.
    JoinBootstrap,
    /// Deployment-global operator NIP-98 allowlist, outside tenant auth.
    OperatorAuth,
    /// Deployment-admin host/session authentication, outside tenant auth.
    AdminAuth,
    /// Per-workflow secret authentication.
    WebhookSecret,
    /// Loopback-only, HMAC-authenticated Git hook callback.
    LocalHookCallback,
    /// Disabled-by-default mesh testbed endpoint.
    TestbedOnly,
}

/// Corporate-identity policy for a registered route.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CorporateIdentityRoutePolicy {
    /// Authenticate and enforce corporate identity during this HTTP request.
    Required,
    /// Enforce when the upgraded WebSocket performs its protocol auth flow.
    RequiredAtSessionAuth,
    /// Public only when protected media reads are disabled; otherwise required.
    RequiredWhenMediaReadsProtected,
    /// Deliberately outside tenant corporate-identity authentication.
    Exempt(CorporateIdentityExemption),
}

impl CorporateIdentityRoutePolicy {
    /// Stable, low-cardinality label used on HTTP trace spans.
    pub(super) const fn trace_label(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::RequiredAtSessionAuth => "required_at_session_auth",
            Self::RequiredWhenMediaReadsProtected => "required_when_media_reads_protected",
            Self::Exempt(CorporateIdentityExemption::PublicMetadata) => "exempt_public_metadata",
            Self::Exempt(CorporateIdentityExemption::HealthProbe) => "exempt_health_probe",
            Self::Exempt(CorporateIdentityExemption::JoinBootstrap) => "exempt_join_bootstrap",
            Self::Exempt(CorporateIdentityExemption::OperatorAuth) => "exempt_operator_auth",
            Self::Exempt(CorporateIdentityExemption::AdminAuth) => "exempt_admin_auth",
            Self::Exempt(CorporateIdentityExemption::WebhookSecret) => "exempt_webhook_secret",
            Self::Exempt(CorporateIdentityExemption::LocalHookCallback) => {
                "exempt_local_hook_callback"
            }
            Self::Exempt(CorporateIdentityExemption::TestbedOnly) => "exempt_testbed_only",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RoutePolicyRule {
    method: &'static str,
    matched_path: &'static str,
    policy: CorporateIdentityRoutePolicy,
}

const REQUIRED: CorporateIdentityRoutePolicy = CorporateIdentityRoutePolicy::Required;
const SESSION: CorporateIdentityRoutePolicy = CorporateIdentityRoutePolicy::RequiredAtSessionAuth;
const PROTECTED_MEDIA: CorporateIdentityRoutePolicy =
    CorporateIdentityRoutePolicy::RequiredWhenMediaReadsProtected;

const fn exempt(exemption: CorporateIdentityExemption) -> CorporateIdentityRoutePolicy {
    CorporateIdentityRoutePolicy::Exempt(exemption)
}

/// Exhaustive inventory of registered relay routes.
///
/// Static UI fallback paths are intentionally absent: they do not have an
/// axum `MatchedPath` and cannot reach an API handler. A missing API entry is
/// visible as `unclassified` in the HTTP trace span and must be added here as
/// part of registering the route.
const ROUTE_POLICY_RULES: &[RoutePolicyRule] = &[
    // Protocol and public metadata.
    RoutePolicyRule {
        method: "GET",
        matched_path: "/",
        policy: SESSION,
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/info",
        policy: exempt(CorporateIdentityExemption::PublicMetadata),
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/.well-known/nostr.json",
        policy: exempt(CorporateIdentityExemption::PublicMetadata),
    },
    // Health routes on the primary and health-only listeners.
    RoutePolicyRule {
        method: "GET",
        matched_path: "/health",
        policy: exempt(CorporateIdentityExemption::HealthProbe),
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/_liveness",
        policy: exempt(CorporateIdentityExemption::HealthProbe),
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/_readiness",
        policy: exempt(CorporateIdentityExemption::HealthProbe),
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/_status",
        policy: exempt(CorporateIdentityExemption::HealthProbe),
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/_mesh",
        policy: exempt(CorporateIdentityExemption::HealthProbe),
    },
    // NIP-98 HTTP bridge.
    RoutePolicyRule {
        method: "POST",
        matched_path: "/events",
        policy: REQUIRED,
    },
    RoutePolicyRule {
        method: "POST",
        matched_path: "/query",
        policy: REQUIRED,
    },
    RoutePolicyRule {
        method: "POST",
        matched_path: "/count",
        policy: REQUIRED,
    },
    // Deployment-global operator control plane.
    RoutePolicyRule {
        method: "GET",
        matched_path: "/operator/communities",
        policy: exempt(CorporateIdentityExemption::OperatorAuth),
    },
    RoutePolicyRule {
        method: "POST",
        matched_path: "/operator/communities",
        policy: exempt(CorporateIdentityExemption::OperatorAuth),
    },
    RoutePolicyRule {
        method: "POST",
        matched_path: "/operator/communities/archive",
        policy: exempt(CorporateIdentityExemption::OperatorAuth),
    },
    RoutePolicyRule {
        method: "POST",
        matched_path: "/operator/communities/unarchive",
        policy: exempt(CorporateIdentityExemption::OperatorAuth),
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/operator/communities/availability",
        policy: exempt(CorporateIdentityExemption::OperatorAuth),
    },
    RoutePolicyRule {
        method: "POST",
        matched_path: "/operator/communities/transfer",
        policy: exempt(CorporateIdentityExemption::OperatorAuth),
    },
    // Invite admission and its deliberately public pre-join policy surface.
    RoutePolicyRule {
        method: "POST",
        matched_path: "/api/invites",
        policy: REQUIRED,
    },
    RoutePolicyRule {
        method: "POST",
        matched_path: "/api/invites/claim",
        policy: REQUIRED,
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/api/join-policy",
        policy: exempt(CorporateIdentityExemption::JoinBootstrap),
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/api/join-policy/terms",
        policy: exempt(CorporateIdentityExemption::JoinBootstrap),
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/api/join-policy/privacy",
        policy: exempt(CorporateIdentityExemption::JoinBootstrap),
    },
    RoutePolicyRule {
        method: "POST",
        matched_path: "/api/invites/accept-policy",
        policy: exempt(CorporateIdentityExemption::JoinBootstrap),
    },
    // Moderation data is tenant-authenticated even though it is not event data.
    RoutePolicyRule {
        method: "GET",
        matched_path: "/moderation/reports",
        policy: REQUIRED,
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/moderation/audit",
        policy: REQUIRED,
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/moderation/restricted",
        policy: REQUIRED,
    },
    // Alternate-auth and test-only callbacks.
    RoutePolicyRule {
        method: "POST",
        matched_path: "/hooks/{id}",
        policy: exempt(CorporateIdentityExemption::WebhookSecret),
    },
    RoutePolicyRule {
        method: "POST",
        matched_path: "/_mesh/demo/echo",
        policy: exempt(CorporateIdentityExemption::TestbedOnly),
    },
    RoutePolicyRule {
        method: "POST",
        matched_path: "/internal/git/policy",
        policy: exempt(CorporateIdentityExemption::LocalHookCallback),
    },
    // Huddle authentication is performed inside the upgraded socket.
    RoutePolicyRule {
        method: "GET",
        matched_path: "/huddle/{channel_id}/audio",
        policy: SESSION,
    },
    // Blossom media: writes are always authenticated; reads are configurable.
    RoutePolicyRule {
        method: "PUT",
        matched_path: "/upload",
        policy: REQUIRED,
    },
    RoutePolicyRule {
        method: "PUT",
        matched_path: "/media/upload",
        policy: REQUIRED,
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/media/{sha256_ext}",
        policy: PROTECTED_MEDIA,
    },
    RoutePolicyRule {
        method: "HEAD",
        matched_path: "/media/{sha256_ext}",
        policy: PROTECTED_MEDIA,
    },
    // Git smart HTTP is tenant-authenticated on every request.
    RoutePolicyRule {
        method: "GET",
        matched_path: "/git/{owner}/{repo}/info/refs",
        policy: REQUIRED,
    },
    RoutePolicyRule {
        method: "POST",
        matched_path: "/git/{owner}/{repo}/git-upload-pack",
        policy: REQUIRED,
    },
    RoutePolicyRule {
        method: "POST",
        matched_path: "/git/{owner}/{repo}/git-receive-pack",
        policy: REQUIRED,
    },
    // Deployment-admin APIs use the dedicated admin-host auth middleware.
    RoutePolicyRule {
        method: "GET",
        matched_path: "/api/admin/v1/reports",
        policy: exempt(CorporateIdentityExemption::AdminAuth),
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/api/admin/v1/reports/{id}",
        policy: exempt(CorporateIdentityExemption::AdminAuth),
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/api/admin/v1/feedback",
        policy: exempt(CorporateIdentityExemption::AdminAuth),
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/api/admin/v1/feedback/{id}",
        policy: exempt(CorporateIdentityExemption::AdminAuth),
    },
    RoutePolicyRule {
        method: "GET",
        matched_path: "/api/admin/v1/feedback/{id}/attachments/{sha256}",
        policy: exempt(CorporateIdentityExemption::AdminAuth),
    },
];

/// Classify a registered method and axum matched-path template.
pub(super) fn classify_matched_route(
    method: &Method,
    matched_path: &str,
) -> Option<CorporateIdentityRoutePolicy> {
    let exact = ROUTE_POLICY_RULES
        .iter()
        .find(|rule| rule.method == method.as_str() && rule.matched_path == matched_path)
        .map(|rule| rule.policy);
    if exact.is_some() || method != Method::HEAD {
        return exact;
    }

    // axum automatically serves HEAD through GET routes when no explicit HEAD
    // handler is registered. Mirror that routing fallback so those requests
    // cannot appear unclassified. The explicit protected-media HEAD rule above
    // wins before this branch.
    ROUTE_POLICY_RULES
        .iter()
        .find(|rule| rule.method == "GET" && rule.matched_path == matched_path)
        .map(|rule| rule.policy)
}

/// Whether this matched template is registered in the inventory for any
/// method. An unknown method on a known template is a 405, not a new route.
pub(super) fn is_known_matched_path(matched_path: &str) -> bool {
    ROUTE_POLICY_RULES
        .iter()
        .any(|rule| rule.matched_path == matched_path)
}

/// RFC 9110 `Allow` value for a known matched template. GET routes include
/// Axum's implicit HEAD support.
pub(super) fn allowed_methods(matched_path: &str) -> Option<String> {
    let mut methods = Vec::new();
    for rule in ROUTE_POLICY_RULES
        .iter()
        .filter(|rule| rule.matched_path == matched_path)
    {
        if !methods.contains(&rule.method) {
            methods.push(rule.method);
        }
        if rule.method == "GET" && !methods.contains(&"HEAD") {
            methods.push("HEAD");
        }
    }
    (!methods.is_empty()).then(|| methods.join(", "))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn policy(method: Method, path: &str) -> CorporateIdentityRoutePolicy {
        classify_matched_route(&method, path)
            .unwrap_or_else(|| panic!("missing route policy for {method} {path}"))
    }

    #[test]
    fn every_policy_rule_has_a_unique_method_and_path() {
        let mut seen = HashSet::new();
        for rule in ROUTE_POLICY_RULES {
            assert!(
                seen.insert((rule.method, rule.matched_path)),
                "duplicate route policy for {} {}",
                rule.method,
                rule.matched_path
            );
        }
    }

    #[test]
    fn every_tenant_authenticated_http_route_requires_corporate_identity() {
        let routes = [
            (Method::POST, "/events"),
            (Method::POST, "/query"),
            (Method::POST, "/count"),
            (Method::POST, "/api/invites"),
            (Method::POST, "/api/invites/claim"),
            (Method::GET, "/moderation/reports"),
            (Method::GET, "/moderation/audit"),
            (Method::GET, "/moderation/restricted"),
            (Method::PUT, "/upload"),
            (Method::PUT, "/media/upload"),
            (Method::GET, "/git/{owner}/{repo}/info/refs"),
            (Method::POST, "/git/{owner}/{repo}/git-upload-pack"),
            (Method::POST, "/git/{owner}/{repo}/git-receive-pack"),
        ];
        for (method, path) in routes {
            assert_eq!(policy(method, path), CorporateIdentityRoutePolicy::Required);
        }
    }

    #[test]
    fn websocket_and_media_policies_capture_deferred_and_conditional_auth() {
        assert_eq!(
            policy(Method::GET, "/"),
            CorporateIdentityRoutePolicy::RequiredAtSessionAuth
        );
        assert_eq!(
            policy(Method::GET, "/huddle/{channel_id}/audio"),
            CorporateIdentityRoutePolicy::RequiredAtSessionAuth
        );
        for method in [Method::GET, Method::HEAD] {
            assert_eq!(
                policy(method, "/media/{sha256_ext}"),
                CorporateIdentityRoutePolicy::RequiredWhenMediaReadsProtected
            );
        }
    }

    #[test]
    fn privileged_non_tenant_surfaces_have_narrow_named_exemptions() {
        let routes = [
            (
                Method::POST,
                "/operator/communities/archive",
                CorporateIdentityExemption::OperatorAuth,
            ),
            (
                Method::GET,
                "/api/admin/v1/reports",
                CorporateIdentityExemption::AdminAuth,
            ),
            (
                Method::POST,
                "/hooks/{id}",
                CorporateIdentityExemption::WebhookSecret,
            ),
            (
                Method::POST,
                "/internal/git/policy",
                CorporateIdentityExemption::LocalHookCallback,
            ),
        ];
        for (method, path, exemption) in routes {
            assert_eq!(
                policy(method, path),
                CorporateIdentityRoutePolicy::Exempt(exemption)
            );
        }
    }

    #[test]
    fn public_routes_are_explicit_and_unknown_routes_are_unclassified() {
        assert_eq!(
            policy(Method::GET, "/.well-known/nostr.json"),
            CorporateIdentityRoutePolicy::Exempt(CorporateIdentityExemption::PublicMetadata)
        );
        assert_eq!(
            policy(Method::GET, "/_readiness"),
            CorporateIdentityRoutePolicy::Exempt(CorporateIdentityExemption::HealthProbe)
        );
        assert_eq!(
            policy(Method::GET, "/api/join-policy"),
            CorporateIdentityRoutePolicy::Exempt(CorporateIdentityExemption::JoinBootstrap)
        );
        assert_eq!(
            policy(Method::POST, "/_mesh/demo/echo"),
            CorporateIdentityRoutePolicy::Exempt(CorporateIdentityExemption::TestbedOnly)
        );
        assert_eq!(
            policy(Method::HEAD, "/info"),
            CorporateIdentityRoutePolicy::Exempt(CorporateIdentityExemption::PublicMetadata),
            "axum's automatic GET-to-HEAD fallback inherits the GET policy"
        );
        assert_eq!(classify_matched_route(&Method::GET, "/events"), None);
        assert_eq!(classify_matched_route(&Method::GET, "/unknown"), None);
        assert_eq!(
            classify_matched_route(&Method::GET, "/media/literal-sha"),
            None,
            "the classifier accepts trusted matched templates, not literal paths"
        );
    }

    #[test]
    fn known_path_detection_distinguishes_method_fallbacks_from_new_routes() {
        assert!(is_known_matched_path("/events"));
        assert!(is_known_matched_path("/health"));
        assert!(!is_known_matched_path("/new-unclassified-route"));
        assert_eq!(allowed_methods("/events").as_deref(), Some("POST"));
        assert_eq!(allowed_methods("/info").as_deref(), Some("GET, HEAD"));
        assert_eq!(allowed_methods("/new-unclassified-route"), None);
    }
}
