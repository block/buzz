//! AWS SigV4 request signing for Bedrock API calls.
//!
//! Uses the `aws-sigv4` crate from the official Rust SDK to sign HTTP
//! requests with Signature Version 4, which AWS Bedrock requires instead
//! of bearer tokens.
//!
//! Credentials are loaded from the environment (`AWS_ACCESS_KEY_ID`,
//! `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`). IAM role support (IMDS,
//! ECS, IRSA) can be added by integrating `aws-config` in a follow-up.

use aws_credential_types::Credentials as AwsCreds;
use aws_sigv4::http_request::{sign, SignableBody, SignableRequest, SigningSettings};
use aws_sigv4::sign::v4;
use aws_smithy_runtime_api::client::identity::Identity;
use http::Request;
use std::time::SystemTime;

/// AWS credentials used for SigV4 signing.
#[derive(Debug, Clone)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

/// Sign an HTTP request with AWS SigV4.
///
/// `service` is typically `"bedrock"`. `region` is the AWS region
/// (e.g. `"us-east-1"`).
pub fn sign_request(
    mut request: Request<Vec<u8>>,
    creds: &AwsCredentials,
    service: &str,
    region: &str,
) -> Result<Request<Vec<u8>>, String> {
    let identity: Identity = AwsCreds::new(
        &creds.access_key_id,
        &creds.secret_access_key,
        creds.session_token.clone(),
        None,
        "buzz-agent",
    )
    .into();

    let uri_str = request.uri().to_string();
    let signable = SignableRequest::new(
        request.method().as_str(),
        uri_str.as_str(),
        request
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str(), v.to_str().unwrap_or_default())),
        SignableBody::Bytes(request.body()),
    )
    .map_err(|e| format!("signable request: {e}"))?;

    let settings = SigningSettings::default();
    let params: aws_sigv4::http_request::SigningParams<'_> = v4::SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name(service)
        .time(SystemTime::now())
        .settings(settings)
        .build()
        .map_err(|e| format!("signing params: {e}"))?
        .into();

    let signing_output = sign(signable, &params).map_err(|e| format!("signing: {e}"))?;

    let (instructions, _signature) = signing_output.into_parts();
    instructions.apply_to_request_http1x(&mut request);

    Ok(request)
}

/// Load AWS credentials from the environment.
///
/// Checks (in order):
/// 1. `AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY` (standard)
/// 2. `AWS_ACCESS_KEY` + `AWS_SECRET_KEY` (legacy)
///
/// `AWS_SESSION_TOKEN` is loaded when present for temporary credentials
/// (STS role assumptions, EKS IRSA, etc.).
pub fn load_aws_credentials() -> Result<AwsCredentials, String> {
    let access_key_id = std::env::var("AWS_ACCESS_KEY_ID")
        .or_else(|_| std::env::var("AWS_ACCESS_KEY"))
        .map_err(|_| "config: AWS_ACCESS_KEY_ID required for Bedrock".to_string())?;
    let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY")
        .or_else(|_| std::env::var("AWS_SECRET_KEY"))
        .map_err(|_| "config: AWS_SECRET_ACCESS_KEY required for Bedrock".to_string())?;
    let session_token = std::env::var("AWS_SESSION_TOKEN")
        .ok()
        .filter(|s| !s.is_empty());

    Ok(AwsCredentials {
        access_key_id,
        secret_access_key,
        session_token,
    })
}

/// Extract the AWS region from a Bedrock runtime base URL.
///
/// Accepts:
/// - `https://bedrock-runtime.{region}.amazonaws.com`
/// - `https://bedrock-runtime.{region}.amazonaws.com/v1`
pub fn parse_bedrock_region(base_url: &str) -> Result<String, String> {
    let rest = base_url
        .strip_prefix("https://bedrock-runtime.")
        .ok_or_else(|| format!("Bedrock: could not extract region from base_url: {base_url}"))?;
    let region = rest
        .strip_suffix(".amazonaws.com")
        .or_else(|| rest.strip_suffix(".amazonaws.com/v1"))
        .ok_or_else(|| {
            format!("Bedrock: could not extract region from base_url: {base_url}")
        })?;
    Ok(region.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_request_adds_authorization_header() {
        let creds = AwsCredentials {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        let body = r#"{"messages":[]}"#;
        let req = Request::builder()
            .uri("https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-3-5-sonnet-20241022-v2:0/converse")
            .method("POST")
            .header("Content-Type", "application/json")
            .body(body.as_bytes().to_vec())
            .unwrap();
        let signed = sign_request(req, &creds, "bedrock", "us-east-1").unwrap();
        let auth = signed
            .headers()
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            auth.starts_with("AWS4-HMAC-SHA256"),
            "expected SigV4 auth header, got: {auth}"
        );
        assert!(
            auth.contains("us-east-1/bedrock/"),
            "expected region/service in credential scope, got: {auth}"
        );
    }

    #[test]
    fn test_sign_request_with_session_token() {
        let creds = AwsCredentials {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: Some("IQoJb3JpZ2luX2IQoJb3JpZ2luX2IQ".into()),
        };
        let body = r#"{"messages":[]}"#;
        let req = Request::builder()
            .uri("https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-3-5-sonnet-20241022-v2:0/converse")
            .method("POST")
            .header("Content-Type", "application/json")
            .body(body.as_bytes().to_vec())
            .unwrap();
        let signed = sign_request(req, &creds, "bedrock", "us-east-1").unwrap();
        let auth = signed
            .headers()
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(auth.starts_with("AWS4-HMAC-SHA256"));
        let st = signed
            .headers()
            .get("x-amz-security-token")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(st, "IQoJb3JpZ2luX2IQoJb3JpZ2luX2IQ");
    }

    #[test]
    fn test_parse_bedrock_region_standard() {
        let region =
            parse_bedrock_region("https://bedrock-runtime.us-east-1.amazonaws.com").unwrap();
        assert_eq!(region, "us-east-1");
    }

    #[test]
    fn test_parse_bedrock_region_with_v1_suffix() {
        let region =
            parse_bedrock_region("https://bedrock-runtime.eu-west-1.amazonaws.com/v1").unwrap();
        assert_eq!(region, "eu-west-1");
    }

    #[test]
    fn test_parse_bedrock_region_invalid_url() {
        let result = parse_bedrock_region("https://api.openai.com/v1");
        assert!(result.is_err());
    }

    #[test]
    fn test_load_credentials_from_env() {
        let old_key = std::env::var("AWS_ACCESS_KEY_ID").ok();
        let old_secret = std::env::var("AWS_SECRET_ACCESS_KEY").ok();
        let old_token = std::env::var("AWS_SESSION_TOKEN").ok();

        std::env::set_var("AWS_ACCESS_KEY_ID", "test-key");
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "test-secret");
        std::env::remove_var("AWS_SESSION_TOKEN");

        let creds = load_aws_credentials().unwrap();
        assert_eq!(creds.access_key_id, "test-key");
        assert_eq!(creds.secret_access_key, "test-secret");
        assert!(creds.session_token.is_none());

        // Restore original env vars
        if let Some(k) = old_key {
            std::env::set_var("AWS_ACCESS_KEY_ID", k);
        } else {
            std::env::remove_var("AWS_ACCESS_KEY_ID");
        }
        if let Some(s) = old_secret {
            std::env::set_var("AWS_SECRET_ACCESS_KEY", s);
        } else {
            std::env::remove_var("AWS_SECRET_ACCESS_KEY");
        }
        if let Some(t) = old_token {
            std::env::set_var("AWS_SESSION_TOKEN", t);
        } else {
            std::env::remove_var("AWS_SESSION_TOKEN");
        }
    }
}
