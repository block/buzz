//! Owner-requested native effort changes at a response boundary. No turn is
//! cancelled or replayed: the exact ACP session is edited while its worker is
//! idle, before the next claim. Runtime overrides never become saved defaults.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::acp::AcpError;
use crate::observer::{ObserverContext, ObserverHandle};
use crate::pool::{AgentPool, SessionState};
use crate::scope::SessionScope;

const CAPACITY: usize = 32;
const RECEIPTS: usize = 256;
const EXPIRY: Duration = Duration::from_secs(300);
const APPLY_TIMEOUT: Duration = Duration::from_secs(5);
const CONFIG_CAPACITY: usize = 128;
const MAX_CONFIG_BYTES: usize = 256 * 1024;

impl SessionState {
    /// Bound native snapshots independently of the adapter's session lifetime.
    /// A session without retained configuration cannot advertise live control.
    pub(crate) fn remember_effort_config(&mut self, scope: &SessionScope, config: &mut Value) {
        config["liveEffortSwitching"] = json!(true);
        let retain = (self.configs.contains_key(scope) || self.configs.len() < CONFIG_CAPACITY)
            && serde_json::to_vec(config).is_ok_and(|bytes| bytes.len() <= MAX_CONFIG_BYTES);
        config["liveEffortSwitching"] = json!(retain);
        if retain {
            self.configs.insert(scope.clone(), config.clone());
        } else {
            self.configs.remove(scope);
        }
    }
}

#[cfg(test)]
#[path = "live_effort_tests.rs"]
mod tests;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    channel_id: Uuid,
    session_id: String,
    session_token: Uuid,
    effort: String,
    request_id: Uuid,
}

struct Pending {
    request: Request,
    received: Instant,
}

#[derive(Default)]
pub(crate) struct LiveEffortQueue {
    pending: VecDeque<Pending>,
    // Never reapply a reconnect replay, including after the first edit settled.
    receipts: VecDeque<(Request, &'static str, Instant)>,
}

impl LiveEffortQueue {
    pub(crate) fn has_session(&self, channel: Uuid, session: &str) -> bool {
        self.pending
            .iter()
            .any(|p| p.request.channel_id == channel && p.request.session_id == session)
    }

    fn finish(
        &mut self,
        request: &Request,
        status: &'static str,
        observer: Option<&ObserverHandle>,
    ) {
        if let Some((_, previous, _)) = self
            .receipts
            .iter_mut()
            .find(|(r, _, _)| r.request_id == request.request_id)
        {
            *previous = status;
        }
        emit_result(request, status, observer);
    }
}

fn context(request: &Request) -> ObserverContext {
    ObserverContext {
        channel_id: Some(request.channel_id.to_string()),
        session_id: Some(request.session_id.clone()),
        turn_id: None,
        started_at: None,
    }
}

fn emit_result(request: &Request, status: &str, observer: Option<&ObserverHandle>) {
    if let Some(observer) = observer {
        observer.emit(
            "control_result",
            None,
            &context(request),
            json!({
                "type": "switch_effort", "requestId": request.request_id,
                "sessionId": request.session_id, "sessionToken": request.session_token, "effort": request.effort, "status": status,
            }),
        );
    }
}

fn effort_option(config: &Value) -> Option<&Value> {
    config
        .get("configOptions")?
        .as_array()?
        .iter()
        .find(|option| {
            option.get("category").and_then(Value::as_str) == Some("thought_level")
                && option.get("type").and_then(Value::as_str) == Some("select")
        })
}

fn supports_value(options: &Value, desired: &str) -> bool {
    options.as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item.get("value").and_then(Value::as_str) == Some(desired)
                || item
                    .get("options")
                    .is_some_and(|group| supports_value(group, desired))
        })
    })
}

impl AgentPool {
    pub(crate) fn queue_live_effort(&mut self, payload: &Value, observer: Option<&ObserverHandle>) {
        let Ok(request) = serde_json::from_value::<Request>(payload.clone()) else {
            return;
        };
        if request.session_id.is_empty()
            || request.session_id.len() > 256
            || request.effort.is_empty()
            || request.effort.len() > 128
            || request
                .session_id
                .chars()
                .chain(request.effort.chars())
                .any(char::is_control)
        {
            emit_result(&request, "invalid_request", observer);
            return;
        }
        self.live_effort
            .receipts
            .retain(|(_, _, at)| at.elapsed() <= EXPIRY * 2);
        if let Some((original, status, _)) = self
            .live_effort
            .receipts
            .iter()
            .find(|(r, _, _)| r.request_id == request.request_id)
        {
            emit_result(original, status, observer);
            return;
        }
        // Keep all receipts for the full replay window; capacity is backpressure,
        // not eviction that would allow an old request to execute a second time.
        if self.live_effort.pending.len() >= CAPACITY || self.live_effort.receipts.len() >= RECEIPTS
        {
            emit_result(&request, "busy", observer);
            return;
        }
        // One outstanding edit per exact session. Later requests must wait for
        // its receipt rather than silently replacing an already queued choice.
        if self.live_effort.pending.iter().any(|p| {
            p.request.channel_id == request.channel_id && p.request.session_id == request.session_id
        }) {
            emit_result(&request, "busy", observer);
            return;
        }
        self.live_effort
            .receipts
            .push_back((request.clone(), "queued", Instant::now()));
        self.live_effort.pending.push_back(Pending {
            request: request.clone(),
            received: Instant::now(),
        });
        emit_result(&request, "queued", observer);
    }

    /// At most one native RPC per main-loop iteration. Bounds control latency
    /// and avoids holding the relay loop for N sequential adapter timeouts.
    pub(crate) async fn apply_pending_effort(&mut self, observer: Option<&ObserverHandle>) {
        let count = self.live_effort.pending.len();
        for _ in 0..count {
            let Some(pending) = self.live_effort.pending.pop_front() else {
                return;
            };
            let request = &pending.request;
            if pending.received.elapsed() > EXPIRY {
                self.live_effort.finish(request, "expired", observer);
                continue;
            }
            // A busy worker's session IDs are not visible here. Wait until all
            // possible owners return before resolving the target: adapters may
            // use process-local IDs shared by siblings in the same channel.
            if self.effort_target_is_busy(request.channel_id) {
                self.live_effort.pending.push_back(pending);
                continue;
            }
            let mut targets = Vec::new();
            for (index, slot) in self.agents_mut().iter().enumerate() {
                if let Some(agent) = slot {
                    for (scope, id) in &agent.state.sessions {
                        if scope.channel_id() == request.channel_id && id == &request.session_id {
                            targets.push((index, scope.clone()));
                        }
                    }
                }
            }
            if targets.len() > 1 {
                // Some adapters use process-local session IDs. Never pick an
                // arbitrary worker if the reported channel/session is ambiguous.
                self.live_effort.finish(request, "unavailable", observer);
                continue;
            }
            let target = targets.pop();
            let Some((index, scope)) = target else {
                self.live_effort.finish(request, "stale_session", observer);
                continue;
            };
            let Some(agent) = self.agents_mut()[index].as_mut() else {
                continue;
            };
            let Some(mut config) = agent.state.configs.get(&scope).cloned() else {
                self.live_effort.finish(request, "unavailable", observer);
                continue;
            };
            if config.get("effortSessionToken").and_then(Value::as_str)
                != Some(request.session_token.to_string().as_str())
            {
                self.live_effort.finish(request, "stale_session", observer);
                continue;
            }
            let Some(option) = effort_option(&config) else {
                self.live_effort.finish(request, "unsupported", observer);
                continue;
            };
            let Some(config_id) = option
                .get("configId")
                .or_else(|| option.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                self.live_effort.finish(request, "unavailable", observer);
                continue;
            };
            if !option
                .get("options")
                .is_some_and(|options| supports_value(options, &request.effort))
            {
                self.live_effort.finish(request, "unsupported", observer);
                continue;
            }
            let result = tokio::time::timeout(
                APPLY_TIMEOUT,
                agent.acp.session_set_config_option(
                    &request.session_id,
                    &config_id,
                    &request.effort,
                ),
            )
            .await;
            let (status, retire) = match result {
                Ok(Ok(response)) => {
                    if let Some(options) = response.get("configOptions").filter(|v| v.is_array()) {
                        config["configOptions"] = options.clone();
                        let confirmed = effort_option(&config)
                            .and_then(|o| o.get("currentValue"))
                            .and_then(Value::as_str)
                            == Some(request.effort.as_str());
                        agent.state.remember_effort_config(&scope, &mut config);
                        if let Some(observer) = observer {
                            observer.emit(
                                "session_config_captured",
                                Some(index),
                                &context(request),
                                config,
                            );
                        }
                        (if confirmed { "applied" } else { "unconfirmed" }, false)
                    } else {
                        ("unconfirmed", false)
                    }
                }
                Ok(Err(AcpError::AgentError { .. })) => ("rejected", false),
                Ok(Err(_)) | Err(_) => ("unconfirmed", true),
            };
            if retire {
                // A timed-out/poisoned stream must not serve another turn. The
                // existing pool maintenance replaces this slot; never fabricate
                // successful application or replay this edit into its successor.
                self.agents_mut()[index].take();
            }
            self.live_effort.finish(request, status, observer);
            return;
        }
    }
}

#[cfg(test)]
#[path = "live_effort_real_test.rs"]
mod real_test;
