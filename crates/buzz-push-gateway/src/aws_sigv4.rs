//! The AWS subject token Google's workload identity federation accepts: a
//! SigV4-signed STS `GetCallerIdentity` request, serialized as Google
//! documents it. Google's token service replays the request to STS, so STS
//! vouches for the caller's AWS identity without any long-lived key.
//!
//! Only the one request shape Google defines is implemented: a bodiless POST
//! to the `sts` service.

use aws_lc_rs::{
    digest::{digest, SHA256},
    hmac,
};
use chrono::DateTime;
use thiserror::Error;
use url::Url;

use crate::aws_credentials::AwsCredentials;

const SERVICE: &str = "sts";
const ALGORITHM: &str = "AWS4-HMAC-SHA256";

/// The request could not be signed. It carries no credential material.
#[derive(Debug, Error)]
pub enum SigningError {
    #[error("the STS verification URL is not an absolute URL with a host")]
    Url,
    #[error("the request time is outside the representable range")]
    Time,
}

/// Build Google's URL-encoded subject token for `credentials`.
///
/// `url_template` is the credential file's `regional_cred_verification_url`
/// with its `{region}` placeholder, and `audience` the file's `audience`.
pub fn subject_token(
    url_template: &str,
    region: &str,
    audience: &str,
    credentials: &AwsCredentials,
    now: i64,
) -> Result<String, SigningError> {
    let request_url = url_template.replace("{region}", region);
    let parsed = Url::parse(&request_url).map_err(|_| SigningError::Url)?;
    let host = match (parsed.host_str().ok_or(SigningError::Url)?, parsed.port()) {
        (host, Some(port)) => format!("{host}:{port}"),
        (host, None) => host.to_owned(),
    };
    let time = DateTime::from_timestamp(now, 0).ok_or(SigningError::Time)?;
    let amz_date = time.format("%Y%m%dT%H%M%SZ").to_string();
    let date = &amz_date[..8];

    // Sorted by lowercase name, as SigV4 requires.
    let mut headers = vec![
        ("host", host),
        ("x-amz-date", amz_date.clone()),
        ("x-goog-cloud-target-resource", audience.to_owned()),
    ];
    if let Some(token) = &credentials.session_token {
        headers.push(("x-amz-security-token", token.clone()));
    }
    headers.sort_by(|a, b| a.0.cmp(b.0));
    let signed_headers = headers
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(";");
    let canonical_headers: String = headers
        .iter()
        .map(|(name, value)| format!("{name}:{}\n", value.trim()))
        .collect();

    let mut query: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (uri_encode(&k), uri_encode(&v)))
        .collect();
    query.sort();
    let canonical_query = query
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let payload_hash = hex::encode(digest(&SHA256, b"").as_ref());
    let canonical_request = format!(
        "POST\n{}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
        parsed.path()
    );

    let scope = format!("{date}/{region}/{SERVICE}/aws4_request");
    let string_to_sign = format!(
        "{ALGORITHM}\n{amz_date}\n{scope}\n{}",
        hex::encode(digest(&SHA256, canonical_request.as_bytes()).as_ref())
    );
    let mut key = hmac_sha256(
        format!("AWS4{}", credentials.secret_access_key).as_bytes(),
        date.as_bytes(),
    );
    for part in [region, SERVICE, "aws4_request"] {
        key = hmac_sha256(&key, part.as_bytes());
    }
    let signature = hex::encode(hmac_sha256(&key, string_to_sign.as_bytes()));
    let authorization = format!(
        "{ALGORITHM} Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        credentials.access_key_id
    );

    // Google's serialization: Authorization first, then every signed header.
    let mut serialized = vec![serde_json::json!({"key": "Authorization", "value": authorization})];
    serialized.extend(
        headers
            .iter()
            .map(|(name, value)| serde_json::json!({"key": name, "value": value})),
    );
    let token = serde_json::json!({
        "url": request_url,
        "method": "POST",
        "headers": serialized,
    });
    Ok(uri_encode(&token.to_string()))
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data)
        .as_ref()
        .to_vec()
}

/// Percent-encode everything except SigV4's unreserved characters. This is
/// also `encodeURIComponent`'s output for the ASCII JSON Google's token holds,
/// apart from the characters `encodeURIComponent` leaves alone (`!'()*`),
/// which Google's token service decodes either way.
fn uri_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUDIENCE: &str = "//iam.googleapis.com/projects/123456789/locations/global/workloadIdentityPools/example-pool/providers/example-aws";
    const URL: &str =
        "https://sts.{region}.amazonaws.com?Action=GetCallerIdentity&Version=2011-06-15";
    /// 2015-08-30T12:36:00Z, AWS's SigV4 documentation date.
    const NOW: i64 = 1_440_938_160;

    fn credentials(session_token: Option<&str>) -> AwsCredentials {
        AwsCredentials {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: session_token.map(Into::into),
        }
    }

    fn decoded(token: &str) -> serde_json::Value {
        let json: Vec<u8> = url::form_urlencoded::parse(format!("x={token}").as_bytes())
            .next()
            .unwrap()
            .1
            .bytes()
            .collect();
        serde_json::from_slice(&json).unwrap()
    }

    /// The signatures were computed by an independent implementation (a
    /// Python hashlib/hmac script following the SigV4 specification), not by
    /// this module.
    #[test]
    fn subject_token_matches_known_answer_with_session_token() {
        let token = subject_token(
            URL,
            "us-east-1",
            AUDIENCE,
            &credentials(Some("EXAMPLETOKEN")),
            NOW,
        )
        .unwrap();
        assert!(!token.contains(['{', '"', ' ', ':', '/']), "{token}");
        assert_eq!(
            decoded(&token),
            serde_json::json!({
                "url": "https://sts.us-east-1.amazonaws.com?Action=GetCallerIdentity&Version=2011-06-15",
                "method": "POST",
                "headers": [
                    {"key": "Authorization", "value": "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/sts/aws4_request, SignedHeaders=host;x-amz-date;x-amz-security-token;x-goog-cloud-target-resource, Signature=a04e017a1fa273c71fc30f2633c72fba1736281950c42041d49495ba6e69611f"},
                    {"key": "host", "value": "sts.us-east-1.amazonaws.com"},
                    {"key": "x-amz-date", "value": "20150830T123600Z"},
                    {"key": "x-amz-security-token", "value": "EXAMPLETOKEN"},
                    {"key": "x-goog-cloud-target-resource", "value": AUDIENCE},
                ],
            })
        );
    }

    #[test]
    fn subject_token_matches_known_answer_without_session_token() {
        let token = subject_token(URL, "us-east-1", AUDIENCE, &credentials(None), NOW).unwrap();
        let token = decoded(&token);
        assert_eq!(
            token["headers"][0]["value"],
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/sts/aws4_request, SignedHeaders=host;x-amz-date;x-goog-cloud-target-resource, Signature=de946cd9ee4a1b9b4fef72f8258d35ba32e7c6e62954b0a297eb2843cf63fcdd"
        );
        assert_eq!(token["headers"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn unsignable_requests_fail() {
        assert!(matches!(
            subject_token("not a url", "us-east-1", AUDIENCE, &credentials(None), NOW),
            Err(SigningError::Url)
        ));
        assert!(matches!(
            subject_token(URL, "us-east-1", AUDIENCE, &credentials(None), i64::MAX),
            Err(SigningError::Time)
        ));
    }
}
