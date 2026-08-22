//! Wiring the broker into the running Desktop app.
//!
//! Everything above this file is testable without Tauri. This is where the
//! host's real parts get attached: the owner's keys, the agent roster, the agent
//! commands, the on-disk execution log, and the relay.
//!
//! # The command is a transport hand-off, not an authority hand-off
//!
//! [`broker_handle_observer_frame`] takes a raw signed event and nothing else.
//! The frontend cannot tell it who is asking, which owner to act as, or whether
//! a request is authorized — those come from the frame's signature and from
//! `AppState`. All the frontend does is forward bytes it already received on its
//! observer subscription, and it learns only whether the frame was broker
//! traffic. That keeps the trust boundary next to the credentials while reusing
//! the subscription that already exists.

use std::path::PathBuf;

use nostr::{Event, JsonUtil, Keys};
use tauri::{AppHandle, Manager, State};

use crate::app_state::AppState;

use super::agents_policy::AgentsAuthorizer;
use super::desktop_agents::{DesktopAgentRoster, DesktopAgentService};
use super::handlers::AgentsHandlers;
use super::ingress::{handle_frame, FrameRejection, Handled, ResultPublisher};
use super::pipeline::{BoxFuture, BrokerContext};
use super::store::{scoped_broker_db_path, SqliteExecutionLog};

/// Seconds allowed for a result frame to reach the relay and be acknowledged.
const RESULT_PUBLISH_TIMEOUT_SECS: u64 = 30;

/// Publishes result frames over the relay WebSocket.
///
/// Kind-24200 frames are only accepted on the relay's WebSocket path — the HTTP
/// bridge that every other Desktop publish uses rejects the kind outright — so
/// this opens a short-lived authenticated connection per result rather than
/// reusing `relay::submit_event`. Results are rare (one per brokered mutation),
/// which is what makes a per-result connection acceptable; a busier capability
/// would want a shared session instead.
pub struct WsResultPublisher {
    relay_url: String,
    keys: Keys,
}

impl WsResultPublisher {
    /// Publish as `keys` against `relay_url`.
    pub fn new(relay_url: String, keys: Keys) -> Self {
        Self { relay_url, keys }
    }
}

impl ResultPublisher for WsResultPublisher {
    fn publish(&self, event: Event) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            buzz_ws_client_pkg::publish_event(
                &self.relay_url,
                event,
                &self.keys,
                // No NIP-OA auth tag: this publishes as the owner's own
                // identity, which the relay authenticates directly. An auth tag
                // is how a *managed agent* proves owner backing, and the owner
                // is not one.
                None,
                RESULT_PUBLISH_TIMEOUT_SECS,
            )
            .await
            .map_err(|error| format!("failed to publish broker result: {error}"))?;
            Ok(())
        })
    }
}

/// What the caller learns about a forwarded frame.
///
/// Deliberately thin. A requester's outcome travels to the requester in a signed
/// result frame; telling the forwarding renderer what a brokered operation did
/// would make the reply path look like an IPC return value when it is not.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameHandled {
    /// True when the frame was a broker request this host executed.
    pub broker_request: bool,
    /// Correlation id, when there was a parseable one.
    pub request_id: Option<String>,
    /// True when the result frame reached the relay.
    ///
    /// `false` with `broker_request: true` means the operation ran and its
    /// outcome is recorded, but the requester did not hear it. A retry with the
    /// same `requestId` replays the recorded outcome rather than re-running.
    pub result_delivered: bool,
}

/// Resolve the execution log path for this owner and relay.
fn execution_log(app: &AppHandle, relay_url: &str, owner_pubkey: &str) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data dir: {error}"))?;
    Ok(scoped_broker_db_path(&base, relay_url, owner_pubkey))
}

/// Execute a broker request carried by an inbound observer frame.
///
/// `event_json` is a signed relay event the caller received on its observer
/// subscription. Frames that are not broker requests are ignored cheaply; that
/// is the common case, since ordinary agent telemetry arrives here too.
///
/// # Errors
///
/// Returns an error for a host fault: unavailable owner keys, an unreadable
/// execution log, or an event that will not parse. A frame that fails
/// verification is not an error — it is reported as `brokerRequest: false`,
/// because a signed refusal addressed at an unverified sender is exactly what
/// this must not emit.
#[tauri::command]
pub async fn broker_handle_observer_frame(
    event_json: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<FrameHandled, String> {
    let event = Event::from_json(&event_json)
        .map_err(|error| format!("invalid observer event: {error}"))?;
    let keys = state.signing_keys()?;
    let relay_url = crate::relay::relay_ws_url_with_override(&state);
    let owner_pubkey = keys.public_key().to_hex();

    let log = SqliteExecutionLog::new(execution_log(&app, &relay_url, &owner_pubkey)?);
    let authorizer = AgentsAuthorizer::new(DesktopAgentRoster::new(app.clone()));
    let handlers = AgentsHandlers::new(DesktopAgentService::new(app.clone()));
    let registry = handlers.registry();
    let publisher = WsResultPublisher::new(relay_url.clone(), keys.clone());
    let now = chrono::Utc::now().timestamp();

    let handled = handle_frame(
        &keys,
        &event,
        &relay_url,
        now,
        BrokerContext {
            authorizer: &authorizer,
            registry: &registry,
            log: &log,
            now,
        },
        &publisher,
    )
    .await?;

    Ok(match handled {
        Handled::Ignored(rejection) => {
            if let FrameRejection::Rejected(message) = &rejection {
                // A frame that claimed to be broker traffic and failed
                // verification is worth seeing; ordinary telemetry is not.
                eprintln!("buzz-desktop: broker: refused an inbound frame: {message}");
            }
            FrameHandled {
                broker_request: false,
                request_id: None,
                result_delivered: false,
            }
        }
        Handled::Executed { response, delivery } => {
            if let Err(error) = &delivery {
                eprintln!(
                    "buzz-desktop: broker: result for {} was not delivered: {error}",
                    response.request_id
                );
            }
            FrameHandled {
                broker_request: true,
                request_id: Some(response.request_id.clone()),
                result_delivered: delivery.is_ok(),
            }
        }
    })
}
