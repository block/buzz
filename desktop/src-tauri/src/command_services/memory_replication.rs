use super::{
    MemoryError, ReplicationResult, TrustedMemoryConfig, MAXIMUM_RESPONSE_BYTES, SYNC_CANCELLED,
};
use chrono::Utc;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE};
use reqwest::{Client, Method, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::runtime::{Builder, Runtime};

const PAGE_SIZE: u64 = 50;
const MAXIMUM_PAGES: u64 = 10_000;
const MAXIMUM_OBJECTS_PER_PAGE: usize = 200;
const MAXIMUM_TOTAL_OBJECTS: u64 = 1_000_000;
const MAXIMUM_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAXIMUM_REQUEST_BYTES: usize = MAXIMUM_RESPONSE_BYTES + 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
const MAXIMUM_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[path = "memory_replication_validation.rs"]
mod validation;
use validation::{object_is_tombstone, validate_envelope, Envelope};
#[cfg(test)]
use validation::{python_canonical_json_bytes, valid_envelope_id};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Capability {
    Read,
    Replicate,
}

struct Node<'a> {
    endpoint: String,
    read_token: &'a str,
    replicate_token: &'a str,
    expected_node_id: &'a str,
}

impl Node<'_> {
    fn token(&self, capability: Capability) -> &str {
        match capability {
            Capability::Read => self.read_token,
            Capability::Replicate => self.replicate_token,
        }
    }
}

#[derive(Default)]
struct TransferBudget {
    bytes: u64,
    objects: u64,
}

impl TransferBudget {
    fn add_bytes(&mut self, count: usize) -> Result<(), MemoryError> {
        self.bytes = self
            .bytes
            .checked_add(count as u64)
            .ok_or(MemoryError::ResponseTooLarge)?;
        if self.bytes > MAXIMUM_TOTAL_BYTES {
            return Err(MemoryError::ResponseTooLarge);
        }
        Ok(())
    }

    fn add_objects(&mut self, count: usize) -> Result<(), MemoryError> {
        self.objects = self
            .objects
            .checked_add(count as u64)
            .ok_or(MemoryError::ResponseTooLarge)?;
        if self.objects > MAXIMUM_TOTAL_OBJECTS {
            return Err(MemoryError::ResponseTooLarge);
        }
        Ok(())
    }
}

trait JsonExchange {
    fn request(
        &mut self,
        node: &Node<'_>,
        request: JsonRequest<'_>,
        deadline: Instant,
        budget: &mut TransferBudget,
    ) -> Result<Value, MemoryError>;
}

struct JsonRequest<'a> {
    method: Method,
    path: &'static str,
    capability: Capability,
    payload: Option<&'a Value>,
}

struct HttpJsonExchange {
    client: Client,
    runtime: Runtime,
}

impl HttpJsonExchange {
    fn new() -> Result<Self, MemoryError> {
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|_| MemoryError::LocalServiceUnavailable)?;
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| MemoryError::LocalServiceUnavailable)?;
        Ok(Self { client, runtime })
    }
}

impl JsonExchange for HttpJsonExchange {
    fn request(
        &mut self,
        node: &Node<'_>,
        request: JsonRequest<'_>,
        deadline: Instant,
        budget: &mut TransferBudget,
    ) -> Result<Value, MemoryError> {
        check_active(deadline)?;
        let timeout = deadline
            .saturating_duration_since(Instant::now())
            .min(MAXIMUM_REQUEST_TIMEOUT);
        if timeout.is_zero() {
            return Err(MemoryError::Timeout);
        }
        let body = request
            .payload
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|_| MemoryError::InvalidResponse)?;
        if body
            .as_ref()
            .is_some_and(|bytes| bytes.len() > MAXIMUM_REQUEST_BYTES)
        {
            return Err(MemoryError::ResponseTooLarge);
        }
        if let Some(bytes) = &body {
            budget.add_bytes(bytes.len())?;
        }
        let mut request_builder = self
            .client
            .request(request.method, format!("{}{}", node.endpoint, request.path))
            .header(ACCEPT, "application/json")
            .header(
                AUTHORIZATION,
                format!("Bearer {}", node.token(request.capability)),
            );
        if let Some(bytes) = body {
            request_builder = request_builder
                .header(CONTENT_TYPE, "application/json")
                .body(bytes);
        }
        let request_deadline = Instant::now() + timeout;
        let result = self.runtime.block_on(async {
            let response = tokio::select! {
                biased;
                error = cancellation_or_deadline(request_deadline) => return Err(error),
                response = request_builder.send() => response
                    .map_err(|_| current_error(deadline, MemoryError::LocalServiceUnavailable))?,
            };
            read_response(response, request_deadline).await
        });
        if matches!(&result, Err(MemoryError::Cancelled | MemoryError::Timeout)) {
            self.runtime.block_on(async {
                tokio::task::yield_now().await;
                tokio::time::sleep(Duration::from_millis(1)).await;
            });
        }
        let (value, response_bytes) = result?;
        budget.add_bytes(response_bytes)?;
        Ok(value)
    }
}

async fn cancellation_or_deadline(deadline: Instant) -> MemoryError {
    loop {
        if SYNC_CANCELLED.load(Ordering::SeqCst) {
            return MemoryError::Cancelled;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return MemoryError::Timeout;
        }
        tokio::time::sleep(remaining.min(CANCELLATION_POLL_INTERVAL)).await;
    }
}

async fn read_response(
    mut response: Response,
    deadline: Instant,
) -> Result<(Value, usize), MemoryError> {
    let status = response.status();
    if matches!(status.as_u16(), 401 | 403) {
        return Err(MemoryError::AuthenticationFailed);
    }
    if status.is_redirection() || !status.is_success() {
        return Err(MemoryError::LocalServiceUnavailable);
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .ok_or(MemoryError::InvalidResponse)?;
    let mut content_type_parts = content_type.split(';').map(str::trim);
    if !content_type_parts
        .next()
        .is_some_and(|value| value.eq_ignore_ascii_case("application/json"))
        || content_type_parts.any(|parameter| {
            !parameter.split_once('=').is_some_and(|(name, value)| {
                name.trim().eq_ignore_ascii_case("charset")
                    && value.trim().eq_ignore_ascii_case("utf-8")
            })
        })
    {
        return Err(MemoryError::InvalidResponse);
    }
    if let Some(value) = response.headers().get(CONTENT_LENGTH) {
        let length = value
            .to_str()
            .ok()
            .and_then(|text| text.parse::<u64>().ok())
            .ok_or(MemoryError::InvalidResponse)?;
        if length > MAXIMUM_RESPONSE_BYTES as u64 {
            return Err(MemoryError::ResponseTooLarge);
        }
    }
    let mut body = Vec::new();
    loop {
        let next = tokio::select! {
            biased;
            error = cancellation_or_deadline(deadline) => return Err(error),
            chunk = response.chunk() => chunk
                .map_err(|_| current_error(deadline, MemoryError::InvalidResponse))?,
        };
        let Some(chunk) = next else {
            break;
        };
        if body.len() + chunk.len() > MAXIMUM_RESPONSE_BYTES {
            return Err(MemoryError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&body).map_err(|_| MemoryError::InvalidResponse)?;
    if !value.is_object() {
        return Err(MemoryError::InvalidResponse);
    }
    Ok((value, body.len()))
}

fn check_active(deadline: Instant) -> Result<(), MemoryError> {
    if SYNC_CANCELLED.load(Ordering::SeqCst) {
        return Err(MemoryError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(MemoryError::Timeout);
    }
    Ok(())
}

fn current_error(deadline: Instant, fallback: MemoryError) -> MemoryError {
    if SYNC_CANCELLED.load(Ordering::SeqCst) {
        MemoryError::Cancelled
    } else if Instant::now() >= deadline {
        MemoryError::Timeout
    } else {
        fallback
    }
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, MemoryError> {
    serde_json::from_value(value).map_err(|_| MemoryError::InvalidResponse)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplicationReadiness {
    status: String,
    schema_version: u32,
    node_id: String,
    #[serde(rename = "revision_count")]
    _revision_count: u64,
    conflict_count: u64,
    max_page_items: u64,
    max_envelope_bytes: u64,
    markdown_canonical: bool,
    sqlite_derived: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Acknowledgement {
    peer_node_id: String,
    cursor: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportResult {
    source_node_id: String,
    accepted: u64,
    duplicates: u64,
    conflicts: u64,
    cursor: u64,
}

#[derive(Default)]
struct Totals {
    accepted: u64,
    duplicates: u64,
    conflicts: u64,
    objects: u64,
    tombstones: u64,
    pages: u64,
}

fn readiness(
    exchange: &mut impl JsonExchange,
    node: &Node<'_>,
    deadline: Instant,
    budget: &mut TransferBudget,
) -> Result<ReplicationReadiness, MemoryError> {
    let value = exchange.request(
        node,
        JsonRequest {
            method: Method::GET,
            path: "/replication/readiness",
            capability: Capability::Read,
            payload: None,
        },
        deadline,
        budget,
    )?;
    let value: ReplicationReadiness = decode(value)?;
    if value.status != "ready"
        || value.schema_version != 1
        || value.node_id != node.expected_node_id
        || value.max_page_items == 0
        || value.max_page_items > 200
        || value.max_envelope_bytes < 1024
        || value.max_envelope_bytes > MAXIMUM_RESPONSE_BYTES as u64
        || !value.markdown_canonical
        || !value.sqlite_derived
    {
        return if value.node_id != node.expected_node_id {
            Err(MemoryError::NodeIdentityMismatch)
        } else {
            Err(MemoryError::InvalidResponse)
        };
    }
    Ok(value)
}

fn checked_add(target: &mut u64, value: u64) -> Result<(), MemoryError> {
    *target = target
        .checked_add(value)
        .ok_or(MemoryError::ResponseTooLarge)?;
    Ok(())
}

fn replicate_with_exchange(
    operation: &str,
    local: &Node<'_>,
    remote: &Node<'_>,
    timeout: Duration,
    exchange: &mut impl JsonExchange,
) -> Result<ReplicationResult, MemoryError> {
    if timeout.is_zero() || timeout > Duration::from_secs(300) {
        return Err(MemoryError::InvalidConfig);
    }
    let deadline = Instant::now() + timeout;
    let (source, target) = match operation {
        "pull" => (remote, local),
        "push" => (local, remote),
        _ => return Err(MemoryError::InvalidConfig),
    };
    let mut budget = TransferBudget::default();
    let source_ready = readiness(exchange, source, deadline, &mut budget)?;
    let target_ready = readiness(exchange, target, deadline, &mut budget)?;
    if source_ready.node_id == target_ready.node_id {
        return Err(MemoryError::NodeIdentityMismatch);
    }
    let initial_ack = serde_json::json!({
        "peer_node_id": target_ready.node_id,
        "cursor": 0
    });
    let acknowledged: Acknowledgement = decode(exchange.request(
        source,
        JsonRequest {
            method: Method::POST,
            path: "/replication/ack",
            capability: Capability::Replicate,
            payload: Some(&initial_ack),
        },
        deadline,
        &mut budget,
    )?)?;
    if acknowledged.peer_node_id != target_ready.node_id {
        return Err(MemoryError::InvalidResponse);
    }
    let from_cursor = acknowledged.cursor;
    let mut cursor = from_cursor;
    let mut totals = Totals::default();
    let mut complete = false;

    for _ in 0..MAXIMUM_PAGES {
        check_active(deadline)?;
        let export = serde_json::json!({"cursor": cursor, "limit": PAGE_SIZE});
        let envelope_value = exchange.request(
            source,
            JsonRequest {
                method: Method::POST,
                path: "/replication/export",
                capability: Capability::Replicate,
                payload: Some(&export),
            },
            deadline,
            &mut budget,
        )?;
        let envelope: Envelope = decode(envelope_value.clone())?;
        validate_envelope(
            &envelope,
            &envelope_value,
            &source_ready.node_id,
            cursor,
            source_ready.max_page_items,
        )?;
        if envelope.revisions.is_empty() {
            if envelope.has_more {
                return Err(MemoryError::InvalidResponse);
            }
            complete = true;
            break;
        }
        budget.add_objects(envelope.objects.len())?;
        let import = serde_json::json!({"envelope": envelope_value});
        let import_bytes = serde_json::to_vec(&import).map_err(|_| MemoryError::InvalidResponse)?;
        if import_bytes.len() as u64 > target_ready.max_envelope_bytes {
            return Err(MemoryError::ResponseTooLarge);
        }
        let imported: ImportResult = decode(exchange.request(
            target,
            JsonRequest {
                method: Method::POST,
                path: "/replication/import",
                capability: Capability::Replicate,
                payload: Some(&import),
            },
            deadline,
            &mut budget,
        )?)?;
        if imported.source_node_id != source_ready.node_id
            || imported
                .accepted
                .checked_add(imported.duplicates)
                .is_none_or(|count| count != envelope.revisions.len() as u64)
            || imported.conflicts > imported.accepted
            || imported.cursor != envelope.to_cursor
        {
            return Err(MemoryError::InvalidResponse);
        }
        let ack = serde_json::json!({
            "peer_node_id": target_ready.node_id,
            "cursor": envelope.to_cursor
        });
        let acknowledged: Acknowledgement = decode(exchange.request(
            source,
            JsonRequest {
                method: Method::POST,
                path: "/replication/ack",
                capability: Capability::Replicate,
                payload: Some(&ack),
            },
            deadline,
            &mut budget,
        )?)?;
        if acknowledged.peer_node_id != target_ready.node_id
            || acknowledged.cursor != envelope.to_cursor
            || acknowledged.cursor <= cursor
        {
            return Err(MemoryError::InvalidResponse);
        }
        checked_add(&mut totals.accepted, imported.accepted)?;
        checked_add(&mut totals.duplicates, imported.duplicates)?;
        checked_add(&mut totals.conflicts, imported.conflicts)?;
        checked_add(&mut totals.objects, envelope.objects.len() as u64)?;
        checked_add(
            &mut totals.tombstones,
            envelope
                .objects
                .values()
                .filter(|value| object_is_tombstone(value))
                .count() as u64,
        )?;
        checked_add(&mut totals.pages, 1)?;
        cursor = envelope.to_cursor;
        if !envelope.has_more {
            complete = true;
            break;
        }
    }
    if !complete {
        return Err(MemoryError::ResponseTooLarge);
    }
    let target_after = readiness(exchange, target, deadline, &mut budget)?;
    if target_after.node_id != target_ready.node_id {
        return Err(MemoryError::NodeIdentityMismatch);
    }
    Ok(ReplicationResult {
        status: "ok".to_string(),
        operation: operation.to_string(),
        source_node_id: source_ready.node_id,
        target_node_id: target_ready.node_id,
        from_cursor,
        to_cursor: cursor,
        accepted: totals.accepted,
        duplicates: totals.duplicates,
        conflicts: totals.conflicts,
        objects: totals.objects,
        tombstones: totals.tombstones,
        pages: totals.pages,
        target_conflict_count: target_after.conflict_count,
        last_success: Utc::now().to_rfc3339(),
    })
}

pub(super) fn replicate_direction(
    operation: &str,
    trusted: &TrustedMemoryConfig,
    tunnel_port: u16,
    timeout: Duration,
) -> Result<ReplicationResult, MemoryError> {
    if tunnel_port == 0 || tunnel_port == trusted.config.local_port {
        return Err(MemoryError::InvalidConfig);
    }
    let local = Node {
        endpoint: format!("http://127.0.0.1:{}", trusted.config.local_port),
        read_token: &trusted.secrets.local_read,
        replicate_token: &trusted.secrets.local_replicate,
        expected_node_id: &trusted.config.local_node_id,
    };
    let remote = Node {
        endpoint: format!("http://127.0.0.1:{tunnel_port}"),
        read_token: &trusted.secrets.remote_read,
        replicate_token: &trusted.secrets.remote_replicate,
        expected_node_id: &trusted.config.home_node_id,
    };
    let mut exchange = HttpJsonExchange::new()?;
    replicate_with_exchange(operation, &local, &remote, timeout, &mut exchange)
}

#[cfg(test)]
#[path = "memory_replication_tests.rs"]
mod tests;
