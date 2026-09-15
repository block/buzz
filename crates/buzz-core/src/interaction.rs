//! Experimental interaction protocol v1. See docs/experimental-interactions.md.
//!
//! Validation is shared by the relay and event builders. Visibility is public
//! *within the channel's existing access boundary*. Private ballots and secret
//! fields are rejected until every read/notification surface can protect them.

use std::collections::{BTreeMap, BTreeSet};

use nostr::{Event, EventBuilder, EventId, Kind, PublicKey, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::kind::{KIND_INTERACTION_PROMPT, KIND_INTERACTION_RESPONSE, KIND_INTERACTION_STATE};

/// NIP-11 feature-detection token; changes when the experimental wire contract changes.
pub const EXTENSION: &str = "buzz-interactions-v1";
/// Maximum prompt lifetime (30 days).
pub const MAX_LIFETIME: u64 = 30 * 24 * 60 * 60;
/// Maximum distinct responders to one experimental prompt.
pub const MAX_RESPONDERS: usize = 256;

/// Invalid or unsupported interaction input.
#[derive(Debug, thiserror::Error)]
#[error("invalid: {0}")]
pub struct InteractionError(pub String);

type Result<T> = std::result::Result<T, InteractionError>;
fn invalid(message: impl Into<String>) -> InteractionError {
    InteractionError(message.into())
}

/// Parsed, bounded prompt schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt {
    /// Channel UUID, including for DMs.
    pub channel: Uuid,
    /// buttons, poll, or form.
    pub itype: String,
    /// Stable choice IDs with display labels and style hints.
    pub options: Vec<Choice>,
    /// Optional additional fields on buttons; required on forms.
    pub fields: Vec<Field>,
    /// members, listed, role:owner, or role:admin (community roles).
    pub responders: String,
    /// Eligible public keys when responders is listed.
    pub listed: BTreeSet<String>,
    /// Minimum choices per answer.
    pub min: usize,
    /// Maximum choices per answer.
    pub max: usize,
    /// first, quorum:N, manual, or expiry.
    pub closes: String,
    /// Absolute Unix deadline, evaluated using relay time.
    pub deadline: u64,
}

/// One selectable option.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    /// Stable identifier used in answers.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// primary, danger, or default.
    pub style: String,
}

/// One non-secret input field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Field {
    /// Stable identifier used in answers.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// text, number, select, boolean, or date.
    pub kind: String,
    /// Whether an answer must include a value.
    pub required: bool,
    /// Stable value IDs with labels, for select fields only.
    pub options: BTreeMap<String, String>,
}

/// Canonical parsed answer. Wire values remain strings; field types are validated.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Answer {
    /// Unique choice IDs.
    pub choices: BTreeSet<String>,
    /// Field ID to validated wire value.
    pub values: BTreeMap<String, String>,
}

/// Latest answer from one attributable actor, retained for replacements/audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote {
    /// The signed response event, or original signed message for text fallback.
    pub source: Event,
    /// Validated answer.
    pub answer: Answer,
}

/// Durable state under the database's per-prompt lock.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InteractionState {
    /// Number of committed transitions, for ordering same-second state events.
    pub revision: u64,
    /// None while open; first, quorum, manual, or expiry when closed.
    pub close_reason: Option<String>,
    /// Latest answer by actor public key (not the fallback relay signer).
    pub votes: BTreeMap<String, Vote>,
}

fn tags<'a>(event: &'a Event, name: &'a str) -> impl Iterator<Item = &'a [String]> {
    event
        .tags
        .iter()
        .map(Tag::as_slice)
        .filter(move |t| t.first().is_some_and(|s| s == name))
}

/// Read a unique two-element tag, rejecting duplicates and malformed instances.
pub fn single_tag<'a>(event: &'a Event, name: &str) -> Result<Option<&'a str>> {
    let mut found = None;
    for t in event.tags.iter().map(Tag::as_slice) {
        if t.first().is_some_and(|s| s == name) {
            if t.len() != 2 || found.is_some() {
                return Err(invalid(format!("expected one {name} tag with one value")));
            }
            found = Some(t[1].as_str());
        }
    }
    Ok(found)
}

fn required_tag<'a>(event: &'a Event, name: &str) -> Result<&'a str> {
    single_tag(event, name)?.ok_or_else(|| invalid(format!("missing {name} tag")))
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
}
fn label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
/// Bound client interaction payloads and reject reserved relay provenance.
pub fn validate_envelope(event: &Event) -> Result<()> {
    if tags(event, "expiration").next().is_some() {
        return Err(invalid(
            "use deadline on the prompt; NIP-40 expiration would hide the signed decision",
        ));
    }
    if event.content.len() > 16_384
        || event.tags.len() > 128
        || event
            .tags
            .iter()
            .flat_map(Tag::as_slice)
            .map(String::len)
            .sum::<usize>()
            > 32_768
    {
        return Err(invalid("interaction exceeds content or tag limits"));
    }
    // These tags assert relay provenance and are never accepted from clients.
    if ["via", "actor", "interaction"]
        .iter()
        .any(|name| tags(event, name).next().is_some())
    {
        return Err(invalid("relay provenance tags are reserved"));
    }
    Ok(())
}

/// Resolve exactly one canonical channel tag.
pub fn channel(event: &Event) -> Result<Uuid> {
    let value = required_tag(event, "h")?;
    let id = Uuid::parse_str(value).map_err(|_| invalid("h must be a channel UUID"))?;
    if id.to_string() != value {
        return Err(invalid("h must be a canonical channel UUID"));
    }
    Ok(id)
}

/// Resolve exactly one prompt-marked e tag. Thread root tags are separate.
pub fn prompt_id(event: &Event) -> Result<EventId> {
    let refs: Vec<_> = tags(event, "e")
        .filter(|t| t.get(3).is_some_and(|m| m == "prompt"))
        .collect();
    if refs.len() != 1 || refs[0].len() != 4 {
        return Err(invalid("expected one e tag marked prompt"));
    }
    EventId::from_hex(&refs[0][1]).map_err(|_| invalid("invalid prompt event ID"))
}

impl Prompt {
    /// Parse a prompt's schema. Validation of its lifetime is separate for history reads.
    pub fn parse(event: &Event) -> Result<Self> {
        validate_envelope(event)?;
        if event.kind.as_u16() as u32 != KIND_INTERACTION_PROMPT || event.content.trim().is_empty()
        {
            return Err(invalid("expected a prompt with nonempty question text"));
        }
        let channel = channel(event)?;
        let itype = required_tag(event, "itype")?.to_string();
        if !["buttons", "poll", "form"].contains(&itype.as_str()) {
            return Err(invalid("unsupported itype"));
        }
        if single_tag(event, "visibility")?.unwrap_or("public") != "public" {
            return Err(invalid(
                "experimental v1 supports public channel-visible answers only",
            ));
        }
        let mut options = Vec::new();
        let mut ids = BTreeSet::new();
        let mut labels = BTreeSet::new();
        for t in tags(event, "opt") {
            if !(3..=4).contains(&t.len()) || !identifier(&t[1]) || !label(&t[2]) {
                return Err(invalid("opt requires id, label, and optional style"));
            }
            if !ids.insert(t[1].to_lowercase()) || !labels.insert(t[2].trim().to_lowercase()) {
                return Err(invalid(
                    "option IDs and labels must be unique ignoring case",
                ));
            }
            let style = t.get(3).map(String::as_str).unwrap_or("default");
            if !["default", "primary", "danger"].contains(&style) {
                return Err(invalid("unknown option style"));
            }
            options.push(Choice {
                id: t[1].clone(),
                label: t[2].clone(),
                style: style.into(),
            });
        }
        // A label must not name another option's ID: fallback must be unambiguous.
        for a in &options {
            if options
                .iter()
                .any(|b| a.id != b.id && a.label.trim().eq_ignore_ascii_case(&b.id))
            {
                return Err(invalid("option label collides with another option ID"));
            }
        }
        let mut fields = Vec::new();
        let mut field_ids = BTreeSet::new();
        for t in tags(event, "field") {
            if t.len() != 5
                || !identifier(&t[1])
                || !label(&t[2])
                || !field_ids.insert(t[1].clone())
            {
                return Err(invalid(
                    "field requires unique id, label, type, required|optional",
                ));
            }
            if !["text", "number", "select", "boolean", "date"].contains(&t[3].as_str()) {
                return Err(invalid(
                    "unsupported field type (secret fields are not supported)",
                ));
            }
            if !["required", "optional"].contains(&t[4].as_str()) {
                return Err(invalid("invalid field requirement"));
            }
            fields.push(Field {
                id: t[1].clone(),
                label: t[2].clone(),
                kind: t[3].clone(),
                required: t[4] == "required",
                options: BTreeMap::new(),
            });
        }
        for t in tags(event, "optsel") {
            if !(3..=4).contains(&t.len()) || !identifier(&t[2]) {
                return Err(invalid("optsel requires field, value, optional label"));
            }
            let field = fields
                .iter_mut()
                .find(|f| f.id == t[1] && f.kind == "select")
                .ok_or_else(|| invalid("optsel must reference a select field"))?;
            let display = t.get(3).unwrap_or(&t[2]);
            if !label(display)
                || field
                    .options
                    .insert(t[2].clone(), display.clone())
                    .is_some()
            {
                return Err(invalid("invalid or duplicate select option"));
            }
        }
        if fields
            .iter()
            .any(|f| f.kind == "select" && !(1..=12).contains(&f.options.len()))
        {
            return Err(invalid("select fields need 1 to 12 options"));
        }
        if fields.len() > 12
            || (itype == "form" && (fields.is_empty() || !options.is_empty()))
            || (itype == "poll" && (!(2..=12).contains(&options.len()) || !fields.is_empty()))
            || (itype == "buttons" && !(1..=8).contains(&options.len()))
        {
            return Err(invalid(
                "options/fields do not satisfy the interaction type limits",
            ));
        }
        let default_min = if itype == "form" { "0" } else { "1" };
        let min = single_tag(event, "min")?
            .unwrap_or(default_min)
            .parse::<usize>()
            .map_err(|_| invalid("min must be an integer"))?;
        let max = single_tag(event, "max")?
            .unwrap_or(default_min)
            .parse::<usize>()
            .map_err(|_| invalid("max must be an integer"))?;
        if min > max
            || max > options.len()
            || (itype == "buttons" && (min != 1 || max != 1))
            || (itype == "poll" && min == 0)
        {
            return Err(invalid("invalid min/max choices"));
        }
        let responders = single_tag(event, "responders")?
            .unwrap_or("members")
            .to_string();
        if !["members", "listed", "role:owner", "role:admin"].contains(&responders.as_str()) {
            return Err(invalid("unsupported responder rule"));
        }
        let mut listed = BTreeSet::new();
        for t in tags(event, "p") {
            if t.len() < 2 {
                return Err(invalid("p tag requires a public key"));
            }
            let pk =
                PublicKey::from_hex(&t[1]).map_err(|_| invalid("invalid responder public key"))?;
            listed.insert(pk.to_hex());
        }
        if listed.len() > MAX_RESPONDERS || (responders == "listed" && listed.is_empty()) {
            return Err(invalid("listed responders need 1 to 256 public keys"));
        }
        let closes = required_tag(event, "closes")?.to_string();
        let quorum = closes
            .strip_prefix("quorum:")
            .and_then(|n| n.parse::<usize>().ok());
        if !["first", "manual", "expiry"].contains(&closes.as_str())
            && !quorum.is_some_and(|n| (1..=MAX_RESPONDERS).contains(&n))
        {
            return Err(invalid(
                "closes must be first, quorum:1..256, manual, or expiry",
            ));
        }
        if itype == "poll" && !["manual", "expiry"].contains(&closes.as_str()) {
            return Err(invalid("polls close manually or at expiry"));
        }
        let deadline = required_tag(event, "deadline")?
            .parse::<u64>()
            .map_err(|_| invalid("deadline must be Unix seconds"))?;
        Ok(Self {
            channel,
            itype,
            options,
            fields,
            responders,
            listed,
            min,
            max,
            closes,
            deadline,
        })
    }

    /// Require an unexpired prompt with a bounded deadline when it is first posted.
    pub fn validate_lifetime(&self, now: u64) -> Result<()> {
        if self.deadline <= now || self.deadline.saturating_sub(now) > MAX_LIFETIME {
            return Err(invalid("deadline must be in the next 30 days"));
        }
        Ok(())
    }

    /// Validate an answer against the complete schema, including duplicate/unknown fields.
    pub fn answer(&self, event: &Event) -> Result<Answer> {
        validate_envelope(event)?;
        if event.kind.as_u16() as u32 != KIND_INTERACTION_RESPONSE
            || channel(event)? != self.channel
        {
            return Err(invalid("response must be in the prompt's channel"));
        }
        prompt_id(event)?;
        let mut answer = Answer::default();
        for t in tags(event, "choice") {
            if t.len() != 2 || !self.options.iter().any(|o| o.id == t[1]) {
                return Err(invalid("choice not in prompt"));
            }
            if !answer.choices.insert(t[1].clone()) {
                return Err(invalid("duplicate choice"));
            }
        }
        if answer.choices.len() < self.min || answer.choices.len() > self.max {
            return Err(invalid("answer violates min/max choices"));
        }
        let mut value_bytes = 0;
        for t in tags(event, "value") {
            if t.len() != 3 {
                return Err(invalid("value requires field ID and value"));
            }
            let field = self
                .fields
                .iter()
                .find(|f| f.id == t[1])
                .ok_or_else(|| invalid("unknown field ID"))?;
            field.validate(&t[2])?;
            value_bytes += t[2].len();
            if value_bytes > 8192 || answer.values.insert(t[1].clone(), t[2].clone()).is_some() {
                return Err(invalid("duplicate field or values exceed 8192 bytes"));
            }
        }
        if self
            .fields
            .iter()
            .any(|f| f.required && !answer.values.contains_key(&f.id))
        {
            return Err(invalid("required field missing"));
        }
        Ok(answer)
    }

    /// Exact whole-reply fallback. Punctuation and prose are intentionally not stripped.
    pub fn text_answer(&self, content: &str) -> Option<Answer> {
        if self.itype == "form" || self.fields.iter().any(|f| f.required) {
            return None;
        }
        let matches: Vec<_> = self
            .options
            .iter()
            .filter(|o| {
                o.id.to_lowercase() == content.trim().to_lowercase()
                    || o.label.trim().to_lowercase() == content.trim().to_lowercase()
            })
            .collect();
        if matches.len() != 1 || self.min > 1 {
            return None;
        }
        Some(Answer {
            choices: BTreeSet::from([matches[0].id.clone()]),
            values: BTreeMap::new(),
        })
    }
}

impl Field {
    fn validate(&self, value: &str) -> Result<()> {
        if value.len() > 4096 || (self.required && value.trim().is_empty()) {
            return Err(invalid(format!("invalid value for {}", self.id)));
        }
        let valid = match self.kind.as_str() {
            "text" => true,
            "number" => value.parse::<f64>().is_ok_and(f64::is_finite),
            "boolean" => ["true", "false"].contains(&value),
            "date" => chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .is_ok_and(|d| d.format("%Y-%m-%d").to_string() == value),
            "select" => self.options.contains_key(value),
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(invalid(format!(
                "invalid {} value for {}",
                self.kind, self.id
            )))
        }
    }
}

impl InteractionState {
    /// Apply one validated, authorized response. Equal timestamps follow NIP-01's lower-ID tie break.
    pub fn respond(
        &mut self,
        prompt: &Prompt,
        source: Event,
        answer: Answer,
        now: u64,
    ) -> Result<bool> {
        let actor = source.pubkey.to_hex();
        if self
            .votes
            .get(&actor)
            .is_some_and(|v| v.source.id == source.id)
        {
            return Ok(false);
        }
        if self.close_reason.is_some() || now >= prompt.deadline {
            return Err(invalid("prompt is closed"));
        }
        if let Some(previous) = self.votes.get(&actor) {
            if source.created_at < previous.source.created_at
                || (source.created_at == previous.source.created_at
                    && source.id > previous.source.id)
            {
                return Err(invalid("response superseded by a newer answer"));
            }
        } else if self.votes.len() >= MAX_RESPONDERS {
            return Err(invalid("prompt responder limit reached"));
        }
        self.votes.insert(actor, Vote { source, answer });
        self.revision += 1;
        if prompt.closes == "first" {
            self.close_reason = Some("first".into());
        }
        if prompt
            .closes
            .strip_prefix("quorum:")
            .and_then(|n| n.parse::<usize>().ok())
            .is_some_and(|n| self.votes.len() >= n)
        {
            self.close_reason = Some("quorum".into());
        }
        Ok(true)
    }

    /// Close exactly once. Expiry is also enforced synchronously on answer ingest.
    pub fn close(&mut self, reason: &str) -> bool {
        if self.close_reason.is_some() {
            return false;
        }
        self.close_reason = Some(reason.into());
        self.revision += 1;
        true
    }

    /// Public channel-visible state. Ties have no winner. The values shortcut is
    /// present only for one responder; event IDs identify every signed answer.
    pub fn summary(&self, prompt: &Prompt) -> serde_json::Value {
        let mut tally: BTreeMap<_, usize> =
            prompt.options.iter().map(|o| (o.id.clone(), 0)).collect();
        for vote in self.votes.values() {
            for choice in &vote.answer.choices {
                if let Some(count) = tally.get_mut(choice) {
                    *count += 1;
                }
            }
        }
        let high = tally.values().copied().max().unwrap_or(0);
        let winners: Vec<_> = tally
            .iter()
            .filter(|(_, n)| **n == high && high > 0)
            .map(|(id, _)| id)
            .collect();
        let winner =
            if self.close_reason.is_some() && winners.len() == 1 && prompt.itype == "buttons" {
                Some(winners[0])
            } else {
                None
            };
        serde_json::json!({
            "version": 1, "revision": self.revision,
            "status": if self.close_reason.is_some() { "closed" } else { "open" },
            "close_reason": self.close_reason, "tally": tally, "winner": winner,
            "values": if self.votes.len() == 1 { self.votes.values().next().map(|v| &v.answer.values) } else { None },
            "responders": self.votes.iter().map(|(pk, v)| serde_json::json!({"pubkey": pk, "event_id": v.source.id.to_hex(), "created_at": v.source.created_at.as_secs(), "choices": v.answer.choices})).collect::<Vec<_>>(),
        })
    }

    /// Build a relay-signed state projection. The caller supplies its monotonic timestamp.
    pub fn event_builder(
        &self,
        prompt_id: EventId,
        prompt: &Prompt,
        timestamp: u64,
    ) -> Result<EventBuilder> {
        let tags = [
            ["d".to_string(), prompt_id.to_hex()],
            ["h".to_string(), prompt.channel.to_string()],
        ]
        .into_iter()
        .map(Tag::parse)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| invalid(e.to_string()))?;
        Ok(EventBuilder::new(
            Kind::Custom(KIND_INTERACTION_STATE as u16),
            self.summary(prompt).to_string(),
        )
        .tags(tags)
        .custom_created_at(Timestamp::from(timestamp)))
    }
}

#[cfg(test)]
#[path = "interaction_tests.rs"]
mod tests;
