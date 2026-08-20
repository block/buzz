use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
#[cfg(test)]
use buzz_core::kind::KIND_STREAM_MESSAGE;
use buzz_core::{
    agent_job::{AgentJobRequest, AGENT_JOB_SCHEMA},
    kind::{KIND_MANAGED_AGENT, KIND_PRESENCE_SNAPSHOT, KIND_USER_STATUS},
};
use buzz_sdk::{mentions::MENTION_CAP, ThreadRef};
use nostr::{Event, EventBuilder, EventId, JsonUtil, Keys, Kind, PublicKey, Tag};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};
use uuid::Uuid;

const MAX_CONTENT_BYTES: usize = 65_536;
const MAX_QUERY_BYTES: usize = 512;
const MAX_MESSAGE_LIMIT: u16 = 100;
const MAX_THREAD_LIMIT: u16 = 200;
const MAX_AGENT_LIMIT: usize = 100;
const MAX_CHANNEL_SCOPE: usize = 100;
const MAX_OUTPUT_BYTES: usize = 512 * 1024;
const MESSAGE_KINDS: [u32; 4] = [9, 40002, 45001, 45003];

type RelayFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, RelayError>> + Send + 'a>>;

#[derive(Debug)]
struct RelayError;

trait RelayTransport: Send + Sync {
    fn query<'a>(&'a self, filter: Value) -> RelayFuture<'a, Vec<Value>>;
    fn publish<'a>(&'a self, event: Event) -> RelayFuture<'a, ()>;
}

#[derive(Clone)]
pub(crate) struct CollaborationClient {
    keys: Keys,
    auth_tag: Option<Tag>,
    relay: Arc<dyn RelayTransport>,
}

struct HttpRelay {
    http: reqwest::Client,
    relay_url: String,
    keys: Keys,
    auth_tag_json: Option<String>,
}

impl CollaborationClient {
    pub(crate) fn from_env(
        expected_pubkey: &str,
        expected_relay_url: &str,
    ) -> Result<Self, ErrorData> {
        let private_key = std::env::var("BUZZ_PRIVATE_KEY").map_err(|_| managed_auth_error())?;
        let keys = Keys::parse(&private_key).map_err(|_| managed_auth_error())?;
        if keys.public_key().to_hex() != expected_pubkey {
            return Err(managed_auth_error());
        }
        let relay_url = std::env::var("BUZZ_RELAY_URL")
            .map(|url| normalize_relay_url(&url))
            .map_err(|_| managed_auth_error())?;
        if relay_url != normalize_relay_url(expected_relay_url) {
            return Err(managed_auth_error());
        }
        let (auth_tag, auth_tag_json) = match std::env::var("BUZZ_AUTH_TAG")
            .ok()
            .filter(|value| !value.is_empty())
        {
            Some(raw) => {
                let tag =
                    buzz_sdk::nip_oa::parse_auth_tag(&raw).map_err(|_| managed_auth_error())?;
                buzz_sdk::nip_oa::verify_auth_tag(&raw, &keys.public_key())
                    .map_err(|_| managed_auth_error())?;
                (Some(tag), Some(raw))
            }
            None => (None, None),
        };
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|_| relay_error("relay_client_unavailable"))?;
        let relay = Arc::new(HttpRelay {
            http,
            relay_url,
            keys: keys.clone(),
            auth_tag_json,
        });
        Ok(Self {
            keys,
            auth_tag,
            relay,
        })
    }

    #[cfg(test)]
    fn for_test(keys: Keys, relay: Arc<dyn RelayTransport>) -> Self {
        Self {
            keys,
            auth_tag: None,
            relay,
        }
    }

    #[cfg(test)]
    pub(crate) fn unavailable_for_test() -> Self {
        Self::for_test(Keys::generate(), Arc::new(UnavailableRelay))
    }

    pub(crate) fn current_pubkey(&self) -> PublicKey {
        self.keys.public_key()
    }

    pub(crate) async fn jobs_request_remote(
        &self,
        channel_id: Uuid,
        target: PublicKey,
        source_event_id: Option<EventId>,
        argv: Vec<String>,
        cwd: String,
        summary: String,
    ) -> Result<String, ErrorData> {
        let target_hex = target.to_hex();
        if target == self.keys.public_key() {
            return Err(invalid_with_code(
                "invalid_remote_target",
                "remote job target must differ from the current identity",
            ));
        }
        let members = self.require_channel_membership(channel_id).await?;
        if !members.contains(&target_hex) {
            return Err(invalid_with_code(
                "target_not_channel_member",
                "remote job target must be a current channel member",
            ));
        }
        if self
            .managed_agents(HashSet::from([target_hex.clone()]))
            .await?
            .is_empty()
        {
            return Err(invalid_with_code(
                "target_not_managed_agent",
                "remote job target must be a managed agent",
            ));
        }
        if let Some(source) = source_event_id.as_ref() {
            let event = self.fetch_event(source).await?;
            ensure_event_channel(&event, channel_id)?;
            ensure_message_kind(&event)?;
        }
        let job_id = Uuid::new_v4();
        let request = AgentJobRequest {
            schema: AGENT_JOB_SCHEMA,
            driver: "lh".to_owned(),
            argv,
            cwd,
            summary,
        };
        let builder = buzz_sdk::build_agent_job_request(
            channel_id,
            target,
            job_id,
            source_event_id,
            None,
            &request,
        )
        .map_err(|_| invalid_with_code("invalid_job_request", "remote job request is invalid"))?;
        let event = self.sign(builder)?;
        let event_id = event.id.to_hex();
        self.relay
            .publish(event)
            .await
            .map_err(|_| relay_error("job_request_publish_failed"))?;
        bounded_json(&RemoteJobOutput {
            job_id,
            event_id,
            state: "requested",
        })
    }
    pub(crate) async fn messages_send(
        &self,
        params: MessagesSendParams,
    ) -> Result<String, ErrorData> {
        let channel_id = parse_channel_id(&params.channel_id)?;
        validate_content(&params.content)?;
        let mentions = parse_mentions(params.mentions)?;
        let members = self.require_channel_membership(channel_id).await?;
        let missing = mentions
            .iter()
            .filter(|pubkey| !members.contains(*pubkey))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(invalid_with_code(
                "mention_not_channel_member",
                "every mentioned identity must be a current channel member",
            ));
        }

        let thread_ref = match params.reply_to {
            Some(parent) => Some(self.resolve_thread_ref(channel_id, &parent).await?),
            None => None,
        };
        let mention_refs = mentions.iter().map(String::as_str).collect::<Vec<_>>();
        let builder = buzz_sdk::build_message(
            channel_id,
            &params.content,
            thread_ref.as_ref(),
            &mention_refs,
            false,
            &[],
        )
        .map_err(|_| invalid_with_code("invalid_message", "message could not be built"))?;
        let event = self.sign(builder)?;
        let event_id = event.id.to_hex();
        self.relay
            .publish(event)
            .await
            .map_err(|_| relay_error("message_publish_failed"))?;
        bounded_json(&SendOutput { event_id })
    }

    pub(crate) async fn messages_get(
        &self,
        params: MessagesGetParams,
    ) -> Result<String, ErrorData> {
        let channel_id = parse_channel_id(&params.channel_id)?;
        self.require_channel_membership(channel_id).await?;
        let limit = bounded_limit(params.limit, MAX_MESSAGE_LIMIT, 50);
        let anchor = match params.since {
            Some(event_id) => Some(self.fetch_scoped_message(channel_id, &event_id).await?),
            None => None,
        };
        let mut filter = json!({
            "kinds": MESSAGE_KINDS,
            "#h": [channel_id.to_string()],
            "limit": limit,
        });
        if let Some(anchor) = &anchor {
            filter["since"] = json!(anchor.created_at.as_secs());
        }
        let events = self
            .relay
            .query(filter)
            .await
            .map_err(|_| relay_error("message_query_failed"))?;
        let anchor_key = anchor
            .as_ref()
            .map(|event| (event.created_at.as_secs(), event.id.to_hex()));
        let messages = scoped_messages(events, channel_id, limit as usize, anchor_key.as_ref())?;
        bounded_json(&messages)
    }

    pub(crate) async fn messages_thread(
        &self,
        params: MessagesThreadParams,
    ) -> Result<String, ErrorData> {
        let requested = parse_event_id(&params.root_event_id, "root_event_id")?;
        let requested_event = self.fetch_event(&requested).await?;
        let channel_id = event_channel(&requested_event)?;
        self.require_channel_membership(channel_id).await?;
        ensure_message_kind(&requested_event)?;
        let anchors = parse_nip10_anchors(&requested_event)?;
        let root_id = anchors.root.or(anchors.reply).unwrap_or(requested);
        let root = if root_id == requested_event.id {
            requested_event
        } else {
            let event = self.fetch_event(&root_id).await?;
            ensure_event_channel(&event, channel_id)?;
            ensure_message_kind(&event)?;
            event
        };
        let limit = bounded_limit(params.limit, MAX_THREAD_LIMIT, 100);
        let replies = self
            .relay
            .query(json!({
                "kinds": MESSAGE_KINDS,
                "#h": [channel_id.to_string()],
                "#e": [root_id.to_hex()],
                "limit": limit,
            }))
            .await
            .map_err(|_| relay_error("thread_query_failed"))?;
        let mut values =
            vec![serde_json::to_value(root).map_err(|_| relay_error("thread_encode_failed"))?];
        values.extend(replies);
        let messages = scoped_messages(values, channel_id, limit as usize, None)?;
        bounded_json(&messages)
    }

    pub(crate) async fn messages_search(
        &self,
        params: MessagesSearchParams,
    ) -> Result<String, ErrorData> {
        validate_query(&params.query)?;
        let limit = bounded_limit(params.limit, MAX_MESSAGE_LIMIT, 20);
        let channels = match params.channel_id {
            Some(channel) => {
                let id = parse_channel_id(&channel)?;
                self.require_channel_membership(id).await?;
                vec![id]
            }
            None => self.accessible_channels().await?,
        };
        if channels.is_empty() {
            return Ok("[]".to_owned());
        }
        let allowed = channels.iter().copied().collect::<HashSet<_>>();
        let events = self
            .relay
            .query(json!({
                "kinds": MESSAGE_KINDS,
                "#h": channels.iter().map(Uuid::to_string).collect::<Vec<_>>(),
                "search": params.query,
                "limit": limit,
            }))
            .await
            .map_err(|_| relay_error("message_search_failed"))?;
        let mut messages = Vec::new();
        for value in events.into_iter().take(limit as usize) {
            let event = parse_event(value)?;
            let channel = event_channel(&event)?;
            if !allowed.contains(&channel) {
                return Err(relay_error("relay_scope_violation"));
            }
            messages.push(to_message(event, channel)?);
        }
        bounded_json(&messages)
    }

    pub(crate) async fn agents_list(&self, params: AgentsListParams) -> Result<String, ErrorData> {
        let members = match params.channel_id {
            Some(channel) => {
                let channel = parse_channel_id(&channel)?;
                self.require_channel_membership(channel).await?
            }
            None => self.accessible_members().await?,
        };
        let agents = self.managed_agents(members).await?;
        bounded_json(&agents)
    }

    pub(crate) async fn agents_status(
        &self,
        params: AgentsStatusParams,
    ) -> Result<String, ErrorData> {
        let agent = parse_pubkey(&params.agent, "agent")?;
        let members = self.accessible_members().await?;
        if !members.contains(&agent) {
            return Err(invalid_with_code(
                "agent_outside_shared_channels",
                "agent must share a current channel with this identity",
            ));
        }
        let mut summaries = self.managed_agents(HashSet::from([agent.clone()])).await?;
        let Some(summary) = summaries.pop() else {
            return Err(invalid_with_code(
                "not_managed_agent",
                "identity is not a managed agent",
            ));
        };
        bounded_json(&summary)
    }

    async fn require_channel_membership(
        &self,
        channel_id: Uuid,
    ) -> Result<HashSet<String>, ErrorData> {
        let events = self
            .relay
            .query(json!({
                "kinds": [39002],
                "#d": [channel_id.to_string()],
                "limit": 1,
            }))
            .await
            .map_err(|_| relay_error("membership_query_failed"))?;
        let event = events.into_iter().next().ok_or_else(|| {
            invalid_with_code(
                "channel_membership_missing",
                "channel membership is unavailable",
            )
        })?;
        let (event_channel_id, members) = parse_membership(&event)?;
        if event_channel_id != channel_id {
            return Err(relay_error("relay_scope_violation"));
        }
        if !members.contains(&self.keys.public_key().to_hex()) {
            return Err(invalid_with_code(
                "not_channel_member",
                "current identity is not a channel member",
            ));
        }
        Ok(members)
    }

    async fn accessible_channels(&self) -> Result<Vec<Uuid>, ErrorData> {
        let self_pubkey = self.keys.public_key().to_hex();
        let events = self
            .relay
            .query(json!({
                "kinds": [39002],
                "#p": [self_pubkey],
                "limit": MAX_CHANNEL_SCOPE,
            }))
            .await
            .map_err(|_| relay_error("membership_query_failed"))?;
        let mut channels = BTreeSet::new();
        for value in events.into_iter().take(MAX_CHANNEL_SCOPE) {
            let (channel, members) = parse_membership(&value)?;
            if members.contains(&self.keys.public_key().to_hex()) {
                channels.insert(channel);
            }
        }
        Ok(channels.into_iter().collect())
    }

    async fn accessible_members(&self) -> Result<HashSet<String>, ErrorData> {
        let channels = self.accessible_channels().await?;
        let mut members = HashSet::new();
        for channel in channels {
            members.extend(self.require_channel_membership(channel).await?);
            if members.len() > MAX_AGENT_LIMIT * MAX_CHANNEL_SCOPE {
                return Err(relay_error("membership_scope_too_large"));
            }
        }
        Ok(members)
    }

    async fn managed_agents(
        &self,
        members: HashSet<String>,
    ) -> Result<Vec<AgentSummary>, ErrorData> {
        if members.is_empty() {
            return Ok(Vec::new());
        }
        let candidates = members
            .into_iter()
            .take(MAX_AGENT_LIMIT)
            .collect::<Vec<_>>();
        let candidate_set = candidates.iter().cloned().collect::<HashSet<_>>();
        let definitions = self
            .relay
            .query(json!({
                "kinds": [KIND_MANAGED_AGENT],
                "#d": candidates,
                "limit": MAX_AGENT_LIMIT,
            }))
            .await
            .map_err(|_| relay_error("agent_query_failed"))?;
        let mut names = BTreeMap::new();
        for value in definitions {
            let event = parse_event(value)?;
            if event.kind.as_u16() as u32 != KIND_MANAGED_AGENT {
                return Err(relay_error("invalid_managed_agent_definition"));
            }
            let d_tags = event
                .tags
                .iter()
                .filter_map(|tag| {
                    let parts = tag.as_slice();
                    (parts.first().map(String::as_str) == Some("d"))
                        .then(|| parts.get(1).cloned())
                        .flatten()
                })
                .collect::<Vec<_>>();
            if d_tags.len() != 1
                || !is_lower_hex64(&d_tags[0])
                || !candidate_set.contains(&d_tags[0])
            {
                return Err(relay_error("invalid_managed_agent_definition"));
            }
            let name = serde_json::from_str::<Value>(&event.content)
                .ok()
                .and_then(|content| {
                    content
                        .get("name")
                        .or_else(|| content.get("display_name"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                });
            if names.insert(d_tags[0].clone(), name).is_some() {
                return Err(relay_error("ambiguous_managed_agent_definition"));
            }
        }
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let pubkeys = names.keys().cloned().collect::<Vec<_>>();
        let details = self
            .relay
            .query(json!({
                "kinds": [0, KIND_USER_STATUS, KIND_PRESENCE_SNAPSHOT],
                "authors": pubkeys,
                "limit": MAX_AGENT_LIMIT * 3,
            }))
            .await
            .map_err(|_| relay_error("agent_status_query_failed"))?;
        let mut summaries = names
            .into_iter()
            .map(|(pubkey, name)| {
                (
                    pubkey.clone(),
                    AgentSummary {
                        pubkey,
                        name,
                        presence: None,
                        work_status: None,
                        updated_at: None,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        for value in details {
            let event = parse_event(value)?;
            let subject = if event.kind.as_u16() as u32 == KIND_PRESENCE_SNAPSHOT {
                tag_from_event(&event, "p").unwrap_or_else(|| event.pubkey.to_hex())
            } else {
                event.pubkey.to_hex()
            };
            let Some(summary) = summaries.get_mut(&subject) else {
                continue;
            };
            let created_at = event.created_at.as_secs();
            match event.kind.as_u16() as u32 {
                0 => {
                    let profile = serde_json::from_str::<Value>(&event.content).ok();
                    let name = profile.as_ref().and_then(|value| {
                        value
                            .get("display_name")
                            .or_else(|| value.get("name"))
                            .and_then(Value::as_str)
                    });
                    if let Some(name) = name {
                        summary.name = Some(name.to_owned());
                    }
                }
                KIND_USER_STATUS => {
                    summary.work_status = Some(event.content);
                    summary.updated_at = Some(summary.updated_at.unwrap_or(0).max(created_at));
                }
                KIND_PRESENCE_SNAPSHOT => {
                    summary.presence = Some(event.content);
                    summary.updated_at = Some(summary.updated_at.unwrap_or(0).max(created_at));
                }
                _ => {}
            }
        }
        Ok(summaries.into_values().collect())
    }

    async fn fetch_scoped_message(
        &self,
        channel_id: Uuid,
        value: &str,
    ) -> Result<Event, ErrorData> {
        let event_id = parse_event_id(value, "since")?;
        let event = self.fetch_event(&event_id).await?;
        ensure_event_channel(&event, channel_id)?;
        ensure_message_kind(&event)?;
        Ok(event)
    }

    async fn fetch_event(&self, event_id: &EventId) -> Result<Event, ErrorData> {
        let events = self
            .relay
            .query(json!({"ids": [event_id.to_hex()], "limit": 1}))
            .await
            .map_err(|_| relay_error("event_query_failed"))?;
        let value = events.into_iter().next().ok_or_else(|| {
            invalid_with_code("event_not_found", "referenced event was not found")
        })?;
        let event = parse_event(value)?;
        if event.id != *event_id {
            return Err(relay_error("relay_scope_violation"));
        }
        Ok(event)
    }

    async fn resolve_thread_ref(
        &self,
        channel_id: Uuid,
        parent: &str,
    ) -> Result<ThreadRef, ErrorData> {
        let parent_id = parse_event_id(parent, "reply_to")?;
        let event = self.fetch_event(&parent_id).await?;
        ensure_event_channel(&event, channel_id)?;
        ensure_message_kind(&event)?;
        let anchors = parse_nip10_anchors(&event)?;
        let root_event_id = anchors.root.or(anchors.reply).unwrap_or(parent_id);
        Ok(ThreadRef {
            root_event_id,
            parent_event_id: parent_id,
        })
    }

    fn sign(&self, builder: EventBuilder) -> Result<Event, ErrorData> {
        let builder = match &self.auth_tag {
            Some(tag) => builder.tags([tag.clone()]),
            None => builder,
        };
        builder
            .sign_with_keys(&self.keys)
            .map_err(|_| relay_error("event_signing_failed"))
    }
}

impl RelayTransport for HttpRelay {
    fn query<'a>(&'a self, filter: Value) -> RelayFuture<'a, Vec<Value>> {
        Box::pin(async move {
            let url = format!("{}/query", self.relay_url);
            let body = serde_json::to_vec(&[filter]).map_err(|_| RelayError)?;
            let response = self.authorized_post(&url, &body).await?;
            serde_json::from_slice(&response).map_err(|_| RelayError)
        })
    }

    fn publish<'a>(&'a self, event: Event) -> RelayFuture<'a, ()> {
        Box::pin(async move {
            let url = format!("{}/events", self.relay_url);
            let body = serde_json::to_vec(&event).map_err(|_| RelayError)?;
            self.authorized_post(&url, &body).await?;
            Ok(())
        })
    }
}

impl HttpRelay {
    async fn authorized_post(&self, url: &str, body: &[u8]) -> Result<Vec<u8>, RelayError> {
        let authorization = sign_nip98(&self.keys, url, body)?;
        let mut request = self
            .http
            .post(url)
            .header("Authorization", authorization)
            .header("Content-Type", "application/json")
            .body(body.to_vec());
        if let Some(auth_tag) = &self.auth_tag_json {
            request = request.header("x-auth-tag", auth_tag);
        }
        let response = request.send().await.map_err(|_| RelayError)?;
        if !response.status().is_success() {
            return Err(RelayError);
        }
        let bytes = response.bytes().await.map_err(|_| RelayError)?;
        if bytes.len() > MAX_OUTPUT_BYTES {
            return Err(RelayError);
        }
        Ok(bytes.to_vec())
    }
}

fn sign_nip98(keys: &Keys, url: &str, body: &[u8]) -> Result<String, RelayError> {
    let payload = hex::encode(Sha256::digest(body));
    let tags = vec![
        Tag::parse(["u", url]).map_err(|_| RelayError)?,
        Tag::parse(["method", "POST"]).map_err(|_| RelayError)?,
        Tag::parse(["payload", payload.as_str()]).map_err(|_| RelayError)?,
        Tag::parse(["nonce", Uuid::new_v4().to_string().as_str()]).map_err(|_| RelayError)?,
    ];
    let event = EventBuilder::new(Kind::Custom(27235), "")
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|_| RelayError)?;
    Ok(format!(
        "Nostr {}",
        BASE64.encode(event.as_json().as_bytes())
    ))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessagesSendParams {
    pub channel_id: String,
    pub content: String,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub mentions: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessagesGetParams {
    pub channel_id: String,
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub limit: Option<u16>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessagesThreadParams {
    pub root_event_id: String,
    #[serde(default)]
    pub limit: Option<u16>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessagesSearchParams {
    pub query: String,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub limit: Option<u16>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentsListParams {
    #[serde(default)]
    pub channel_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentsStatusParams {
    pub agent: String,
}

#[derive(Debug, Serialize)]
struct SendOutput {
    event_id: String,
}

#[derive(Debug, Serialize)]
struct RemoteJobOutput {
    job_id: Uuid,
    event_id: String,
    state: &'static str,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Message {
    event_id: String,
    channel_id: String,
    sender_pubkey: String,
    created_at: u64,
    content: String,
    root_event_id: Option<String>,
    reply_to: Option<String>,
    mentions: Vec<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct AgentSummary {
    pubkey: String,
    name: Option<String>,
    presence: Option<String>,
    work_status: Option<String>,
    updated_at: Option<u64>,
}

#[derive(Default)]
struct Nip10Anchors {
    root: Option<EventId>,
    reply: Option<EventId>,
}

fn parse_nip10_anchors(event: &Event) -> Result<Nip10Anchors, ErrorData> {
    let mut anchors = Nip10Anchors::default();
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) != Some("e") {
            continue;
        }
        if parts.len() < 4 {
            return Err(invalid_with_code(
                "malformed_thread_anchor",
                "thread event contains an unmarked e tag",
            ));
        }
        let id = EventId::from_hex(&parts[1]).map_err(|_| {
            invalid_with_code(
                "malformed_thread_anchor",
                "thread anchor is not an event ID",
            )
        })?;
        match parts[3].as_str() {
            "root" if anchors.root.replace(id).is_none() => {}
            "reply" if anchors.reply.replace(id).is_none() => {}
            "root" | "reply" => {
                return Err(invalid_with_code(
                    "malformed_thread_anchor",
                    "thread event contains duplicate NIP-10 anchors",
                ));
            }
            _ => {
                return Err(invalid_with_code(
                    "malformed_thread_anchor",
                    "thread e tag has an invalid NIP-10 marker",
                ));
            }
        }
    }
    if anchors.root.is_some() && anchors.reply.is_none() {
        return Err(invalid_with_code(
            "malformed_thread_anchor",
            "thread root anchor requires a reply anchor",
        ));
    }
    if anchors.root == anchors.reply && anchors.root.is_some() {
        return Err(invalid_with_code(
            "malformed_thread_anchor",
            "nested thread root and reply anchors must differ",
        ));
    }
    Ok(anchors)
}

fn scoped_messages(
    values: Vec<Value>,
    channel_id: Uuid,
    limit: usize,
    anchor: Option<&(u64, String)>,
) -> Result<Vec<Message>, ErrorData> {
    let mut messages = BTreeMap::new();
    for value in values {
        let event = parse_event(value)?;
        ensure_event_channel(&event, channel_id)?;
        ensure_message_kind(&event)?;
        let key = (event.created_at.as_secs(), event.id.to_hex());
        if anchor.is_some_and(|anchor| key <= *anchor) {
            continue;
        }
        messages.insert(key, to_message(event, channel_id)?);
    }
    Ok(messages.into_values().take(limit).collect())
}

fn to_message(event: Event, channel_id: Uuid) -> Result<Message, ErrorData> {
    if event.content.len() > MAX_CONTENT_BYTES {
        return Err(relay_error("message_content_too_large"));
    }
    let anchors = parse_nip10_anchors(&event)?;
    let mut mentions = event
        .tags
        .iter()
        .filter_map(|tag| {
            let parts = tag.as_slice();
            (parts.first().map(String::as_str) == Some("p"))
                .then(|| parts.get(1).cloned())
                .flatten()
        })
        .filter(|pubkey| is_lower_hex64(pubkey))
        .collect::<Vec<_>>();
    mentions.sort();
    mentions.dedup();
    if mentions.len() > MENTION_CAP {
        return Err(relay_error("message_mentions_too_large"));
    }
    Ok(Message {
        event_id: event.id.to_hex(),
        channel_id: channel_id.to_string(),
        sender_pubkey: event.pubkey.to_hex(),
        created_at: event.created_at.as_secs(),
        content: event.content,
        root_event_id: anchors.root.map(|id| id.to_hex()),
        reply_to: anchors.reply.map(|id| id.to_hex()),
        mentions,
    })
}

fn parse_event(value: Value) -> Result<Event, ErrorData> {
    let event: Event =
        serde_json::from_value(value).map_err(|_| relay_error("invalid_relay_event"))?;
    event
        .verify()
        .map_err(|_| relay_error("invalid_relay_event_signature"))?;
    Ok(event)
}

fn parse_membership(value: &Value) -> Result<(Uuid, HashSet<String>), ErrorData> {
    let event = parse_event(value.clone())?;
    if event.kind.as_u16() != 39002 {
        return Err(relay_error("invalid_membership_snapshot"));
    }
    let channels = event
        .tags
        .iter()
        .filter_map(|tag| {
            let parts = tag.as_slice();
            (parts.first().map(String::as_str) == Some("d"))
                .then(|| parts.get(1).cloned())
                .flatten()
        })
        .collect::<Vec<_>>();
    if channels.len() != 1 {
        return Err(relay_error("invalid_membership_snapshot"));
    }
    let channel =
        Uuid::parse_str(&channels[0]).map_err(|_| relay_error("invalid_membership_snapshot"))?;
    let members = event
        .tags
        .iter()
        .filter_map(|tag| {
            let parts = tag.as_slice();
            (parts.first().map(String::as_str) == Some("p"))
                .then(|| parts.get(1).cloned())
                .flatten()
        })
        .filter(|pubkey| is_lower_hex64(pubkey))
        .collect::<HashSet<_>>();
    Ok((channel, members))
}

fn event_channel(event: &Event) -> Result<Uuid, ErrorData> {
    let values = event
        .tags
        .iter()
        .filter_map(|tag| {
            let parts = tag.as_slice();
            (parts.first().map(String::as_str) == Some("h"))
                .then(|| parts.get(1).cloned())
                .flatten()
        })
        .collect::<Vec<_>>();
    if values.len() != 1 {
        return Err(relay_error("invalid_channel_scope"));
    }
    Uuid::parse_str(&values[0]).map_err(|_| relay_error("invalid_channel_scope"))
}

fn ensure_event_channel(event: &Event, expected: Uuid) -> Result<(), ErrorData> {
    if event_channel(event)? != expected {
        return Err(invalid_with_code(
            "event_outside_channel",
            "referenced event is outside the requested channel",
        ));
    }
    Ok(())
}

fn ensure_message_kind(event: &Event) -> Result<(), ErrorData> {
    if MESSAGE_KINDS.contains(&(event.kind.as_u16() as u32)) {
        Ok(())
    } else {
        Err(invalid_with_code(
            "not_message_event",
            "referenced event is not a supported message",
        ))
    }
}

fn tag_from_event(event: &Event, name: &str) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        (parts.first().map(String::as_str) == Some(name))
            .then(|| parts.get(1).cloned())
            .flatten()
    })
}

fn parse_channel_id(value: &str) -> Result<Uuid, ErrorData> {
    let channel = Uuid::parse_str(value)
        .map_err(|_| invalid_with_code("invalid_channel_id", "channel_id must be a UUID"))?;
    if channel.is_nil() {
        return Err(invalid_with_code(
            "invalid_channel_id",
            "channel_id must not be nil",
        ));
    }
    Ok(channel)
}

fn parse_event_id(value: &str, field: &str) -> Result<EventId, ErrorData> {
    if !is_lower_hex64(value) {
        return Err(invalid_with_code(
            "invalid_event_id",
            &format!("{field} must be 64 lowercase hex characters"),
        ));
    }
    EventId::from_hex(value)
        .map_err(|_| invalid_with_code("invalid_event_id", "event ID is invalid"))
}

fn parse_pubkey(value: &str, field: &str) -> Result<String, ErrorData> {
    PublicKey::parse(value)
        .map(|pubkey| pubkey.to_hex())
        .map_err(|_| {
            invalid_with_code(
                "invalid_pubkey",
                &format!("{field} must be a pubkey or npub"),
            )
        })
}

fn parse_mentions(values: Vec<String>) -> Result<Vec<String>, ErrorData> {
    if values.len() > MENTION_CAP {
        return Err(invalid_with_code(
            "too_many_mentions",
            "mentions exceeds the maximum of 50",
        ));
    }
    let mut mentions = BTreeSet::new();
    for value in values {
        mentions.insert(parse_pubkey(&value, "mention")?);
    }
    if mentions.len() > MENTION_CAP {
        return Err(invalid_with_code(
            "too_many_mentions",
            "mentions exceeds the maximum of 50",
        ));
    }
    Ok(mentions.into_iter().collect())
}

fn validate_content(content: &str) -> Result<(), ErrorData> {
    if content.is_empty() || content.len() > MAX_CONTENT_BYTES {
        return Err(invalid_with_code(
            "invalid_message_content",
            "content must be non-empty and at most 64 KiB",
        ));
    }
    Ok(())
}

fn validate_query(query: &str) -> Result<(), ErrorData> {
    if query.trim().is_empty() || query.len() > MAX_QUERY_BYTES {
        return Err(invalid_with_code(
            "invalid_search_query",
            "query must be non-empty and at most 512 bytes",
        ));
    }
    Ok(())
}

#[cfg(test)]
struct UnavailableRelay;

#[cfg(test)]
impl RelayTransport for UnavailableRelay {
    fn query<'a>(&'a self, _filter: Value) -> RelayFuture<'a, Vec<Value>> {
        Box::pin(async { Err(RelayError) })
    }

    fn publish<'a>(&'a self, _event: Event) -> RelayFuture<'a, ()> {
        Box::pin(async { Err(RelayError) })
    }
}

fn bounded_limit(value: Option<u16>, max: u16, default: u16) -> u16 {
    value.unwrap_or(default).clamp(1, max)
}

fn bounded_json<T: Serialize>(value: &T) -> Result<String, ErrorData> {
    let bytes = serde_json::to_vec(value).map_err(|_| relay_error("response_encode_failed"))?;
    if bytes.len() > MAX_OUTPUT_BYTES {
        return Err(relay_error("managed_response_too_large"));
    }
    String::from_utf8(bytes).map_err(|_| relay_error("response_encode_failed"))
}

fn is_lower_hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn normalize_relay_url(url: &str) -> String {
    url.replacen("wss://", "https://", 1)
        .replacen("ws://", "http://", 1)
        .trim_end_matches('/')
        .to_owned()
}

fn managed_auth_error() -> ErrorData {
    ErrorData::invalid_request("managed collaboration identity is unavailable", None)
}

fn invalid_with_code(code: &str, message: &str) -> ErrorData {
    ErrorData::invalid_params(message.to_owned(), Some(json!({"code": code})))
}

fn relay_error(code: &str) -> ErrorData {
    ErrorData::internal_error(
        "managed collaboration request failed",
        Some(json!({"code": code})),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    #[derive(Default)]
    struct MockRelay {
        responses: Mutex<Vec<Vec<Value>>>,
        queries: Mutex<Vec<Value>>,
        published: Mutex<Vec<Event>>,
    }

    impl MockRelay {
        fn with_responses(responses: Vec<Vec<Value>>) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(responses.into_iter().rev().collect()),
                ..Self::default()
            })
        }
    }

    fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    impl RelayTransport for MockRelay {
        fn query<'a>(&'a self, filter: Value) -> RelayFuture<'a, Vec<Value>> {
            Box::pin(async move {
                guard(&self.queries).push(filter);
                guard(&self.responses).pop().ok_or(RelayError)
            })
        }

        fn publish<'a>(&'a self, event: Event) -> RelayFuture<'a, ()> {
            Box::pin(async move {
                guard(&self.published).push(event);
                Ok(())
            })
        }
    }

    fn signed_value(keys: &Keys, kind: u16, content: &str, tags: Vec<Tag>) -> Value {
        let event = EventBuilder::new(Kind::Custom(kind), content)
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap();
        serde_json::to_value(event).unwrap()
    }

    fn tag(parts: &[&str]) -> Tag {
        Tag::parse(parts.iter().copied()).unwrap()
    }

    fn membership(keys: &Keys, channel: Uuid, members: &[String]) -> Value {
        let mut tags = vec![tag(&["d", &channel.to_string()])];
        tags.extend(members.iter().map(|member| tag(&["p", member])));
        signed_value(keys, 39002, "", tags)
    }

    #[tokio::test]
    async fn remote_job_is_signed_public_request_not_local_spawn() {
        let requester = Keys::generate();
        let target = Keys::generate();
        let owner = Keys::generate();
        let channel = Uuid::from_u128(10);
        let target_hex = target.public_key().to_hex();
        let relay = MockRelay::with_responses(vec![
            vec![membership(
                &owner,
                channel,
                &[requester.public_key().to_hex(), target_hex.clone()],
            )],
            vec![signed_value(
                &owner,
                KIND_MANAGED_AGENT as u16,
                r#"{"name":"coworker"}"#,
                vec![tag(&["d", &target_hex])],
            )],
            vec![],
        ]);
        let client = CollaborationClient::for_test(requester.clone(), relay.clone());
        let output = client
            .jobs_request_remote(
                channel,
                target.public_key(),
                None,
                vec!["lockdown".into(), "run".into()],
                "/tmp/workspace".into(),
                "governed review".into(),
            )
            .await
            .unwrap();
        let published = guard(&relay.published);
        assert_eq!(published.len(), 1);
        let event = &published[0];
        assert_eq!(event.pubkey, requester.public_key());
        assert_eq!(
            event.kind.as_u16() as u32,
            buzz_core::kind::KIND_JOB_REQUEST
        );
        event.verify().unwrap();
        assert_eq!(tag_from_event(event, "h"), Some(channel.to_string()));
        assert_eq!(tag_from_event(event, "p"), Some(target_hex));
        let payload: AgentJobRequest = serde_json::from_str(&event.content).unwrap();
        assert_eq!(payload.driver, "lh");
        assert_eq!(payload.argv, vec!["lockdown", "run"]);
        let output: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(output["event_id"], event.id.to_hex());
        assert_eq!(output["state"], "requested");
    }

    #[tokio::test]
    async fn remote_job_rejects_nonmember_before_publish() {
        let requester = Keys::generate();
        let target = Keys::generate();
        let signer = Keys::generate();
        let channel = Uuid::from_u128(11);
        let relay = MockRelay::with_responses(vec![vec![membership(
            &signer,
            channel,
            &[requester.public_key().to_hex()],
        )]]);
        let client = CollaborationClient::for_test(requester, relay.clone());
        assert!(client
            .jobs_request_remote(
                channel,
                target.public_key(),
                None,
                vec![],
                "/tmp/workspace".into(),
                "governed review".into(),
            )
            .await
            .is_err());
        assert_eq!(guard(&relay.queries).len(), 1);
        assert!(guard(&relay.published).is_empty());
    }

    #[tokio::test]
    async fn send_signs_with_managed_identity_and_preserves_nip10_anchors() {
        let agent = Keys::generate();
        let relay_signer = Keys::generate();
        let channel = Uuid::from_u128(1);
        let parent = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "parent")
            .tags([tag(&["h", &channel.to_string()])])
            .sign_with_keys(&relay_signer)
            .unwrap();
        let responses = vec![
            vec![membership(
                &relay_signer,
                channel,
                &[agent.public_key().to_hex()],
            )],
            vec![serde_json::to_value(&parent).unwrap()],
        ];
        let relay = MockRelay::with_responses(responses);
        let client = CollaborationClient::for_test(agent.clone(), relay.clone());
        let output = client
            .messages_send(MessagesSendParams {
                channel_id: channel.to_string(),
                content: "reply".into(),
                reply_to: Some(parent.id.to_hex()),
                mentions: vec![],
            })
            .await
            .unwrap();
        let published = guard(&relay.published);
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].pubkey, agent.public_key());
        published[0].verify().unwrap();
        let anchors = parse_nip10_anchors(&published[0]).unwrap();
        assert_eq!(anchors.reply, Some(parent.id));
        assert!(output.contains(&published[0].id.to_hex()));
    }

    #[tokio::test]
    async fn membership_mentions_and_bounds_fail_before_publish() {
        let agent = Keys::generate();
        let outsider = Keys::generate().public_key().to_hex();
        let relay_signer = Keys::generate();
        let channel = Uuid::from_u128(2);
        let relay = MockRelay::with_responses(vec![vec![membership(
            &relay_signer,
            channel,
            &[agent.public_key().to_hex()],
        )]]);
        let client = CollaborationClient::for_test(agent, relay.clone());
        assert!(client
            .messages_send(MessagesSendParams {
                channel_id: channel.to_string(),
                content: "hello".into(),
                reply_to: None,
                mentions: vec![outsider],
            })
            .await
            .is_err());
        assert!(guard(&relay.published).is_empty());

        let relay = MockRelay::with_responses(vec![]);
        let client = CollaborationClient::for_test(Keys::generate(), relay.clone());
        assert!(client
            .messages_send(MessagesSendParams {
                channel_id: channel.to_string(),
                content: "x".repeat(MAX_CONTENT_BYTES + 1),
                reply_to: None,
                mentions: vec![],
            })
            .await
            .is_err());
        assert!(guard(&relay.queries).is_empty());
        assert!(guard(&relay.published).is_empty());

        let relay = MockRelay::with_responses(vec![]);
        let client = CollaborationClient::for_test(Keys::generate(), relay.clone());
        let mentions = (0..=MENTION_CAP)
            .map(|_| Keys::generate().public_key().to_hex())
            .collect();
        assert!(client
            .messages_send(MessagesSendParams {
                channel_id: channel.to_string(),
                content: "hello".into(),
                reply_to: None,
                mentions,
            })
            .await
            .is_err());
        assert!(guard(&relay.queries).is_empty());
        assert!(guard(&relay.published).is_empty());
    }

    #[tokio::test]
    async fn malformed_parent_anchor_fails_before_publish() {
        let agent = Keys::generate();
        let relay_signer = Keys::generate();
        let channel = Uuid::from_u128(3);
        let malformed = signed_value(
            &relay_signer,
            KIND_STREAM_MESSAGE as u16,
            "bad",
            vec![
                tag(&["h", &channel.to_string()]),
                tag(&["e", &"a".repeat(64)]),
            ],
        );
        let event_id = malformed["id"].as_str().unwrap().to_owned();
        let relay = MockRelay::with_responses(vec![
            vec![membership(
                &relay_signer,
                channel,
                &[agent.public_key().to_hex()],
            )],
            vec![malformed],
        ]);
        let client = CollaborationClient::for_test(agent, relay.clone());
        assert!(client
            .messages_send(MessagesSendParams {
                channel_id: channel.to_string(),
                content: "reply".into(),
                reply_to: Some(event_id),
                mentions: vec![],
            })
            .await
            .is_err());
        assert!(guard(&relay.published).is_empty());
    }

    #[tokio::test]
    async fn get_thread_and_search_return_signed_scoped_messages() {
        let agent = Keys::generate();
        let signer = Keys::generate();
        let channel = Uuid::from_u128(4);
        let root = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "root")
            .tags([tag(&["h", &channel.to_string()])])
            .sign_with_keys(&signer)
            .unwrap();
        let reply = buzz_sdk::build_message(
            channel,
            "reply",
            Some(&ThreadRef {
                root_event_id: root.id,
                parent_event_id: root.id,
            }),
            &[],
            false,
            &[],
        )
        .unwrap()
        .sign_with_keys(&signer)
        .unwrap();
        let member = membership(&signer, channel, &[agent.public_key().to_hex()]);
        let relay = MockRelay::with_responses(vec![
            vec![serde_json::to_value(&root).unwrap()],
            vec![member.clone()],
            vec![serde_json::to_value(&reply).unwrap()],
            vec![member.clone()],
            vec![serde_json::to_value(&reply).unwrap()],
            vec![member],
            vec![serde_json::to_value(&reply).unwrap()],
        ]);
        let client = CollaborationClient::for_test(agent, relay);
        let thread = client
            .messages_thread(MessagesThreadParams {
                root_event_id: root.id.to_hex(),
                limit: None,
            })
            .await
            .unwrap();
        let thread: Vec<Message> = serde_json::from_str(&thread).unwrap();
        assert_eq!(thread.len(), 2);
        let search = client
            .messages_search(MessagesSearchParams {
                query: "reply".into(),
                channel_id: Some(channel.to_string()),
                limit: None,
            })
            .await
            .unwrap();
        let search: Vec<Message> = serde_json::from_str(&search).unwrap();
        assert_eq!(search[0].event_id, reply.id.to_hex());
        let messages = client
            .messages_get(MessagesGetParams {
                channel_id: channel.to_string(),
                since: None,
                limit: None,
            })
            .await
            .unwrap();
        let messages: Vec<Message> = serde_json::from_str(&messages).unwrap();
        assert_eq!(messages[0].event_id, reply.id.to_hex());
    }

    #[test]
    fn collaboration_has_no_process_or_cli_path() {
        let source = include_str!("collaboration.rs");
        let forbidden = [
            ["std::", "process"].concat(),
            ["Command", "::new"].concat(),
            ["buzz_", "cli::"].concat(),
            [" /", "bin/"].concat(),
            ["sh", "ell("].concat(),
        ];
        for forbidden in forbidden {
            assert!(
                !source.contains(&forbidden),
                "found forbidden path: {forbidden}"
            );
        }
    }
}
