//! Runtime event transport selected by provisioning.
//!
//! Local mode delegates to the existing authenticated relay client. Broker
//! mode polls `channel.read` and routes keyless storage and live signals back
//! through the broker. Capabilities with no broker action remain relay-only.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use buzz_broker_client::HttpBrokerClient;
use buzz_sdk::broker::{
    ActionArgs, ActionOutcome, BrokerClientExt, BrokerErrorCode, BrokerRequest, BrokerResult,
    ChannelReadArgs, EventPublished, LivenessPingArgs, ObserverEmitArgs, ObserverFrame,
    ObserverReceipt, PresenceSetArgs, StorageAddressArgs, StorageGetArgs, StorageRecord,
    TypingSetArgs,
};
use nostr::{Event, Keys};
use tokio::time::MissedTickBehavior;
use uuid::Uuid;

use crate::config::ChannelFilter;
use crate::relay::{
    BuzzEvent, ChannelInfo, HarnessRelay, RelayError, RelayEventPublisher, RestClient,
};

pub enum RuntimeTransport {
    Local(HarnessRelay),
    Broker(BrokerRuntime),
}

#[derive(Clone)]
pub enum RuntimeSignalPublisher {
    Local {
        publisher: RelayEventPublisher,
        keys: Keys,
    },
    Broker(BrokerActions),
}

impl RuntimeSignalPublisher {
    pub async fn presence_set(
        &self,
        status: buzz_core::presence::PresenceStatus,
    ) -> Result<(), RelayError> {
        match self {
            Self::Local { publisher, keys } => {
                use buzz_core::kind::KIND_PRESENCE_UPDATE;
                use nostr::{EventBuilder, Kind};

                let event = EventBuilder::new(
                    Kind::Custom(KIND_PRESENCE_UPDATE as u16),
                    status.to_string(),
                )
                .tags([])
                .sign_with_keys(keys)
                .map_err(|error| RelayError::Http(format!("presence sign error: {error}")))?;
                publisher.publish_event(event).await
            }
            Self::Broker(actions) => actions.presence_set(status).await.map(|_| ()),
        }
    }

    pub async fn typing_set(
        &self,
        channel_id: Uuid,
        root_event_id: Option<String>,
        parent_event_id: Option<String>,
    ) -> Result<(), RelayError> {
        match self {
            Self::Local { publisher, keys } => {
                use buzz_core::kind::KIND_TYPING_INDICATOR;
                use nostr::{EventBuilder, Kind, Tag};

                let h_tag = Tag::parse(["h", &channel_id.to_string()])
                    .map_err(|error| RelayError::Http(error.to_string()))?;
                let mut tags = vec![h_tag];
                if let Some(parent) = parent_event_id.as_deref() {
                    if let Some(root) = root_event_id.as_deref() {
                        if root != parent {
                            tags.push(
                                Tag::parse(["e", root, "", "root"])
                                    .map_err(|error| RelayError::Http(error.to_string()))?,
                            );
                        }
                    }
                    tags.push(
                        Tag::parse(["e", parent, "", "reply"])
                            .map_err(|error| RelayError::Http(error.to_string()))?,
                    );
                }
                let event = EventBuilder::new(Kind::Custom(KIND_TYPING_INDICATOR as u16), "")
                    .tags(tags)
                    .sign_with_keys(keys)
                    .map_err(|error| RelayError::Http(format!("typing sign error: {error}")))?;
                publisher.try_publish_event(event)
            }
            Self::Broker(actions) => actions.typing_set(channel_id).await.map(|_| ()),
        }
    }
}

pub struct BrokerRuntime {
    client: HttpBrokerClient,
    channel_ids: Vec<Uuid>,
    filters: HashMap<Uuid, ChannelFilter>,
    filter_tx: tokio::sync::watch::Sender<HashMap<Uuid, ChannelFilter>>,
    event_rx: tokio::sync::mpsc::Receiver<BuzzEvent>,
    poll_task: tokio::task::JoinHandle<()>,
    placeholder_keys: Keys,
    terminal_error: Arc<Mutex<Option<String>>>,
}

struct BrokerPoller {
    client: HttpBrokerClient,
    channel_ids: Vec<Uuid>,
    filter_rx: tokio::sync::watch::Receiver<HashMap<Uuid, ChannelFilter>>,
    cursors: HashMap<Uuid, String>,
    seen_current: HashSet<String>,
    seen_previous: HashSet<String>,
    poll: tokio::time::Interval,
    terminal_error: Arc<Mutex<Option<String>>>,
}

/// Cloneable handle for broker-backed capabilities used outside the polling
/// loop (memory, presence, typing, observer telemetry, and turn liveness).
#[derive(Clone)]
pub struct BrokerActions {
    client: HttpBrokerClient,
}

impl BrokerActions {
    async fn run(&self, args: ActionArgs) -> Result<ActionOutcome, RelayError> {
        execute(&self.client, args)
            .await
            .map_err(BrokerActionError::relay_error)
    }

    pub async fn storage_get(&self, slug: String) -> Result<StorageRecord, RelayError> {
        match self
            .run(ActionArgs::StorageGet(StorageGetArgs { slug }))
            .await?
        {
            ActionOutcome::StorageGet(record) => Ok(record),
            _ => Err(wrong_outcome("storage.get")),
        }
    }

    pub async fn presence_set(
        &self,
        status: buzz_core::presence::PresenceStatus,
    ) -> Result<EventPublished, RelayError> {
        match self
            .run(ActionArgs::PresenceSet(PresenceSetArgs { status }))
            .await?
        {
            ActionOutcome::PresenceSet(published) => Ok(published),
            _ => Err(wrong_outcome("presence.set")),
        }
    }

    pub async fn typing_set(&self, channel_id: Uuid) -> Result<EventPublished, RelayError> {
        match self
            .run(ActionArgs::TypingSet(TypingSetArgs {
                channel_id: channel_id.to_string(),
            }))
            .await?
        {
            ActionOutcome::TypingSet(published) => Ok(published),
            _ => Err(wrong_outcome("typing.set")),
        }
    }

    pub async fn observer_emit(
        &self,
        frames: Vec<ObserverFrame>,
    ) -> Result<ObserverReceipt, RelayError> {
        match self
            .run(ActionArgs::ObserverEmit(ObserverEmitArgs { frames }))
            .await?
        {
            ActionOutcome::ObserverEmit(receipt) => Ok(receipt),
            _ => Err(wrong_outcome("observer.emit")),
        }
    }

    pub async fn liveness_ping(
        &self,
        channel_id: Uuid,
        turn_id: String,
    ) -> Result<EventPublished, RelayError> {
        match self
            .run(ActionArgs::LivenessPing(LivenessPingArgs {
                channel_id: channel_id.to_string(),
                turn_id,
            }))
            .await?
        {
            ActionOutcome::LivenessPing(published) => Ok(published),
            _ => Err(wrong_outcome("liveness.ping")),
        }
    }
}

fn wrong_outcome(action: &str) -> RelayError {
    RelayError::Http(format!("broker returned the wrong outcome for {action}"))
}

fn advance_cursor(
    cursors: &mut HashMap<Uuid, String>,
    channel_id: Uuid,
    next_cursor: Option<String>,
) {
    if let Some(cursor) = next_cursor {
        cursors.insert(channel_id, cursor);
    } else {
        cursors.remove(&channel_id);
    }
}

struct BrokerActionError {
    detail: String,
    code: Option<BrokerErrorCode>,
}

impl std::fmt::Display for BrokerActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

impl BrokerActionError {
    fn relay_error(self) -> RelayError {
        RelayError::Http(self.detail)
    }
}

impl BrokerRuntime {
    pub async fn connect(
        base_url: String,
        credential: String,
        channel_ids: Vec<Uuid>,
        poll_interval: Duration,
        placeholder_keys: Keys,
    ) -> Result<(Self, String), RelayError> {
        let client = HttpBrokerClient::new(base_url, credential)
            .map_err(|error| RelayError::Http(format!("broker config: {error}")))?;
        let outcome = execute(
            &client,
            ActionArgs::StorageAddress(StorageAddressArgs {
                slug: "core".into(),
            }),
        )
        .await
        .map_err(BrokerActionError::relay_error)?;
        let ActionOutcome::StorageAddress(address) = outcome else {
            return Err(RelayError::Http(
                "broker returned the wrong outcome for storage.address".into(),
            ));
        };
        let agent_pubkey = address.author_pubkey.as_str().to_string();
        let mut poll = tokio::time::interval(poll_interval);
        poll.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let filters = HashMap::new();
        let (filter_tx, filter_rx) = tokio::sync::watch::channel(filters.clone());
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(256);
        let terminal_error = Arc::new(Mutex::new(None));
        let poller = BrokerPoller {
            client: client.clone(),
            channel_ids: channel_ids.clone(),
            filter_rx,
            cursors: HashMap::new(),
            seen_current: HashSet::new(),
            seen_previous: HashSet::new(),
            poll,
            terminal_error: Arc::clone(&terminal_error),
        };
        let poll_task = tokio::spawn(poller.run(event_tx));
        Ok((
            Self {
                client,
                channel_ids,
                filters,
                filter_tx,
                event_rx,
                poll_task,
                placeholder_keys,
                terminal_error,
            },
            agent_pubkey,
        ))
    }

    fn set_filter(&mut self, channel_id: Uuid, filter: Option<ChannelFilter>) {
        if let Some(filter) = filter {
            self.filters.insert(channel_id, filter);
        } else {
            self.filters.remove(&channel_id);
        }
        self.filter_tx.send_replace(self.filters.clone());
    }

    async fn next_event(&mut self) -> Option<BuzzEvent> {
        self.event_rx.recv().await
    }
}

impl Drop for BrokerRuntime {
    fn drop(&mut self) {
        self.poll_task.abort();
    }
}

impl BrokerPoller {
    fn remember(&mut self, event_id: String) -> bool {
        if self.seen_current.contains(&event_id) || self.seen_previous.contains(&event_id) {
            return false;
        }
        self.seen_current.insert(event_id);
        if self.seen_current.len() >= 2_000 {
            self.seen_previous = std::mem::take(&mut self.seen_current);
        }
        true
    }

    async fn run(mut self, event_tx: tokio::sync::mpsc::Sender<BuzzEvent>) {
        loop {
            self.poll.tick().await;
            let filters = self.filter_rx.borrow().clone();
            let subscriptions: Vec<(Uuid, bool)> = self
                .channel_ids
                .iter()
                .filter_map(|channel_id| {
                    filters
                        .get(channel_id)
                        .map(|filter| (*channel_id, filter.require_mention))
                })
                .collect();
            for (channel_id, mentions_only) in subscriptions {
                let result = execute(
                    &self.client,
                    ActionArgs::ChannelRead(ChannelReadArgs {
                        channel_id: channel_id.to_string(),
                        root_event_id: None,
                        mentions_only,
                        cursor: self.cursors.get(&channel_id).cloned(),
                        limit: Some(100),
                    }),
                )
                .await;
                let page = match result {
                    Ok(ActionOutcome::ChannelRead(page)) => page,
                    Ok(_) => {
                        tracing::warn!(%channel_id, "broker returned the wrong channel.read outcome");
                        continue;
                    }
                    Err(error) if error.code == Some(BrokerErrorCode::Unauthenticated) => {
                        tracing::error!(%channel_id, "broker credential was rejected: {error}");
                        *self.terminal_error.lock().expect("terminal error lock") =
                            Some(error.to_string());
                        return;
                    }
                    Err(error) => {
                        tracing::warn!(%channel_id, "broker channel.read failed: {error}");
                        continue;
                    }
                };
                // `nextCursor` is a pagination continuation, not a durable
                // tail watermark. Return to the host's current default window
                // after draining so newly-arrived events remain visible; the
                // rotating dedup sets absorb the replay.
                advance_cursor(&mut self.cursors, channel_id, page.next_cursor);
                for message in page.messages {
                    if let Err(error) = message.verify() {
                        tracing::warn!(%channel_id, "broker returned an unverifiable event: {error}");
                        continue;
                    }
                    let event = message.0;
                    let addressed_channel = event
                        .tags
                        .iter()
                        .find(|tag| tag.kind().to_string() == "h")
                        .and_then(|tag| tag.content())
                        .and_then(|value| Uuid::parse_str(value).ok());
                    if addressed_channel != Some(channel_id) {
                        tracing::warn!(%channel_id, "broker returned an event for a different channel");
                        continue;
                    }
                    if self.remember(event.id.to_hex())
                        && event_tx
                            .send(BuzzEvent { channel_id, event })
                            .await
                            .is_err()
                    {
                        return;
                    }
                }
            }
        }
    }
}

async fn execute(
    client: &HttpBrokerClient,
    args: ActionArgs,
) -> Result<ActionOutcome, BrokerActionError> {
    let request = BrokerRequest::new(Uuid::new_v4().to_string(), args)
        .and_then(BrokerRequest::prepare)
        .map_err(|error| BrokerActionError {
            detail: format!("broker request: {error}"),
            code: None,
        })?;
    let response = client
        .execute(&request)
        .await
        .map_err(|error| BrokerActionError {
            detail: format!("broker transport: {error}"),
            code: None,
        })?;
    match response.into_envelope().result {
        BrokerResult::Succeeded { outcome } => Ok(outcome),
        BrokerResult::Failed { error } | BrokerResult::Indeterminate { error } => {
            Err(BrokerActionError {
                detail: format!(
                    "broker verdict: {} [{}]",
                    error.message,
                    error.code.as_str()
                ),
                code: Some(error.code),
            })
        }
    }
}

impl RuntimeTransport {
    pub fn local(relay: HarnessRelay) -> Self {
        Self::Local(relay)
    }

    pub async fn broker(
        base_url: String,
        credential: String,
        channel_ids: Vec<Uuid>,
        poll_interval: Duration,
        placeholder_keys: Keys,
    ) -> Result<(Self, String), RelayError> {
        let (runtime, pubkey) = BrokerRuntime::connect(
            base_url,
            credential,
            channel_ids,
            poll_interval,
            placeholder_keys,
        )
        .await?;
        Ok((Self::Broker(runtime), pubkey))
    }

    pub async fn set_startup_watermark(&self, timestamp: u64) -> Result<(), RelayError> {
        match self {
            Self::Local(relay) => relay.set_startup_watermark(timestamp).await,
            Self::Broker(_) => Ok(()),
        }
    }

    pub async fn subscribe_membership_notifications(&mut self) -> Result<(), RelayError> {
        match self {
            Self::Local(relay) => relay.subscribe_membership_notifications().await,
            Self::Broker(_) => Ok(()),
        }
    }

    pub async fn subscribe_observer_controls(&mut self) -> Result<(), RelayError> {
        match self {
            Self::Local(relay) => relay.subscribe_observer_controls().await,
            Self::Broker(_) => Ok(()),
        }
    }

    pub fn take_observer_control_rx(&mut self) -> Option<tokio::sync::mpsc::Receiver<Event>> {
        match self {
            Self::Local(relay) => relay.take_observer_control_rx(),
            Self::Broker(_) => None,
        }
    }

    pub async fn discover_channels(&self) -> Result<HashMap<Uuid, ChannelInfo>, RelayError> {
        match self {
            Self::Local(relay) => relay.discover_channels().await,
            Self::Broker(runtime) => Ok(runtime
                .channel_ids
                .iter()
                .map(|channel_id| {
                    (
                        *channel_id,
                        ChannelInfo {
                            name: channel_id.to_string(),
                            // The frozen broker contract does not expose channel
                            // metadata. Keep the type unknown so the runtime's
                            // author gate treats it as a DM (fail closed).
                            channel_type: "unknown".into(),
                            description: None,
                        },
                    )
                })
                .collect()),
        }
    }

    pub async fn subscribe_channel(
        &mut self,
        channel_id: Uuid,
        filter: ChannelFilter,
    ) -> Result<(), RelayError> {
        match self {
            Self::Local(relay) => relay.subscribe_channel(channel_id, filter).await,
            Self::Broker(runtime) => {
                runtime.set_filter(channel_id, Some(filter));
                Ok(())
            }
        }
    }

    pub async fn subscribe_channel_from(
        &mut self,
        channel_id: Uuid,
        filter: ChannelFilter,
        replay_since: Option<u64>,
    ) -> Result<(), RelayError> {
        match self {
            Self::Local(relay) => {
                relay
                    .subscribe_channel_from(channel_id, filter, replay_since)
                    .await
            }
            Self::Broker(runtime) => {
                runtime.set_filter(channel_id, Some(filter));
                Ok(())
            }
        }
    }

    pub async fn unsubscribe_channel(&mut self, channel_id: Uuid) -> Result<(), RelayError> {
        match self {
            Self::Local(relay) => relay.unsubscribe_channel(channel_id).await,
            Self::Broker(runtime) => {
                runtime.set_filter(channel_id, None);
                Ok(())
            }
        }
    }

    pub async fn next_event(&mut self) -> Option<BuzzEvent> {
        match self {
            Self::Local(relay) => relay.next_event().await,
            Self::Broker(runtime) => runtime.next_event().await,
        }
    }

    pub async fn reconnect(&mut self) -> Result<(), RelayError> {
        match self {
            Self::Local(relay) => relay.reconnect().await,
            Self::Broker(runtime) => {
                match &*runtime.terminal_error.lock().expect("terminal error lock") {
                    Some(error) => Err(RelayError::Http(error.clone())),
                    None => Ok(()),
                }
            }
        }
    }

    pub fn event_publisher(&self) -> RelayEventPublisher {
        match self {
            Self::Local(relay) => relay.event_publisher(),
            Self::Broker(_) => RelayEventPublisher::disabled(),
        }
    }

    pub fn broker_actions(&self) -> Option<BrokerActions> {
        match self {
            Self::Local(_) => None,
            Self::Broker(runtime) => Some(BrokerActions {
                client: runtime.client.clone(),
            }),
        }
    }

    pub fn signal_publisher(&self, keys: Keys) -> RuntimeSignalPublisher {
        match self {
            Self::Local(relay) => RuntimeSignalPublisher::Local {
                publisher: relay.event_publisher(),
                keys,
            },
            Self::Broker(runtime) => RuntimeSignalPublisher::Broker(BrokerActions {
                client: runtime.client.clone(),
            }),
        }
    }

    pub fn rest_client(&self) -> RestClient {
        match self {
            Self::Local(relay) => relay.rest_client(),
            Self::Broker(runtime) => RestClient {
                http: reqwest::Client::new(),
                base_url: "http://127.0.0.1:1".into(),
                keys: runtime.placeholder_keys.clone(),
                auth_tag_json: None,
            },
        }
    }

    pub async fn shutdown(self) {
        if let Self::Local(relay) = self {
            relay.shutdown().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, Bytes};
    use axum::http::StatusCode;
    use axum::response::Response;
    use axum::routing::post;
    use axum::Router;
    use nostr::{EventBuilder, Kind, Tag};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    #[test]
    fn terminal_page_returns_polling_to_the_default_window() {
        let channel_id = Uuid::new_v4();
        let mut cursors = HashMap::new();
        advance_cursor(&mut cursors, channel_id, Some("continuation".into()));
        assert_eq!(
            cursors.get(&channel_id).map(String::as_str),
            Some("continuation")
        );

        advance_cursor(&mut cursors, channel_id, None);
        assert!(!cursors.contains_key(&channel_id));
    }

    #[tokio::test]
    async fn broker_actions_route_storage_and_live_signals() {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let app = Router::new().route(
            "/v1/action",
            post({
                let seen = Arc::clone(&seen);
                move |body: Bytes| {
                    let seen = Arc::clone(&seen);
                    async move {
                        let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                        let request_id = request["requestId"].as_str().unwrap();
                        let action = request["action"].as_str().unwrap();
                        seen.lock().unwrap().push(action.to_string());
                        let outcome = match action {
                            "storage.get" => serde_json::json!({ "value": "core" }),
                            "observer.emit" => serde_json::json!({ "accepted": 1 }),
                            _ => serde_json::json!({
                                "eventId": "cacf5f811cc8ef3f4af3f92cc222f92a86cdf6a26728a144c8e63b74ab6db359",
                                "kind": 20001,
                                "createdAt": 1_700_000_000u64,
                            }),
                        };
                        let response = serde_json::json!({
                            "type": "broker_result",
                            "protocolVersion": 1,
                            "requestId": request_id,
                            "status": "succeeded",
                            "action": action,
                            "outcome": outcome,
                        });
                        Response::builder()
                            .status(StatusCode::OK)
                            .body(Body::from(response.to_string()))
                            .unwrap()
                    }
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let actions = BrokerActions {
            client: HttpBrokerClient::new(format!("http://{address}"), "credential").unwrap(),
        };
        let channel_id = Uuid::new_v4();

        assert_eq!(
            actions
                .storage_get("core".into())
                .await
                .unwrap()
                .value
                .as_deref(),
            Some("core")
        );
        actions
            .presence_set(buzz_core::presence::PresenceStatus::Online)
            .await
            .unwrap();
        actions.typing_set(channel_id).await.unwrap();
        actions
            .observer_emit(vec![ObserverFrame {
                kind: "turn_started".into(),
                payload: "{}".into(),
            }])
            .await
            .unwrap();
        actions
            .liveness_ping(channel_id, "turn-1".into())
            .await
            .unwrap();

        assert_eq!(
            *seen.lock().unwrap(),
            [
                "storage.get",
                "presence.set",
                "typing.set",
                "observer.emit",
                "liveness.ping",
            ]
        );
    }

    #[tokio::test]
    async fn polling_survives_main_loop_timer_cancellation_and_reaches_later_channels() {
        let slow_channel = Uuid::new_v4();
        let ready_channel = Uuid::new_v4();
        let keys = Keys::generate();
        let ready_event = EventBuilder::new(Kind::Custom(9), "ready")
            .tags([Tag::parse(["h", &ready_channel.to_string()]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        let pubkey = keys.public_key().to_hex();
        let app = Router::new().route(
            "/v1/action",
            post(move |body: Bytes| {
                let ready_event = ready_event.clone();
                let pubkey = pubkey.clone();
                async move {
                    let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    let request_id = request["requestId"].as_str().unwrap();
                    let action = request["action"].as_str().unwrap();
                    let outcome = if action == "storage.address" {
                        serde_json::json!({
                            "authorPubkey": pubkey,
                            "kind": 30078,
                            "dTag": "a".repeat(64),
                        })
                    } else {
                        let channel = request["args"]["channelId"].as_str().unwrap();
                        if channel == slow_channel.to_string() {
                            tokio::time::sleep(Duration::from_millis(80)).await;
                            serde_json::json!({ "messages": [] })
                        } else {
                            serde_json::json!({ "messages": [ready_event] })
                        }
                    };
                    let response = serde_json::json!({
                        "type": "broker_result",
                        "protocolVersion": 1,
                        "requestId": request_id,
                        "status": "succeeded",
                        "action": action,
                        "outcome": outcome,
                    });
                    Response::builder()
                        .status(StatusCode::OK)
                        .body(Body::from(response.to_string()))
                        .unwrap()
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let (mut runtime, _) = BrokerRuntime::connect(
            format!("http://{address}"),
            "credential".into(),
            vec![slow_channel, ready_channel],
            Duration::from_millis(1),
            Keys::generate(),
        )
        .await
        .unwrap();
        let filter = ChannelFilter {
            kinds: Some(vec![9]),
            require_mention: false,
        };
        runtime.set_filter(slow_channel, Some(filter.clone()));
        runtime.set_filter(ready_channel, Some(filter));

        let mut typing_timer = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_millis(10),
            Duration::from_millis(10),
        );
        let received = tokio::time::timeout(Duration::from_millis(300), async {
            loop {
                tokio::select! {
                    _ = typing_timer.tick() => {}
                    event = runtime.next_event() => break event,
                }
            }
        })
        .await
        .expect("timer ticks must not restart the slow first channel")
        .expect("poller stays open");

        assert_eq!(received.channel_id, ready_channel);
        assert_eq!(received.event.content, "ready");
    }
}
