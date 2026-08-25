use crate::acp::AcpClient;
use crate::relay::RestClient;
use crate::turn_observer::TurnObserver;
use nostr::Event;
use std::any::Any;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

const PUBLISH_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RESPONSE_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct BusinessResponseObservation {
    pub response_buzz_event_id: Option<String>,
    pub publish_attempted: bool,
    pub publish_succeeded: bool,
    pub finding_count: i32,
    pub resource_ref_count: i32,
    pub duration_ms: i64,
    pub anomaly_tool_used: bool,
}

pub(crate) struct CapturedBusinessResponse {
    pub(crate) text: Option<String>,
    pub(crate) observation: BusinessResponseObservation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BusinessAuditToolKind {
    BusinessResult,
}

#[derive(Default)]
struct BusinessResponseCapture {
    audit_tools: HashMap<String, BusinessAuditToolKind>,
    observation: BusinessResponseObservation,
    message_id: Option<String>,
    text: String,
    too_large: bool,
}

pub(crate) fn start_capture(acp: &mut AcpClient) {
    acp.set_turn_observer(Some(Box::new(BusinessResponseCapture::default())));
}

pub(crate) fn finish_capture(acp: &mut AcpClient) -> Option<CapturedBusinessResponse> {
    let observer = acp.take_turn_observer()?;
    let capture = observer
        .into_any()
        .downcast::<BusinessResponseCapture>()
        .ok()?;
    Some(capture.finish())
}

impl BusinessResponseCapture {
    fn finish(mut self) -> CapturedBusinessResponse {
        let text = if self.too_large {
            None
        } else {
            (!self.text.trim().is_empty()).then_some(std::mem::take(&mut self.text))
        };
        CapturedBusinessResponse {
            text,
            observation: self.observation,
        }
    }

    fn capture_response_chunk(&mut self, update: &serde_json::Value, text: &str) {
        let Some(message_id) = update.get("messageId").and_then(|value| value.as_str()) else {
            return;
        };
        if self.message_id.as_deref() != Some(message_id) {
            self.message_id = Some(message_id.to_owned());
            self.text.clear();
            self.too_large = false;
        }
        if self.too_large {
            return;
        }
        if self.text.len().saturating_add(text.len()) > MAX_RESPONSE_BYTES {
            self.text.clear();
            self.too_large = true;
            return;
        }
        self.text.push_str(text);
    }

    fn track_tool(&mut self, update: &serde_json::Value, title: &str) {
        let Some(tool_id) = update.get("toolCallId").and_then(|value| value.as_str()) else {
            return;
        };
        if !is_business_read_tool(title) {
            return;
        }
        if is_business_anomaly_tool(title) {
            self.observation.anomaly_tool_used = true;
        }
        self.audit_tools
            .insert(tool_id.to_owned(), BusinessAuditToolKind::BusinessResult);
    }

    fn observe_tool_result(&mut self, update: &serde_json::Value, tool_id: &str, status: &str) {
        let Some(BusinessAuditToolKind::BusinessResult) = self.audit_tools.remove(tool_id) else {
            return;
        };
        if status != "completed" {
            return;
        }
        let Some(text) = update
            .pointer("/content/0/content/text")
            .and_then(|value| value.as_str())
        else {
            return;
        };
        let Some((finding_count, resource_ref_count)) = parse_business_result_counts(text) else {
            return;
        };
        self.observation.finding_count = self
            .observation
            .finding_count
            .saturating_add(finding_count)
            .min(100);
        self.observation.resource_ref_count = self
            .observation
            .resource_ref_count
            .saturating_add(resource_ref_count)
            .min(1000);
    }
}

impl TurnObserver for BusinessResponseCapture {
    fn on_session_update(&mut self, update: &serde_json::Value) {
        match update.get("sessionUpdate").and_then(|value| value.as_str()) {
            Some("agent_message_chunk") => {
                if let Some(text) = update
                    .pointer("/content/text")
                    .and_then(|value| value.as_str())
                {
                    self.capture_response_chunk(update, text);
                }
            }
            Some("tool_call") => {
                if let Some(title) = update.get("title").and_then(|value| value.as_str()) {
                    self.track_tool(update, title);
                }
            }
            Some("tool_call_update") => {
                let tool_id = update.get("toolCallId").and_then(|value| value.as_str());
                let status = update.get("status").and_then(|value| value.as_str());
                if let (Some(tool_id), Some(status)) = (tool_id, status) {
                    self.observe_tool_result(update, tool_id, status);
                }
            }
            _ => {}
        }
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}

fn is_business_read_tool(title: &str) -> bool {
    const TOOLS: [&str; 16] = [
        "get_sales_order",
        "search_sales_orders",
        "get_purchase_order",
        "search_purchase_orders",
        "query_inventory_balance",
        "query_receivables",
        "query_payables",
        "query_order_profit",
        "search_business_anomalies",
        "get_business_anomaly",
        "analyze_order_profit_risks",
        "analyze_receivable_risks",
        "analyze_inventory_risks",
        "analyze_purchase_cost_risks",
        "analyze_cross_domain_risks",
        "explain_profit_change",
    ];
    TOOLS
        .iter()
        .any(|tool| title == *tool || title.ends_with(&format!("__{tool}")))
}

fn is_business_anomaly_tool(title: &str) -> bool {
    const TOOLS: [&str; 8] = [
        "search_business_anomalies",
        "get_business_anomaly",
        "analyze_order_profit_risks",
        "analyze_receivable_risks",
        "analyze_inventory_risks",
        "analyze_purchase_cost_risks",
        "analyze_cross_domain_risks",
        "explain_profit_change",
    ];
    TOOLS
        .iter()
        .any(|tool| title == *tool || title.ends_with(&format!("__{tool}")))
}

fn parse_business_result_counts(text: &str) -> Option<(i32, i32)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let findings = value
        .get("findings")
        .and_then(|items| items.as_array())
        .map_or(0, Vec::len);
    let top_level_refs = value
        .get("resourceRefs")
        .and_then(|items| items.as_array())
        .map_or(0, Vec::len);
    let finding_refs = value
        .get("findings")
        .and_then(|items| items.as_array())
        .into_iter()
        .flatten()
        .map(|finding| {
            1 + finding
                .get("relatedResources")
                .and_then(|items| items.as_array())
                .map_or(0, Vec::len)
        })
        .sum::<usize>();
    Some((
        i32::try_from(findings).ok()?.min(100),
        i32::try_from(top_level_refs.saturating_add(finding_refs))
            .ok()?
            .min(1000),
    ))
}

pub(crate) async fn publish(
    rest: &RestClient,
    channel_id: Uuid,
    source_event: &Event,
    content: &str,
    mut observation: BusinessResponseObservation,
) -> BusinessResponseObservation {
    observation.publish_attempted = true;
    let started = Instant::now();
    let event = match build_event(rest, channel_id, source_event, content) {
        Ok(event) => event,
        Err(error) => {
            tracing::warn!(channel = %channel_id, "Business Agent response build failed: {error}");
            return observation;
        }
    };
    let expected_id = event.id.to_hex();
    let response = tokio::time::timeout(PUBLISH_TIMEOUT, rest.submit_event(&event)).await;
    observation.duration_ms = started.elapsed().as_millis().min(120_000) as i64;
    match response {
        Ok(Ok(value))
            if value.get("accepted").and_then(|item| item.as_bool()) == Some(true)
                && value.get("event_id").and_then(|item| item.as_str())
                    == Some(expected_id.as_str()) =>
        {
            observation.publish_succeeded = true;
            observation.response_buzz_event_id = Some(expected_id);
        }
        Ok(Ok(_)) => {
            tracing::warn!(channel = %channel_id, "Business Agent response was not accepted")
        }
        Ok(Err(error)) => {
            tracing::warn!(channel = %channel_id, "Business Agent response publish failed: {error}")
        }
        Err(_) => {
            tracing::warn!(channel = %channel_id, "Business Agent response publish timed out")
        }
    }
    observation
}

fn build_event(
    rest: &RestClient,
    channel_id: Uuid,
    source_event: &Event,
    content: &str,
) -> Result<Event, String> {
    let parsed = crate::queue::parse_thread_tags(source_event);
    let root_id = parsed
        .root_event_id
        .as_deref()
        .and_then(|value| nostr::EventId::from_hex(value).ok())
        .unwrap_or(source_event.id);
    let thread_ref = buzz_sdk::ThreadRef {
        root_event_id: root_id,
        parent_event_id: source_event.id,
    };
    buzz_sdk::build_message(channel_id, content, Some(&thread_ref), &[], false, &[])
        .map_err(|error| error.to_string())?
        .sign_with_keys(&rest.keys)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn response_audit_counts_findings_and_server_links() {
        let result = serde_json::json!({
            "findings": [
                {"primaryResource": {"bizUri":"biz://sales-order/SO-1"}, "relatedResources":[{"bizUri":"biz://customer/C1"}]},
                {"primaryResource": {"bizUri":"biz://inventory/SKU-1"}, "relatedResources":[]}
            ]
        });
        assert_eq!(
            parse_business_result_counts(&result.to_string()),
            Some((2, 3))
        );
        assert!(is_business_read_tool(
            "business-read-mcp__analyze_cross_domain_risks"
        ));
        assert!(!is_business_read_tool("dev__shell"));
    }

    #[test]
    fn response_is_signed_as_a_reply_to_the_trusted_source_event() {
        let source_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let channel_id = Uuid::new_v4();
        let source = EventBuilder::new(Kind::Custom(9), "question")
            .tags([Tag::parse(["h", &channel_id.to_string()]).unwrap()])
            .sign_with_keys(&source_keys)
            .unwrap();
        let rest = RestClient {
            http: reqwest::Client::new(),
            base_url: "http://127.0.0.1:1".into(),
            keys: agent_keys.clone(),
            auth_tag_json: None,
        };

        let response = build_event(&rest, channel_id, &source, "answer").unwrap();

        assert_eq!(response.pubkey, agent_keys.public_key());
        assert_eq!(response.content, "answer");
        let expected_channel = channel_id.to_string();
        let expected_source = source.id.to_hex();
        let tags = serde_json::to_value(response.tags).unwrap();
        assert!(tags.as_array().unwrap().iter().any(|tag| {
            tag.as_array()
                .and_then(|tag| tag.get(1))
                .and_then(|v| v.as_str())
                == Some(expected_channel.as_str())
        }));
        assert_eq!(
            tags.as_array()
                .unwrap()
                .iter()
                .filter(|tag| tag.get(0).and_then(|v| v.as_str()) == Some("e"))
                .count(),
            1
        );
        assert!(tags.as_array().unwrap().iter().any(|tag| {
            tag.as_array()
                .and_then(|tag| tag.get(1))
                .and_then(|v| v.as_str())
                == Some(expected_source.as_str())
        }));
    }

    #[tokio::test]
    async fn accepted_http_publish_records_the_real_event_id_only() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let body = loop {
                let mut chunk = [0u8; 4096];
                let read = stream.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                request.extend_from_slice(&chunk[..read]);
                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|value| value.parse::<usize>().ok())
                    })
                    .unwrap();
                let body_start = header_end + 4;
                if request.len() >= body_start + length {
                    break request[body_start..body_start + length].to_vec();
                }
            };
            let event: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let event_id = event["id"].as_str().unwrap();
            let response_body = serde_json::json!({
                "event_id": event_id,
                "accepted": true,
                "message": "saved"
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(), response_body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let source = EventBuilder::new(Kind::Custom(9), "question")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let rest = RestClient {
            http: reqwest::Client::new(),
            base_url: format!("http://{address}"),
            keys: Keys::generate(),
            auth_tag_json: None,
        };

        let observation = publish(
            &rest,
            Uuid::new_v4(),
            &source,
            "bounded final answer",
            BusinessResponseObservation::default(),
        )
        .await;

        server.await.unwrap();
        assert!(observation.publish_attempted);
        assert!(observation.publish_succeeded);
        assert_eq!(observation.response_buzz_event_id.unwrap().len(), 64);
    }
}
