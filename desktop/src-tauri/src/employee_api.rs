//! Explicit staging pilot routing; the identity origin and signed URLs stay fixed.

use reqwest::{header::HeaderValue, Method, Request, RequestBuilder};

const TARGET_ENV: &str = "BUZZ_EMPLOYEE_NATIVE_PLAYPEN_TARGET";

pub(crate) fn request(builder: RequestBuilder) -> Result<Request, String> {
    let target = std::env::var(TARGET_ENV).ok();
    request_with_target(builder, target.as_deref())
}

fn request_with_target(builder: RequestBuilder, target: Option<&str>) -> Result<Request, String> {
    let mut request = builder
        .build()
        .map_err(|_| "Could not prepare Square identity request")?;
    // A request's ambient trace context must not choose its identity backend.
    request.headers_mut().remove("baggage");
    let Some(target) = target else {
        return Ok(request);
    };
    let valid_target = !target.is_empty()
        && target.len() <= 63
        && target.as_bytes()[0].is_ascii_lowercase()
        && target
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && target
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    let action = match request.url().path() {
        "/api/goose/v1/auth/merchant-team-member/native-handoff/redeem" => {
            request.method() == Method::POST
        }
        "/api/goose/v1/auth/me" => request.method() == Method::GET,
        "/api/goose/v1/buzz/identity/assertions" => request.method() == Method::POST,
        _ => false,
    };
    // reqwest removes URL user info and turns it into Basic authentication.
    // These fixed identity actions never accept that alternate auth channel.
    let valid_authorization = match request.headers().get("authorization") {
        None => true,
        Some(value) => {
            request.url().path() == "/api/goose/v1/buzz/identity/assertions"
                && value
                    .to_str()
                    .is_ok_and(|value| value.starts_with("Nostr "))
        }
    };
    let expected = format!(
        "https://square.builderlabstaging.xyz{}",
        request.url().path()
    );
    if !valid_target || !action || !valid_authorization || request.url().as_str() != expected {
        return Err("Square pilot routing is invalid".to_owned());
    }
    let value = HeaderValue::from_str(&format!("kgoose-builderbot-playpen={target}"))
        .map_err(|_| "Square pilot routing is invalid")?;
    request.headers_mut().insert("baggage", value);
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pilot_routing_is_exact_overwrites_ambient_and_preserves_signed_request() {
        let client = reqwest::Client::new();
        for (method, path) in [
            (
                Method::POST,
                "/v1/auth/merchant-team-member/native-handoff/redeem",
            ),
            (Method::GET, "/v1/auth/me"),
            (Method::POST, "/v1/buzz/identity/assertions"),
        ] {
            let url = format!("{}{path}", crate::employee_login::API_BASE);
            let mut builder = client
                .request(method.clone(), &url)
                .header("baggage", "kgoose-builderbot-playpen=old")
                .header("baggage", "untrusted=value")
                .header("X-BB-Session-Credential", "synthetic-credential")
                .body("exact-signed-body");
            let is_assertion = path == "/v1/buzz/identity/assertions";
            if is_assertion {
                builder = builder.header("Authorization", "Nostr synthetic-proof");
            }
            let request = request_with_target(builder, Some("employee-pilot-621")).unwrap();
            assert_eq!(request.url().as_str(), url);
            assert_eq!(request.method(), method);
            assert_eq!(
                request.body().unwrap().as_bytes(),
                Some(b"exact-signed-body".as_slice())
            );
            if is_assertion {
                assert_eq!(request.headers()["Authorization"], "Nostr synthetic-proof");
            } else {
                assert!(!request.headers().contains_key("authorization"));
            }
            assert_eq!(
                request.headers()["X-BB-Session-Credential"],
                "synthetic-credential"
            );
            assert_eq!(request.headers().get_all("baggage").iter().count(), 1);
            assert_eq!(
                request.headers()["baggage"],
                "kgoose-builderbot-playpen=employee-pilot-621"
            );
        }
        let request = request_with_target(
            client
                .get(format!("{}/v1/auth/me", crate::employee_login::API_BASE))
                .header("baggage", "kgoose-builderbot-playpen=ambient"),
            None,
        )
        .unwrap();
        assert!(!request.headers().contains_key("baggage"));
    }

    #[test]
    fn routing_refuses_other_origins_actions_methods_and_header_injection() {
        let client = reqwest::Client::new();
        for target in [
            "",
            "EMP621",
            "-emp621",
            "emp621-",
            "emp621,other=value",
            "emp621\r\nx: y",
            "emp_621",
            &"e".repeat(64),
        ] {
            assert!(request_with_target(
                client.get(format!("{}/v1/auth/me", crate::employee_login::API_BASE)),
                Some(target)
            )
            .is_err());
        }
        for (method,url) in [
            (Method::GET, "https://app.builderlab.xyz/api/goose/v1/auth/me"),
            (Method::GET, "https://login.dev.builderlab.xyz/oauth/token"),
            (Method::GET, "http://square.builderlabstaging.xyz/api/goose/v1/auth/me"),
            (Method::GET, "https://square.builderlabstaging.xyz:444/api/goose/v1/auth/me"),
            (Method::GET, "https://user@square.builderlabstaging.xyz/api/goose/v1/auth/me"),
            (Method::GET, "https://square.builderlabstaging.xyz/api/goose/v1/auth/me?target=other"),
            (Method::GET, "https://square.builderlabstaging.xyz/api/goose/v1/auth/me#other"),
            (Method::POST, "https://square.builderlabstaging.xyz/api/goose/v1/auth/me"),
            (Method::GET, "https://square.builderlabstaging.xyz/api/goose/v1/buzz/identity/assertions"),
            (Method::POST, "https://square.builderlabstaging.xyz/api/goose/v1/auth/merchant-team-member/exchange"),
        ] {
            assert!(request_with_target(client.request(method,url), Some("emp621")).is_err(), "{url}");
        }
    }
}
