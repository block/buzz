//! Owner-private lifecycle requests. Signed order is intent, not process state.
mod configuration;
use crate::{
    desktop_stop::{hex, read, sign, StopTarget},
    kind::{KIND_DESKTOP_LIFECYCLE, KIND_DESKTOP_LIFECYCLE_RESULT},
};
pub use configuration::{RuntimeConfigurationRef, RuntimeConfigurationSummary};
use nostr::{Event, Keys, Tag};
use serde::{Deserialize, Serialize};

/// Start chooses a destination. Restart is a current-host-only one-shot.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Explicit ensure-running, without a remote reachability gate.
    Start,
    /// Ordinary Stop then one fresh launch, only on the resolved current host.
    Restart,
    /// Read actual local process status; never starts or stops anything.
    Status,
    /// Read a bounded page of destination-local configuration summaries.
    Catalog,
    /// Check exact launch prerequisites without changing placement or processes.
    Preflight,
}

/// Immutable request; retries retain its exact signed bytes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Existing owner/community/agent/Desktop target shape.
    pub target: StopTarget,
    /// Requested operation, never shell text or configuration.
    pub action: Action,
    /// Restart's fresh successful Status request ID. None for other actions.
    pub observed: Option<String>,
    /// Explicit next-launch revision. Missing legacy refs never authorize new launch.
    #[serde(default)]
    pub configuration: Option<RuntimeConfigurationRef>,
    /// Exclusive catalog cursor; never a launch target or placement intent.
    #[serde(default)]
    pub cursor: Option<String>,
}

/// No credentials, paths, PIDs or raw runtime errors on the wire.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Ordinary process registration/actual status confirms running locally.
    Running,
    /// Actual status confirms no managed process at this target.
    Stopped,
    /// Legacy receiver refusal, retained for wire compatibility. New receivers
    /// use destination-local credentials and report Ineligible when unavailable.
    ProvisioningUnavailable,
    /// Runtime/readiness/ownership rejected the request.
    Failed,
    /// Superseded, interrupted, evicted or uncertain; never success.
    Unknown,
    /// Fresh, positive read-only catalog/preflight response; not launch authority.
    Ready,
    /// Exact configuration is absent, stale, or not currently launchable.
    Ineligible,
    /// A process exists, but it did not launch the requested configuration.
    DifferentConfiguration,
}

/// One summary per response keeps encrypted payloads below the existing 4096-byte bound.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogPage {
    /// None is an authoritative empty final page, not an unavailable host.
    pub entry: Option<RuntimeConfigurationSummary>,
    /// Exclusive configuration ID cursor when more entries remain.
    pub next: Option<String>,
}

/// Safe, short-lived observation. No launch plan, credentials or local paths.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    /// Unix seconds; bounded by the original request's timestamp plus 30 seconds.
    pub valid_until: u64,
    /// Launch-time identity, never inferred from the current selection.
    pub running_configuration: Option<RuntimeConfigurationRef>,
    /// Catalog data appears only on Catalog results.
    pub catalog: Option<CatalogPage>,
}

/// Signed Desktop outcome, not agent-signed termination proof.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResultMessage {
    /// Original immutable payload.
    pub request: Request,
    /// Original signed event identity.
    pub id: String,
    /// Local Desktop result.
    pub outcome: Outcome,
    /// Optional for legacy results, which cannot establish configuration readiness.
    #[serde(default)]
    pub observation: Option<Observation>,
}

/// Public envelope gate before persistence; content remains owner encrypted.
pub fn validate_envelope(event: &Event) -> Result<(), &'static str> {
    let kind = event.kind.as_u16() as u32;
    let tags: Vec<_> = event.tags.iter().map(|t| t.as_slice()).collect();
    let result = kind == KIND_DESKTOP_LIFECYCLE_RESULT;
    if !matches!(kind, KIND_DESKTOP_LIFECYCLE | KIND_DESKTOP_LIFECYCLE_RESULT)
        || !(132..=4096).contains(&event.content.len())
        || tags.len() != if result { 2 } else { 1 }
        || tags[0].len() != 2
        || tags[0][0] != "d"
        || !hex(&tags[0][1], 32)
        || (result && (tags[1].len() != 2 || tags[1][0] != "e" || !hex(&tags[1][1], 64)))
    {
        return Err("invalid Desktop lifecycle envelope");
    }
    Ok(())
}

impl Request {
    /// Validate target, action and correlation without inventing credentials.
    pub fn validate(&self, community: &str) -> Result<(), String> {
        self.target.validate(community)?;
        if let Some(reference) = &self.configuration {
            reference.validate()?;
        }
        if self
            .cursor
            .as_ref()
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_err())
            || (self.action != Action::Catalog && self.cursor.is_some())
            || (matches!(self.action, Action::Catalog | Action::Status)
                && self.configuration.is_some())
            || (self.action == Action::Preflight && self.configuration.is_none())
        {
            return Err("Invalid lifecycle configuration target".into());
        }
        match (self.action, &self.observed) {
            (Action::Restart, Some(id)) if hex(id, 64) => Ok(()),
            (Action::Start | Action::Status | Action::Catalog | Action::Preflight, None) => Ok(()),
            _ => Err("invalid Desktop lifecycle observation".into()),
        }
    }
    /// Prepare once; retries must not create a new event/order.
    pub fn sign(&self, keys: &Keys) -> Result<Event, String> {
        self.validate(&self.target.community)?;
        sign(
            self,
            keys,
            KIND_DESKTOP_LIFECYCLE,
            vec![Tag::identifier(&self.target.desktop)],
        )
    }
    /// Authenticate owner, content, routing and captured community.
    pub fn read(event: &Event, keys: &Keys, community: &str) -> Result<Self, String> {
        let value: Self = read(event, keys, KIND_DESKTOP_LIFECYCLE)?;
        value.validate(community)?;
        if event.tags.identifier() != Some(value.target.desktop.as_str()) {
            return Err("Desktop lifecycle routing mismatch".into());
        }
        Ok(value)
    }
}
impl ResultMessage {
    fn validate_observation(&self, stamp: u64) -> Result<(), String> {
        let fail = || "Invalid lifecycle observation".to_string();
        if self.outcome == Outcome::Ready
            && (!matches!(self.request.action, Action::Catalog | Action::Preflight)
                || self.observation.is_none())
        {
            return Err(fail());
        }
        if let Some(observation) = &self.observation {
            if observation.valid_until <= stamp
                || observation.valid_until > stamp.saturating_add(30)
            {
                return Err(fail());
            }
            if let Some(reference) = &observation.running_configuration {
                reference.validate()?;
                if !matches!(
                    self.outcome,
                    Outcome::Running | Outcome::DifferentConfiguration
                ) {
                    return Err(fail());
                }
            }
            if let Some(page) = &observation.catalog {
                if self.request.action != Action::Catalog || self.outcome != Outcome::Ready {
                    return Err(fail());
                }
                if let Some(entry) = &page.entry {
                    entry.validate(&self.request.target.desktop)?;
                    if self
                        .request
                        .cursor
                        .as_ref()
                        .is_some_and(|cursor| &entry.configuration.id <= cursor)
                    {
                        return Err(fail());
                    }
                }
                if page.next.as_ref().is_some_and(|next| {
                    page.entry.as_ref().map(|e| &e.configuration.id) != Some(next)
                }) {
                    return Err(fail());
                }
            } else if self.request.action == Action::Catalog && self.outcome == Outcome::Ready {
                return Err(fail());
            }
        }
        if self.outcome == Outcome::Running
            && self.request.configuration.is_some()
            && self
                .observation
                .as_ref()
                .and_then(|o| o.running_configuration.as_ref())
                != self.request.configuration.as_ref()
        {
            return Err(fail());
        }
        if matches!(self.request.action, Action::Catalog | Action::Preflight)
            && matches!(
                self.outcome,
                Outcome::Running | Outcome::Stopped | Outcome::DifferentConfiguration
            )
        {
            return Err(fail());
        }
        Ok(())
    }
    /// Sign the actual Desktop result. It is immutable for this request.
    pub fn sign(&self, keys: &Keys) -> Result<Event, String> {
        self.request.validate(&self.request.target.community)?;
        if !hex(&self.id, 64) {
            return Err("invalid lifecycle request ID".into());
        }
        sign(
            self,
            keys,
            KIND_DESKTOP_LIFECYCLE_RESULT,
            vec![
                Tag::identifier(&self.request.target.desktop),
                Tag::parse(["e", &self.id]).map_err(|e| e.to_string())?,
            ],
        )
    }
    /// Bind every correlation field to the original authenticated request.
    pub fn read(
        event: &Event,
        keys: &Keys,
        request: &Event,
        community: &str,
    ) -> Result<Self, String> {
        let original = Request::read(request, keys, community)?;
        let value: Self = read(event, keys, KIND_DESKTOP_LIFECYCLE_RESULT)?;
        value.validate_observation(request.created_at.as_secs())?;
        if value.request != original
            || value.id != request.id.to_hex()
            || event.tags.identifier() != Some(original.target.desktop.as_str())
            || event.tags.iter().nth(1).and_then(|t| t.content()) != Some(value.id.as_str())
        {
            return Err("Desktop lifecycle result mismatch".into());
        }
        Ok(value)
    }
}

#[cfg(test)]
mod protocol_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_action_and_result_are_bound_to_one_signed_request() {
        let keys = Keys::generate();
        let mut request = Request {
            target: StopTarget {
                v: 1,
                community: "wss://one.example".into(),
                desktop: "a".repeat(32),
                agent: Keys::generate().public_key().to_hex(),
            },
            action: Action::Start,
            observed: None,
            configuration: None,
            cursor: None,
        };
        let event = request.sign(&keys).unwrap();
        assert_eq!(
            Request::read(&event, &keys, &request.target.community).unwrap(),
            request
        );
        assert!(Request::read(&event, &Keys::generate(), &request.target.community).is_err());
        assert!(Request::read(&event, &keys, "wss://other.example").is_err());
        assert!(
            crate::desktop_stop::StopTarget::read(&event, &keys, &request.target.community)
                .is_err()
        );
        let result = ResultMessage {
            request: request.clone(),
            id: event.id.to_hex(),
            outcome: Outcome::ProvisioningUnavailable,
            observation: None,
        }
        .sign(&keys)
        .unwrap();
        assert_eq!(
            ResultMessage::read(&result, &keys, &event, &request.target.community)
                .unwrap()
                .outcome,
            Outcome::ProvisioningUnavailable
        );
        let other = request.sign(&keys).unwrap();
        assert!(ResultMessage::read(&result, &keys, &other, &request.target.community).is_err());
        request.action = Action::Restart;
        assert!(request.sign(&keys).is_err());
        request.observed = Some(event.id.to_hex());
        assert!(request.sign(&keys).is_ok());
        request.action = Action::Start;
        assert!(request.sign(&keys).is_err());
    }
}
