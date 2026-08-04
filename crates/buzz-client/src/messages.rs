use std::collections::{HashMap, HashSet};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use bytes::Bytes;
use nostr::{EventBuilder, EventId, JsonUtil, Kind, PublicKey, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use buzz_sdk::mentions::{
    extract_at_mentions_with_known, extract_nostr_uris, strip_code_regions, MENTION_CAP,
};
use buzz_sdk::ThreadRef;

use crate::{BuzzClient, ClientError, DeliveryOutcome};

const ALLOWED_MIMES: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "video/mp4",
];
const MAX_IMAGE_BYTES: usize = 50 * 1024 * 1024;
const MAX_VIDEO_BYTES: usize = 500 * 1024 * 1024;

/// Message event variant supported by the initial reusable client slice.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    /// NIP-29 stream message (kind 9).
    #[default]
    Stream,
    /// Forum thread root (kind 45001).
    ForumPost,
    /// Forum reply (kind 45003).
    ForumComment,
}

/// In-memory media acquired by an application adapter for message sending.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageAttachment {
    /// Complete media bytes.
    pub bytes: Bytes,
    /// Detected MIME type used for relay upload policy.
    pub mime_type: String,
}

/// Normalized request for one scoped Buzz message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SendMessageRequest {
    /// Target channel in the client's bound community.
    pub channel_id: Uuid,
    /// Message body before uploaded-media links are appended.
    pub content: String,
    /// Message event variant.
    pub kind: MessageKind,
    /// Immediate parent event for a reply.
    pub reply_to: Option<EventId>,
    /// Whether a stream message carries the Buzz broadcast marker.
    pub broadcast: bool,
    /// Media bytes to upload to the bound community.
    pub attachments: Vec<MessageAttachment>,
    /// Explicit notification identities.
    pub mentions: Vec<PublicKey>,
}

/// Message delivery plus identities represented by signed `p` tags.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SendMessageResult {
    /// Semantic accepted, rejected, or delivery-unknown result.
    pub delivery: DeliveryOutcome,
    /// Deduplicated identities represented in the signed event.
    pub mention_pubkeys: Vec<PublicKey>,
}

#[derive(Debug, Deserialize)]
struct BlobDescriptor {
    url: String,
    sha256: String,
    size: u64,
    #[serde(rename = "type")]
    mime_type: String,
    #[serde(default)]
    dim: Option<String>,
    #[serde(default)]
    blurhash: Option<String>,
    #[serde(default)]
    thumb: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
}

impl BuzzClient {
    /// Resolve, build, sign once, and submit a message to the bound community.
    pub async fn send_message(
        &self,
        request: SendMessageRequest,
    ) -> Result<SendMessageResult, ClientError> {
        if request.content.len() > 64 * 1024 {
            return Err(ClientError::InvalidInput(
                "message content exceeds 65536 bytes".into(),
            ));
        }
        if request.kind == MessageKind::ForumComment && request.reply_to.is_none() {
            return Err(ClientError::InvalidInput(
                "reply_to is required for forum comments".into(),
            ));
        }

        let mention_pubkeys = self.resolve_mentions(&request).await?;
        let mut media_tags = Vec::new();
        let mut media_content = String::new();
        for attachment in &request.attachments {
            let descriptor = self.upload_attachment(attachment).await?;
            media_tags.push(imeta_tag(&descriptor));
            media_content.push_str(if descriptor.mime_type.starts_with("video/") {
                "\n![video]("
            } else {
                "\n![image]("
            });
            media_content.push_str(&descriptor.url);
            media_content.push(')');
        }
        let content = format!("{}{media_content}", request.content);
        let thread = match request.reply_to {
            Some(parent) => Some(self.resolve_thread(parent).await?),
            None => None,
        };
        let mention_hex: Vec<String> = mention_pubkeys.iter().map(PublicKey::to_hex).collect();
        let mention_refs: Vec<&str> = mention_hex.iter().map(String::as_str).collect();
        let builder = match request.kind {
            MessageKind::Stream => buzz_sdk::build_message(
                request.channel_id,
                &content,
                thread.as_ref(),
                &mention_refs,
                request.broadcast,
                &media_tags,
            ),
            MessageKind::ForumPost => {
                buzz_sdk::build_forum_post(request.channel_id, &content, &mention_refs, &media_tags)
            }
            MessageKind::ForumComment => buzz_sdk::build_forum_comment(
                request.channel_id,
                &content,
                thread.as_ref().ok_or_else(|| {
                    ClientError::InvalidInput("reply_to is required for forum comments".into())
                })?,
                &mention_refs,
                &media_tags,
            ),
        }
        .map_err(|error| ClientError::InvalidInput(error.to_string()))?;
        let event = self.sign_builder(builder).await?;
        let emitted_mentions = event
            .tags
            .iter()
            .filter_map(|tag| {
                let parts = tag.as_slice();
                (parts.first().map(String::as_str) == Some("p"))
                    .then(|| parts.get(1))
                    .flatten()
                    .and_then(|value| PublicKey::from_hex(value).ok())
            })
            .collect();
        let delivery = self.submit_stored_event(event).await?;
        Ok(SendMessageResult {
            delivery,
            mention_pubkeys: emitted_mentions,
        })
    }

    async fn resolve_mentions(
        &self,
        request: &SendMessageRequest,
    ) -> Result<Vec<PublicKey>, ClientError> {
        let stripped = strip_code_regions(&request.content);
        let mut mentions = request.mentions.clone();
        for uri in extract_nostr_uris(&stripped) {
            if let Ok(pubkey) = PublicKey::from_hex(&uri) {
                if !mentions.contains(&pubkey) {
                    mentions.push(pubkey);
                }
            }
        }
        let has_explicit = !mentions.is_empty();
        if !stripped.contains('@') && !has_explicit {
            return Ok(mentions);
        }
        let membership_events = self
            .query_filters(&[serde_json::json!({
                "kinds": [39002],
                "#d": [request.channel_id.to_string()],
                "limit": 1,
            })])
            .await?;
        let members = membership_events
            .first()
            .map(member_pubkeys)
            .unwrap_or_default();

        if stripped.contains('@') {
            let authors: Vec<String> = members.iter().map(PublicKey::to_hex).collect();
            let profiles = self
                .query_filters(&[serde_json::json!({
                    "kinds": [0],
                    "authors": authors,
                    "limit": members.len(),
                })])
                .await?;
            let (names, display_names) = profile_names(&profiles);
            let known: Vec<&str> = display_names.iter().map(String::as_str).collect();
            for name in extract_at_mentions_with_known(&stripped, &known) {
                match names.get(&name) {
                    Some(candidates) if candidates.len() == 1 => {
                        if !mentions.contains(&candidates[0]) {
                            mentions.push(candidates[0]);
                        }
                    }
                    None if has_explicit => {}
                    Some(_) if has_explicit => {}
                    None => {
                        return Err(ClientError::InvalidInput(format!(
                            "mention '@{name}' does not match a current channel member; retry with an explicit pubkey"
                        )));
                    }
                    Some(candidates) => {
                        return Err(ClientError::InvalidInput(format!(
                            "mention '@{name}' is ambiguous; candidates: {}",
                            candidates
                                .iter()
                                .map(PublicKey::to_hex)
                                .collect::<Vec<_>>()
                                .join(", ")
                        )));
                    }
                }
            }
        }
        if mentions.len() > MENTION_CAP {
            return Err(ClientError::InvalidInput(format!(
                "too many unique message mentions (max {MENTION_CAP})"
            )));
        }
        let member_set: HashSet<PublicKey> = members.into_iter().collect();
        let missing: Vec<String> = mentions
            .iter()
            .filter(|pubkey| !member_set.contains(pubkey))
            .map(PublicKey::to_hex)
            .collect();
        if !missing.is_empty() {
            return Err(ClientError::InvalidInput(
                serde_json::json!({
                    "message": "mentioned pubkeys are not channel members; add them explicitly before retrying",
                    "missing_member_pubkeys": missing,
                    "add_member_command": format!(
                        "buzz channels add-member --channel {} --pubkey <pubkey> --role <member|bot>",
                        request.channel_id
                    ),
                })
                .to_string(),
            ));
        }
        Ok(mentions)
    }

    async fn resolve_thread(&self, parent: EventId) -> Result<ThreadRef, ClientError> {
        let parent_hex = parent.to_hex();
        let events = self
            .query_filters(&[serde_json::json!({"ids": [parent_hex], "limit": 1})])
            .await?;
        let event = events.first().ok_or_else(|| {
            ClientError::InvalidInput(format!("parent event {parent_hex} not found"))
        })?;
        let root = thread_root(event)
            .and_then(|hex| EventId::from_hex(hex).ok())
            .filter(|root| *root != parent)
            .unwrap_or(parent);
        Ok(ThreadRef {
            root_event_id: root,
            parent_event_id: parent,
        })
    }

    async fn upload_attachment(
        &self,
        attachment: &MessageAttachment,
    ) -> Result<BlobDescriptor, ClientError> {
        if !ALLOWED_MIMES.contains(&attachment.mime_type.as_str()) {
            return Err(ClientError::InvalidInput(format!(
                "unsupported file type: {}",
                attachment.mime_type
            )));
        }
        let max = if attachment.mime_type.starts_with("video/") {
            MAX_VIDEO_BYTES
        } else {
            MAX_IMAGE_BYTES
        };
        if attachment.bytes.len() > max {
            return Err(ClientError::InvalidInput(format!(
                "file too large: {} bytes (max {max})",
                attachment.bytes.len()
            )));
        }
        let sha256 = hex::encode(Sha256::digest(&attachment.bytes));
        let primary = self.endpoint().upload_url();
        match self.upload_to(&primary, attachment, &sha256).await {
            Err(ClientError::Relay {
                status: 404 | 405, ..
            }) => {
                let mut legacy = self.endpoint().http_base_url().clone();
                legacy.set_path("/media/upload");
                self.upload_to(&legacy, attachment, &sha256).await
            }
            result => result,
        }
    }

    async fn upload_to(
        &self,
        url: &url::Url,
        attachment: &MessageAttachment,
        sha256: &str,
    ) -> Result<BlobDescriptor, ClientError> {
        let attempts = self.config().retry_policy().max_attempts();
        for attempt in 0..attempts {
            let auth = self
                .blossom_upload_header(sha256, &attachment.mime_type)
                .await?;
            let mut request = self
                .http
                .put(url.clone())
                .header("Authorization", auth)
                .header("Content-Type", &attachment.mime_type)
                .header("X-SHA-256", sha256)
                .body(attachment.bytes.clone());
            if let Some(ambient) = self.auth.ambient() {
                request = request.header("x-auth-tag", &ambient.json);
            }
            match request.send().await {
                Err(_source) if attempt + 1 < attempts => {
                    self.sleep_before_retry(attempt).await;
                }
                Err(source) => return Err(ClientError::Network(source)),
                Ok(response) => {
                    let status = response.status().as_u16();
                    let text = response.text().await.map_err(ClientError::Network)?;
                    if (200..300).contains(&status) {
                        let descriptor: BlobDescriptor =
                            serde_json::from_str(&text).map_err(ClientError::Serialization)?;
                        self.endpoint().same_authority_url(&descriptor.url)?;
                        return Ok(descriptor);
                    }
                    if attempt + 1 < attempts && matches!(status, 429 | 502 | 503 | 504) {
                        self.sleep_before_retry(attempt).await;
                        continue;
                    }
                    return Err(ClientError::Relay {
                        status,
                        reason: relay_reason(&text),
                        retry_safe: matches!(status, 429 | 502 | 503 | 504),
                    });
                }
            }
        }
        Err(ClientError::Configuration(
            "retry policy allowed no upload attempts".into(),
        ))
    }

    async fn blossom_upload_header(
        &self,
        sha256: &str,
        mime_type: &str,
    ) -> Result<String, ClientError> {
        let expiry = if mime_type.starts_with("video/") {
            3600
        } else {
            600
        };
        let expiration = (Timestamp::now().as_secs() + expiry).to_string();
        let server =
            self.endpoint().http_base_url().host_str().ok_or_else(|| {
                ClientError::InvalidInput("community endpoint has no host".into())
            })?;
        let tags = vec![
            parse_tag(["t", "upload"])?,
            parse_tag(["x", sha256])?,
            parse_tag(["expiration", expiration.as_str()])?,
            parse_tag(["server", server])?,
        ];
        let unsigned = EventBuilder::new(Kind::Custom(24242), "Upload file")
            .tags(tags)
            .build(self.public_key());
        let event = self
            .signer
            .sign_event(unsigned)
            .await
            .map_err(ClientError::Signer)?;
        Ok(format!(
            "Nostr {}",
            URL_SAFE_NO_PAD.encode(event.as_json().as_bytes())
        ))
    }
}

fn member_pubkeys(event: &serde_json::Value) -> Vec<PublicKey> {
    event
        .get("tags")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_array)
        .filter(|parts| parts.first().and_then(serde_json::Value::as_str) == Some("p"))
        .filter_map(|parts| parts.get(1).and_then(serde_json::Value::as_str))
        .filter_map(|hex| PublicKey::from_hex(hex).ok())
        .collect()
}

fn profile_names(events: &[serde_json::Value]) -> (HashMap<String, Vec<PublicKey>>, Vec<String>) {
    let mut names: HashMap<String, Vec<PublicKey>> = HashMap::new();
    let mut display_names = Vec::new();
    for event in events {
        let Some(pubkey) = event
            .get("pubkey")
            .and_then(serde_json::Value::as_str)
            .and_then(|hex| PublicKey::from_hex(hex).ok())
        else {
            continue;
        };
        let Some(profile) = event
            .get("content")
            .and_then(serde_json::Value::as_str)
            .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok())
        else {
            continue;
        };
        let Some(name) = profile
            .get("display_name")
            .or_else(|| profile.get("name"))
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        names
            .entry(name.to_ascii_lowercase())
            .or_default()
            .push(pubkey);
        display_names.push(name.to_string());
    }
    (names, display_names)
}

fn thread_root(event: &serde_json::Value) -> Option<&str> {
    let mut root = None;
    let mut reply = None;
    for parts in event
        .get("tags")?
        .as_array()?
        .iter()
        .filter_map(serde_json::Value::as_array)
    {
        if parts.len() < 4 || parts.first().and_then(serde_json::Value::as_str) != Some("e") {
            continue;
        }
        let id = parts.get(1).and_then(serde_json::Value::as_str);
        match (parts.get(3).and_then(serde_json::Value::as_str), id) {
            (Some("root"), Some(id)) => root = Some(id),
            (Some("reply"), Some(id)) => reply = Some(id),
            _ => {}
        }
    }
    root.or(reply)
}

fn imeta_tag(descriptor: &BlobDescriptor) -> Vec<String> {
    let mut tag = vec![
        "imeta".to_string(),
        format!("url {}", descriptor.url),
        format!("m {}", descriptor.mime_type),
        format!("x {}", descriptor.sha256),
        format!("size {}", descriptor.size),
    ];
    if let Some(dim) = &descriptor.dim {
        tag.push(format!("dim {dim}"));
    }
    if let Some(blurhash) = &descriptor.blurhash {
        tag.push(format!("blurhash {blurhash}"));
    }
    if let Some(thumb) = &descriptor.thumb {
        tag.push(format!("thumb {thumb}"));
    }
    if let Some(duration) = descriptor.duration {
        tag.push(format!("duration {duration}"));
    }
    tag
}

fn parse_tag<const N: usize>(parts: [&str; N]) -> Result<Tag, ClientError> {
    Tag::parse(parts).map_err(|error| ClientError::InvalidInput(error.to_string()))
}

fn relay_reason(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .or_else(|| value.get("message"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.to_string())
}
