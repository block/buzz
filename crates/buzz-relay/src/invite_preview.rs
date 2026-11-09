//! Public, generic invite metadata. Never inspect or redeem the invite token.

use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use std::path::Path;
use url::Url;

pub(crate) async fn response(index: &Path, relay_url: &str) -> Response {
    match tokio::fs::read_to_string(index).await {
        Ok(html) => Html(with_metadata(html, relay_url)).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn with_metadata(html: String, relay_url: &str) -> String {
    let Ok(mut origin) = Url::parse(relay_url) else {
        return html;
    };
    let scheme = match origin.scheme() {
        "wss" | "https" => "https",
        "ws" | "http" => "http",
        _ => return html,
    };
    if origin.set_scheme(scheme).is_err() || origin.host_str().is_none() {
        return html;
    }
    // Use the operator's canonical public origin, never the request Host or token.
    let origin = origin.origin().ascii_serialization();
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

    #[test]
    fn canonical_public_media_without_credentials_or_paths() {
        let document = "<html><head><title>Buzz</title></head><body>app</body></html>";
        for (relay, expected, secure) in [
            ("wss://example.org/relay", "https://example.org", true),
            (
                "https://user:password@example.org:8443/path?secret=x#fragment",
                "https://example.org:8443",
                true,
            ),
            ("ws://localhost:8080", "http://localhost:8080", false),
        ] {
            let html = with_metadata(document.into(), relay);
            assert!(html.contains(&format!(
                "content=\"{expected}/assets/invite-preview/buzz-invite-loop-v1.mp4\""
            )));
            assert!(html.contains("buzz-invite-poster-v1.png"));
            assert_eq!(html.contains("og:video:secure_url"), secure);
            assert!(!html.contains("password"));
            assert!(!html.contains("secret"));
            assert!(html.ends_with("</head><body>app</body></html>"));
        }
        for invalid in ["invalid", "file:///tmp/test", "javascript:alert(1)"] {
            assert_eq!(with_metadata(document.into(), invalid), document);
        }
    }
}
