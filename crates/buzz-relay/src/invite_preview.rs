//! Public, generic invite metadata. Never inspect or redeem the invite token.

use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use std::{path::Path, time::Duration};
use url::Url;

/// The lookup only serves link-preview metadata; never hold the page for it.
const HOST_LOOKUP_DEADLINE: Duration = Duration::from_millis(250);

pub(crate) async fn response<R: crate::tenant::HostResolver>(
    index: &Path,
    relay_url: &str,
    resolver: &R,
    raw_host: &str,
) -> Response {
    match tokio::fs::read_to_string(index).await {
        Ok(html) => {
            let tenant = tokio::time::timeout(
                HOST_LOOKUP_DEADLINE,
                crate::tenant::bind_community(resolver, raw_host),
            )
            .await
            .ok()
            .and_then(Result::ok);
            Html(with_metadata(html, relay_url, tenant.as_ref())).into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn with_metadata(
    html: String,
    relay_url: &str,
    tenant: Option<&buzz_core::tenant::TenantContext>,
) -> String {
    let Some(tenant) = tenant else {
        return html;
    };
    let Ok(relay) = Url::parse(relay_url) else {
        return html;
    };
    let scheme = match relay.scheme() {
        "wss" | "https" => "https",
        "ws" | "http" => "http",
        _ => return html,
    };
    // The deployment URL supplies TLS posture only. The resolved request host
    // is authoritative; never advertise a neighboring community's seed host.
    let origin = format!("{scheme}://{}", tenant.host());
    let origin = origin
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let poster = format!("{origin}/assets/invite-preview/buzz-invite-poster-v1.png");
    let video = format!("{origin}/assets/invite-preview/buzz-invite-loop-v1.mp4");
    let secure = if scheme == "https" {
        format!("<meta property=\"og:video:secure_url\" content=\"{video}\">\n")
    } else {
        String::new()
    };
    let tags = format!(
        r#"<meta property="og:title" content="Join me on Buzz">
<meta property="og:description" content="A place for people and agents to work together.">
<meta property="og:site_name" content="Buzz">
<meta property="og:type" content="website">
<meta property="og:image" content="{poster}">
<meta property="og:image:type" content="image/png">
<meta property="og:image:width" content="960">
<meta property="og:image:height" content="540">
<meta property="og:image:alt" content="Buzz wordmark on a chartreuse background">
<meta property="og:video" content="{video}">
{secure}<meta property="og:video:type" content="video/mp4">
<meta property="og:video:width" content="960">
<meta property="og:video:height" content="540">
"#
    );
    html.replacen("</head>", &format!("{tags}</head>"), 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Resolver;
    impl crate::tenant::HostResolver for Resolver {
        type Error = ();
        async fn resolve_host(
            &self,
            host: &str,
        ) -> Result<Option<buzz_core::tenant::CommunityId>, ()> {
            match host {
                "alpha.example" | "beta.example:8443" => {
                    Ok(Some(buzz_core::tenant::CommunityId::from_uuid(
                        uuid::Uuid::from_u128(if host.starts_with("alpha") { 1 } else { 2 }),
                    )))
                }
                "error.example" => Err(()),
                "slow.example" => std::future::pending().await,
                _ => Ok(None),
            }
        }
    }

    #[tokio::test]
    async fn media_uses_only_resolved_request_host_and_fails_closed() {
        let document = "<html><head><title>Buzz</title></head><body>app</body></html>";
        let dir = tempfile::tempdir().unwrap();
        let index = dir.path().join("index.html");
        std::fs::write(&index, document).unwrap();
        for relay in [
            "wss://seed.example/relay",
            "https://user:password@seed.example/path?secret=x",
            "ws://localhost:3000",
        ] {
            for host in [
                "ALPHA.example",
                "beta.example:8443",
                "unknown.example",
                "error.example",
                "slow.example",
                "",
            ] {
                let response = response(&index, relay, &Resolver, host).await;
                let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let html = std::str::from_utf8(&bytes).unwrap();
                if host == "ALPHA.example" || host == "beta.example:8443" {
                    let scheme = if relay.starts_with("ws://") {
                        "http"
                    } else {
                        "https"
                    };
                    let origin = format!("{scheme}://{}", host.to_lowercase());
                    for file in ["buzz-invite-loop-v1.mp4", "buzz-invite-poster-v1.png"] {
                        assert!(html.contains(&format!(
                            "content=\"{origin}/assets/invite-preview/{file}\""
                        )));
                    }
                    assert_eq!(html.contains("og:video:secure_url"), scheme == "https");
                    assert!(!html.contains("seed.example"));
                    assert!(!html.contains("password"));
                    assert!(!html.contains("secret"));
                } else {
                    assert_eq!(html, document);
                }
            }
        }
    }
}
