//! Windows static proxy discovery.
//!
//! WinINet allows `ProxyServer` to contain protocol-specific entries such as
//! `http=proxy:8080;https=proxy:8443`. Hyper-util currently treats the entire
//! registry value as one URI, so Buzz normalizes the list before building the
//! shared matcher.

#[cfg(windows)]
use hyper_util::client::proxy::matcher::Matcher;

#[cfg(windows)]
const INTERNET_SETTINGS_KEY: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings";

#[cfg(windows)]
pub(super) fn system_proxy_matcher() -> Matcher {
    if std::env::var_os("REQUEST_METHOD").is_some() {
        return Matcher::from_env();
    }

    let mut http = first_env(&["HTTP_PROXY", "http_proxy"]);
    let mut https = first_env(&["HTTPS_PROXY", "https_proxy"]);
    let all = first_env(&["ALL_PROXY", "all_proxy"]);
    let mut no = first_env(&["NO_PROXY", "no_proxy"]);

    if let Some(system) = read_system_proxy() {
        if http.is_empty() && all.is_empty() {
            http = system.http.unwrap_or_default();
        }
        if https.is_empty() && all.is_empty() {
            https = system.https.unwrap_or_default();
        }
        if no.is_empty() {
            no = system.no;
        }
    }

    Matcher::builder()
        .all(all)
        .http(http)
        .https(https)
        .no(no)
        .build()
}

#[cfg(windows)]
fn first_env(names: &[&str]) -> String {
    names
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .unwrap_or_default()
}

#[cfg(windows)]
fn read_system_proxy() -> Option<SystemProxy> {
    let settings = windows_registry::CURRENT_USER
        .open(INTERNET_SETTINGS_KEY)
        .ok()?;
    if settings.get_u32("ProxyEnable").unwrap_or(0) == 0 {
        return None;
    }

    let mut proxy = settings
        .get_string("ProxyServer")
        .ok()
        .map(|value| parse_proxy_server(&value))
        .unwrap_or_default();
    proxy.no = settings
        .get_string("ProxyOverride")
        .ok()
        .map(|value| normalize_proxy_override(&value))
        .unwrap_or_default();
    Some(proxy)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SystemProxy {
    http: Option<String>,
    https: Option<String>,
    no: String,
}

fn parse_proxy_server(value: &str) -> SystemProxy {
    let mut proxy = SystemProxy::default();
    let mut default = None;

    for entry in value
        .split(|character: char| character == ';' || character.is_ascii_whitespace())
        .filter(|entry| !entry.is_empty())
    {
        let Some((protocol, address)) = entry.split_once('=') else {
            default.get_or_insert_with(|| entry.to_string());
            continue;
        };
        let address = address.trim();
        if address.is_empty() {
            continue;
        }

        if protocol.eq_ignore_ascii_case("http") {
            proxy.http = Some(address.to_string());
        } else if protocol.eq_ignore_ascii_case("https") {
            proxy.https = Some(address.to_string());
        }
    }

    proxy.http = proxy.http.or_else(|| default.clone());
    proxy.https = proxy.https.or(default);
    proxy
}

fn normalize_proxy_override(value: &str) -> String {
    value
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>()
        .join(",")
        .replace("*.", "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_proxy_applies_to_http_and_https() {
        let proxy = parse_proxy_server("127.0.0.1:7890");

        assert_eq!(proxy.http.as_deref(), Some("127.0.0.1:7890"));
        assert_eq!(proxy.https.as_deref(), Some("127.0.0.1:7890"));
    }

    #[test]
    fn protocol_proxy_list_preserves_each_destination_proxy() {
        let proxy = parse_proxy_server("http=127.0.0.1:7890;https=secure.example:8443");

        assert_eq!(proxy.http.as_deref(), Some("127.0.0.1:7890"));
        assert_eq!(proxy.https.as_deref(), Some("secure.example:8443"));
    }

    #[test]
    fn default_proxy_fills_unspecified_protocols() {
        let proxy =
            parse_proxy_server("HTTP=http.example:8080 fallback.example:3128 ftp=ftp.example:21");

        assert_eq!(proxy.http.as_deref(), Some("http.example:8080"));
        assert_eq!(proxy.https.as_deref(), Some("fallback.example:3128"));
    }

    #[test]
    fn proxy_override_is_normalized_for_matcher() {
        assert_eq!(
            normalize_proxy_override("localhost; *.example.com ;10.0.0.0/8"),
            "localhost,example.com,10.0.0.0/8"
        );
    }
}
