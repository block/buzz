use std::io::Read;

use rand::Rng;

use crate::client::{sign_nip98_request, BuzzClient};
use crate::{AuthCmd, CliError};

const MAX_REQUEST_BODY_BYTES: usize = 1024;
const MAX_RESPONSE_BODY_BYTES: usize = 1024 * 1024;

pub async fn dispatch(sub: AuthCmd, client: &BuzzClient) -> Result<(), CliError> {
    match sub {
        AuthCmd::Nip98Request {
            method,
            url,
            audience,
            body,
        } => nip98_request(client, &method, &url, &audience, &body).await,
    }
}

async fn nip98_request(
    client: &BuzzClient,
    method: &str,
    url: &str,
    audience: &str,
    body_source: &str,
) -> Result<(), CliError> {
    let parsed = validate_request(method, url, body_source)?;

    let mut input = std::io::stdin().take((MAX_REQUEST_BODY_BYTES + 1) as u64);
    let mut body = Vec::new();
    input
        .read_to_end(&mut body)
        .map_err(|e| CliError::Usage(format!("failed to read request body: {e}")))?;
    if body.len() > MAX_REQUEST_BODY_BYTES {
        return Err(CliError::Usage(format!(
            "request body exceeds {MAX_REQUEST_BODY_BYTES} bytes"
        )));
    }

    let mut nonce = [0_u8; 32];
    rand::rng().fill_bytes(&mut nonce);
    let authorization = sign_nip98_request(
        client.keys(),
        method,
        url,
        Some(&body),
        Some(audience),
        &hex::encode(nonce),
    )?;

    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = http
        .post(parsed)
        .header(reqwest::header::AUTHORIZATION, authorization)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_RESPONSE_BODY_BYTES {
        return Err(CliError::Other(format!(
            "response body exceeds {MAX_RESPONSE_BODY_BYTES} bytes"
        )));
    }
    let body = String::from_utf8_lossy(&bytes);
    let body_value = serde_json::from_slice::<serde_json::Value>(&bytes)
        .unwrap_or_else(|_| serde_json::Value::String(body.into_owned()));
    println!(
        "{}",
        serde_json::json!({
            "status": status.as_u16(),
            "body": body_value,
        })
    );
    Ok(())
}

fn validate_request(method: &str, url: &str, body_source: &str) -> Result<url::Url, CliError> {
    if method != "POST" {
        return Err(CliError::Usage(
            "nip98-request currently permits only POST".into(),
        ));
    }
    if body_source != "-" {
        return Err(CliError::Usage(
            "nip98-request body must be read from stdin with --body -".into(),
        ));
    }

    let parsed =
        url::Url::parse(url).map_err(|e| CliError::Usage(format!("invalid request URL: {e}")))?;
    if parsed.scheme() != "https" {
        return Err(CliError::Usage(
            "nip98-request requires an https URL".into(),
        ));
    }
    if parsed.fragment().is_some() || parsed.username() != "" || parsed.password().is_some() {
        return Err(CliError::Usage(
            "nip98-request URL cannot contain credentials or a fragment".into(),
        ));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_bounded_https_post_from_stdin() {
        let parsed = validate_request("POST", "https://broker.example/health", "-").unwrap();
        assert_eq!(parsed.as_str(), "https://broker.example/health");
    }

    #[test]
    fn rejects_unsafe_request_shapes() {
        for (method, url, body) in [
            ("GET", "https://broker.example/health", "-"),
            ("POST", "http://broker.example/health", "-"),
            ("POST", "https://user@broker.example/health", "-"),
            ("POST", "https://broker.example/health#fragment", "-"),
            ("POST", "https://broker.example/health", "request.json"),
        ] {
            assert!(
                validate_request(method, url, body).is_err(),
                "request must be rejected: {method} {url} {body}"
            );
        }
    }
}
