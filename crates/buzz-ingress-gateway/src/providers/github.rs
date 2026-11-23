//! GitHub signs the raw body with HMAC-SHA256 under `X-Hub-Signature-256`.
//! There is no timestamp, so freshness comes from dedupe on
//! `X-GitHub-Delivery` alone.
use super::MAX_EVENT_TYPE_BYTES;
use super::{
    any_secret_signs, header_str, valid_delivery_id, ProviderAdapter, Verified, VerifyError,
};
use crate::store::Provider;
use axum::http::HeaderMap;
use chrono::{DateTime, Duration, Utc};

const SIGNATURE_HEADER: &str = "x-hub-signature-256";
const DELIVERY_HEADER: &str = "x-github-delivery";
const EVENT_HEADER: &str = "x-github-event";
const MAX_EVENT_NAME_BYTES: usize = 64;

/// GitHub webhook adapter: HMAC-SHA256 over the raw body, delivery id from
/// `X-GitHub-Delivery`, event type from `X-GitHub-Event` plus `action`.
pub struct GitHub;

impl ProviderAdapter for GitHub {
    fn provider(&self) -> Provider {
        Provider::Github
    }

    fn retained_headers(&self) -> &'static [&'static str] {
        &[
            "content-type",
            DELIVERY_HEADER,
            EVENT_HEADER,
            "x-github-hook-id",
            "x-github-hook-installation-target-id",
            "x-github-hook-installation-target-type",
        ]
    }

    fn verify(
        &self,
        headers: &HeaderMap,
        body: &[u8],
        secrets: &[Vec<u8>],
        _now: DateTime<Utc>,
        _tolerance: Duration,
    ) -> Result<Verified, VerifyError> {
        let signature = header_str(headers, SIGNATURE_HEADER)
            .ok_or(VerifyError::MissingSignature)?
            .strip_prefix("sha256=")
            .ok_or(VerifyError::MalformedHeaders)?;
        let signature = hex::decode(signature).map_err(|_| VerifyError::MalformedHeaders)?;
        if signature.len() != 32 {
            return Err(VerifyError::MalformedHeaders);
        }
        let delivery_id =
            header_str(headers, DELIVERY_HEADER).ok_or(VerifyError::MalformedHeaders)?;
        if !valid_delivery_id(delivery_id) {
            return Err(VerifyError::MalformedHeaders);
        }
        let event = header_str(headers, EVENT_HEADER).ok_or(VerifyError::MalformedHeaders)?;
        if event.is_empty()
            || event.len() > MAX_EVENT_NAME_BYTES
            || !event
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(VerifyError::MalformedHeaders);
        }
        if !any_secret_signs(secrets, body, &signature) {
            return Err(VerifyError::BadSignature);
        }
        let payload: serde_json::Value =
            serde_json::from_slice(body).map_err(|_| VerifyError::UnsupportedEvent)?;
        let event_type = match payload.get("action").and_then(|a| a.as_str()) {
            Some(action)
                if !action.is_empty()
                    && action
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') =>
            {
                format!("{event}.{action}")
            }
            Some(_) => return Err(VerifyError::UnsupportedEvent),
            None => event.to_owned(),
        };
        if event_type.len() > MAX_EVENT_TYPE_BYTES {
            return Err(VerifyError::UnsupportedEvent);
        }
        Ok(Verified {
            delivery_id: delivery_id.to_owned(),
            event_type,
            provider_timestamp: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    pub(crate) fn sign(secret: &[u8], body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    fn headers(signature: &str, delivery: &str, event: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(SIGNATURE_HEADER, signature.parse().unwrap());
        h.insert(DELIVERY_HEADER, delivery.parse().unwrap());
        h.insert(EVENT_HEADER, event.parse().unwrap());
        h
    }

    fn verify(h: &HeaderMap, body: &[u8], secrets: &[Vec<u8>]) -> Result<Verified, VerifyError> {
        GitHub.verify(h, body, secrets, Utc::now(), Duration::seconds(300))
    }

    #[test]
    fn any_active_secret_verifies_and_the_event_type_includes_the_action() {
        let body = br#"{"action":"opened","number":7}"#;
        let old = b"old-secret".to_vec();
        let new = b"new-secret".to_vec();
        let h = headers(
            &sign(&old, body),
            "72d3162e-cc78-11e3-81ab-4c9367dc0958",
            "pull_request",
        );
        let verified = verify(&h, body, &[new.clone(), old.clone()]).unwrap();
        assert_eq!(verified.event_type, "pull_request.opened");
        assert_eq!(verified.delivery_id, "72d3162e-cc78-11e3-81ab-4c9367dc0958");
        assert_eq!(verified.provider_timestamp, None);
        assert_eq!(
            verify(&h, body, &[new]).unwrap_err(),
            VerifyError::BadSignature
        );
        assert_eq!(
            verify(&h, b"{}", &[old]).unwrap_err(),
            VerifyError::BadSignature
        );
    }

    #[test]
    fn malformed_headers_and_payloads_are_classified() {
        let secret = b"s".to_vec();
        let body = br#"{"zen":"Keep it logically awesome."}"#;
        let good = sign(&secret, body);
        let ok = verify(
            &headers(&good, "d1", "ping"),
            body,
            std::slice::from_ref(&secret),
        )
        .unwrap();
        assert_eq!(ok.event_type, "ping");

        let mut missing = headers(&good, "d1", "ping");
        missing.remove(SIGNATURE_HEADER);
        assert_eq!(
            verify(&missing, body, std::slice::from_ref(&secret)).unwrap_err(),
            VerifyError::MissingSignature
        );
        assert_eq!(
            verify(
                &headers("sha1=abcd", "d1", "ping"),
                body,
                std::slice::from_ref(&secret)
            )
            .unwrap_err(),
            VerifyError::MalformedHeaders
        );
        assert_eq!(
            verify(
                &headers("sha256=zz", "d1", "ping"),
                body,
                std::slice::from_ref(&secret)
            )
            .unwrap_err(),
            VerifyError::MalformedHeaders
        );
        assert_eq!(
            verify(
                &headers(&good, "", "ping"),
                body,
                std::slice::from_ref(&secret)
            )
            .unwrap_err(),
            VerifyError::MalformedHeaders
        );
        assert_eq!(
            verify(
                &headers(&good, "d1", "Pull Request"),
                body,
                std::slice::from_ref(&secret)
            )
            .unwrap_err(),
            VerifyError::MalformedHeaders
        );
        let form = b"payload=%7B%7D";
        assert_eq!(
            verify(
                &headers(&sign(&secret, form), "d1", "ping"),
                form,
                std::slice::from_ref(&secret)
            )
            .unwrap_err(),
            VerifyError::UnsupportedEvent
        );
        let odd_action = br#"{"action":"Opened Now"}"#;
        assert_eq!(
            verify(
                &headers(&sign(&secret, odd_action), "d1", "ping"),
                odd_action,
                &[secret]
            )
            .unwrap_err(),
            VerifyError::UnsupportedEvent
        );
    }
}
