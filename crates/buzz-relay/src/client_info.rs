//! Advisory parsing for the mobile `Buzz-Client` structured field.

use axum::http::HeaderMap;
use sfv::{BareItem, Dictionary, ListEntry, Parser};

/// Parsed, untrusted metadata supplied by a Buzz client.
///
/// This data is for observability only. It must never participate in
/// authentication, authorization, or tenant selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientInfo {
    /// Structured header format version.
    pub format_version: i64,
    /// Logical application identifier.
    pub app: String,
    /// Client platform (`ios` or `android`).
    pub platform: String,
    /// User-visible application version.
    pub app_version: String,
    /// Bounded, normalized label derived from the application version.
    metric_app_version: String,
    /// Platform build identifier, constrained to decimal digits.
    pub app_build: String,
    /// Coarse public operating-system version.
    pub os_version: String,
    /// Android API level, present only on Android.
    pub os_api: Option<i64>,
}

impl ClientInfo {
    /// Parse `Buzz-Client` from a request header map.
    ///
    /// A missing header is a supported state and returns `None` without a
    /// metric. A present but invalid header increments the parse-failure
    /// counter and also returns `None`, so it can never reject a request.
    #[must_use]
    pub fn from_headers(headers: &HeaderMap) -> Option<Self> {
        let all_values = headers.get_all("buzz-client");
        let mut values = all_values.iter();
        let first = values.next()?;
        let parsed = (|| {
            let mut raw = first.to_str().map_err(|_| ())?.to_owned();
            for value in values {
                raw.push_str(", ");
                raw.push_str(value.to_str().map_err(|_| ())?);
            }
            Self::parse(&raw)
        })()
        .ok();
        if parsed.is_none() {
            metrics::counter!("buzz_client_header_parse_failures_total").increment(1);
        }
        parsed
    }

    fn parse(raw: &str) -> Result<Self, ()> {
        let dictionary: Dictionary = Parser::new(raw).parse_dictionary().map_err(|_| ())?;

        let format_version = integer(&dictionary, "v")?;
        if format_version != 1 {
            return Err(());
        }

        let app = token(&dictionary, "app")?;
        if app != "buzz-mobile" {
            return Err(());
        }

        let platform = token(&dictionary, "platform")?;
        if platform != "ios" && platform != "android" {
            return Err(());
        }

        let app_version = string(&dictionary, "app-version")?;
        let metric_app_version = normalize_app_version(&app_version)?;
        let app_build = string(&dictionary, "app-build")?;
        let os_version = string(&dictionary, "os-version")?;
        if app_build.is_empty()
            || !app_build.bytes().all(|byte| byte.is_ascii_digit())
            || os_version.is_empty()
        {
            return Err(());
        }

        let os_api = optional_integer(&dictionary, "os-api")?;
        match platform.as_str() {
            "android" if !matches!(os_api, Some(api) if api > 0) => return Err(()),
            "ios" if os_api.is_some() => return Err(()),
            _ => {}
        }

        Ok(Self {
            format_version,
            app,
            platform,
            app_version,
            metric_app_version,
            app_build,
            os_version,
            os_api,
        })
    }

    /// Record a low-cardinality observation for a parsed client.
    pub fn record_observation(&self) {
        metrics::counter!(
            "buzz_client_connections_total",
            "app" => self.app.clone(),
            "platform" => self.platform.clone(),
            "app_version" => self.metric_app_version.clone(),
        )
        .increment(1);
    }
}

const MAX_APP_VERSION_COMPONENT_LENGTH: usize = 5;

fn normalize_app_version(app_version: &str) -> Result<String, ()> {
    let mut components = app_version.split('.');
    let Some(major) = components.next() else {
        return Err(());
    };
    let Some(minor) = components.next() else {
        return Err(());
    };
    let patch = components.next();
    if components.next().is_some() {
        return Err(());
    }

    if ![Some(major), Some(minor), patch]
        .into_iter()
        .flatten()
        .all(|component| {
            !component.is_empty()
                && component.len() <= MAX_APP_VERSION_COMPONENT_LENGTH
                && component.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Err(());
    }

    Ok(format!("{major}.{minor}"))
}

fn bare_item<'a>(dictionary: &'a Dictionary, key: &str) -> Result<&'a BareItem, ()> {
    let Some(ListEntry::Item(item)) = dictionary.get(key) else {
        return Err(());
    };
    if !item.params.is_empty() {
        return Err(());
    }
    Ok(&item.bare_item)
}

fn token(dictionary: &Dictionary, key: &str) -> Result<String, ()> {
    bare_item(dictionary, key)?
        .as_token()
        .map(|value| value.as_str().to_owned())
        .ok_or(())
}

fn string(dictionary: &Dictionary, key: &str) -> Result<String, ()> {
    bare_item(dictionary, key)?
        .as_string()
        .map(|value| value.as_str().to_owned())
        .ok_or(())
}

fn integer(dictionary: &Dictionary, key: &str) -> Result<i64, ()> {
    bare_item(dictionary, key)?
        .as_integer()
        .map(Into::into)
        .ok_or(())
}

fn optional_integer(dictionary: &Dictionary, key: &str) -> Result<Option<i64>, ()> {
    if !dictionary.contains_key(key) {
        return Ok(None);
    }
    integer(dictionary, key).map(Some)
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    use super::*;

    fn metric_counter(
        recorder: &DebuggingRecorder,
        name: &str,
    ) -> Vec<(Vec<(String, String)>, u64)> {
        recorder
            .snapshotter()
            .snapshot()
            .into_vec()
            .into_iter()
            .filter_map(|(key, _, _, value)| {
                (key.key().name() == name).then(|| {
                    let labels = key
                        .key()
                        .labels()
                        .map(|label| (label.key().to_owned(), label.value().to_owned()))
                        .collect();
                    let DebugValue::Counter(value) = value else {
                        panic!("{name} must be a counter");
                    };
                    (labels, value)
                })
            })
            .collect()
    }

    fn parse_failures(recorder: &DebuggingRecorder) -> u64 {
        metric_counter(recorder, "buzz_client_header_parse_failures_total")
            .into_iter()
            .map(|(_, value)| value)
            .sum()
    }

    #[test]
    fn parses_valid_ios_and_android_headers() {
        let ios = ClientInfo::parse(
            r#"v=1, app=buzz-mobile, platform=ios, app-version="0.4.5", app-build="6", os-version="18.5""#,
        )
        .expect("valid iOS header");
        assert_eq!(
            ios,
            ClientInfo {
                format_version: 1,
                app: "buzz-mobile".to_owned(),
                platform: "ios".to_owned(),
                app_version: "0.4.5".to_owned(),
                metric_app_version: "0.4".to_owned(),
                app_build: "6".to_owned(),
                os_version: "18.5".to_owned(),
                os_api: None,
            }
        );

        let android = ClientInfo::parse(
            r#"v=1, app=buzz-mobile, platform=android, app-version="0.4.5", app-build="7", os-version="15", os-api=35, future-key=ignored"#,
        )
        .expect("valid Android header");
        assert_eq!(android.os_api, Some(35));
    }

    #[test]
    fn observations_bucket_versions_to_major_minor() {
        let client = ClientInfo::parse(
            r#"v=1, app=buzz-mobile, platform=ios, app-version="12.34.56", app-build="6", os-version="18.5""#,
        )
        .expect("valid iOS header");
        let recorder = DebuggingRecorder::new();

        metrics::with_local_recorder(&recorder, || client.record_observation());

        let counters = metric_counter(&recorder, "buzz_client_connections_total");
        assert_eq!(counters.len(), 1);
        let (labels, value) = &counters[0];
        assert_eq!(*value, 1);
        assert!(labels.contains(&("app".to_owned(), "buzz-mobile".to_owned())));
        assert!(labels.contains(&("platform".to_owned(), "ios".to_owned())));
        assert!(labels.contains(&("app_version".to_owned(), "12.34".to_owned())));
    }

    #[test]
    fn missing_header_is_absent_without_parse_failure() {
        let recorder = DebuggingRecorder::new();
        metrics::with_local_recorder(&recorder, || {
            assert_eq!(ClientInfo::from_headers(&HeaderMap::new()), None);
        });
        assert_eq!(parse_failures(&recorder), 0);
    }

    #[test]
    fn joins_multiple_header_lines_before_parsing() {
        let mut headers = HeaderMap::new();
        headers.append(
            "buzz-client",
            HeaderValue::from_static(r#"v=1, app=buzz-mobile, platform=ios, app-version="0.4.5""#),
        );
        headers.append(
            "buzz-client",
            HeaderValue::from_static(r#"app-build="6", os-version="18.5""#),
        );

        let client = ClientInfo::from_headers(&headers).expect("valid combined header");
        assert_eq!(client.app_version, "0.4.5");
        assert_eq!(client.app_build, "6");
        assert_eq!(client.os_version, "18.5");
    }

    #[test]
    fn rejects_non_utf8_in_any_header_line() {
        let recorder = DebuggingRecorder::new();
        let mut headers = HeaderMap::new();
        headers.append(
            "buzz-client",
            HeaderValue::from_static(r#"v=1, app=buzz-mobile, platform=ios, app-version="0.4.5""#),
        );
        headers.append(
            "buzz-client",
            HeaderValue::from_bytes(&[0xff]).expect("opaque test header value"),
        );

        metrics::with_local_recorder(&recorder, || {
            assert_eq!(ClientInfo::from_headers(&headers), None);
        });
        assert_eq!(parse_failures(&recorder), 1);
    }

    #[test]
    fn malformed_or_semantically_invalid_header_is_absent_and_counted() {
        for raw in [
            "not a dictionary",
            r#"v=2, app=buzz-mobile, platform=ios, app-version="1", app-build="1", os-version="18""#,
            r#"v=1, app=buzz-mobile, platform=android, app-version="1", app-build="1", os-version="15""#,
            r#"v=1, app=buzz-mobile, platform=ios, app-version="1", app-build="1.beta", os-version="18""#,
            r#"v=1, app=buzz-mobile, platform=ios, app-version="random-connection-value", app-build="1", os-version="18""#,
            r#"v=1, app=buzz-mobile, platform=ios, app-version="1.2.3.4", app-build="1", os-version="18""#,
            r#"v=1, app=buzz-mobile, platform=ios, app-version="123456.2", app-build="1", os-version="18""#,
        ] {
            let recorder = DebuggingRecorder::new();
            let mut headers = HeaderMap::new();
            headers.insert(
                "buzz-client",
                HeaderValue::from_str(raw).expect("test header value"),
            );
            metrics::with_local_recorder(&recorder, || {
                assert_eq!(ClientInfo::from_headers(&headers), None, "{raw}");
            });
            assert_eq!(parse_failures(&recorder), 1, "{raw}");
        }
    }
}
