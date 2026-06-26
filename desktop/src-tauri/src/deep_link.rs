use tauri::Emitter;
use url::Url;

/// Parse the query string of a `buzz://message?…` URL into the JSON
/// payload emitted on `deep-link-message`. Returns `None` when a required
/// param (`channel`, `id`) is missing or empty — mirroring the validation
/// policy of the `connect` arm so the frontend never sees a half-formed
/// payload (e.g. `channelId: ""` from `channel=&id=foo`).
///
/// Pulled out of `handle_deep_link_url` so it can be unit-tested without
/// a live `tauri::AppHandle`.
fn parse_message_deep_link(url: &Url) -> Option<serde_json::Value> {
    let mut channel: Option<String> = None;
    let mut message_id: Option<String> = None;
    let mut thread: Option<String> = None;
    for (k, v) in url.query_pairs() {
        let v = v.into_owned();
        if v.is_empty() {
            continue;
        }
        match k.as_ref() {
            "channel" => channel = Some(v),
            "id" => message_id = Some(v),
            "thread" => thread = Some(v),
            _ => {}
        }
    }
    let (channel_id, message_id) = (channel?, message_id?);
    Some(serde_json::json!({
        "channelId": channel_id,
        "messageId": message_id,
        "threadRootId": thread,
    }))
}

/// Handle an incoming `buzz://` deep link URL.
///
/// Currently supports:
/// - `buzz://connect?relay=<ws(s)://...>` — emits `deep-link-connect` to the frontend
pub(crate) fn handle_deep_link_url(app: &tauri::AppHandle, url_str: &str) {
    let url = match Url::parse(url_str) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("buzz-desktop: invalid deep link URL {url_str:?}: {e}");
            return;
        }
    };

    if url.scheme() != "buzz" {
        eprintln!("buzz-desktop: ignoring unsupported deep link scheme: {url_str}");
        return;
    }

    match url.host_str() {
        Some("connect") => {
            let relay = url
                .query_pairs()
                .find(|(k, _)| k == "relay")
                .map(|(_, v)| v.into_owned());
            let Some(relay_url) = relay else {
                eprintln!("buzz-desktop: connect deep link missing relay param: {url_str}");
                return;
            };
            // Validate the relay URL is ws:// or wss://
            match Url::parse(&relay_url) {
                Ok(parsed) if parsed.scheme() == "ws" || parsed.scheme() == "wss" => {}
                Ok(parsed) => {
                    eprintln!(
                        "buzz-desktop: rejecting non-websocket relay URL scheme {:?}: {relay_url}",
                        parsed.scheme()
                    );
                    return;
                }
                Err(e) => {
                    eprintln!("buzz-desktop: invalid relay URL {relay_url:?}: {e}");
                    return;
                }
            }
            let _ = app.emit("deep-link-connect", relay_url);
        }
        Some("message") => {
            // `buzz://message?channel=<uuid>&id=<eventId>[&thread=<rootId>]`
            //
            // Validation policy mirrors the `connect` arm: parse what we
            // need, refuse to emit anything if a required param is missing
            // so the frontend never sees a half-formed payload. The
            // frontend listener mirrors `parseMessageLink` in TS — we keep
            // structure on this side (serde JSON) and let the TS code own
            // any further normalisation.
            let Some(payload) = parse_message_deep_link(&url) else {
                eprintln!("buzz-desktop: message deep link missing channel or id: {url_str}");
                return;
            };
            let _ = app.emit("deep-link-message", payload);
        }
        Some(action) => {
            eprintln!("buzz-desktop: unknown deep link action: {action}");
        }
        None => {
            eprintln!("buzz-desktop: deep link missing action: {url_str}");
        }
    }
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::parse_message_deep_link;

    #[test]
    fn parse_message_deep_link_extracts_required_params() {
        let url = Url::parse("buzz://message?channel=abc&id=xyz").unwrap();
        let payload = parse_message_deep_link(&url).expect("required params present");
        assert_eq!(payload["channelId"], "abc");
        assert_eq!(payload["messageId"], "xyz");
        assert!(payload["threadRootId"].is_null());
    }

    #[test]
    fn parse_message_deep_link_accepts_buzz_scheme() {
        let url = Url::parse("buzz://message?channel=abc&id=xyz").unwrap();
        let payload = parse_message_deep_link(&url).expect("required params present");
        assert_eq!(payload["channelId"], "abc");
        assert_eq!(payload["messageId"], "xyz");
    }

    #[test]
    fn parse_message_deep_link_includes_thread_root() {
        let url = Url::parse("buzz://message?channel=abc&id=xyz&thread=root1").unwrap();
        let payload = parse_message_deep_link(&url).expect("required params present");
        assert_eq!(payload["threadRootId"], "root1");
    }

    #[test]
    fn parse_message_deep_link_rejects_missing_id() {
        let url = Url::parse("buzz://message?channel=abc").unwrap();
        assert!(parse_message_deep_link(&url).is_none());
    }

    #[test]
    fn parse_message_deep_link_rejects_empty_channel() {
        // Regression: `channel=&id=foo` previously produced channelId: "".
        let url = Url::parse("buzz://message?channel=&id=foo").unwrap();
        assert!(parse_message_deep_link(&url).is_none());
    }

    #[test]
    fn parse_message_deep_link_rejects_empty_id() {
        let url = Url::parse("buzz://message?channel=abc&id=").unwrap();
        assert!(parse_message_deep_link(&url).is_none());
    }

    #[test]
    fn parse_message_deep_link_treats_empty_thread_as_absent() {
        let url = Url::parse("buzz://message?channel=abc&id=xyz&thread=").unwrap();
        let payload = parse_message_deep_link(&url).expect("required params present");
        assert!(payload["threadRootId"].is_null());
    }
}
