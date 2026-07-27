//! Remote Agency discovery and binding persistence.
//!
//! This module intentionally implements only the host-side projection.  The
//! source runtime remains authoritative for prompts, memory, tools, and
//! signing keys.  Execution is supplied by the separately packaged
//! `buzz-a2a-acp` adapter.

use std::{collections::BTreeSet, net::IpAddr, path::PathBuf, sync::OnceLock, time::Duration};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};
use url::Url;

const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
const MAX_ITEMS: usize = 128;
const MAX_TEXT_BYTES: usize = 512;
const MAX_BEARER_TOKEN_BYTES: usize = 16 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

fn is_private_address(address: IpAddr) -> bool {
    let address = match address {
        IpAddr::V6(address) => address
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(address)),
        address => address,
    };
    match address {
        IpAddr::V4(address) => {
            let octets = address.octets();
            address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_unspecified()
                || address.is_multicast()
                || octets[0] == 0
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        }
        IpAddr::V6(address) => {
            let segments = address.segments();
            address.is_unique_local()
                || address.is_loopback()
                || address.is_unicast_link_local()
                || address.is_unspecified()
                || address.is_multicast()
                || (segments[0] == 0x0064
                    && segments[1] == 0xff9b
                    && segments[2..6] == [0, 0, 0, 0])
                || segments[0] == 0x2002
                || (segments[0] == 0x2001 && segments[1] == 0)
                || segments[..6] == [0, 0, 0, 0, 0, 0]
        }
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

fn equivalent_loopback_agency_source(left: &str, right: &str) -> bool {
    let (Ok(left), Ok(right)) = (Url::parse(left), Url::parse(right)) else {
        return false;
    };
    let (Some(left_host), Some(right_host)) = (left.host_str(), right.host_str()) else {
        return false;
    };
    is_loopback_host(&normalized_host(left_host))
        && is_loopback_host(&normalized_host(right_host))
        && left.scheme() == right.scheme()
        && left.port_or_known_default() == right.port_or_known_default()
        && left.path() == right.path()
        && left.query() == right.query()
        && left.fragment() == right.fragment()
        && left.username() == right.username()
        && left.password() == right.password()
}

fn remote_agency_bearer_token_key_from_urls(record_url: &Url, endpoint: &Url) -> String {
    let digest = Sha256::digest(format!("{record_url}\n{endpoint}").as_bytes());
    format!("remote-agency-a2a:{}", hex::encode(digest))
}

fn remote_agency_bearer_token_key(record_url: &str, endpoint: &str) -> Result<String, String> {
    let record_url = validate_remote_agency_url(record_url)?;
    let endpoint = validate_remote_agency_url(endpoint)?;
    Ok(remote_agency_bearer_token_key_from_urls(
        &record_url,
        &endpoint,
    ))
}

fn remote_agency_bearer_token_keys(
    record_url: &str,
    endpoint: &str,
) -> Result<Vec<String>, String> {
    let record_url = validate_remote_agency_url(record_url)?;
    let endpoint = validate_remote_agency_url(endpoint)?;
    let mut keys = vec![remote_agency_bearer_token_key_from_urls(
        &record_url,
        &endpoint,
    )];

    let loopback_pair = record_url.host_str().zip(endpoint.host_str()).is_some_and(
        |(record_host, endpoint_host)| {
            is_loopback_host(&normalized_host(record_host))
                && is_loopback_host(&normalized_host(endpoint_host))
        },
    );
    if loopback_pair {
        for host in ["localhost", "127.0.0.1", "[::1]"] {
            let mut record_alias = record_url.clone();
            let mut endpoint_alias = endpoint.clone();
            record_alias
                .set_host(Some(host))
                .map_err(|_| "Remote Agency record loopback alias is invalid".to_string())?;
            endpoint_alias
                .set_host(Some(host))
                .map_err(|_| "Remote Agency endpoint loopback alias is invalid".to_string())?;
            let key = remote_agency_bearer_token_key_from_urls(&record_alias, &endpoint_alias);
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }

    Ok(keys)
}

pub(crate) fn load_remote_agency_bearer_token(
    record_url: &str,
    endpoint: &str,
) -> Result<Option<String>, String> {
    let store = crate::secret_store::SecretStore::shared(crate::app_state::keyring_service());
    for key in remote_agency_bearer_token_keys(record_url, endpoint)? {
        if let Some(token) = store.load(&key)? {
            return Ok(Some(token));
        }
    }
    Ok(None)
}

fn sanitize_untrusted_text(value: &str) -> String {
    static CONTROL_OR_FORMAT: OnceLock<Regex> = OnceLock::new();
    CONTROL_OR_FORMAT
        .get_or_init(|| Regex::new(r"[\p{Cc}\p{Cf}]").expect("static Unicode category regex"))
        .replace_all(value, "")
        .trim()
        .to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgencyAgent {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub record_url: Option<String>,
    pub record_revision: Option<String>,
    pub a2a_endpoint: Option<String>,
    pub agent_card_url: Option<String>,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgencySurface {
    pub id: String,
    pub name: String,
    pub surface_type: Option<String>,
    pub locator: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgencySpace {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub surfaces: Vec<RemoteAgencySurface>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgencyDescriptor {
    pub source_url: String,
    pub agency_id: String,
    pub name: String,
    pub description: Option<String>,
    pub agents: Vec<RemoteAgencyAgent>,
    pub spaces: Vec<RemoteAgencySpace>,
    pub protocols: Vec<String>,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgencyBinding {
    pub source_url: String,
    pub agency_id: String,
    pub agent_ids: Vec<String>,
    pub space_ids: Vec<String>,
    pub channel_ids: Vec<String>,
    #[serde(default)]
    pub proxies: Vec<RemoteAgencyProxy>,
    pub joined_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgencyProxy {
    pub agent_id: String,
    pub pubkey: String,
    pub channel_id: String,
    pub space_id: Option<String>,
    pub record_url: String,
    pub record_revision: Option<String>,
}

fn text(value: Option<&Value>) -> Option<String> {
    let value = sanitize_untrusted_text(value?.as_str()?);
    if value.is_empty() || value.len() > MAX_TEXT_BYTES {
        return None;
    }
    Some(value)
}

fn id(value: Option<&Value>) -> Option<String> {
    text(value).filter(|value| value.len() <= 128)
}

fn strings(value: Option<&Value>) -> Vec<String> {
    let Some(values) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut result = BTreeSet::new();
    for value in values.iter().take(MAX_ITEMS) {
        if let Some(value) = value.as_str().and_then(|value| {
            let value = sanitize_untrusted_text(value);
            (!value.is_empty() && value.len() <= MAX_TEXT_BYTES).then_some(value)
        }) {
            result.insert(value);
        } else if let Some(value) = value.get("name").and_then(|value| value.as_str()) {
            let value = sanitize_untrusted_text(value);
            if !value.is_empty() && value.len() <= MAX_TEXT_BYTES {
                result.insert(value);
            }
        }
    }
    result.into_iter().collect()
}

fn same_origin_reference(source: &Url, value: Option<&Value>) -> Option<String> {
    let candidate = text(value)
        .or_else(|| value?.get("url").and_then(|value| text(Some(value))))
        .or_else(|| value?.get("href").and_then(|value| text(Some(value))))?;
    let parsed = Url::parse(&candidate)
        .or_else(|_| source.join(&candidate))
        .ok()?;
    if parsed.scheme() != source.scheme()
        || parsed.host_str() != source.host_str()
        || parsed.port_or_known_default() != source.port_or_known_default()
    {
        return None;
    }
    Some(parsed.to_string())
}

fn linked_urls(source: &Url, document: &Value) -> Vec<(String, String)> {
    if let Some(links) = document.get("links").and_then(Value::as_array) {
        return links
            .iter()
            .filter_map(|link| {
                let kind = relation_kind(link.get("rel"))?;
                same_origin_reference(source, link.get("href").or_else(|| link.get("url")))
                    .map(|url| (kind.to_string(), url))
            })
            .take(8)
            .collect();
    }
    let Some(links) = document
        .get("links")
        .or_else(|| document.get("resources"))
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };
    links
        .iter()
        .take(8)
        .filter_map(|(kind, value)| {
            same_origin_reference(source, Some(value)).map(|url| (kind.clone(), url))
        })
        .collect()
}

fn relation_kind(value: Option<&Value>) -> Option<&'static str> {
    let classify = |relation: &str| {
        let relation = relation.trim_end_matches('/');
        if relation.ends_with("agents") || relation.ends_with("agent-records") {
            Some("agents")
        } else if relation.ends_with("spaces") {
            Some("spaces")
        } else {
            None
        }
    };
    match value {
        Some(Value::String(value)) => classify(value),
        Some(Value::Array(values)) => values.iter().filter_map(Value::as_str).find_map(classify),
        _ => None,
    }
}

fn linked_collection_values<'a>(document: &'a Value, kind: &str) -> Option<&'a [Value]> {
    document
        .as_array()
        .map(Vec::as_slice)
        .or_else(|| {
            document
                .get(kind)
                .and_then(Value::as_array)
                .map(Vec::as_slice)
        })
        .or_else(|| {
            document
                .get("data")
                .and_then(Value::as_object)
                .and_then(|data| data.get(kind))
                .and_then(Value::as_array)
                .map(Vec::as_slice)
        })
}

fn merge_linked_collections<I>(mut document: Value, linked: I) -> Value
where
    I: IntoIterator<Item = (String, Value)>,
{
    for (kind, linked_document) in linked {
        let Some(values) = linked_collection_values(&linked_document, &kind) else {
            continue;
        };
        if matches!(kind.as_str(), "agents" | "agent_records" | "spaces") {
            document[kind] = Value::Array(values.iter().take(MAX_ITEMS).cloned().collect());
        }
    }
    document
}

fn parse_preview_document(
    source_url: &str,
    document: Value,
    linked: impl IntoIterator<Item = (String, Value)>,
) -> Result<RemoteAgencyDescriptor, String> {
    let merged = merge_linked_collections(document, linked);
    let bytes = serde_json::to_vec(&merged)
        .map_err(|error| format!("failed to normalize Remote Agency descriptor: {error}"))?;
    parse_remote_agency_document(source_url, &bytes)
}

fn first_reference(source: &Url, value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_ITEMS)
        .find_map(|value| same_origin_reference(source, Some(value)))
}

fn first_jsonrpc_reference(source: &Url, value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_ITEMS)
        .filter(|value| {
            value
                .get("protocolBinding")
                .or_else(|| value.get("protocol_binding"))
                .and_then(Value::as_str)
                .is_some_and(|binding| binding.to_ascii_lowercase().contains("jsonrpc"))
        })
        .find_map(|value| same_origin_reference(source, Some(value)))
}

fn parse_agent(source: &Url, value: &Value) -> Option<RemoteAgencyAgent> {
    let agent_id = id(value.get("id").or_else(|| value.get("identifier")))
        .or_else(|| id(value.get("agent_id")))?;
    let name = text(value.get("name"))
        .or_else(|| text(value.get("display_name")))
        .or_else(|| text(value.get("displayName")))
        .unwrap_or_else(|| agent_id.clone());
    let card = value
        .get("agent_card_url")
        .or_else(|| value.get("agentCardUrl"))
        .or_else(|| value.get("agent_card"))
        .or_else(|| value.get("card"))
        .or_else(|| value.get("url"))
        .and_then(|value| same_origin_reference(source, Some(value)));
    let record = value
        .get("record_url")
        .or_else(|| value.get("recordUrl"))
        .or_else(|| value.get("oasf_record_url"))
        .or_else(|| value.get("oasfRecordUrl"))
        .or_else(|| value.get("record"))
        .or_else(|| value.get("artifact"))
        .and_then(|value| same_origin_reference(source, Some(value)));
    let record = record.or_else(|| first_reference(source, value.get("locators")));
    let a2a_endpoint = value
        .get("a2a_endpoint")
        .or_else(|| value.get("a2aEndpoint"))
        .or_else(|| value.get("endpoint"))
        .or_else(|| value.get("a2a"))
        .and_then(|value| same_origin_reference(source, Some(value)));
    let a2a_endpoint = a2a_endpoint.or_else(|| {
        first_jsonrpc_reference(
            source,
            value
                .get("supported_interfaces")
                .or_else(|| value.get("supportedInterfaces")),
        )
    });
    Some(RemoteAgencyAgent {
        id: agent_id,
        name,
        description: text(value.get("description")),
        record_url: record,
        record_revision: text(
            value
                .get("record_revision")
                .or_else(|| value.get("revision")),
        ),
        a2a_endpoint,
        agent_card_url: card,
        capabilities: strings(value.get("capabilities").or_else(|| value.get("skills"))),
    })
}

fn parse_surface(source: &Url, value: &Value) -> Option<RemoteAgencySurface> {
    let surface_id = id(value.get("id").or_else(|| value.get("identifier")))?;
    let name = text(value.get("name")).unwrap_or_else(|| surface_id.clone());
    let locator = value
        .get("locator")
        .or_else(|| value.get("url"))
        .or_else(|| value.get("artifact"))
        .and_then(|value| same_origin_reference(source, Some(value)));
    Some(RemoteAgencySurface {
        id: surface_id,
        name,
        surface_type: text(value.get("type").or_else(|| value.get("experience_type"))),
        locator,
    })
}

fn parse_space(source: &Url, value: &Value) -> Option<RemoteAgencySpace> {
    let space_id = id(value.get("id").or_else(|| value.get("identifier")))
        .or_else(|| id(value.get("space_id")))?;
    let name = text(value.get("name")).unwrap_or_else(|| space_id.clone());
    let surfaces = value
        .get("surfaces")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .take(MAX_ITEMS)
                .filter_map(|value| parse_surface(source, value))
                .collect()
        })
        .unwrap_or_default();
    Some(RemoteAgencySpace {
        id: space_id,
        name,
        description: text(value.get("description")),
        surfaces,
    })
}

/// Validate a descriptor URL before any network request is made.
pub fn validate_remote_agency_url(raw: &str) -> Result<Url, String> {
    let parsed = Url::parse(raw.trim()).map_err(|_| "Remote Agency URL is invalid".to_string())?;
    if parsed.username() != "" || parsed.password().is_some() {
        return Err("Remote Agency URL must not contain credentials".to_string());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "Remote Agency URL must include a host".to_string())?;
    let host = normalized_host(host);
    let local_host = is_loopback_host(&host);
    if parsed.scheme() != "https" && !(parsed.scheme() == "http" && local_host) {
        return Err(
            "Remote Agency URL must use HTTPS (HTTP is allowed only for local development)"
                .to_string(),
        );
    }
    if host.ends_with(".local") || host.contains('%') {
        return Err("Remote Agency URL host is not allowed".to_string());
    }
    if let Ok(address) = host.parse::<IpAddr>() {
        let private = is_private_address(address);
        if private && !(local_host && parsed.scheme() == "http") {
            return Err("Remote Agency URL must not target a private network".to_string());
        }
    }
    Ok(parsed)
}

/// Parse only the public projection needed for the join preview.  This never
/// copies prompts, memory, tool definitions, environment variables, keys, or
/// executable instructions from the source document.
pub fn parse_remote_agency_document(
    source_url: &str,
    bytes: &[u8],
) -> Result<RemoteAgencyDescriptor, String> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err("Remote Agency descriptor exceeds the 1 MiB limit".to_string());
    }
    let source = validate_remote_agency_url(source_url)?;
    let document: Value = serde_json::from_slice(bytes)
        .map_err(|_| "Remote Agency descriptor is not valid JSON".to_string())?;
    let agency = document
        .get("agency")
        .filter(|value| value.is_object())
        .unwrap_or(&document);
    let agency_id = id(agency.get("id").or_else(|| agency.get("identifier")))
        .or_else(|| id(agency.get("agency_id")))
        .ok_or_else(|| "Remote Agency descriptor is missing an agency id".to_string())?;
    let name = text(agency.get("name")).unwrap_or_else(|| agency_id.clone());
    let agents_value = agency
        .get("agents")
        .or_else(|| document.get("agents"))
        .and_then(Value::as_array);
    let agents = agents_value
        .map(|values| {
            values
                .iter()
                .take(MAX_ITEMS)
                .filter_map(|value| parse_agent(&source, value))
                .collect()
        })
        .unwrap_or_default();
    let spaces_value = agency
        .get("spaces")
        .or_else(|| document.get("spaces"))
        .and_then(Value::as_array);
    let spaces = spaces_value
        .map(|values| {
            values
                .iter()
                .take(MAX_ITEMS)
                .filter_map(|value| parse_space(&source, value))
                .collect()
        })
        .unwrap_or_default();
    let protocols = strings(
        document
            .get("protocols")
            .or_else(|| agency.get("protocols")),
    );
    let capabilities = strings(
        document
            .get("capabilities")
            .or_else(|| agency.get("capabilities")),
    );
    Ok(RemoteAgencyDescriptor {
        source_url: source.to_string(),
        agency_id,
        name,
        description: text(agency.get("description")),
        agents,
        spaces,
        protocols,
        capabilities,
    })
}

fn binding_path(app: &AppHandle) -> Result<PathBuf, String> {
    let path = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data dir: {error}"))?;
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("failed to create app data dir: {error}"))?;
    Ok(path.join("remote-agencies.json"))
}

fn load_bindings(app: &AppHandle) -> Result<Vec<RemoteAgencyBinding>, String> {
    let path = binding_path(app)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes =
        std::fs::read(&path).map_err(|error| format!("failed to read remote agencies: {error}"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to parse remote agencies: {error}"))
}

async fn public_addresses(source: &Url) -> Result<Vec<std::net::SocketAddr>, String> {
    let host = source
        .host_str()
        .ok_or_else(|| "Remote Agency URL must include a host".to_string())?;
    let host = normalized_host(host);
    let port = source.port_or_known_default().unwrap_or(443);
    let addresses = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| "Remote Agency host could not be resolved".to_string())?;
    let addresses: Vec<_> = addresses.collect();
    if addresses.is_empty() {
        return Err("Remote Agency host did not resolve to an address".to_string());
    }
    let local_http = source.scheme() == "http" && is_loopback_host(&host);
    if local_http {
        if addresses.iter().any(|address| !address.ip().is_loopback()) {
            return Err("Local Remote Agency URL resolved outside loopback".to_string());
        }
    } else if addresses
        .iter()
        .any(|address| is_private_address(address.ip()))
    {
        return Err("Remote Agency URL resolved to a private network".to_string());
    }
    Ok(addresses)
}

async fn fetch_json_document(source: &Url) -> Result<Value, String> {
    let addresses = public_addresses(source).await?;
    let host = source
        .host_str()
        .ok_or_else(|| "Remote Agency URL must include a host".to_string())?;
    let host = normalized_host(host);
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(FETCH_TIMEOUT)
        .resolve_to_addrs(&host, &addresses)
        .build()
        .map_err(|error| format!("failed to create Remote Agency client: {error}"))?;
    let mut response = client
        .get(source.clone())
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| format!("Remote Agency request failed: {error}"))?;
    if response.status().is_redirection() {
        return Err("Remote Agency redirects are not allowed".to_string());
    }
    if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
        return Err("Remote Agency linked projection requires authentication; use a public record or configure adapter credentials".to_string());
    }
    if !response.status().is_success() {
        return Err(format!("Remote Agency returned HTTP {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_DOCUMENT_BYTES as u64)
    {
        return Err("Remote Agency descriptor exceeds the 1 MiB limit".to_string());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("failed to read Remote Agency descriptor: {error}"))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_DOCUMENT_BYTES {
            return Err("Remote Agency descriptor exceeds the 1 MiB limit".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "Remote Agency descriptor is not valid JSON".to_string())
}

#[tauri::command]
pub async fn preview_remote_agency(source_url: String) -> Result<RemoteAgencyDescriptor, String> {
    let parsed = validate_remote_agency_url(&source_url)?;
    let document = fetch_json_document(&parsed).await?;
    let mut links = linked_urls(&parsed, &document);
    if let Some(agency) = document.get("agency") {
        links.extend(linked_urls(&parsed, agency));
    }
    let mut linked_documents = Vec::new();
    for (kind, url) in links.into_iter().take(4) {
        let linked_url = Url::parse(&url).map_err(|_| "Remote Agency linked URL is invalid")?;
        let linked_document = fetch_json_document(&linked_url).await?;
        linked_documents.push((kind, linked_document));
    }
    parse_preview_document(parsed.as_str(), document, linked_documents)
}

#[tauri::command]
pub fn list_remote_agencies(app: AppHandle) -> Result<Vec<RemoteAgencyBinding>, String> {
    load_bindings(&app)
}

#[tauri::command]
pub fn store_remote_agency_bearer_token(
    record_url: String,
    endpoint: String,
    token: String,
) -> Result<(), String> {
    let key = remote_agency_bearer_token_key(&record_url, &endpoint)?;
    let store = crate::secret_store::SecretStore::shared(crate::app_state::keyring_service());
    if token.is_empty() {
        return store.delete(&key);
    }
    if token.len() > MAX_BEARER_TOKEN_BYTES {
        return Err(format!(
            "Remote Agency bearer token exceeds the {MAX_BEARER_TOKEN_BYTES}-byte limit"
        ));
    }
    if token.chars().any(char::is_whitespace) || token.chars().any(char::is_control) {
        return Err(
            "Remote Agency bearer token must not contain whitespace or control characters"
                .to_string(),
        );
    }
    store.store(&key, &token)
}

#[tauri::command]
pub fn save_remote_agency_binding(
    mut binding: RemoteAgencyBinding,
    app: AppHandle,
) -> Result<RemoteAgencyBinding, String> {
    let source = validate_remote_agency_url(&binding.source_url)?;
    binding.source_url = source.to_string();
    binding.agency_id = binding.agency_id.trim().to_string();
    if binding.agency_id.is_empty() || binding.agency_id.len() > 128 {
        return Err("Remote Agency binding has an invalid agency id".to_string());
    }
    binding.agent_ids.sort();
    binding.agent_ids.dedup();
    binding.space_ids.sort();
    binding.space_ids.dedup();
    binding.channel_ids.sort();
    binding.channel_ids.dedup();
    for proxy in &binding.proxies {
        if proxy.agent_id.is_empty()
            || proxy.agent_id.len() > 128
            || proxy.channel_id.is_empty()
            || proxy.channel_id.len() > 128
            || proxy.pubkey.len() != 64
            || !proxy.pubkey.chars().all(|value| value.is_ascii_hexdigit())
            || proxy.record_url.is_empty()
            || proxy.record_url.len() > MAX_TEXT_BYTES
        {
            return Err("Remote Agency binding has an invalid proxy mapping".to_string());
        }
        if let Some(space_id) = proxy.space_id.as_deref() {
            if space_id.is_empty() || space_id.len() > 128 {
                return Err("Remote Agency binding has an invalid Space id".to_string());
            }
        }
        validate_remote_agency_url(&proxy.record_url)?;
    }
    binding.proxies.sort_by(|left, right| {
        (&left.agent_id, &left.channel_id, &left.space_id).cmp(&(
            &right.agent_id,
            &right.channel_id,
            &right.space_id,
        ))
    });
    binding.proxies.dedup_by(|left, right| {
        left.agent_id == right.agent_id
            && left.channel_id == right.channel_id
            && left.space_id == right.space_id
    });
    let mut bindings = load_bindings(&app)?;
    bindings.retain(|existing| {
        existing.agency_id != binding.agency_id
            || (existing.source_url != binding.source_url
                && !equivalent_loopback_agency_source(&existing.source_url, &binding.source_url))
    });
    bindings.push(binding.clone());
    bindings.sort_by(|left, right| left.source_url.cmp(&right.source_url));
    let payload = serde_json::to_vec_pretty(&bindings)
        .map_err(|error| format!("failed to serialize remote agencies: {error}"))?;
    crate::managed_agents::atomic_write_json_restricted(&binding_path(&app)?, &payload)?;
    Ok(binding)
}

#[cfg(test)]
#[path = "remote_agencies_tests.rs"]
mod tests;
