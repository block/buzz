//! Deployment opt-in selects trusted staging relay origins, never employee authority.
//! The fixed identity service still checks the authenticated merchant/workspace and
//! own-key proof for every selected relay before issuing NIP-FI evidence.
use url::Url;

const STAGING_NAMESPACE: &str = "communities.square.builderlabstaging.xyz";

pub(crate) enum RelayPolicy {
    Exact(String),
    StagingNamespace,
}

pub(crate) fn configured() -> Result<Option<RelayPolicy>, String> {
    policy(
        std::env::var("BUZZ_EMPLOYEE_NATIVE_SIGN_IN")
            .ok()
            .as_deref(),
        std::env::var("BUZZ_EMPLOYEE_NATIVE_RELAY_URL")
            .ok()
            .as_deref(),
        std::env::var("BUZZ_EMPLOYEE_NATIVE_RELAY_NAMESPACE")
            .ok()
            .as_deref(),
    )
}

fn policy(
    enabled: Option<&str>,
    exact: Option<&str>,
    namespace: Option<&str>,
) -> Result<Option<RelayPolicy>, String> {
    if enabled != Some("1") {
        return Ok(None);
    }
    match (exact, namespace) {
        (None, Some(STAGING_NAMESPACE)) => Ok(Some(RelayPolicy::StagingNamespace)),
        (Some(value), None) => {
            let parsed = Url::parse(value).map_err(|_| "Square relay is invalid")?;
            let canonical = format!(
                "wss://{}",
                &parsed[url::Position::BeforeHost..url::Position::AfterPort]
            );
            if parsed.scheme() != "wss"
                || parsed.host_str().is_none()
                || parsed.port() == Some(80)
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || value != canonical
            {
                return Err("Square relay must be one canonical WSS origin".to_owned());
            }
            Ok(Some(RelayPolicy::Exact(canonical)))
        }
        _ => Err("Configure one Square relay or the staging community namespace".to_owned()),
    }
}

pub(crate) fn relay_for_url(value: &str) -> Result<Option<String>, String> {
    configured()?.map_or(Ok(None), |policy| policy.relay_for_url(value))
}

impl RelayPolicy {
    fn relay_for_url(&self, value: &str) -> Result<Option<String>, String> {
        let parsed = Url::parse(value).map_err(|_| "Invalid relay URL")?;
        let authority = &parsed[url::Position::BeforeHost..url::Position::AfterPort];
        let canonical = format!("wss://{authority}");
        let matches = match self {
            Self::Exact(expected) => canonical == *expected,
            Self::StagingNamespace => parsed.host_str().is_some_and(|host| {
                host.strip_suffix(STAGING_NAMESPACE)
                    .and_then(|prefix| prefix.strip_suffix('.'))
                    .is_some_and(|label| {
                        !label.is_empty()
                            && label.len() <= 63
                            && !label.starts_with('-')
                            && !label.ends_with('-')
                            && label.bytes().all(|byte| {
                                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                            })
                    })
            }),
        };
        if !matches {
            return Ok(None);
        }
        if !matches!(parsed.scheme(), "https" | "wss")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
            || !value
                .strip_prefix(&format!("{}://{authority}", parsed.scheme()))
                .is_some_and(|tail| {
                    tail.is_empty() || tail.starts_with('/') || tail.starts_with('?')
                })
            || (matches!(self, Self::StagingNamespace) && parsed.port().is_some())
        {
            return Err("Employee relay requires its exact TLS origin".to_owned());
        }
        Ok(Some(canonical))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_mode_is_explicit_staging_only_and_unambiguous() {
        assert!(policy(None, None, Some(STAGING_NAMESPACE))
            .unwrap()
            .is_none());
        assert!(policy(Some("1"), None, Some(STAGING_NAMESPACE)).is_ok());
        for namespace in [
            "",
            "*.communities.square.builderlabstaging.xyz",
            "builderlabstaging.xyz",
            "communities.square.builderlab.xyz",
            "evil.example",
        ] {
            assert!(policy(Some("1"), None, Some(namespace)).is_err());
        }
        assert!(policy(Some("1"), None, None).is_err());
        assert!(policy(
            Some("1"),
            Some("wss://pilot.example"),
            Some(STAGING_NAMESPACE)
        )
        .is_err());
        assert!(policy(Some("1"), Some("wss://pilot.example"), None).is_ok());
        for exact in [
            "https://pilot.example",
            "wss://pilot.example/",
            "wss://user@pilot.example",
            "wss://pilot.example?x=1",
        ] {
            assert!(policy(Some("1"), Some(exact), None).is_err());
        }
    }

    #[test]
    fn every_staging_merchant_uses_its_own_canonical_origin() {
        let policy = RelayPolicy::StagingNamespace;
        for label in [
            "viva3",
            "merchant-0123456789abcdef0123456789abcdef0123456789abcdef",
        ] {
            let host = format!("{label}.{STAGING_NAMESPACE}");
            for value in [
                format!("wss://{host}"),
                format!("https://{host}/query"),
                format!("https://{host}/media/file?size=small"),
            ] {
                assert_eq!(
                    policy.relay_for_url(&value).unwrap(),
                    Some(format!("wss://{host}"))
                );
            }
        }
        let exact = RelayPolicy::Exact(format!("wss://viva3.{STAGING_NAMESPACE}"));
        assert!(exact
            .relay_for_url(&format!("wss://salted.{STAGING_NAMESPACE}"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn namespace_refuses_lookalikes_nested_hosts_and_noncanonical_transport() {
        let policy = RelayPolicy::StagingNamespace;
        for value in [
            format!("wss://{STAGING_NAMESPACE}"),
            format!("wss://child.viva3.{STAGING_NAMESPACE}"),
            format!("wss://viva3.{STAGING_NAMESPACE}.evil.example"),
            format!("wss://viva3.{STAGING_NAMESPACE}."),
            format!("wss://-merchant.{STAGING_NAMESPACE}"),
            format!("wss://merchant-.{STAGING_NAMESPACE}"),
            format!("wss://{}.{STAGING_NAMESPACE}", "a".repeat(64)),
            "wss://viva3.communities.square.builderlab.xyz".to_owned(),
            "wss://127.0.0.1".to_owned(),
        ] {
            assert!(policy.relay_for_url(&value).unwrap().is_none(), "{value}");
        }
        for value in [
            format!("http://viva3.{STAGING_NAMESPACE}/query"),
            format!("ws://viva3.{STAGING_NAMESPACE}"),
            format!("wss://user@viva3.{STAGING_NAMESPACE}"),
            format!("https://viva3.{STAGING_NAMESPACE}:8443/query"),
            format!("https://viva3.{STAGING_NAMESPACE}:443/query"),
            format!("https://viva3.{STAGING_NAMESPACE}/query#fragment"),
            format!("https://VIVA3.{STAGING_NAMESPACE}/query"),
        ] {
            assert!(policy.relay_for_url(&value).is_err(), "{value}");
        }
    }
}
