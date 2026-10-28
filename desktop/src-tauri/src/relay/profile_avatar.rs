//! Materialize an agent avatar in the community receiving its kind:0 profile.
//! The saved source is never replaced with the community-local projection.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use futures_util::StreamExt;
use nostr::{JsonUtil, Keys};
use sha2::{Digest, Sha256};

use crate::{
    app_state::AppState,
    commands::media::{
        detect_and_validate_mime, sign_blossom_get_auth_header, sign_blossom_upload_auth,
        BlobDescriptor,
    },
};

use super::{classify_request_error, relay_http_base_url};

const MAX_AVATAR_BYTES: usize = 10 * 1024 * 1024;
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn community_base(relay: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(&relay_http_base_url(relay)).map_err(|e| e.to_string())?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || (url.scheme() == "http" && local))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("avatar community must be an HTTPS origin (HTTP only on localhost)".into());
    }
    Ok(url)
}

fn media_hash(url: &url::Url) -> Result<&str, String> {
    let filename = url.path().strip_prefix("/media/").unwrap_or("");
    let hash = filename.split('.').next().unwrap_or("");
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || filename.contains('/')
        || filename.contains(".thumb.")
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("agent avatar must reference an original content-addressed image".into());
    }
    Ok(hash)
}

/// Copy only user-configured community media. Public external images and inline
/// emoji retain their existing behavior; no credentials are sent to their hosts.
/// All I/O uses the caller's agent identity and target, never active workspace keys.
pub(crate) async fn localize_avatar(
    state: &AppState,
    relay: &str,
    keys: &Keys,
    avatar: Option<&str>,
    auth_tag: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(avatar) = avatar else {
        return Ok(None);
    };
    let Ok(source) = url::Url::parse(avatar) else {
        return Ok(Some(avatar.into()));
    };
    if !matches!(source.scheme(), "http" | "https") || !source.path().starts_with("/media/") {
        return Ok(Some(avatar.into()));
    }
    let target = community_base(relay)?;
    if source.origin() == target.origin() {
        return Ok(Some(avatar.into()));
    }
    // Classify against user configuration, never a profile-provided host.
    // The source permission is rechecked after the target probe below.
    let communities = state
        .agent_avatar_communities
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    let Some(source_base) = communities
        .iter()
        .filter_map(|relay| community_base(relay).ok())
        .find(|base| base.origin() == source.origin())
    else {
        // A removed community is no longer trusted for source reads. Its saved
        // source may still have an identical target projection: keep that image
        // for both reconcile and explicit metadata edits, without rewriting
        // unrelated public URLs or authenticating to their hosts.
        if media_hash(&source).is_ok() {
            if let Some(picture) =
                existing_projection(state, &target, keys, &source, auth_tag).await?
            {
                return Ok(Some(picture));
            }
        }
        return Ok(Some(avatar.into()));
    };
    let hash = media_hash(&source)?;
    // Content-addressing gives a deterministic projection. Once copied, an
    // unavailable source community must not break restart/reconciliation.
    let localized = target.join(source.path()).map_err(|e| e.to_string())?;
    let target_auth = sign_blossom_get_auth_header(keys, target.as_str(), 60)?;
    let mut probe = state
        .media_fetch_client
        .head(localized.clone())
        .timeout(TIMEOUT)
        .header("Authorization", target_auth);
    if let Some(tag) = auth_tag {
        probe = probe.header("x-auth-tag", tag);
    }
    let probe = probe.send().await.map_err(|e| classify_request_error(&e))?;
    if probe.status().is_success() {
        return Ok(Some(localized.into()));
    }
    if probe.status() != reqwest::StatusCode::NOT_FOUND {
        return Err(format!(
            "could not check the community avatar: HTTP {}",
            probe.status()
        ));
    }
    if !state
        .agent_avatar_communities
        .lock()
        .map_err(|e| e.to_string())?
        .iter()
        .filter_map(|relay| community_base(relay).ok())
        .any(|base| base.origin() == source_base.origin())
    {
        return Err(
            "avatar source community was removed during transfer; retry after choosing an image"
                .into(),
        );
    }
    let auth = sign_blossom_get_auth_header(keys, source_base.as_str(), 60)?;
    let mut request = state
        .media_fetch_client
        .get(source.clone())
        .timeout(TIMEOUT)
        .header("Authorization", auth);
    if let Some(tag) = auth_tag {
        request = request.header("x-auth-tag", tag);
    }
    let response = request
        .send()
        .await
        .map_err(|e| classify_request_error(&e))?;
    let bytes = read_bounded(response, MAX_AVATAR_BYTES).await?;
    if hex::encode(Sha256::digest(&bytes)) != hash {
        return Err("agent avatar content hash does not match its URL".into());
    }
    let mime = detect_and_validate_mime(&bytes)?;
    if !mime.starts_with("image/") {
        return Err("agent avatar must be an image".into());
    }
    // Preserve the already-uploaded bytes (including animation). Re-encoding
    // changes the content hash and needlessly destroys the portable identity.
    let upload_auth = sign_blossom_upload_auth(keys, hash, 60, target.as_str())?;
    let auth = format!(
        "Nostr {}",
        URL_SAFE_NO_PAD.encode(upload_auth.as_json().as_bytes())
    );
    let body = bytes::Bytes::from(bytes);
    let mut response = None;
    for path in ["upload", "media/upload"] {
        let url = target.join(path).map_err(|e| e.to_string())?;
        let mut request = state
            .media_fetch_client
            .put(url)
            .timeout(TIMEOUT)
            .header("Authorization", &auth)
            .header("Content-Type", &mime)
            .header("X-SHA-256", hash)
            .body(body.clone());
        if let Some(tag) = auth_tag {
            request = request.header("x-auth-tag", tag);
        }
        let uploaded = request
            .send()
            .await
            .map_err(|e| classify_request_error(&e))?;
        let legacy = matches!(
            uploaded.status(),
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED
        );
        response = Some(uploaded);
        if !legacy {
            break;
        }
    }
    let response = response.ok_or("agent avatar upload did not run")?;
    let descriptor: BlobDescriptor =
        serde_json::from_slice(&read_bounded(response, 64 * 1024).await?)
            .map_err(|e| format!("invalid avatar upload response: {e}"))?;
    let localized = url::Url::parse(&descriptor.url).map_err(|e| e.to_string())?;
    if localized.origin() != target.origin()
        || media_hash(&localized)? != hash
        || descriptor.sha256 != hash
        || descriptor.mime_type != mime
        || descriptor.size != body.len() as u64
    {
        return Err("avatar upload returned a mismatched image or community".into());
    }
    Ok(Some(localized.into()))
}

/// The target's signed profile is evidence of a prior projection, not source trust.
async fn existing_projection(
    state: &AppState,
    target: &url::Url,
    keys: &Keys,
    source: &url::Url,
    auth_tag: Option<&str>,
) -> Result<Option<String>, String> {
    let url = target.join("query").map_err(|e| e.to_string())?;
    let body = serde_json::to_vec(&serde_json::json!([{
        "authors": [keys.public_key().to_hex()], "kinds": [0], "limit": 1
    }]))
    .map_err(|e| e.to_string())?;
    let auth =
        super::build_nip98_auth_header_for_keys(keys, &reqwest::Method::POST, url.as_str(), &body)?;
    let response = super::build_authenticated_relay_request(
        &state.media_fetch_client,
        reqwest::Method::POST,
        url.as_str(),
        &auth,
        Some(body),
        auth_tag,
        Some(TIMEOUT),
    )
    .send()
    .await
    .map_err(|e| classify_request_error(&e))?;
    let events: Vec<nostr::Event> =
        serde_json::from_slice(&read_bounded(response, 64 * 1024).await?)
            .map_err(|e| format!("invalid avatar profile response: {e}"))?;
    let Some(event) = events.first() else {
        return Ok(None);
    };
    if event.kind.as_u16() != 0 || event.pubkey != keys.public_key() || event.verify().is_err() {
        return Err("invalid avatar profile signer or kind".into());
    }
    let profile: serde_json::Value =
        serde_json::from_str(&event.content).map_err(|e| e.to_string())?;
    let Some(picture) = profile["picture"]
        .as_str()
        .and_then(|p| url::Url::parse(p).ok())
    else {
        return Ok(None);
    };
    Ok((picture.origin() == target.origin()
        && media_hash(&picture).ok() == Some(media_hash(source)?))
    .then(|| picture.into()))
}

async fn read_bounded(response: reqwest::Response, cap: usize) -> Result<Vec<u8>, String> {
    if !response.status().is_success() {
        return Err(format!(
            "agent avatar transfer failed: HTTP {} (redirects are not followed)",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size > cap as u64)
    {
        return Err("agent avatar transfer exceeds size limit".into());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| classify_request_error(&e))?;
        if bytes.len().saturating_add(chunk.len()) > cap {
            return Err("agent avatar transfer exceeds size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
