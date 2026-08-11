//! macOS static proxy discovery.
//!
//! Hyper-util reads the configured HTTP and HTTPS proxies on macOS, but does
//! not currently include `ExceptionsList` or `ExcludeSimpleHostnames`. This
//! module preserves its environment-variable precedence while adding those
//! system bypass settings to Buzz's shared matcher.

use std::io::Cursor;

use hyper_util::client::proxy::matcher::Matcher;
use plist::{Dictionary, Value};
use system_configuration::{
    core_foundation::{
        base::{CFType, TCFType},
        dictionary::CFDictionary,
        propertylist::{create_data, kCFPropertyListBinaryFormat_v1_0},
        string::CFString,
    },
    dynamic_store::SCDynamicStoreBuilder,
};

use super::SystemProxySettings;

pub(super) fn system_proxy_settings() -> SystemProxySettings {
    if std::env::var_os("REQUEST_METHOD").is_some() {
        return SystemProxySettings::new(Matcher::from_env());
    }

    let mut http = first_env(&["HTTP_PROXY", "http_proxy"]);
    let mut https = first_env(&["HTTPS_PROXY", "https_proxy"]);
    let all = first_env(&["ALL_PROXY", "all_proxy"]);
    let mut no = first_env(&["NO_PROXY", "no_proxy"]);
    let mut exclude_simple_hostnames = false;

    if let Some(system) = read_system_proxy() {
        if http.is_empty() && all.is_empty() {
            http = system.http.unwrap_or_default();
        }
        if https.is_empty() && all.is_empty() {
            https = system.https.unwrap_or_default();
        }
        if no.is_empty() {
            no = system.no;
            exclude_simple_hostnames = system.exclude_simple_hostnames;
        }
    }

    SystemProxySettings {
        matcher: Matcher::builder()
            .all(all)
            .http(http)
            .https(https)
            .no(no)
            .build(),
        exclude_simple_hostnames,
    }
}

fn first_env(names: &[&str]) -> String {
    names
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .unwrap_or_default()
}

fn read_system_proxy() -> Option<SystemProxy> {
    let store = SCDynamicStoreBuilder::new("buzz-ws-client").build()?;
    let proxies = store.get_proxies()?;
    let dictionary = proxy_dictionary(&proxies)?;
    Some(parse_system_proxy(&dictionary))
}

fn proxy_dictionary(proxies: &CFDictionary<CFString, CFType>) -> Option<Dictionary> {
    let data = create_data(
        proxies.as_CFTypeRef().cast(),
        kCFPropertyListBinaryFormat_v1_0,
    )
    .ok()?;
    Value::from_reader(Cursor::new(data.bytes()))
        .ok()?
        .into_dictionary()
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SystemProxy {
    http: Option<String>,
    https: Option<String>,
    no: String,
    exclude_simple_hostnames: bool,
}

fn parse_system_proxy(proxies: &Dictionary) -> SystemProxy {
    SystemProxy {
        http: proxy_endpoint(proxies, "HTTPEnable", "HTTPProxy", "HTTPPort"),
        https: proxy_endpoint(proxies, "HTTPSEnable", "HTTPSProxy", "HTTPSPort"),
        no: proxy_exceptions(proxies),
        exclude_simple_hostnames: setting_enabled(proxies.get("ExcludeSimpleHostnames")),
    }
}

fn proxy_endpoint(
    proxies: &Dictionary,
    enabled_key: &str,
    host_key: &str,
    port_key: &str,
) -> Option<String> {
    if !setting_enabled(proxies.get(enabled_key)) {
        return None;
    }

    let host = proxies.get(host_key)?.as_string()?.trim();
    if host.is_empty() {
        return None;
    }

    let port = proxies
        .get(port_key)
        .and_then(integer_value)
        .and_then(|value| u16::try_from(value).ok());
    Some(match port {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

fn proxy_exceptions(proxies: &Dictionary) -> String {
    proxies
        .get("ExceptionsList")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_string)
        .filter_map(normalize_exception)
        .collect::<Vec<_>>()
        .join(",")
}

fn normalize_exception(value: &str) -> Option<&str> {
    let value = value.trim();
    let value = value.strip_prefix("*.").unwrap_or(value);
    (!value.is_empty()).then_some(value)
}

fn setting_enabled(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value.as_boolean() == Some(true)
            || value.as_signed_integer() == Some(1)
            || value.as_unsigned_integer() == Some(1)
    })
}

fn integer_value(value: &Value) -> Option<i64> {
    value.as_signed_integer().or_else(|| {
        value
            .as_unsigned_integer()
            .and_then(|value| i64::try_from(value).ok())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_enabled_http_and_https_proxies() {
        let mut proxies = Dictionary::new();
        proxies.insert("HTTPEnable".into(), Value::Integer(1.into()));
        proxies.insert("HTTPProxy".into(), Value::String("proxy.local".into()));
        proxies.insert("HTTPPort".into(), Value::Integer(8080.into()));
        proxies.insert("HTTPSEnable".into(), Value::Boolean(true));
        proxies.insert(
            "HTTPSProxy".into(),
            Value::String("secure-proxy.local".into()),
        );
        proxies.insert("HTTPSPort".into(), Value::Integer(8443.into()));

        let proxy = parse_system_proxy(&proxies);

        assert_eq!(proxy.http.as_deref(), Some("proxy.local:8080"));
        assert_eq!(proxy.https.as_deref(), Some("secure-proxy.local:8443"));
    }

    #[test]
    fn normalizes_macos_proxy_exceptions_for_hyper_util() {
        let mut proxies = Dictionary::new();
        proxies.insert(
            "ExceptionsList".into(),
            Value::Array(vec![
                Value::String(" *.example.com ".into()),
                Value::String("10.0.0.0/8".into()),
                Value::String("localhost".into()),
                Value::String(String::new()),
            ]),
        );

        let exceptions = proxy_exceptions(&proxies);
        assert_eq!(exceptions, "example.com,10.0.0.0/8,localhost");

        let matcher = Matcher::builder()
            .all("http://proxy.local:8080")
            .no(exceptions)
            .build();
        let excluded = "https://relay.example.com".parse().unwrap();
        let proxied = "https://notexample.com".parse().unwrap();
        assert!(matcher.intercept(&excluded).is_none());
        assert!(matcher.intercept(&proxied).is_some());
    }

    #[test]
    fn reads_exclude_simple_hostnames_boolean_and_integer_values() {
        let mut proxies = Dictionary::new();
        proxies.insert("ExcludeSimpleHostnames".into(), Value::Integer(1.into()));
        assert!(parse_system_proxy(&proxies).exclude_simple_hostnames);

        proxies.insert("ExcludeSimpleHostnames".into(), Value::Boolean(false));
        assert!(!parse_system_proxy(&proxies).exclude_simple_hostnames);
    }
}
