//! Router adapter for the provider-neutral protected-surface inventory.

use axum::http::Method;

#[cfg(test)]
use crate::protected_surface::SurfaceExemption as CorporateIdentityExemption;
pub(super) use crate::protected_surface::SurfaceProtection as CorporateIdentityRoutePolicy;

/// Classify a registered method and Axum matched-path template.
pub(super) fn classify_matched_route(
    method: &Method,
    matched_path: &str,
) -> Option<CorporateIdentityRoutePolicy> {
    crate::protected_surface::classify_http(method, matched_path).map(|entry| entry.protection)
}

/// Whether this matched template is registered for any method.
pub(super) fn is_known_matched_path(matched_path: &str) -> bool {
    crate::protected_surface::is_known_http_path(matched_path)
}

/// RFC 9110 `Allow` value for a known matched template.
pub(super) fn allowed_methods(matched_path: &str) -> Option<String> {
    crate::protected_surface::allowed_http_methods(matched_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_auth::AuthorizationCapability;

    fn policy(method: Method, path: &str) -> CorporateIdentityRoutePolicy {
        classify_matched_route(&method, path)
            .unwrap_or_else(|| panic!("missing route policy for {method} {path}"))
    }

    #[test]
    fn tenant_routes_map_to_exact_portable_capabilities() {
        let read_routes = [
            (
                Method::POST,
                "/query",
                AuthorizationCapability::CommunityRead,
            ),
            (
                Method::GET,
                "/moderation/reports",
                AuthorizationCapability::Moderate,
            ),
        ];
        for (method, path, capability) in read_routes {
            assert_eq!(
                policy(method, path),
                CorporateIdentityRoutePolicy::Capability(capability)
            );
        }
        let unavailable_mutation_routes = [(
            Method::POST,
            "/hooks/{id}",
            AuthorizationCapability::CommunityWrite,
        )];
        for (method, path, capability) in unavailable_mutation_routes {
            assert_eq!(
                policy(method, path),
                CorporateIdentityRoutePolicy::AtomicMutationUnavailable(capability)
            );
        }
        for (method, path, capability) in [
            (
                Method::POST,
                "/api/invites",
                AuthorizationCapability::InviteMint,
            ),
            (
                Method::POST,
                "/api/invites/claim",
                AuthorizationCapability::InviteClaim,
            ),
            (Method::PUT, "/upload", AuthorizationCapability::MediaWrite),
            (
                Method::POST,
                "/git/{owner}/{repo}/git-receive-pack",
                AuthorizationCapability::GitWrite,
            ),
        ] {
            assert_eq!(
                policy(method, path),
                CorporateIdentityRoutePolicy::AtomicMutation(capability)
            );
        }
        assert_eq!(
            policy(Method::POST, "/events"),
            CorporateIdentityRoutePolicy::DynamicAtomicMutation
        );
        assert_eq!(
            policy(Method::GET, "/git/{owner}/{repo}/info/refs"),
            CorporateIdentityRoutePolicy::DynamicCapability
        );
    }

    #[test]
    fn websocket_and_media_policies_are_deferred_or_conditional() {
        assert_eq!(
            policy(Method::GET, "/"),
            CorporateIdentityRoutePolicy::ConditionalWebSocketUpgrade
        );
        assert_eq!(
            policy(Method::HEAD, "/"),
            CorporateIdentityRoutePolicy::Exempt(CorporateIdentityExemption::PublicMetadata)
        );
        assert_eq!(
            policy(Method::GET, "/huddle/{channel_id}/audio"),
            CorporateIdentityRoutePolicy::LeasedSession {
                capability: AuthorizationCapability::AudioJoin
            }
        );
        for method in [Method::GET, Method::HEAD] {
            assert_eq!(
                policy(method, "/media/{sha256_ext}"),
                CorporateIdentityRoutePolicy::ConditionalMediaRead(
                    AuthorizationCapability::MediaRead
                )
            );
        }
    }

    #[test]
    fn exemptions_are_narrow_and_unknown_routes_are_unclassified() {
        assert_eq!(
            policy(Method::GET, "/_readiness"),
            CorporateIdentityRoutePolicy::Exempt(CorporateIdentityExemption::HealthProbe)
        );
        assert_eq!(
            policy(Method::POST, "/internal/git/policy"),
            CorporateIdentityRoutePolicy::Exempt(CorporateIdentityExemption::LocalHookCallback)
        );
        assert_eq!(
            policy(Method::HEAD, "/info"),
            CorporateIdentityRoutePolicy::Exempt(CorporateIdentityExemption::PublicMetadata)
        );
        assert_eq!(classify_matched_route(&Method::GET, "/unknown"), None);
        assert!(is_known_matched_path("/events"));
        assert_eq!(allowed_methods("/events").as_deref(), Some("POST"));
    }
}
