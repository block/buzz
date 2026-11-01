//! Huddle lifecycle reconstruction.
//!
//! A port of desktop's `HuddlePresenceTracker`
//! (`desktop/src/features/huddle/lib/huddlePresence.ts`) so the CLI and the app
//! agree on who is in a huddle and whether it ended. Keep the two in step; the
//! unit tests replay recorded sequences that desktop's own tracker also accepts.
//!
//! The CLI reads over HTTP, where the relay's synthetic liveness snapshots
//! (kind 48104) are not served, so generations are never authoritative here.

use std::collections::{HashMap, VecDeque};

use buzz_core::kind::{
    KIND_HUDDLE_ENDED, KIND_HUDDLE_PARTICIPANT_JOINED, KIND_HUDDLE_PARTICIPANT_LEFT,
    KIND_HUDDLE_STARTED,
};
use serde_json::Value;

use crate::error::CliError;

/// Departed admissions remembered per session before older ones are compacted.
const MAX_DEPARTED_ADMISSIONS: usize = 1_000;

/// Largest integer JavaScript represents exactly (`Number.MAX_SAFE_INTEGER`).
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// One huddle lifecycle event, reduced to the fields the tracker reads.
#[derive(Debug, Clone)]
pub(crate) struct LifecycleEvent {
    pub id: String,
    pub kind: u32,
    pub pubkey: String,
    pub created_at: u64,
    participant: Option<String>,
    session: Option<String>,
    roster_revision: Option<u64>,
    admission_id: Option<String>,
    generation: Option<String>,
}

impl LifecycleEvent {
    /// Reduce a relay event JSON object. Content that is not the expected
    /// JSON yields an event with no session, which the tracker ignores.
    pub(crate) fn parse(event: &Value) -> Result<Self, CliError> {
        let field = |name: &str| {
            event
                .get(name)
                .ok_or_else(|| CliError::Other(format!("relay event missing {name}")))
        };
        let id = field("id")?
            .as_str()
            .ok_or_else(|| CliError::Other("relay event id is not a string".into()))?
            .to_string();
        let kind = field("kind")?
            .as_u64()
            .and_then(|kind| u32::try_from(kind).ok())
            .ok_or_else(|| CliError::Other(format!("relay event {id} has an invalid kind")))?;
        let pubkey = normalize_pubkey(
            field("pubkey")?
                .as_str()
                .ok_or_else(|| CliError::Other(format!("relay event {id} has no pubkey")))?,
        );
        let created_at = field("created_at")?.as_u64().ok_or_else(|| {
            CliError::Other(format!("relay event {id} has an invalid created_at"))
        })?;
        let tags = field("tags")?
            .as_array()
            .ok_or_else(|| CliError::Other(format!("relay event {id} tags are not an array")))?;
        let participant = tags
            .iter()
            .find(|tag| tag.get(0).and_then(Value::as_str) == Some("p"))
            .and_then(|tag| tag.get(1).and_then(Value::as_str))
            .map(normalize_pubkey)
            .or_else(|| (!pubkey.is_empty()).then(|| pubkey.clone()))
            .filter(|participant| !participant.is_empty());

        let content = event
            .get("content")
            .and_then(Value::as_str)
            .and_then(|content| serde_json::from_str::<Value>(content).ok())
            .unwrap_or(Value::Null);
        let text = |name: &str| {
            content
                .get(name)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        Ok(Self {
            id,
            kind,
            pubkey,
            created_at,
            participant,
            session: text("ephemeral_channel_id"),
            roster_revision: content.get("roster_revision").and_then(safe_integer),
            admission_id: text("admission_id"),
            generation: text("generation"),
        })
    }
}

fn normalize_pubkey(value: &str) -> String {
    value.trim().to_lowercase()
}

fn safe_integer(value: &Value) -> Option<u64> {
    if let Some(n) = value.as_u64() {
        return (n <= MAX_SAFE_INTEGER).then_some(n);
    }
    let f = value.as_f64()?;
    (f >= 0.0 && f.fract() == 0.0 && f <= MAX_SAFE_INTEGER as f64).then_some(f as u64)
}

fn lifecycle_phase(kind: u32) -> u8 {
    match kind {
        KIND_HUDDLE_STARTED => 0,
        KIND_HUDDLE_ENDED => 2,
        _ => 1,
    }
}

/// Order lifecycle events as desktop does: time, then start < join/leave <
/// end, then the relay's roster revision, then event id.
pub(crate) fn compare_lifecycle_events(
    left: &LifecycleEvent,
    right: &LifecycleEvent,
) -> std::cmp::Ordering {
    left.created_at
        .cmp(&right.created_at)
        .then_with(|| lifecycle_phase(left.kind).cmp(&lifecycle_phase(right.kind)))
        .then_with(|| match (left.roster_revision, right.roster_revision) {
            (Some(l), Some(r)) => l.cmp(&r),
            _ => std::cmp::Ordering::Equal,
        })
        .then_with(|| left.id.cmp(&right.id))
}

#[derive(Debug, Clone)]
struct Admission {
    present: bool,
    roster_revision: Option<u64>,
    created_at: u64,
    event_id: String,
}

fn is_newer(candidate: &Admission, existing: Option<&Admission>) -> bool {
    let Some(existing) = existing else {
        return true;
    };
    if let (Some(c), Some(e)) = (candidate.roster_revision, existing.roster_revision) {
        if c != e {
            return c > e;
        }
    }
    if candidate.created_at != existing.created_at {
        return candidate.created_at > existing.created_at;
    }
    candidate.event_id > existing.event_id
}

/// The reconstructed state of one huddle.
#[derive(Debug, Clone)]
pub(crate) struct HuddleSession {
    pub start_creator: String,
    pub start_created_at: u64,
    pub start_event_id: String,
    pub ended_at: Option<u64>,
    end: Option<Admission>,
    admissions: HashMap<String, HashMap<String, Admission>>,
    legacy: HashMap<String, Admission>,
    latest_roster_revision: Option<u64>,
    latest_roster_created_at: u64,
    latest_roster_event_id: String,
    generation: Option<String>,
    departed: VecDeque<(String, Admission)>,
    compacted_revision_floor: Option<u64>,
}

impl HuddleSession {
    fn clear_roster(&mut self) {
        self.admissions.clear();
        self.legacy.clear();
        self.departed.clear();
        self.compacted_revision_floor = None;
    }

    fn departed_position(&self, admission_id: &str) -> Option<usize> {
        self.departed.iter().position(|(id, _)| id == admission_id)
    }

    /// Present participants with the time they were first admitted, oldest
    /// admission first, ties broken by pubkey.
    pub(crate) fn participants(&self) -> Vec<(String, u64)> {
        let mut roster: HashMap<&str, u64> = HashMap::new();
        for (participant, admissions) in &self.admissions {
            for admission in admissions.values().filter(|a| a.present) {
                roster
                    .entry(participant)
                    .and_modify(|at| *at = (*at).min(admission.created_at))
                    .or_insert(admission.created_at);
            }
        }
        for (participant, admission) in &self.legacy {
            let has_admissions = self
                .admissions
                .get(participant)
                .is_some_and(|admissions| !admissions.is_empty());
            if !has_admissions && admission.present {
                roster.insert(participant, admission.created_at);
            }
        }
        let mut roster: Vec<(String, u64)> = roster
            .into_iter()
            .map(|(participant, at)| (participant.to_string(), at))
            .collect();
        roster.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        roster
    }
}

/// Reconstructs huddles from relay-authenticated lifecycle events.
pub(crate) struct HuddleTracker {
    relay_self: String,
    sessions: HashMap<String, HuddleSession>,
}

impl HuddleTracker {
    pub(crate) fn new(relay_self_pubkey: &str) -> Self {
        Self {
            relay_self: normalize_pubkey(relay_self_pubkey),
            sessions: HashMap::new(),
        }
    }

    /// Replay events in desktop's lifecycle order.
    pub(crate) fn apply_history(&mut self, events: &[LifecycleEvent]) {
        let mut sorted: Vec<&LifecycleEvent> = events.iter().collect();
        sorted.sort_by(|a, b| compare_lifecycle_events(a, b));
        for event in sorted {
            self.apply(event);
        }
    }

    pub(crate) fn sessions(&self) -> &HashMap<String, HuddleSession> {
        &self.sessions
    }

    fn apply(&mut self, event: &LifecycleEvent) {
        if self.relay_self.is_empty() {
            return;
        }
        let Some(session_id) = event.session.as_deref() else {
            return;
        };

        if event.kind == KIND_HUDDLE_STARTED {
            if event.pubkey.is_empty() {
                return;
            }
            if let Some(existing) = self.sessions.get(session_id) {
                if existing.start_event_id == event.id
                    || event.created_at < existing.start_created_at
                    || (event.created_at == existing.start_created_at
                        && event.id > existing.start_event_id)
                {
                    return;
                }
            }
            self.sessions.insert(
                session_id.to_string(),
                HuddleSession {
                    start_creator: event.pubkey.clone(),
                    start_created_at: event.created_at,
                    start_event_id: event.id.clone(),
                    ended_at: None,
                    end: None,
                    admissions: HashMap::new(),
                    legacy: HashMap::new(),
                    latest_roster_revision: None,
                    latest_roster_created_at: 0,
                    latest_roster_event_id: String::new(),
                    generation: None,
                    departed: VecDeque::new(),
                    compacted_revision_floor: None,
                },
            );
            return;
        }

        let Some(session) = self.sessions.get_mut(session_id) else {
            return;
        };
        let generation = event.generation.as_deref();

        if event.kind == KIND_HUDDLE_ENDED {
            if event.pubkey != self.relay_self && event.pubkey != session.start_creator {
                return;
            }
            if let (Some(generation), Some(current)) = (generation, session.generation.as_deref()) {
                if generation != current {
                    return;
                }
            }
            if let Some(generation) = generation {
                session.generation = Some(generation.to_string());
            }
            let next_end = Admission {
                present: false,
                roster_revision: None,
                created_at: event.created_at,
                event_id: event.id.clone(),
            };
            if !is_newer(&next_end, session.end.as_ref()) {
                return;
            }
            session.ended_at = Some(event.created_at);
            session.end = Some(next_end);
            session.clear_roster();
            return;
        }

        if session.end.is_some() {
            return;
        }
        if event.kind != KIND_HUDDLE_PARTICIPANT_JOINED
            && event.kind != KIND_HUDDLE_PARTICIPANT_LEFT
        {
            return;
        }
        if event.pubkey != self.relay_self {
            return;
        }
        let Some(participant) = event.participant.as_deref() else {
            return;
        };

        if let (Some(generation), Some(current)) = (generation, session.generation.as_deref()) {
            if generation != current {
                if event.kind == KIND_HUDDLE_PARTICIPANT_LEFT {
                    return;
                }
                session.clear_roster();
            }
        }

        let present = event.kind == KIND_HUDDLE_PARTICIPANT_JOINED;
        let next = Admission {
            present,
            roster_revision: event.roster_revision,
            created_at: event.created_at,
            event_id: event.id.clone(),
        };

        let is_after_latest_roster_event = event.created_at > session.latest_roster_created_at
            || (event.created_at == session.latest_roster_created_at
                && event.id > session.latest_roster_event_id);
        if let (Some(revision), Some(latest)) =
            (event.roster_revision, session.latest_roster_revision)
        {
            let different_generation = match (generation, session.generation.as_deref()) {
                (Some(g), Some(current)) => g != current,
                _ => true,
            };
            if revision < latest
                && is_after_latest_roster_event
                && different_generation
                && session
                    .compacted_revision_floor
                    .is_none_or(|floor| revision > floor)
            {
                session.clear_roster();
            }
        }
        if let Some(generation) = generation {
            session.generation = Some(generation.to_string());
        }

        if !present {
            if let (Some(revision), Some(floor)) =
                (next.roster_revision, session.compacted_revision_floor)
            {
                if revision <= floor {
                    return;
                }
            }
        }

        if let Some(admission_id) = event.admission_id.as_deref() {
            let departed_at = session.departed_position(admission_id);
            let existing = session
                .admissions
                .get(participant)
                .and_then(|admissions| admissions.get(admission_id))
                .or_else(|| departed_at.map(|index| &session.departed[index].1));
            if !is_newer(&next, existing) {
                return;
            }
            if existing.is_none() {
                if let (Some(revision), Some(floor)) =
                    (next.roster_revision, session.compacted_revision_floor)
                {
                    if revision <= floor {
                        return;
                    }
                }
            }
            if let Some(index) = departed_at {
                if present {
                    session.departed.remove(index);
                }
            }
            if present {
                session
                    .admissions
                    .entry(participant.to_string())
                    .or_default()
                    .insert(admission_id.to_string(), next);
            } else {
                if let Some(admissions) = session.admissions.get_mut(participant) {
                    admissions.remove(admission_id);
                    if admissions.is_empty() {
                        session.admissions.remove(participant);
                    }
                }
                if let Some(index) = session.departed_position(admission_id) {
                    session.departed.remove(index);
                }
                session.departed.push_back((admission_id.to_string(), next));
                while session.departed.len() > MAX_DEPARTED_ADMISSIONS {
                    let Some((_, oldest)) = session.departed.pop_front() else {
                        break;
                    };
                    if let Some(revision) = oldest.roster_revision {
                        session.compacted_revision_floor = Some(
                            session
                                .compacted_revision_floor
                                .map_or(revision, |f| f.max(revision)),
                        );
                    }
                }
            }
        } else {
            if !is_newer(&next, session.legacy.get(participant)) {
                return;
            }
            session.legacy.insert(participant.to_string(), next);
        }
        if event.roster_revision.is_some() && is_after_latest_roster_event {
            session.latest_roster_revision = event.roster_revision;
            session.latest_roster_created_at = event.created_at;
            session.latest_roster_event_id = event.id.clone();
        }
    }
}
