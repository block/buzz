//! Live Slack Connect ↔ Buzz message bridge.

use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use buzz_ws_client::{NostrWsConnection, RelayMessage, WsClientError};
use nostr::{Event, EventBuilder, EventId, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    config::{ChannelMapping, Config},
    slack::{
        escape_markdown_label, slack_mrkdwn_to_markdown, SlackClient, SlackDelivery, SlackEvent,
        WebhookControl,
    },
    state::{SlackMessageRef, StateStore},
};

const SUBSCRIPTION_ID: &str = "slack-connect-bridge";
const BRIDGE_NAME: &str = "slack-connect-bridge";
const BRIDGE_DISPLAY_NAME: &str = "Slack Connect Bridge";
const BRIDGE_ABOUT: &str =
    "Bridges explicitly mapped Buzz channels and Slack Connect channels without impersonating users.";
const BRIDGE_ICON_DATA_URL: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 128 128'%3E%3Crect width='128' height='128' rx='28' fill='%23131622'/%3E%3Cpath d='M36 64h56M64 36v56' stroke='%237dd3fc' stroke-width='13' stroke-linecap='round'/%3E%3Ccircle cx='36' cy='64' r='13' fill='%23facc15'/%3E%3Ccircle cx='92' cy='64' r='13' fill='%23a78bfa'/%3E%3C/svg%3E";
const RECONNECT_MAX_SECS: u64 = 30;
const RELAY_POLL_TIMEOUT: Duration = Duration::from_secs(1);
const PROFILE_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct Bridge {
    config: Config,
    state: StateStore,
    slack: SlackClient,
    slack_bot_user_id: String,
    delivery_rx: mpsc::Receiver<SlackDelivery>,
    webhook: WebhookControl,
    slack_user_names: HashMap<String, String>,
    buzz_user_names: HashMap<String, String>,
    profile_subscription_sequence: AtomicU64,
}

enum SessionOutcome {
    Reconnect(anyhow::Error),
    Shutdown,
}

struct SlackMessageInput<'a> {
    event_id: &'a str,
    team_id: &'a str,
    channel_id: &'a str,
    user_id: &'a str,
    text: &'a str,
    ts: &'a str,
    thread_ts: Option<&'a str>,
    is_ext_shared: Option<bool>,
}

struct SlackOriginInput<'a> {
    buzz_channel_id: Uuid,
    content: &'a str,
    team_id: &'a str,
    channel_id: &'a str,
    slack_ts: &'a str,
    user_id: &'a str,
    thread_ts: Option<&'a str>,
    reply_to: Option<EventId>,
}

impl Bridge {
    pub(crate) async fn initialize(
        config: Config,
        delivery_rx: mpsc::Receiver<SlackDelivery>,
        webhook: WebhookControl,
    ) -> Result<Self> {
        let state = StateStore::load(config.state_path.clone())?;
        let slack = SlackClient::new(config.slack_bot_token.clone())?;
        let identity = slack.auth_test().await?;
        validate_installation(&config, &identity.team_id)?;

        let mut bridge = Self {
            config,
            state,
            slack,
            slack_bot_user_id: identity.user_id,
            delivery_rx,
            webhook,
            slack_user_names: HashMap::new(),
            buzz_user_names: HashMap::new(),
            profile_subscription_sequence: AtomicU64::new(0),
        };
        bridge.validate_slack_routes().await?;
        Ok(bridge)
    }

    pub(crate) async fn run(mut self) -> Result<()> {
        let mut reconnect_delay = 1_u64;
        loop {
            let session_started = tokio::time::Instant::now();
            let outcome = tokio::select! {
                result = self.run_session() => match result {
                    Ok(()) => SessionOutcome::Reconnect(anyhow::anyhow!("Buzz relay session ended")),
                    Err(error) => SessionOutcome::Reconnect(error),
                },
                signal = tokio::signal::ctrl_c() => {
                    signal.context("failed to listen for shutdown signal")?;
                    SessionOutcome::Shutdown
                }
            };
            self.webhook.set_ready(false);
            if session_started.elapsed() >= Duration::from_secs(RECONNECT_MAX_SECS) {
                reconnect_delay = 1;
            }

            match outcome {
                SessionOutcome::Shutdown => {
                    info!("Slack Connect bridge shutting down");
                    return Ok(());
                }
                SessionOutcome::Reconnect(error) => {
                    error!(reason = %error, reconnect_delay, "Buzz relay session failed");
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(reconnect_delay)) => {}
                signal = tokio::signal::ctrl_c() => {
                    signal.context("failed to listen for shutdown signal")?;
                    info!("Slack Connect bridge shutting down");
                    return Ok(());
                }
            }
            reconnect_delay = (reconnect_delay * 2).min(RECONNECT_MAX_SECS);
        }
    }

    async fn run_session(&mut self) -> Result<()> {
        let mut connection = self.connect_buzz().await?;
        self.webhook.set_ready(true);
        info!(
            routes = self.config.channels.len(),
            "Slack Connect bridge is ready"
        );

        loop {
            tokio::select! {
                delivery = self.delivery_rx.recv() => {
                    let Some(delivery) = delivery else {
                        bail!("Slack delivery queue closed");
                    };
                    let result = self
                        .process_slack_event(&mut connection, delivery.event)
                        .await;
                    let completion = result
                        .as_ref()
                        .map(|_| ())
                        .map_err(|error| error.to_string());
                    let _ = delivery.completion.send(completion);
                    if let Err(error) = result {
                        warn!(reason = %error, "Slack event was not bridged");
                    }
                }
                relay_message = connection.next_event(RELAY_POLL_TIMEOUT) => {
                    match relay_message {
                        Ok(RelayMessage::Event { subscription_id, event })
                            if subscription_id == SUBSCRIPTION_ID =>
                        {
                            self.process_buzz_event(&event).await?;
                            self.state.record_buzz_cursor(event.created_at.as_secs())?;
                        }
                        Ok(RelayMessage::Closed { subscription_id, message })
                            if subscription_id == SUBSCRIPTION_ID =>
                        {
                            bail!("Buzz relay closed bridge subscription: {message}");
                        }
                        Ok(RelayMessage::Notice { message }) => {
                            warn!(%message, "Buzz relay notice");
                        }
                        Ok(_) => {}
                        Err(WsClientError::Timeout) => {}
                        Err(error) => return Err(error).context("Buzz relay receive failed"),
                    }
                }
            }
        }
    }

    async fn connect_buzz(&mut self) -> Result<NostrWsConnection> {
        let mut connection = NostrWsConnection::connect_authenticated(
            &self.config.relay_url,
            &self.config.bridge_keys,
            self.config.owner_auth_tag.as_ref(),
        )
        .await
        .context("failed to connect and authenticate to Buzz relay")?;

        self.publish_bridge_profile(&mut connection).await?;
        for route in &self.config.channels {
            self.announce_channel_membership(&mut connection, route)
                .await;
        }

        let channels: Vec<String> = self
            .config
            .channels
            .iter()
            .map(|route| route.buzz_channel_id.to_string())
            .collect();
        let since = self
            .state
            .subscription_since(Timestamp::now().as_secs(), self.config.replay_lookback_secs)?;
        connection
            .send_raw(&json!([
                "REQ",
                SUBSCRIPTION_ID,
                {
                    "kinds": [
                        buzz_sdk::kind::KIND_STREAM_MESSAGE,
                        buzz_sdk::kind::KIND_STREAM_MESSAGE_V2
                    ],
                    "#h": channels,
                    "since": since
                }
            ]))
            .await
            .context("failed to subscribe to mapped Buzz channels")?;
        Ok(connection)
    }

    async fn publish_bridge_profile(&self, connection: &mut NostrWsConnection) -> Result<()> {
        let event = buzz_sdk::build_profile(
            Some(BRIDGE_DISPLAY_NAME),
            Some(BRIDGE_NAME),
            Some(BRIDGE_ICON_DATA_URL),
            Some(BRIDGE_ABOUT),
            None,
        )?
        .sign_with_keys(&self.config.bridge_keys)?;
        send_event_checked(connection, event)
            .await
            .context("failed to publish bridge profile")
    }

    async fn announce_channel_membership(
        &self,
        connection: &mut NostrWsConnection,
        route: &ChannelMapping,
    ) {
        let event = build_membership_event(route.buzz_channel_id, &self.config.bridge_keys);
        let result = match event {
            Ok(event) => send_event_checked(connection, event).await,
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            warn!(
                buzz_channel_id = %route.buzz_channel_id,
                reason = %error,
                "bridge could not self-add as channel bot; private channels require an owner/admin to add the bridge pubkey"
            );
        }
    }

    async fn process_slack_event(
        &mut self,
        connection: &mut NostrWsConnection,
        event: SlackEvent,
    ) -> Result<()> {
        match event {
            SlackEvent::Message {
                event_id,
                team_id,
                channel_id,
                user_id,
                text,
                ts,
                thread_ts,
                is_ext_shared,
            } => {
                self.bridge_slack_message(
                    connection,
                    SlackMessageInput {
                        event_id: &event_id,
                        team_id: &team_id,
                        channel_id: &channel_id,
                        user_id: &user_id,
                        text: &text,
                        ts: &ts,
                        thread_ts: thread_ts.as_deref(),
                        is_ext_shared,
                    },
                )
                .await
            }
            SlackEvent::ChannelIdChanged {
                event_id,
                team_id,
                old_channel_id,
                new_channel_id,
            } => {
                if self
                    .route_for_slack(&team_id, &old_channel_id)
                    .or_else(|| self.route_for_slack(&team_id, &new_channel_id))
                    .is_some()
                {
                    self.state.record_channel_id_change(
                        &team_id,
                        &old_channel_id,
                        &new_channel_id,
                    )?;
                    info!(
                        %event_id,
                        %team_id,
                        %old_channel_id,
                        %new_channel_id,
                        "updated mapped Slack channel ID"
                    );
                }
                Ok(())
            }
            SlackEvent::ChannelShared {
                event_id,
                team_id,
                channel_id,
            } => {
                if let Some(route) = self.route_for_slack(&team_id, &channel_id).cloned() {
                    self.state.set_route_paused(route.buzz_channel_id, false)?;
                    info!(
                        %event_id,
                        %team_id,
                        %channel_id,
                        buzz_channel_id = %route.buzz_channel_id,
                        "resumed shared-channel route"
                    );
                }
                Ok(())
            }
            SlackEvent::ChannelUnshared {
                event_id,
                team_id,
                channel_id,
                is_ext_shared,
            } => {
                if let Some(route) = self.route_for_slack(&team_id, &channel_id).cloned() {
                    if is_ext_shared {
                        info!(
                            %event_id,
                            %team_id,
                            %channel_id,
                            buzz_channel_id = %route.buzz_channel_id,
                            "one organization left the Slack Connect channel; route remains shared"
                        );
                    } else {
                        self.state.set_route_paused(route.buzz_channel_id, true)?;
                        warn!(
                            %event_id,
                            %team_id,
                            %channel_id,
                            buzz_channel_id = %route.buzz_channel_id,
                            "paused route because Slack reported channel_unshared"
                        );
                    }
                }
                Ok(())
            }
        }
    }

    async fn bridge_slack_message(
        &mut self,
        connection: &mut NostrWsConnection,
        input: SlackMessageInput<'_>,
    ) -> Result<()> {
        let SlackMessageInput {
            event_id,
            team_id,
            channel_id,
            user_id,
            text,
            ts,
            thread_ts,
            is_ext_shared,
        } = input;
        if user_id == self.slack_bot_user_id {
            return Ok(());
        }
        if is_ext_shared == Some(false) && !self.config.allow_non_shared_channels {
            warn!(
                %event_id,
                %team_id,
                %channel_id,
                "ignored message explicitly marked as non-shared"
            );
            return Ok(());
        }

        let Some(route) = self.route_for_slack(team_id, channel_id).cloned() else {
            return Ok(());
        };
        if self.state.route_is_paused(route.buzz_channel_id) {
            warn!(
                %event_id,
                buzz_channel_id = %route.buzz_channel_id,
                "ignored message for paused Slack Connect route"
            );
            return Ok(());
        }
        if self
            .state
            .buzz_event_for_slack(route.buzz_channel_id, ts)
            .is_some()
        {
            return Ok(());
        }
        if text.trim().is_empty() {
            info!(%event_id, "ignored Slack message without text");
            return Ok(());
        }

        let author = self.slack_display_name(user_id).await?;
        let reply_to = thread_ts
            .filter(|root_ts| *root_ts != ts)
            .and_then(|root_ts| {
                self.state
                    .buzz_event_for_slack(route.buzz_channel_id, root_ts)
            })
            .map(EventId::from_hex)
            .transpose()
            .context("bridge state contains an invalid Buzz event ID")?;
        let thread_fallback = thread_ts
            .filter(|root_ts| *root_ts != ts)
            .is_some_and(|_| reply_to.is_none());
        let fallback_label = if thread_fallback {
            "↳ _Slack thread root was not bridged; showing this reply at channel level._\n\n"
        } else {
            ""
        };
        let content = format!(
            "**{} · Slack**\n{}{}",
            escape_markdown_label(&author),
            fallback_label,
            slack_mrkdwn_to_markdown(text)
        );
        let event = build_slack_origin_event(
            &self.config.bridge_keys,
            SlackOriginInput {
                buzz_channel_id: route.buzz_channel_id,
                content: &content,
                team_id,
                channel_id,
                slack_ts: ts,
                user_id,
                thread_ts,
                reply_to,
            },
        )?;
        let buzz_event_id = event.id.to_hex();
        send_event_checked(connection, event).await?;

        let canonical_channel = self.state.canonical_channel_id(team_id, channel_id);
        self.state.record_message_pair(
            route.buzz_channel_id,
            &buzz_event_id,
            SlackMessageRef {
                team_id: team_id.to_owned(),
                channel_id: canonical_channel,
                ts: ts.to_owned(),
                thread_ts: thread_ts.map(str::to_owned),
            },
        )?;
        info!(
            %event_id,
            %buzz_event_id,
            buzz_channel_id = %route.buzz_channel_id,
            "bridged Slack message to Buzz"
        );
        Ok(())
    }

    async fn process_buzz_event(&mut self, event: &Event) -> Result<()> {
        if event.pubkey == self.config.bridge_keys.public_key()
            || has_slack_origin(event)
            || self
                .state
                .slack_message_for_buzz(&event.id.to_hex())
                .is_some()
        {
            return Ok(());
        }
        let Some(buzz_channel_id) = event_channel_id(event) else {
            return Ok(());
        };
        let Some(route) = self.route_for_buzz(buzz_channel_id).cloned() else {
            return Ok(());
        };
        if self.state.route_is_paused(route.buzz_channel_id) {
            return Ok(());
        }

        let thread_root = event_thread_root(event);
        let thread_ts = thread_root.as_deref().and_then(|root_id| {
            self.state.slack_message_for_buzz(root_id).map(|reference| {
                reference
                    .thread_ts
                    .as_deref()
                    .unwrap_or(&reference.ts)
                    .to_owned()
            })
        });
        let author = self.buzz_display_name(&event.pubkey.to_hex()).await;
        let fallback_label = if thread_root.is_some() && thread_ts.is_none() {
            "↳ Buzz thread root was not bridged; showing this reply at channel level.\n\n"
        } else {
            ""
        };
        let text = format!(
            "*{} · Buzz*\n{}{}",
            escape_slack_label(&author),
            fallback_label,
            escape_slack_message_body(&event.content)
        );
        let channel_id = self
            .state
            .canonical_channel_id(&route.slack_team_id, &route.slack_channel_id);
        let posted = self
            .slack
            .post_message(&channel_id, &text, thread_ts.as_deref(), &event.id.to_hex())
            .await
            .with_context(|| {
                format!(
                    "failed to post Buzz event {} to mapped Slack channel",
                    event.id.to_hex()
                )
            })?;

        self.state.record_message_pair(
            route.buzz_channel_id,
            &event.id.to_hex(),
            SlackMessageRef {
                team_id: route.slack_team_id,
                channel_id,
                ts: posted.ts,
                thread_ts,
            },
        )?;
        info!(
            buzz_event_id = %event.id.to_hex(),
            buzz_channel_id = %route.buzz_channel_id,
            "bridged Buzz message to Slack"
        );
        Ok(())
    }

    async fn slack_display_name(&mut self, user_id: &str) -> Result<String> {
        if let Some(name) = self.slack_user_names.get(user_id) {
            return Ok(name.clone());
        }
        if let Some(name) = self.state.slack_user_name(user_id) {
            let name = name.to_owned();
            self.slack_user_names
                .insert(user_id.to_owned(), name.clone());
            return Ok(name);
        }
        let name = match self.slack.user_display_name(user_id).await {
            Ok(name) => name,
            Err(error) => {
                warn!(%user_id, reason = %error, "could not resolve Slack display name");
                user_id.to_owned()
            }
        };
        self.state.record_slack_user_name(user_id, &name)?;
        self.slack_user_names
            .insert(user_id.to_owned(), name.clone());
        Ok(name)
    }

    async fn buzz_display_name(&mut self, pubkey: &str) -> String {
        if let Some(name) = self.buzz_user_names.get(pubkey) {
            return name.clone();
        }
        let fallback = abbreviated_pubkey(pubkey);
        let name = match self.query_buzz_profile(pubkey).await {
            Ok(Some(name)) => name,
            Ok(None) => fallback,
            Err(error) => {
                warn!(pubkey = %abbreviated_pubkey(pubkey), reason = %error, "could not resolve Buzz profile");
                fallback
            }
        };
        self.buzz_user_names.insert(pubkey.to_owned(), name.clone());
        name
    }

    async fn query_buzz_profile(&self, pubkey: &str) -> Result<Option<String>> {
        let mut connection = NostrWsConnection::connect_authenticated(
            &self.config.relay_url,
            &self.config.bridge_keys,
            self.config.owner_auth_tag.as_ref(),
        )
        .await?;
        let sequence = self
            .profile_subscription_sequence
            .fetch_add(1, Ordering::Relaxed);
        let subscription_id = format!("slack-profile-{sequence}");
        connection
            .send_raw(&json!([
                "REQ",
                subscription_id,
                { "kinds": [0], "authors": [pubkey], "limit": 1 }
            ]))
            .await?;

        let deadline = tokio::time::Instant::now() + PROFILE_QUERY_TIMEOUT;
        let mut result = None;
        while let Some(remaining) = deadline.checked_duration_since(tokio::time::Instant::now()) {
            match connection.next_event(remaining).await {
                Ok(RelayMessage::Event {
                    subscription_id: response_id,
                    event,
                }) if response_id == subscription_id => {
                    result = profile_display_name(&event.content);
                }
                Ok(RelayMessage::Eose {
                    subscription_id: response_id,
                }) if response_id == subscription_id => break,
                Ok(RelayMessage::Closed {
                    subscription_id: response_id,
                    message,
                }) if response_id == subscription_id => {
                    bail!("Buzz profile query closed: {message}");
                }
                Ok(_) => {}
                Err(WsClientError::Timeout) => break,
                Err(error) => return Err(error.into()),
            }
        }
        let _ = connection.disconnect().await;
        Ok(result)
    }

    async fn validate_slack_routes(&mut self) -> Result<()> {
        for route in self.config.channels.clone() {
            let channel_id = self
                .state
                .canonical_channel_id(&route.slack_team_id, &route.slack_channel_id);
            let conversation = self.slack.conversation_info(&channel_id).await?;
            if conversation.is_archived {
                bail!(
                    "Slack channel {channel_id} ({}) is archived",
                    conversation.name
                );
            }
            if !conversation.is_ext_shared && !self.config.allow_non_shared_channels {
                bail!(
                    "Slack channel {channel_id} ({}) is not a Slack Connect channel; set allow_non_shared_channels only after reviewing the disclosure boundary",
                    conversation.name
                );
            }
            if conversation.is_ext_shared {
                self.state.set_route_paused(route.buzz_channel_id, false)?;
            }
            info!(
                %channel_id,
                channel_name = %conversation.name,
                is_private = conversation.is_private,
                buzz_channel_id = %route.buzz_channel_id,
                "validated Slack channel route"
            );
        }
        Ok(())
    }

    fn route_for_slack(&self, team_id: &str, channel_id: &str) -> Option<&ChannelMapping> {
        let incoming = self.state.canonical_channel_id(team_id, channel_id);
        self.config.channels.iter().find(|route| {
            route.slack_team_id == team_id
                && self
                    .state
                    .canonical_channel_id(team_id, &route.slack_channel_id)
                    == incoming
        })
    }

    fn route_for_buzz(&self, channel_id: Uuid) -> Option<&ChannelMapping> {
        self.config
            .channels
            .iter()
            .find(|route| route.buzz_channel_id == channel_id)
    }
}

fn validate_installation(config: &Config, installed_team_id: &str) -> Result<()> {
    for route in &config.channels {
        if route.slack_team_id != installed_team_id {
            bail!(
                "Slack bot token is installed in {installed_team_id}, but a route uses {}; run one bridge process per Slack installation",
                route.slack_team_id
            );
        }
    }
    Ok(())
}

fn build_membership_event(channel_id: Uuid, keys: &Keys) -> Result<Event> {
    let channel_id = channel_id.to_string();
    let pubkey = keys.public_key().to_hex();
    Ok(
        EventBuilder::new(Kind::Custom(buzz_sdk::kind::KIND_NIP29_PUT_USER as u16), "")
            .tags([
                Tag::parse(["h", channel_id.as_str()])?,
                Tag::parse(["p", pubkey.as_str()])?,
                Tag::parse(["role", "bot"])?,
            ])
            .sign_with_keys(keys)?,
    )
}

fn build_slack_origin_event(keys: &Keys, input: SlackOriginInput<'_>) -> Result<Event> {
    let SlackOriginInput {
        buzz_channel_id,
        content,
        team_id,
        channel_id,
        slack_ts,
        user_id,
        thread_ts,
        reply_to,
    } = input;
    if content.len() > 64 * 1024 {
        bail!("Slack message exceeds Buzz's 64 KiB message limit");
    }
    let external_id = format!("slack:{team_id}:{channel_id}:{slack_ts}");
    let mut tags = vec![
        Tag::parse(["h", buzz_channel_id.to_string().as_str()])?,
        Tag::parse(["i", external_id.as_str()])?,
        Tag::parse(["proxy", "slack", team_id, channel_id, slack_ts, user_id])?,
        Tag::parse(["client", BRIDGE_NAME])?,
    ];
    if let Some(thread_ts) = thread_ts {
        tags.push(Tag::parse(["slack_thread_ts", thread_ts])?);
    }
    if let Some(reply_to) = reply_to {
        let reply_to = reply_to.to_hex();
        tags.push(Tag::parse(["e", reply_to.as_str(), "", "reply"])?);
    }
    let created_at = slack_timestamp(slack_ts)?;
    Ok(EventBuilder::new(
        Kind::Custom(buzz_sdk::kind::KIND_STREAM_MESSAGE as u16),
        content,
    )
    .tags(tags)
    .custom_created_at(created_at)
    .sign_with_keys(keys)?)
}

async fn send_event_checked(connection: &mut NostrWsConnection, event: Event) -> Result<()> {
    let event_id = event.id.to_hex();
    let response = connection.send_event(event).await?;
    if !response.accepted {
        bail!("Buzz relay rejected event {event_id}: {}", response.message);
    }
    Ok(())
}

fn slack_timestamp(ts: &str) -> Result<Timestamp> {
    let seconds = ts
        .split_once('.')
        .map_or(ts, |(seconds, _)| seconds)
        .parse::<u64>()
        .context("Slack message has an invalid ts")?;
    Ok(Timestamp::from(seconds))
}

fn has_slack_origin(event: &Event) -> bool {
    event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.first().map(String::as_str) == Some("proxy")
            && parts.get(1).map(String::as_str) == Some("slack")
    })
}

fn event_channel_id(event: &Event) -> Option<Uuid> {
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        (parts.first().map(String::as_str) == Some("h"))
            .then(|| parts.get(1))
            .flatten()
            .and_then(|value| Uuid::parse_str(value).ok())
    })
}

fn event_thread_root(event: &Event) -> Option<String> {
    let mut reply = None;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) != Some("e") {
            continue;
        }
        let id = parts.get(1).filter(|id| {
            id.len() == 64 && id.chars().all(|character| character.is_ascii_hexdigit())
        });
        match (parts.get(3).map(String::as_str), id) {
            (Some("root"), Some(id)) => return Some(id.clone()),
            (Some("reply"), Some(id)) => reply = Some(id.clone()),
            _ => {}
        }
    }
    reply
}

fn profile_display_name(content: &str) -> Option<String> {
    let value: Value = serde_json::from_str(content).ok()?;
    for field in ["display_name", "name"] {
        if let Some(name) = value.get(field).and_then(Value::as_str) {
            if !name.trim().is_empty() {
                return Some(name.trim().to_owned());
            }
        }
    }
    None
}

fn abbreviated_pubkey(pubkey: &str) -> String {
    if pubkey.len() <= 16 {
        return pubkey.to_owned();
    }
    format!("{}…{}", &pubkey[..8], &pubkey[pubkey.len() - 6..])
}

fn escape_slack_label(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace(['\r', '\n', '*', '_', '`'], " ")
        .chars()
        .take(160)
        .collect()
}

/// Slack interprets angle-bracket control sequences as mentions and links.
/// Escape Buzz-authored content before posting it into an externally shared
/// channel so a string such as `<!channel>` cannot become a mass mention.
fn escape_slack_message_body(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slack_origin_is_deterministic_and_threaded() {
        let keys = Keys::generate();
        let channel = Uuid::new_v4();
        let root = EventId::from_hex(&"a".repeat(64)).unwrap();
        let first = build_slack_origin_event(
            &keys,
            SlackOriginInput {
                buzz_channel_id: channel,
                content: "hello",
                team_id: "T12345678",
                channel_id: "C12345678",
                slack_ts: "1700000000.000001",
                user_id: "U12345678",
                thread_ts: Some("1699999999.000001"),
                reply_to: Some(root),
            },
        )
        .unwrap();
        let second = build_slack_origin_event(
            &keys,
            SlackOriginInput {
                buzz_channel_id: channel,
                content: "hello",
                team_id: "T12345678",
                channel_id: "C12345678",
                slack_ts: "1700000000.000001",
                user_id: "U12345678",
                thread_ts: Some("1699999999.000001"),
                reply_to: Some(root),
            },
        )
        .unwrap();
        assert_eq!(first.id, second.id);
        assert!(has_slack_origin(&first));
        assert_eq!(event_thread_root(&first), Some("a".repeat(64)));
        assert_eq!(event_channel_id(&first), Some(channel));
    }

    #[test]
    fn direct_and_nested_replies_resolve_to_root() {
        let keys = Keys::generate();
        let root = "a".repeat(64);
        let parent = "b".repeat(64);
        let direct = EventBuilder::new(Kind::Custom(9), "direct")
            .tags([Tag::parse(["e", root.as_str(), "", "reply"]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(event_thread_root(&direct), Some(root.clone()));

        let nested = EventBuilder::new(Kind::Custom(9), "nested")
            .tags([
                Tag::parse(["e", root.as_str(), "", "root"]).unwrap(),
                Tag::parse(["e", parent.as_str(), "", "reply"]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(event_thread_root(&nested), Some(root));
    }

    #[test]
    fn profile_name_prefers_display_name() {
        assert_eq!(
            profile_display_name(r#"{"name":"alice","display_name":"Alice A."}"#),
            Some("Alice A.".into())
        );
        assert_eq!(profile_display_name("{}"), None);
    }

    #[test]
    fn install_token_cannot_cross_team_boundaries() {
        let config = Config {
            relay_url: "ws://localhost:3000".into(),
            bridge_keys: Keys::generate(),
            owner_auth_tag: None,
            slack_signing_secret: "secret".into(),
            slack_bot_token: "token".into(),
            listen_addr: "127.0.0.1:3100".parse().unwrap(),
            state_path: "state.json".into(),
            allow_non_shared_channels: false,
            replay_lookback_secs: 60,
            channels: vec![ChannelMapping {
                slack_team_id: "T12345678".into(),
                slack_channel_id: "C12345678".into(),
                buzz_channel_id: Uuid::new_v4(),
            }],
        };
        let error = validate_installation(&config, "T87654321")
            .unwrap_err()
            .to_string();
        assert!(error.contains("one bridge process per Slack installation"));
    }

    #[test]
    fn buzz_content_cannot_create_slack_control_mentions() {
        assert_eq!(
            escape_slack_message_body("deploy <!channel> and <@U12345678>"),
            "deploy &lt;!channel&gt; and &lt;@U12345678&gt;"
        );
    }
}
