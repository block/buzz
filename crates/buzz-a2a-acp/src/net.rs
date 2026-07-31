use crate::AdapterError;
use reqwest::Client;
use std::{
    net::{IpAddr, SocketAddr},
    path::Path,
};
use url::Url;

pub(super) fn pinned_http_client(
    url: &Url,
    addresses: &[SocketAddr],
) -> Result<Client, AdapterError> {
    let raw_host = url
        .host_str()
        .ok_or_else(|| AdapterError::UnsafeEndpoint(url.to_string()))?;
    let host = normalized_host(raw_host);
    let mut builder = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30));
    // Pin every address that passed our policy check. This prevents reqwest
    // from performing a second DNS lookup while preserving IPv4/IPv6 fallback.
    if host.parse::<IpAddr>().is_err() {
        if addresses.is_empty() {
            return Err(AdapterError::UnsafeEndpoint(url.to_string()));
        }
        builder = builder.resolve_to_addrs(&host, addresses);
    }
    builder
        .build()
        .map_err(|error| AdapterError::Request(format!("build HTTP client: {error}")))
}

pub(super) fn validate_http_url(raw: &str) -> Result<Url, AdapterError> {
    let url = Url::parse(raw).map_err(|_| AdapterError::UnsafeEndpoint(raw.to_owned()))?;
    let raw_host = url
        .host_str()
        .ok_or_else(|| AdapterError::UnsafeEndpoint(raw.to_owned()))?;
    let host = normalized_host(raw_host);
    if let Ok(ip) = host.parse::<IpAddr>() {
        // Local A2A runtimes are allowed over loopback HTTP. Private and
        // link-local addresses remain rejected for every other scheme.
        if url.scheme() == "http" && ip.is_loopback() {
            return Ok(url);
        }
        if is_private_ip(ip) {
            return Err(AdapterError::UnsafeEndpoint(raw.to_owned()));
        }
    }
    if url.scheme() == "https" && host.eq_ignore_ascii_case("localhost") {
        return Err(AdapterError::UnsafeEndpoint(raw.to_owned()));
    }
    match url.scheme() {
        "https" => Ok(url),
        "http" if is_loopback_host(&host) => Ok(url),
        _ => Err(AdapterError::UnsafeEndpoint(raw.to_owned())),
    }
}

fn normalized_host(host: &str) -> String {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

pub(super) fn is_private_ip(ip: IpAddr) -> bool {
    let ip = match ip {
        IpAddr::V6(address) => address
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(address)),
        address => address,
    };
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || octets[0] == 0
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                // IPv4-transitional address ranges can encode private IPv4
                // targets while still presenting as IPv6 DNS answers.
                || (segments[0] == 0x0064
                    && segments[1] == 0xff9b
                    && segments[2..6] == [0, 0, 0, 0])
                || segments[0] == 0x2002
                || (segments[0] == 0x2001 && segments[1] == 0)
                || segments[..6] == [0, 0, 0, 0, 0, 0]
        }
    }
}

pub(super) async fn resolve_network_url(url: &Url) -> Result<Vec<SocketAddr>, AdapterError> {
    let raw_host = url
        .host_str()
        .ok_or_else(|| AdapterError::UnsafeEndpoint(url.to_string()))?;
    let host = normalized_host(raw_host);
    if let Ok(ip) = host.parse::<IpAddr>() {
        if url.scheme() == "http" && ip.is_loopback() {
            return Ok(vec![SocketAddr::new(
                ip,
                url.port_or_known_default().unwrap_or(80),
            )]);
        }
        if !is_private_ip(ip) {
            return Ok(vec![SocketAddr::new(
                ip,
                url.port_or_known_default().unwrap_or(443),
            )]);
        }
        return Err(AdapterError::UnsafeEndpoint(url.to_string()));
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| AdapterError::UnsafeEndpoint(url.to_string()))?;
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| AdapterError::UnsafeEndpoint(url.to_string()))?
        .collect();
    validate_resolved_addresses(url, &addresses)?;
    Ok(addresses)
}

pub(super) fn validate_resolved_addresses(
    url: &Url,
    addresses: &[SocketAddr],
) -> Result<(), AdapterError> {
    let raw_host = url
        .host_str()
        .ok_or_else(|| AdapterError::UnsafeEndpoint(url.to_string()))?;
    let host = normalized_host(raw_host);
    if addresses.is_empty() {
        return Err(AdapterError::UnsafeEndpoint(url.to_string()));
    }
    let is_local_http = url.scheme() == "http" && is_loopback_host(&host);
    if url.scheme() == "http" && !is_local_http {
        return Err(AdapterError::UnsafeEndpoint(url.to_string()));
    }
    if is_local_http {
        if addresses.iter().any(|address| !address.ip().is_loopback()) {
            return Err(AdapterError::UnsafeEndpoint(url.to_string()));
        }
    } else if addresses.iter().any(|address| is_private_ip(address.ip())) {
        return Err(AdapterError::UnsafeEndpoint(url.to_string()));
    }
    Ok(())
}

pub(super) async fn response_bytes(
    mut response: reqwest::Response,
    what: &'static str,
    limit: usize,
) -> Result<Vec<u8>, AdapterError> {
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(AdapterError::TooLarge { what, limit });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| AdapterError::Request(format!("read {what}: {error}")))?
    {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(AdapterError::TooLarge { what, limit });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(super) async fn read_source(
    source: &str,
    what: &'static str,
    limit: usize,
) -> Result<Vec<u8>, AdapterError> {
    if source.trim().is_empty() {
        return Err(AdapterError::EmptyRecord);
    }
    if let Ok(url) = Url::parse(source) {
        if matches!(url.scheme(), "http" | "https") {
            validate_http_url(source)
                .map_err(|_| AdapterError::InvalidSource(source.to_owned()))?;
            let addresses = resolve_network_url(&url).await?;
            let response = pinned_http_client(&url, &addresses)?
                .get(url)
                .send()
                .await
                .map_err(|error| AdapterError::Request(format!("fetch {what}: {error}")))?;
            let status = response.status();
            if !status.is_success() {
                return Err(AdapterError::HttpStatus { what, status });
            }
            return response_bytes(response, what, limit).await;
        }
        if source.contains("://") {
            return Err(AdapterError::InvalidSource(source.to_owned()));
        }
    }
    let body = tokio::fs::read(Path::new(source))
        .await
        .map_err(|source| AdapterError::Read { what, source })?;
    if body.len() > limit {
        return Err(AdapterError::TooLarge { what, limit });
    }
    Ok(body)
}
