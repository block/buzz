//! PagerDuty v3 webhooks sign the raw body with HMAC-SHA256 and send one or
//! more `v1=<hex>` signatures in `X-PagerDuty-Signature`; during a
//! provider-side rotation both the old and new signature are present.
use super::{
    any_secret_signs, header_str, valid_delivery_id, valid_event_type, ProviderAdapter, Verified,
    VerifyError,
};
use crate::store::Provider;
use axum::http::HeaderMap;
use chrono::{DateTime, Duration, Utc};

const SIGNATURE_HEADER: &str = "x-pagerduty-signature";
const MAX_SIGNATURES: usize = 8;

/// PagerDuty adapter: delivery id from `event.id`, type from
/// `event.event_type`, timestamp from `event.occurred_at`.
pub struct PagerDuty;

fn signatures(header: &str) -> Result<Vec<Vec<u8>>, VerifyError> {
    let mut out = Vec::new();
    for part in header.split(',') {
        let hex_digest = part
            .trim()
            .strip_prefix("v1=")
            .ok_or(VerifyError::MalformedHeaders)?;
        let digest = hex::decode(hex_digest).map_err(|_| VerifyError::MalformedHeaders)?;
        if digest.len() != 32 {
            return Err(VerifyError::MalformedHeaders);
        }
        out.push(digest);
        if out.len() > MAX_SIGNATURES {
            return Err(VerifyError::MalformedHeaders);
        }
    }
    if out.is_empty() {
        return Err(VerifyError::MalformedHeaders);
    }
    Ok(out)
}

impl ProviderAdapter for PagerDuty {
    fn provider(&self) -> Provider {
        Provider::Pagerduty
    }

    fn retained_headers(&self) -> &'static [&'static str] {
        &["content-type", "x-webhook-id", "x-webhook-subscription"]
    }

    fn verify(
        &self,
        headers: &HeaderMap,
        body: &[u8],
        secrets: &[Vec<u8>],
        _now: DateTime<Utc>,
        _tolerance: Duration,
    ) -> Result<Verified, VerifyError> {
        let header = header_str(headers, SIGNATURE_HEADER).ok_or(VerifyError::MissingSignature)?;
        let signatures = signatures(header)?;
        if !signatures
            .iter()
            .any(|signature| any_secret_signs(secrets, body, signature))
        {
            return Err(VerifyError::BadSignature);
        }
        let payload: serde_json::Value =
            serde_json::from_slice(body).map_err(|_| VerifyError::UnsupportedEvent)?;
        let event = payload.get("event").ok_or(VerifyError::UnsupportedEvent)?;
        let delivery_id = event
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|id| valid_delivery_id(id))
            .ok_or(VerifyError::UnsupportedEvent)?;
        let event_type = event
            .get("event_type")
            .and_then(|v| v.as_str())
            .filter(|t| valid_event_type(t))
            .ok_or(VerifyError::UnsupportedEvent)?;
        let provider_timestamp = event
            .get("occurred_at")
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Utc));
        Ok(Verified {
            delivery_id: delivery_id.to_owned(),
            event_type: event_type.to_owned(),
            provider_timestamp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    fn sign(secret: &[u8], body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(body);
        format!("v1={}", hex::encode(mac.finalize().into_bytes()))
    }

    fn headers(signature: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(SIGNATURE_HEADER, signature.parse().unwrap());
        h
    }

    fn verify(h: &HeaderMap, body: &[u8], secrets: &[Vec<u8>]) -> Result<Verified, VerifyError> {
        PagerDuty.verify(h, body, secrets, Utc::now(), Duration::seconds(300))
    }

    const BODY: &[u8] = br#"{"event":{"id":"01BRB6ZP4M6T8ZG4X6BP63ZB9P","event_type":"incident.triggered","resource_type":"incident","occurred_at":"2024-05-01T12:00:00.169Z","data":{"id":"PGR0VU2"}}}"#;

    #[test]
    fn any_listed_signature_under_any_active_secret_verifies() {
        let current = b"current".to_vec();
        let previous = b"previous".to_vec();
        let other = b"someone-else".to_vec();
        let rotated = format!("{},{}", sign(&other, BODY), sign(&previous, BODY));
        let verified = verify(&headers(&rotated), BODY, &[current.clone(), previous]).unwrap();
        assert_eq!(verified.delivery_id, "01BRB6ZP4M6T8ZG4X6BP63ZB9P");
        assert_eq!(verified.event_type, "incident.triggered");
        assert_eq!(
            verified.provider_timestamp.unwrap().to_rfc3339(),
            "2024-05-01T12:00:00.169+00:00"
        );
        assert_eq!(
            verify(&headers(&rotated), BODY, &[current]).unwrap_err(),
            VerifyError::BadSignature
        );
    }

    #[test]
    fn malformed_or_missing_signatures_are_rejected_before_hashing() {
        let secret = b"s".to_vec();
        assert_eq!(
            verify(&HeaderMap::new(), BODY, std::slice::from_ref(&secret)).unwrap_err(),
            VerifyError::MissingSignature
        );
        for bad in [
            "",
            "sha256=abcd",
            "v1=zz",
            "v1=abcd",
            &format!("{},", sign(&secret, BODY)),
        ] {
            assert_eq!(
                verify(&headers(bad), BODY, std::slice::from_ref(&secret)).unwrap_err(),
                VerifyError::MalformedHeaders,
                "{bad:?}"
            );
        }
        let many = vec![sign(&secret, BODY); MAX_SIGNATURES + 1].join(",");
        assert_eq!(
            verify(&headers(&many), BODY, std::slice::from_ref(&secret)).unwrap_err(),
            VerifyError::MalformedHeaders
        );
    }

    #[test]
    fn verified_bodies_still_need_a_well_formed_event() {
        let secret = b"s".to_vec();
        for body in [
            &b"not json"[..],
            br#"{"event":{"event_type":"incident.triggered"}}"#,
            br#"{"event":{"id":"x","event_type":"Incident Triggered"}}"#,
            br#"{"event":{"id":"","event_type":"incident.triggered"}}"#,
        ] {
            assert_eq!(
                verify(
                    &headers(&sign(&secret, body)),
                    body,
                    std::slice::from_ref(&secret)
                )
                .unwrap_err(),
                VerifyError::UnsupportedEvent,
                "{}",
                String::from_utf8_lossy(body)
            );
        }
        let no_time =
            br#"{"event":{"id":"x","event_type":"incident.resolved","occurred_at":"yesterday"}}"#;
        let verified = verify(
            &headers(&sign(&secret, no_time)),
            no_time,
            std::slice::from_ref(&secret),
        )
        .unwrap();
        assert_eq!(verified.provider_timestamp, None);
    }
}
