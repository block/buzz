use std::{collections::BTreeSet, time::Duration};

use buzz_core::kind::{
    KIND_NIP29_GROUP_MEMBERS, KIND_NIP29_GROUP_METADATA, KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
};
use buzz_ifc::{
    derive_execution_domain, AuthorizedPublication, CapabilityPolicy, CapabilitySet, CommunityId,
    ConversationKind, DomainFacts, ExecutionDomain, MembershipEpoch, OperationEffect, Principal,
};
use nostr::{Event, EventId, Keys, Tag};
use serde_json::{json, Value};
use uuid::Uuid;

use super::{denied, Scope, READ, REPLY};
use crate::{client::sign_nip98, error::CliError};

const RELAY_BYTES: usize = 1024 * 1024;

pub(super) struct Relay {
    http: reqwest::Client,
    url: String,
    keys: Keys,
    auth: Option<Tag>,
}

impl Relay {
    pub(super) fn new(url: String, keys: Keys, auth: Option<Tag>) -> Result<Self, CliError> {
        let parsed = url::Url::parse(&url).map_err(|_| denied("invalid relay URL"))?;
        let loopback = parsed.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if !(parsed.scheme() == "https" || parsed.scheme() == "http" && loopback)
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.path() != "/"
        {
            return Err(denied(
                "relay must be an HTTPS origin (HTTP is allowed only on loopback)",
            ));
        }
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| denied("cannot create relay client"))?;
        Ok(Self {
            http,
            url: url.trim_end_matches('/').to_owned(),
            keys,
            auth,
        })
    }

    async fn post(&self, path: &str, bytes: Vec<u8>) -> Result<Value, CliError> {
        let url = format!("{}{path}", self.url);
        let auth = sign_nip98(&self.keys, "POST", &url, Some(&bytes))?;
        let mut request = self
            .http
            .post(url)
            .header("Authorization", auth)
            .header("Content-Type", "application/json")
            .body(bytes);
        if let Some(tag) = &self.auth {
            request = request.header(
                "x-auth-tag",
                serde_json::to_string(tag.as_slice())
                    .map_err(|_| denied("cannot encode owner attestation"))?,
            );
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| denied("relay request failed; publication outcome may be unknown"))?;
        if !response.status().is_success() {
            // Never forward an unscoped relay error body to the agent.
            return Err(denied(format!(
                "relay rejected request (HTTP {})",
                response.status().as_u16()
            )));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| denied("incomplete relay response; publication outcome may be unknown"))?
        {
            if chunk.len() > RELAY_BYTES.saturating_sub(body.len()) {
                return Err(denied("relay response is too large"));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|_| denied("invalid relay response"))
    }

    async fn query(&self, filter: Value) -> Result<Vec<Event>, CliError> {
        let body = serde_json::to_vec(&[filter]).map_err(|_| denied("cannot encode query"))?;
        serde_json::from_value(self.post("/query", body).await?)
            .map_err(|_| denied("invalid relay events"))
    }

    pub(super) async fn domain(&self, scope: &Scope) -> Result<ExecutionDomain, CliError> {
        let events = self
            .query(json!({
                "kinds": [KIND_NIP29_GROUP_METADATA, KIND_NIP29_GROUP_MEMBERS],
                "authors": [scope.relay_key.to_hex()], "#d": [scope.channel.to_string()], "limit": 2
            }))
            .await?;
        let mut metadata = None;
        let mut membership = None;
        for event in &events {
            if event.pubkey != scope.relay_key
                || event.verify().is_err()
                || !exact_tag(event, "d", &scope.channel.to_string())
            {
                return Err(denied("untrusted channel state"));
            }
            let slot = match u32::from(event.kind.as_u16()) {
                KIND_NIP29_GROUP_METADATA => &mut metadata,
                KIND_NIP29_GROUP_MEMBERS => &mut membership,
                _ => return Err(denied("unexpected channel state")),
            };
            if slot.replace(event).is_some() {
                return Err(denied("ambiguous channel state"));
            }
        }
        let (Some(metadata), Some(membership)) = (metadata, membership) else {
            return Err(denied("channel metadata or membership is missing"));
        };
        let public = flag(metadata, "public");
        if public == flag(metadata, "private")
            || !exact_tag(metadata, "t", "stream")
            || metadata
                .tags
                .iter()
                .any(|tag| tag.as_slice().first().is_some_and(|s| s == "archived"))
        {
            return Err(denied(
                "broker requires an active public or private stream channel",
            ));
        }
        let mut members = BTreeSet::new();
        for tag in membership
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().is_some_and(|s| s == "p"))
        {
            let key = tag
                .as_slice()
                .get(1)
                .ok_or_else(|| denied("invalid channel member"))?;
            members.insert(Principal::from_hex(key).map_err(|_| denied("invalid channel member"))?);
        }
        let agent = Principal::from_public_key(&self.keys.public_key())
            .map_err(|_| denied("invalid agent key"))?;
        let requester = Principal::from_public_key(&scope.requester)
            .map_err(|_| denied("invalid requester key"))?;
        if !members.contains(&agent) || !members.contains(&requester) {
            return Err(denied(
                "agent and requester must be current channel members",
            ));
        }
        let capabilities = CapabilitySet::from_operations([
            (READ, OperationEffect::NonEgressing),
            (REPLY, OperationEffect::Publication),
        ]);
        derive_execution_domain(
            DomainFacts {
                community: CommunityId::from_uuid(scope.community),
                channel_id: scope.channel,
                kind: if public {
                    ConversationKind::Public
                } else {
                    ConversationKind::Restricted
                },
                epoch: MembershipEpoch::new(format!("{}:{}", metadata.id, membership.id)),
                members,
                executing_agent: agent,
                requesters: BTreeSet::from([requester]),
                system_principal: None,
                owner: None,
            },
            &CapabilityPolicy::new(capabilities.clone(), capabilities),
        )
        .map_err(|_| denied("invalid execution domain"))
    }

    pub(super) async fn read(&self, channel: Uuid, limit: u32) -> Result<Vec<Event>, CliError> {
        let events = self
            .query(json!({
                "kinds": [KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_V2],
                "#h": [channel.to_string()], "limit": limit
            }))
            .await?;
        if events.len() > limit as usize
            || events.iter().any(|event| {
                !matches!(
                    u32::from(event.kind.as_u16()),
                    KIND_STREAM_MESSAGE | KIND_STREAM_MESSAGE_V2
                ) || !exact_tag(event, "h", &channel.to_string())
                    || event.verify().is_err()
            })
        {
            return Err(denied("relay returned invalid or out-of-channel messages"));
        }
        Ok(events)
    }

    pub(super) fn message(&self, channel: Uuid, content: &str) -> Result<Event, CliError> {
        let mut builder =
            buzz_sdk::builders::build_message(channel, content, None, &[], false, &[], &[])
                .map_err(|_| denied("invalid message"))?;
        if let Some(auth) = &self.auth {
            builder = builder.tags([auth.clone()]);
        }
        builder
            .sign_with_keys(&self.keys)
            .map_err(|_| denied("cannot sign reply"))
    }

    pub(super) async fn publish(
        &self,
        authorization: AuthorizedPublication,
        expected: EventId,
    ) -> Result<Value, CliError> {
        let (_, _, bytes) = authorization.into_parts();
        // Send the exact serialized event checked by IFC. No retries: a lost
        // acknowledgement must not silently turn into a second message.
        let response = self.post("/events", bytes).await?;
        if response.get("accepted").and_then(Value::as_bool) != Some(true)
            || response.get("event_id").and_then(Value::as_str) != Some(expected.to_hex().as_str())
        {
            return Err(denied("relay did not acknowledge the exact reply"));
        }
        Ok(json!({"event_id": expected, "accepted": true, "message": ""}))
    }
}

// Reject duplicate routing tags even when they agree. Different consumers
// must not be able to disagree about which channel a signed event names.
fn exact_tag(event: &Event, name: &str, value: &str) -> bool {
    let mut tags = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().is_some_and(|s| s == name));
    tags.next()
        .is_some_and(|tag| tag.as_slice() == [name, value])
        && tags.next().is_none()
}

fn flag(event: &Event, name: &str) -> bool {
    event.tags.iter().any(|tag| tag.as_slice() == [name])
}
