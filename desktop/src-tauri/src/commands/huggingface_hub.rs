//! Native Hugging Face Hub browsing for the Mesh model picker.
//!
//! The webview never receives the bearer token. It sends only search terms,
//! cursors, and repository ids; this module resolves credentials and performs
//! all Hub requests with redirects disabled.

use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{header, StatusCode};
use serde::{Deserialize, Serialize};
use url::Url;

const HUB_BASE_URL: &str = "https://huggingface.co/";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_SEARCH_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_DETAIL_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_PAGE_SIZE: u8 = 8;
const MAX_PAGE_SIZE: u8 = 10;
const MAX_QUERY_CHARS: usize = 128;
const MAX_CURSOR_CHARS: usize = 4096;
const MAX_GGUF_FILES: usize = 200;

type CmdResult<T> = Result<T, String>;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HuggingFaceSearchRequest {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub page_size: Option<u8>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HuggingFaceModelFile {
    pub path: String,
    pub size_bytes: Option<u64>,
    pub quantization: Option<String>,
    pub multipart: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HuggingFaceModelSummary {
    pub repo_id: String,
    pub revision: String,
    pub gated: bool,
    pub approval_mode: Option<String>,
    pub private: bool,
    pub license: Option<String>,
    pub downloads: u64,
    pub files: Vec<HuggingFaceModelFile>,
    pub web_url: String,
    /// The pinned MeshLLM runtime currently reads HF_TOKEN directly. This is
    /// false for keyring-only auth until its SDK accepts an in-memory token.
    pub gated_download_ready: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HuggingFaceSearchResponse {
    pub repositories: Vec<HuggingFaceModelSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum HubGated {
    Bool(bool),
    Mode(String),
}

impl HubGated {
    fn enabled(&self) -> bool {
        match self {
            Self::Bool(value) => *value,
            Self::Mode(_) => true,
        }
    }

    fn approval_mode(&self) -> Option<String> {
        match self {
            Self::Bool(_) => None,
            Self::Mode(mode) => Some(mode.clone()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct HubSibling {
    rfilename: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    lfs: Option<HubLfs>,
}

#[derive(Debug, Deserialize)]
struct HubLfs {
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HubModel {
    #[serde(alias = "id")]
    model_id: String,
    sha: Option<String>,
    #[serde(default)]
    gated: Option<HubGated>,
    #[serde(default)]
    private: bool,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    siblings: Vec<HubSibling>,
    #[serde(default, rename = "cardData")]
    card_data: Option<HubCardData>,
}

#[derive(Debug, Deserialize)]
struct HubCardData {
    #[serde(default)]
    license: Option<String>,
}

struct HubClient {
    http: reqwest::Client,
    base_url: Url,
    token: Option<String>,
}

impl HubClient {
    fn new(base_url: Url, token: Option<String>) -> CmdResult<Self> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| format!("failed to build Hugging Face client: {error}"))?;
        Ok(Self {
            http,
            base_url,
            token,
        })
    }

    async fn search(
        &self,
        request: HuggingFaceSearchRequest,
    ) -> CmdResult<HuggingFaceSearchResponse> {
        let query = validate_query(&request.query)?;
        let cursor = request.cursor.as_deref().map(validate_cursor).transpose()?;
        let page_size = request
            .page_size
            .unwrap_or(DEFAULT_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE);
        let mut url = self
            .base_url
            .join("api/models")
            .map_err(|_| "invalid Hugging Face models URL".to_string())?;
        {
            let mut pairs = url.query_pairs_mut();
            if !query.is_empty() {
                pairs.append_pair("search", query);
            }
            pairs
                .append_pair("filter", "gguf")
                .append_pair("pipeline_tag", "text-generation")
                .append_pair("sort", "downloads")
                .append_pair("direction", "-1")
                .append_pair("limit", &page_size.to_string())
                .append_pair("full", "true");
            if let Some(cursor) = cursor {
                pairs.append_pair("cursor", cursor);
            }
        }

        let response = self.send(self.http.get(url.clone())).await?;
        let next_cursor = response
            .headers()
            .get(header::LINK)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| next_cursor_from_link(value, &url));
        let models: Vec<HubModel> = read_json(response, MAX_SEARCH_RESPONSE_BYTES).await?;
        let repositories = models
            .into_iter()
            .filter_map(|model| summary_from_hub_model(model, false))
            .collect();
        Ok(HuggingFaceSearchResponse {
            repositories,
            next_cursor,
        })
    }

    async fn detail(&self, repo_id: &str) -> CmdResult<HuggingFaceModelSummary> {
        validate_repo_id(repo_id)?;
        let mut url = self.base_url.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| "invalid Hugging Face base URL".to_string())?;
            segments.pop_if_empty();
            segments.push("api").push("models");
            for segment in repo_id.split('/') {
                segments.push(segment);
            }
        }
        url.query_pairs_mut().append_pair("blobs", "true");
        let response = self.send(self.http.get(url)).await?;
        let model: HubModel = read_json(response, MAX_DETAIL_RESPONSE_BYTES).await?;
        summary_from_hub_model(model, true)
            .ok_or_else(|| "Hugging Face repository has no downloadable GGUF files".to_string())
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> CmdResult<reqwest::Response> {
        let request = match self.token.as_deref() {
            Some(token) => request.bearer_auth(token),
            None => request,
        };
        let response = request
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|error| format!("Hugging Face request failed: {error}"))?;
        match response.status() {
            status if status.is_success() => Ok(response),
            StatusCode::UNAUTHORIZED => Err(
                "Hugging Face authentication failed. Update the saved Hugging Face token."
                    .to_string(),
            ),
            StatusCode::FORBIDDEN => Err(
                "Hugging Face denied access. Accept the model license and grant the token read access."
                    .to_string(),
            ),
            StatusCode::NOT_FOUND => Err(
                "Hugging Face model was not found or the token does not have access.".to_string(),
            ),
            StatusCode::TOO_MANY_REQUESTS => {
                Err("Hugging Face rate limit reached. Try again shortly.".to_string())
            }
            status => Err(format!("Hugging Face returned HTTP {}", status.as_u16())),
        }
    }
}

#[tauri::command]
pub async fn search_huggingface_models(
    request: HuggingFaceSearchRequest,
) -> CmdResult<HuggingFaceSearchResponse> {
    hub_client()?.search(request).await
}

#[tauri::command]
pub async fn get_huggingface_model(repo_id: String) -> CmdResult<HuggingFaceModelSummary> {
    hub_client()?.detail(&repo_id).await
}

fn hub_client() -> CmdResult<HubClient> {
    let base_url =
        Url::parse(HUB_BASE_URL).map_err(|_| "invalid built-in Hugging Face URL".to_string())?;
    HubClient::new(base_url, resolved_hf_token())
}

fn resolved_hf_token() -> Option<String> {
    crate::commands::load_provider_secret("huggingface")
        .ok()
        .flatten()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn mesh_runtime_hf_token_available() -> bool {
    // MeshLLM v0.75.1's host-runtime download entry points construct their HF
    // client from process environment and do not accept an in-memory token.
    // Keep this boundary explicit; do not mutate the process environment from
    // a keyring value. Replace this check when the pinned SDK grows that API.
    std::env::var("HF_TOKEN")
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
}

fn validate_query(query: &str) -> CmdResult<&str> {
    let query = query.trim();
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(format!(
            "Hugging Face search must be at most {MAX_QUERY_CHARS} characters"
        ));
    }
    if query.chars().any(char::is_control) {
        return Err("Hugging Face search contains unsupported characters".to_string());
    }
    Ok(query)
}

fn validate_cursor(cursor: &str) -> CmdResult<&str> {
    if cursor.is_empty()
        || cursor.len() > MAX_CURSOR_CHARS
        || !cursor
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'='))
    {
        return Err("invalid Hugging Face pagination cursor".to_string());
    }
    Ok(cursor)
}

fn validate_repo_id(repo_id: &str) -> CmdResult<()> {
    let mut segments = repo_id.split('/');
    let owner = segments.next().unwrap_or_default();
    let repo = segments.next().unwrap_or_default();
    if owner.is_empty()
        || repo.is_empty()
        || segments.next().is_some()
        || !owner.bytes().all(is_repo_char)
        || !repo.bytes().all(is_repo_char)
    {
        return Err("invalid Hugging Face repository id".to_string());
    }
    Ok(())
}

fn is_repo_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
}

fn next_cursor_from_link(link: &str, request_url: &Url) -> Option<String> {
    for part in link.split(',') {
        if !part.contains("rel=\"next\"") {
            continue;
        }
        let target = part.trim().strip_prefix('<')?.split_once('>')?.0;
        let url = Url::parse(target).ok()?;
        if url.scheme() != request_url.scheme()
            || url.host_str() != request_url.host_str()
            || url.port_or_known_default() != request_url.port_or_known_default()
        {
            return None;
        }
        return url
            .query_pairs()
            .find_map(|(key, value)| (key == "cursor").then(|| value.into_owned()))
            .filter(|cursor| validate_cursor(cursor).is_ok());
    }
    None
}

fn summary_from_hub_model(model: HubModel, include_sizes: bool) -> Option<HuggingFaceModelSummary> {
    if validate_repo_id(&model.model_id).is_err() {
        return None;
    }
    let revision = model.sha.filter(|sha| is_commit_sha(sha))?;
    let gated = model.gated.as_ref().is_some_and(HubGated::enabled);
    let approval_mode = model.gated.as_ref().and_then(HubGated::approval_mode);
    let license = model
        .card_data
        .and_then(|card| card.license)
        .or_else(|| license_from_tags(&model.tags));
    let files = gguf_files(&model.siblings, include_sizes);
    if files.is_empty() {
        return None;
    }
    let web_url = format!("https://huggingface.co/{}", model.model_id);
    Some(HuggingFaceModelSummary {
        repo_id: model.model_id,
        revision,
        gated,
        approval_mode,
        private: model.private,
        license,
        downloads: model.downloads,
        files,
        web_url,
        gated_download_ready: mesh_runtime_hf_token_available(),
    })
}

fn is_commit_sha(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn license_from_tags(tags: &[String]) -> Option<String> {
    tags.iter()
        .find_map(|tag| tag.strip_prefix("license:"))
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn gguf_files(siblings: &[HubSibling], include_sizes: bool) -> Vec<HuggingFaceModelFile> {
    siblings
        .iter()
        .filter(|sibling| valid_hub_gguf_path(&sibling.rfilename))
        .filter(|sibling| is_first_or_single_gguf(&sibling.rfilename))
        .take(MAX_GGUF_FILES)
        .map(|sibling| {
            let multipart = multipart_prefix_and_total(&sibling.rfilename).is_some();
            let size_bytes = include_sizes.then(|| {
                if let Some((prefix, total)) = multipart_prefix_and_total(&sibling.rfilename) {
                    siblings
                        .iter()
                        .filter(|candidate| multipart_member(&candidate.rfilename, prefix, total))
                        .filter_map(sibling_size)
                        .fold(0_u64, u64::saturating_add)
                } else {
                    sibling_size(sibling).unwrap_or_default()
                }
            });
            HuggingFaceModelFile {
                path: sibling.rfilename.clone(),
                size_bytes: size_bytes.filter(|size| *size > 0),
                quantization: quantization_from_filename(&sibling.rfilename),
                multipart,
            }
        })
        .collect()
}

fn valid_hub_gguf_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path.to_ascii_lowercase().ends_with(".gguf")
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."))
}

fn sibling_size(sibling: &HubSibling) -> Option<u64> {
    sibling
        .size
        .or_else(|| sibling.lfs.as_ref().and_then(|lfs| lfs.size))
}

fn is_first_or_single_gguf(file: &str) -> bool {
    !file.contains("-of-") || multipart_prefix_and_total(file).is_some()
}

fn multipart_prefix_and_total(file: &str) -> Option<(&str, &str)> {
    let stem = file.strip_suffix(".gguf")?;
    let (prefix, total) = stem.rsplit_once("-00001-of-")?;
    (!prefix.is_empty() && total.len() == 5 && total.bytes().all(|byte| byte.is_ascii_digit()))
        .then_some((prefix, total))
}

fn multipart_member(file: &str, prefix: &str, total: &str) -> bool {
    let Some(stem) = file.strip_suffix(".gguf") else {
        return false;
    };
    let Some(rest) = stem
        .strip_prefix(prefix)
        .and_then(|value| value.strip_prefix('-'))
    else {
        return false;
    };
    let Some((part, candidate_total)) = rest.split_once("-of-") else {
        return false;
    };
    part.len() == 5 && part.bytes().all(|byte| byte.is_ascii_digit()) && candidate_total == total
}

fn quantization_from_filename(file: &str) -> Option<String> {
    let basename = file.rsplit('/').next().unwrap_or(file);
    let mut stem = basename.strip_suffix(".gguf").unwrap_or(basename);
    if let Some((prefix, _)) = multipart_prefix_and_total(basename) {
        stem = prefix.rsplit('/').next().unwrap_or(prefix);
    }
    let parts: Vec<&str> = stem.split(['-', '.']).collect();
    for (index, part) in parts.iter().enumerate().rev() {
        let upper = part.to_ascii_uppercase();
        let first = upper.as_bytes().first().copied();
        let looks_quantized = upper == "BF16"
            || upper == "F16"
            || upper == "F32"
            || upper.starts_with("IQ")
            || upper.starts_with("TQ")
            || (first == Some(b'Q') && upper.as_bytes().get(1).is_some_and(u8::is_ascii_digit));
        if looks_quantized {
            if index > 0 && parts[index - 1].eq_ignore_ascii_case("UD") {
                return Some(format!("UD-{upper}"));
            }
            return Some(upper);
        }
    }
    None
}

async fn read_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    limit: usize,
) -> CmdResult<T> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err("Hugging Face returned an oversized response".to_string());
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|error| format!("reading Hugging Face response failed: {error}"))?;
        if body.len().saturating_add(chunk.len()) > limit {
            return Err("Hugging Face returned an oversized response".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body)
        .map_err(|_| "Hugging Face returned malformed model metadata".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sibling(path: &str, size: u64) -> HubSibling {
        HubSibling {
            rfilename: path.to_string(),
            size: Some(size),
            lfs: None,
        }
    }

    #[test]
    fn extracts_same_origin_cursor_only() {
        let request = Url::parse("https://huggingface.co/api/models?limit=8").unwrap();
        assert_eq!(
            next_cursor_from_link(
                "<https://huggingface.co/api/models?limit=8&cursor=abc_123>; rel=\"next\"",
                &request
            )
            .as_deref(),
            Some("abc_123")
        );
        assert_eq!(
            next_cursor_from_link(
                "<https://attacker.invalid/api/models?cursor=abc_123>; rel=\"next\"",
                &request
            ),
            None
        );
    }

    #[test]
    fn filters_non_gguf_and_collapses_multipart_files() {
        let files = gguf_files(
            &[
                sibling("model-Q4_K_M-00001-of-00002.gguf", 10),
                sibling("model-Q4_K_M-00002-of-00002.gguf", 20),
                sibling("model-Q8_0.gguf", 40),
                sibling("README.md", 5),
            ],
            true,
        );
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].size_bytes, Some(30));
        assert_eq!(files[0].quantization.as_deref(), Some("Q4_K_M"));
        assert!(files[0].multipart);
        assert_eq!(files[1].quantization.as_deref(), Some("Q8_0"));
    }

    #[test]
    fn multipart_size_saturates_untrusted_hub_metadata() {
        let files = gguf_files(
            &[
                sibling("model-Q4_K_M-00001-of-00002.gguf", u64::MAX),
                sibling("model-Q4_K_M-00002-of-00002.gguf", 10),
            ],
            true,
        );
        assert_eq!(files[0].size_bytes, Some(u64::MAX));
    }

    #[test]
    fn validates_user_controlled_inputs() {
        assert!(validate_repo_id("org/repo").is_ok());
        assert!(validate_repo_id("org/repo/extra").is_err());
        assert!(validate_repo_id("org/../repo").is_err());
        assert!(validate_cursor("eyJkb2xsYXIiOiIxIn0=").is_ok());
        assert!(validate_cursor("https://attacker.invalid").is_err());
        assert!(valid_hub_gguf_path("quant/model-Q4_K_M.gguf"));
        assert!(!valid_hub_gguf_path("../model.gguf"));
        assert!(!valid_hub_gguf_path("models\\model.gguf"));
    }
}
