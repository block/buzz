use axum::http::HeaderMap;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct RequestContext {
    pub actor_user_id: Uuid,
    pub trace_id: Uuid,
}

#[derive(Clone)]
pub struct ServiceAuthenticator {
    credential_hash: [u8; 32],
    audience: String,
}

impl ServiceAuthenticator {
    pub fn new(credential: &str, audience: String) -> Self {
        Self {
            credential_hash: Sha256::digest(credential.as_bytes()).into(),
            audience,
        }
    }

    pub fn authenticate(&self, headers: &HeaderMap) -> Option<RequestContext> {
        let credential = headers
            .get("x-business-service-credential")?
            .to_str()
            .ok()?;
        let audience = headers.get("x-service-audience")?.to_str().ok()?;
        let provided_hash: [u8; 32] = Sha256::digest(credential.as_bytes()).into();
        if !bool::from(provided_hash.ct_eq(&self.credential_hash)) || audience != self.audience {
            return None;
        }
        let actor_user_id = headers
            .get("x-enterprise-user-id")?
            .to_str()
            .ok()
            .and_then(|value| Uuid::parse_str(value).ok())?;
        let trace_id = headers
            .get("x-trace-id")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| Uuid::parse_str(value).ok())
            .unwrap_or_else(Uuid::new_v4);
        Some(RequestContext {
            actor_user_id,
            trace_id,
        })
    }
}

pub fn valid_key(value: &str, max: usize) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    value.len() <= max
        && first.is_ascii_lowercase()
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ":_-".contains(ch))
}

pub fn valid_code(value: &str) -> bool {
    (2..=32).contains(&value.len())
        && value.bytes().all(|value| {
            value.is_ascii_uppercase() || value.is_ascii_digit() || b"_-".contains(&value)
        })
}

pub fn safe_text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.chars().count() <= max && !value.contains(['\r', '\n', '\0'])
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn service_auth_requires_all_bindings() {
        let user = Uuid::new_v4();
        let auth = ServiceAuthenticator::new(&"s".repeat(32), "business-core".into());
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-business-service-credential",
            HeaderValue::from_str(&"s".repeat(32)).unwrap(),
        );
        headers.insert(
            "x-service-audience",
            HeaderValue::from_static("business-core"),
        );
        headers.insert(
            "x-enterprise-user-id",
            HeaderValue::from_str(&user.to_string()).unwrap(),
        );
        assert_eq!(auth.authenticate(&headers).unwrap().actor_user_id, user);
        headers.insert("x-service-audience", HeaderValue::from_static("wrong"));
        assert!(auth.authenticate(&headers).is_none());
    }

    #[test]
    fn identifiers_are_closed_form() {
        assert!(valid_code("CN-SH_01"));
        assert!(!valid_code("cn sh"));
        assert!(valid_key("sales:order_read", 96));
        assert!(!valid_key("Sales.Read", 96));
    }
}
