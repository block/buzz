use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use bytes::Bytes;
use nostr::{Event, EventBuilder, JsonUtil, Kind, Tag};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

use crate::{
    AcceptedDelivery, BuzzClient, ClientError, DeliveryOutcome, DeliveryUnknown, RejectedDelivery,
};

const QUERY_PAGE_SIZE: u32 = 500;

impl BuzzClient {
    pub(crate) async fn sign_builder(&self, builder: EventBuilder) -> Result<Event, ClientError> {
        let builder = match self.auth_context().nip_oa_tag() {
            Some(tag) => builder.tags([tag.clone()]),
            None => builder,
        };
        let unsigned = builder.build(self.public_key());
        let event = self
            .signer
            .sign_event(unsigned)
            .await
            .map_err(ClientError::Signer)?;
        let auth_count = event
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("auth"))
            .count();
        let expected = usize::from(self.auth_context().has_nip_oa());
        if auth_count != expected {
            return Err(ClientError::InvalidInput(format!(
                "signed event contains {auth_count} ambient auth tags; expected {expected}"
            )));
        }
        Ok(event)
    }

    pub(crate) async fn query_paginated(
        &self,
        mut filter: serde_json::Value,
        limit: u32,
    ) -> Result<Vec<serde_json::Value>, ClientError> {
        let mut events = Vec::new();
        let mut seen = HashSet::new();
        while events.len() < limit as usize {
            let page_limit = (limit as usize - events.len()).min(QUERY_PAGE_SIZE as usize);
            filter["limit"] = serde_json::json!(page_limit);
            let page = self.query_filters(&[filter.clone()]).await?;
            let full_page = page.len() == page_limit;
            if full_page {
                advance_query_cursor(&mut filter, &page)?;
            }
            for event in page {
                let id = event
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                if id.as_ref().is_none_or(|id| seen.insert(id.clone())) {
                    events.push(event);
                    if events.len() == limit as usize {
                        break;
                    }
                }
            }
            if !full_page {
                break;
            }
        }
        Ok(events)
    }

    pub(crate) async fn query_filters(
        &self,
        filters: &[serde_json::Value],
    ) -> Result<Vec<serde_json::Value>, ClientError> {
        let body = Bytes::from(serde_json::to_vec(filters).map_err(ClientError::Serialization)?);
        let url = self.endpoint().query_url();
        let attempts = self.config().retry_policy().max_attempts();
        for attempt in 0..attempts {
            let auth = self.nip98_header("POST", url.as_str(), Some(&body)).await?;
            let mut request = self
                .http
                .post(url.clone())
                .header("Authorization", auth)
                .header("Content-Type", "application/json")
                .body(body.clone());
            if let Some(ambient) = self.auth.ambient() {
                request = request.header("x-auth-tag", &ambient.json);
            }
            match request.send().await {
                Err(source) if attempt + 1 < attempts && is_retryable_network(&source) => {
                    self.sleep_before_retry(attempt).await;
                }
                Err(source) => return Err(ClientError::Network(source)),
                Ok(response) => {
                    let status = response.status().as_u16();
                    let text = match response.text().await {
                        Ok(text) => text,
                        Err(source) if attempt + 1 < attempts && is_retryable_network(&source) => {
                            self.sleep_before_retry(attempt).await;
                            continue;
                        }
                        Err(source) => return Err(ClientError::Network(source)),
                    };
                    if (200..300).contains(&status) {
                        return serde_json::from_str(&text).map_err(ClientError::Serialization);
                    }
                    if attempt + 1 < attempts && is_retryable_status(status) {
                        self.sleep_before_retry(attempt).await;
                        continue;
                    }
                    return Err(ClientError::Relay {
                        status,
                        reason: relay_reason(&text),
                        retry_safe: is_retryable_status(status),
                    });
                }
            }
        }
        Err(ClientError::Configuration(
            "retry policy allowed no query attempts".into(),
        ))
    }

    pub(crate) async fn submit_stored_event(
        &self,
        event: Event,
    ) -> Result<DeliveryOutcome, ClientError> {
        let event_id = event.id;
        let body = Bytes::from(serde_json::to_vec(&event).map_err(ClientError::Serialization)?);
        let url = self.endpoint().events_url();
        let attempts = self.config().retry_policy().max_attempts();
        for attempt in 0..attempts {
            let auth = self.nip98_header("POST", url.as_str(), Some(&body)).await?;
            let mut request = self
                .http
                .post(url.clone())
                .header("Authorization", auth)
                .header("Content-Type", "application/json")
                .body(body.clone());
            if let Some(ambient) = self.auth.ambient() {
                request = request.header("x-auth-tag", &ambient.json);
            }
            let response = match request.send().await {
                Err(source) if attempt + 1 < attempts && is_retryable_network(&source) => {
                    self.sleep_before_retry(attempt).await;
                    continue;
                }
                Err(source) if source.is_connect() => return Err(ClientError::Network(source)),
                Err(source) => {
                    return Ok(DeliveryOutcome::Unknown(DeliveryUnknown {
                        event_id,
                        reason: source.to_string(),
                    }));
                }
                Ok(response) => response,
            };

            let status = response.status().as_u16();
            let text = match response.text().await {
                Ok(text) => text,
                Err(source) if attempt + 1 < attempts && is_retryable_network(&source) => {
                    self.sleep_before_retry(attempt).await;
                    continue;
                }
                Err(source) => {
                    return Ok(DeliveryOutcome::Unknown(DeliveryUnknown {
                        event_id,
                        reason: source.to_string(),
                    }));
                }
            };
            if (200..300).contains(&status) {
                let parsed: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(parsed) => parsed,
                    Err(source) => {
                        return Ok(DeliveryOutcome::Unknown(DeliveryUnknown {
                            event_id,
                            reason: format!("invalid relay success response: {source}"),
                        }));
                    }
                };
                let accepted = parsed
                    .get("accepted")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let message = parsed
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                return if accepted {
                    Ok(DeliveryOutcome::Accepted(AcceptedDelivery {
                        event_id,
                        relay_message: message,
                    }))
                } else {
                    Ok(DeliveryOutcome::Rejected(RejectedDelivery {
                        event_id,
                        status,
                        reason: message,
                        retry_safe: false,
                    }))
                };
            }
            if attempt + 1 < attempts && is_retryable_status(status) {
                self.sleep_before_retry(attempt).await;
                continue;
            }
            if matches!(status, 502..=504) {
                return Ok(DeliveryOutcome::Unknown(DeliveryUnknown {
                    event_id,
                    reason: format!("HTTP {status}: {}", relay_reason(&text)),
                }));
            }
            return Ok(DeliveryOutcome::Rejected(RejectedDelivery {
                event_id,
                status,
                reason: relay_reason(&text),
                retry_safe: status == 429,
            }));
        }
        Err(ClientError::Configuration(
            "retry policy allowed no submission attempts".into(),
        ))
    }

    async fn nip98_header(
        &self,
        method: &str,
        url: &str,
        body: Option<&[u8]>,
    ) -> Result<String, ClientError> {
        let nonce = uuid::Uuid::new_v4().to_string();
        let mut tags = vec![
            parse_tag(["u", url])?,
            parse_tag(["method", method])?,
            parse_tag(["nonce", nonce.as_str()])?,
        ];
        if let Some(body) = body {
            let hash = hex::encode(Sha256::digest(body));
            tags.push(parse_tag(["payload", hash.as_str()])?);
        }
        let unsigned = EventBuilder::new(Kind::Custom(27235), "")
            .tags(tags)
            .build(self.public_key());
        let event = self
            .signer
            .sign_event(unsigned)
            .await
            .map_err(ClientError::Signer)?;
        Ok(format!(
            "Nostr {}",
            BASE64_STANDARD.encode(event.as_json().as_bytes())
        ))
    }

    pub(crate) async fn sleep_before_retry(&self, attempt: u32) {
        let retry = self.config().retry_policy();
        let multiplier = 1_u32.checked_shl(attempt).unwrap_or(u32::MAX);
        let delay = retry
            .base_delay()
            .saturating_mul(multiplier)
            .min(retry.max_delay());
        tokio::time::sleep(delay).await;
    }
}

fn parse_tag<const N: usize>(parts: [&str; N]) -> Result<Tag, ClientError> {
    Tag::parse(parts).map_err(|error| ClientError::InvalidInput(error.to_string()))
}

fn advance_query_cursor(
    filter: &mut serde_json::Value,
    page: &[serde_json::Value],
) -> Result<(), ClientError> {
    let last = page
        .last()
        .ok_or_else(|| ClientError::InvalidInput("full query page is empty".into()))?;
    let created_at = last
        .get("created_at")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| ClientError::InvalidInput("query event missing created_at".into()))?;
    let id = last
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| id.len() == 64 && id.chars().all(|character| character.is_ascii_hexdigit()))
        .ok_or_else(|| ClientError::InvalidInput("query event missing valid id".into()))?;
    filter["until"] = serde_json::json!(created_at);
    filter["before_id"] = serde_json::json!(id);
    Ok(())
}

fn is_retryable_network(error: &reqwest::Error) -> bool {
    error.is_connect()
        || error.is_timeout()
        || error.is_request()
        || error.is_body()
        || error.is_decode()
}

fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
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
