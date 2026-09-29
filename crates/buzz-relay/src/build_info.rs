//! Build-time identity compiled into the relay binary.

/// Full source commit SHA, or `unknown` outside a provenance-aware build.
pub(crate) fn source_sha() -> &'static str {
    option_env!("BUZZ_SOURCE_SHA").unwrap_or("unknown")
}

/// Stable build identifier, or `local` outside CI.
pub(crate) fn build_id() -> &'static str {
    option_env!("BUZZ_BUILD_ID").unwrap_or("local")
}

/// Build details URL, or `unknown` outside CI.
pub(crate) fn build_url() -> &'static str {
    option_env!("BUZZ_BUILD_URL").unwrap_or("unknown")
}
