//! Retryable projection of authoritative Buzz experience events into Memory MCP.

use std::future::Future;

use buzz_command_sources::mcp_http::{McpHttpClient, McpHttpError};
use serde_json::Value;

use crate::experience_outbox::ExperienceOutbox;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProjectionReport {
    pub projected: usize,
    pub delayed: usize,
    pub poisoned: usize,
}

pub(crate) struct ExperienceProjector {
    client: McpHttpClient,
}

impl ExperienceProjector {
    pub(crate) fn from_endpoint(endpoint: &str) -> Result<Self, McpHttpError> {
        let endpoint = url::Url::parse(endpoint).map_err(|_| McpHttpError::InvalidEndpoint)?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(McpHttpError::InvalidEndpoint);
        }
        Ok(Self {
            client: McpHttpClient::new(endpoint)?,
        })
    }

    pub(crate) async fn project_pending(&self, outbox: &ExperienceOutbox) -> ProjectionReport {
        project_pending_with(outbox, |arguments| async move {
            self.client
                .call_tool("record_projected_event", arguments)
                .await
                .map(|_| ())
        })
        .await
    }
}

pub(crate) async fn project_pending_with<F, Fut, E>(
    outbox: &ExperienceOutbox,
    mut project: F,
) -> ProjectionReport
where
    F: FnMut(Value) -> Fut,
    Fut: Future<Output = Result<(), E>>,
{
    let entries = match outbox.ready_for_projection() {
        Ok(entries) => entries,
        Err(_) => {
            return ProjectionReport {
                delayed: 1,
                ..ProjectionReport::default()
            };
        }
    };
    let mut report = ProjectionReport::default();
    for entry in entries {
        if !valid_projection(&entry.projection_payload, &entry.signed_event.id.to_hex()) {
            report.poisoned += 1;
            continue;
        }
        match project(entry.projection_payload).await {
            Ok(()) => match outbox.mark_projected(&entry.record_id) {
                Ok(()) => report.projected += 1,
                Err(_) => report.delayed += 1,
            },
            Err(_) => report.delayed += 1,
        }
    }
    report
}

fn valid_projection(value: &Value, signed_event_id: &str) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.get("source_event_id").and_then(Value::as_str) == Some(signed_event_id)
        && object
            .get("timestamp")
            .and_then(Value::as_str)
            .is_some_and(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok())
        && object
            .get("event_type")
            .and_then(Value::as_str)
            .is_some_and(|value| value == "command_experience")
        && object.get("content").and_then(Value::as_str).is_some()
        && object.get("metadata").and_then(Value::as_object).is_some()
}
