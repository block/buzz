//! The opt-in `buzz.work/*/v1` convention for non-code work, read strictly.
//!
//! A channel message (kind 9) is typed work only when it carries exactly one
//! `buzz.work` `t` tag naming a known role and version, its references have
//! the required shape, and its JSON content has every required field. Any
//! other message is conversation. A message that tries to be typed and fails
//! is reported with the reason and never read as work, acceptance, or
//! progress. Outcomes are NIP-32 labels (kind 1985) in [`LABEL_NAMESPACE`].

use nostr::{Event, EventBuilder, EventId, Kind, PublicKey};
use serde::Serialize;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use super::evidence::tags;
use super::{tag, LABEL_NAMESPACE};
use crate::SdkError;

/// The `t` value prefix of the convention.
pub const WORK_PREFIX: &str = "buzz.work/";

/// The role a typed message plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkRole {
    /// A task: what is wanted and how it will be judged.
    Request,
    /// The signer claims responsibility for a request. Not completion.
    Commitment,
    /// The signer reports working or blocked. Not verified progress.
    Update,
    /// A deliverable revision for a request.
    Submission,
}

impl WorkRole {
    /// Every role, in convention order.
    pub const ALL: [Self; 4] = [
        Self::Request,
        Self::Commitment,
        Self::Update,
        Self::Submission,
    ];

    /// The `t` tag value of this role in version 1.
    pub fn tag(self) -> &'static str {
        match self {
            Self::Request => "buzz.work/request/v1",
            Self::Commitment => "buzz.work/commitment/v1",
            Self::Update => "buzz.work/update/v1",
            Self::Submission => "buzz.work/submission/v1",
        }
    }
}

/// Outcome claims a `buzz.wot` label may make.
pub const OUTCOME_CLAIMS: [&str; 5] = ["accepted", "corrected", "rejected", "disputed", "reused"];

/// A typed request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RequestBody {
    pub(super) title: String,
    pub(super) task_type: String,
    pub(super) acceptance_criteria: Vec<String>,
}

/// A typed commitment, update, or submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Part {
    pub(super) role: WorkRole,
    /// The request this answers: `e` root and `e` reply, equal.
    pub(super) request: EventId,
    /// Keys named by `p` tags; one must be the requester.
    pub(super) mentions: Vec<PublicKey>,
    pub(super) summary: String,
    pub(super) state: Option<String>,
    pub(super) artifact: Option<String>,
    pub(super) artifact_revision: Option<String>,
    /// The previous submission, from one `q` tag.
    pub(super) previous: Option<EventId>,
}

/// How one channel message reads under the convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Message {
    /// Not typed: ordinary conversation.
    Plain,
    Request(RequestBody),
    Part(Part),
    /// Tries to be typed and fails; never read as work.
    Malformed(String),
}

/// A `buzz.wot` outcome label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Label {
    pub(super) claim: &'static str,
    /// The exact contribution judged: the only `e` tag.
    pub(super) target: EventId,
    /// The contribution's signer: the only `p` tag.
    pub(super) subject: PublicKey,
    /// The only `h` tag.
    pub(super) channel: String,
    pub(super) reason: String,
    pub(super) criteria_checked: Vec<String>,
    pub(super) evidence_refs: Vec<String>,
}

/// Read a channel message under the convention.
pub(super) fn read(event: &Event) -> Message {
    let typed: Vec<&str> = tags(event, "t")
        .filter_map(|p| p.get(1))
        .map(String::as_str)
        .filter(|t| t.starts_with(WORK_PREFIX))
        .collect();
    match typed.as_slice() {
        [] => Message::Plain,
        [t] => match WorkRole::ALL.into_iter().find(|r| r.tag() == *t) {
            Some(role) => typed_message(event, role).unwrap_or_else(Message::Malformed),
            None => Message::Malformed(format!("{t} is not a known role and version")),
        },
        _ => Message::Malformed("more than one buzz.work tag".into()),
    }
}

fn typed_message(event: &Event, role: WorkRole) -> Result<Message, String> {
    if event.kind.as_u16() != 9 {
        return Err(format!(
            "kind {} is not a channel message",
            event.kind.as_u16()
        ));
    }
    let content = object(&event.content)?;
    let refs: Vec<&[String]> = tags(event, "e").collect();
    if role == WorkRole::Request {
        if !refs.is_empty() {
            return Err("a request must be a top-level message, not a reply".into());
        }
        return Ok(Message::Request(RequestBody {
            title: text(&content, "title")?,
            task_type: text(&content, "task_type")?,
            acceptance_criteria: texts(&content, "acceptance_criteria")?,
        }));
    }
    let request = match refs.as_slice() {
        [a, b] if marker(a) == Some("root") && marker(b) == Some("reply") && a[1] == b[1] => {
            EventId::from_hex(&a[1]).map_err(|_| "e tag is not an event id".to_owned())?
        }
        [a, b] if marker(a) == Some("reply") && marker(b) == Some("root") && a[1] == b[1] => {
            EventId::from_hex(&a[1]).map_err(|_| "e tag is not an event id".to_owned())?
        }
        _ => {
            return Err("needs exactly one e root and one e reply, both naming the request".into())
        }
    };
    let mentions: Vec<PublicKey> = tags(event, "p")
        .filter_map(|p| p.get(1))
        .filter_map(|pk| PublicKey::from_hex(pk).ok())
        .collect();
    if mentions.is_empty() {
        return Err("needs a p tag naming the requester".into());
    }
    let quotes: Vec<&[String]> = tags(event, "q").collect();
    let previous = match (role, quotes.as_slice()) {
        (_, []) => None,
        (WorkRole::Submission, [q]) => Some(
            q.get(1)
                .and_then(|id| EventId::from_hex(id).ok())
                .ok_or("q tag is not an event id")?,
        ),
        (WorkRole::Submission, _) => return Err("more than one q tag".into()),
        _ => return Err("only a submission names a previous revision".into()),
    };
    let state = match role {
        WorkRole::Update => match text(&content, "state")?.as_str() {
            s @ ("working" | "blocked") => Some(s.to_owned()),
            other => return Err(format!("state {other:?} is not working or blocked")),
        },
        _ => None,
    };
    let (artifact, artifact_revision) = match role {
        WorkRole::Submission => (
            Some(text(&content, "artifact")?),
            Some(text(&content, "artifact_revision")?),
        ),
        _ => (None, None),
    };
    Ok(Message::Part(Part {
        role,
        request,
        mentions,
        summary: text(&content, "summary")?,
        state,
        artifact,
        artifact_revision,
        previous,
    }))
}

/// Read a kind-1985 event as a `buzz.wot` outcome. `None` when it is not a
/// label in this namespace; `Err` when it is one but is malformed.
pub(super) fn read_label(event: &Event) -> Option<Result<Label, String>> {
    if event.kind.as_u16() != 1985
        || !tags(event, "L").any(|p| p.get(1).is_some_and(|ns| ns == LABEL_NAMESPACE))
    {
        return None;
    }
    Some(label(event))
}

fn label(event: &Event) -> Result<Label, String> {
    let claims: Vec<&str> = tags(event, "l")
        .filter(|p| p.get(2).is_some_and(|ns| ns == LABEL_NAMESPACE))
        .filter_map(|p| p.get(1))
        .map(String::as_str)
        .collect();
    let claim = match claims.as_slice() {
        [c] => OUTCOME_CLAIMS
            .into_iter()
            .find(|k| k == c)
            .ok_or_else(|| format!("{c:?} is not an outcome claim"))?,
        _ => return Err("needs exactly one buzz.wot l tag".into()),
    };
    let one = |name: &str| -> Result<String, String> {
        let values: Vec<&String> = tags(event, name).filter_map(|p| p.get(1)).collect();
        match values.as_slice() {
            [v] => Ok((*v).clone()),
            _ => Err(format!("needs exactly one {name} tag")),
        }
    };
    let target = EventId::from_hex(&one("e")?).map_err(|_| "e tag is not an event id")?;
    let subject = PublicKey::from_hex(&one("p")?).map_err(|_| "p tag is not a key")?;
    let channel = one("h")?;
    let content = object(&event.content)?;
    let evidence_refs = match content.get("evidence_refs") {
        Some(Value::Array(refs)) => refs
            .iter()
            .map(|r| {
                r.as_str()
                    .map(str::to_owned)
                    .ok_or("evidence_refs holds a non-string")
            })
            .collect::<Result<_, _>>()?,
        _ => return Err("content needs an evidence_refs array".into()),
    };
    Ok(Label {
        claim,
        target,
        subject,
        channel,
        reason: text(&content, "reason")?,
        criteria_checked: texts(&content, "criteria_checked")?,
        evidence_refs,
    })
}

fn marker(parts: &[String]) -> Option<&str> {
    parts.get(3).map(String::as_str)
}

fn object(content: &str) -> Result<Map<String, Value>, String> {
    match serde_json::from_str(content) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err("content is not a JSON object".into()),
    }
}

fn text(content: &Map<String, Value>, key: &str) -> Result<String, String> {
    match content.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
        _ => Err(format!("content needs a non-empty {key}")),
    }
}

fn texts(content: &Map<String, Value>, key: &str) -> Result<Vec<String>, String> {
    let err = || format!("content needs {key} as a non-empty list of non-empty strings");
    let Some(Value::Array(items)) = content.get(key) else {
        return Err(err());
    };
    let out: Vec<String> = items
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_owned)
        })
        .collect::<Option<_>>()
        .ok_or_else(err)?;
    if out.is_empty() {
        return Err(err());
    }
    Ok(out)
}

/// Build an unsigned typed request: a top-level channel message.
///
/// There is no thread parameter: a typed request is always top-level in this
/// version, so the relay records it as its own thread root.
///
/// # Errors
///
/// Returns [`SdkError::InvalidInput`] when `title` or `task_type` is blank or
/// `acceptance_criteria` is empty or holds a blank entry.
pub fn build_work_request(
    channel: Uuid,
    title: &str,
    task_type: &str,
    acceptance_criteria: &[String],
) -> Result<EventBuilder, SdkError> {
    let content = json!({
        "title": title,
        "task_type": task_type,
        "acceptance_criteria": acceptance_criteria,
    });
    let tags = vec![
        tag(["h", &channel.to_string()])?,
        tag(["t", WorkRole::Request.tag()])?,
    ];
    typed(&content, tags, WorkRole::Request)
}

/// What a commitment, update, or submission says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartSpec {
    /// The contributor's summary.
    pub summary: String,
    /// `working` or `blocked`; updates only.
    pub state: Option<String>,
    /// Artifact locator; submissions only.
    pub artifact: Option<String>,
    /// Artifact revision; submissions only.
    pub artifact_revision: Option<String>,
}

/// Build an unsigned commitment, update, or submission answering `request`.
///
/// Every reference is derived from the signed events: the channel and
/// requester from `request`, which must be a typed request, and `q` from
/// `previous`, which must be a typed submission to the same request.
///
/// # Errors
///
/// Returns [`SdkError::InvalidInput`] when `role` is [`WorkRole::Request`],
/// `request` is not a typed request with one channel, `previous` is given for
/// a role other than submission or is not a submission to `request`, or the
/// content misses a field the role requires.
pub fn build_work_part(
    request: &Event,
    role: WorkRole,
    spec: &PartSpec,
    previous: Option<&Event>,
) -> Result<EventBuilder, SdkError> {
    if role == WorkRole::Request {
        return Err(SdkError::InvalidInput(
            "a request is built with build_work_request".into(),
        ));
    }
    match read(request) {
        Message::Request(_) => {}
        Message::Malformed(reason) => {
            return Err(SdkError::InvalidInput(format!(
                "event {} is not a valid typed request ({reason})",
                request.id
            )))
        }
        _ => {
            return Err(SdkError::InvalidInput(format!(
                "event {} is not a typed request; contributions answer a typed request",
                request.id
            )))
        }
    }
    let channel = single(request, "h").ok_or_else(|| {
        SdkError::InvalidInput(format!("request {} names no single channel", request.id))
    })?;
    let id = request.id.to_hex();
    let mut tags = vec![
        tag(["h", channel])?,
        tag(["e", &id, "", "root"])?,
        tag(["e", &id, "", "reply"])?,
        tag(["p", &request.pubkey.to_hex()])?,
        tag(["t", role.tag()])?,
    ];
    if let Some(prior) = previous {
        if role != WorkRole::Submission {
            return Err(SdkError::InvalidInput(
                "only a submission names a previous revision".into(),
            ));
        }
        let same = matches!(read(prior), Message::Part(p)
            if p.role == WorkRole::Submission && p.request == request.id);
        if !same {
            return Err(SdkError::InvalidInput(format!(
                "event {} is not a typed submission to request {}",
                prior.id, request.id
            )));
        }
        tags.push(tag(["q", &prior.id.to_hex()])?);
    }
    let mut content = json!({ "summary": spec.summary });
    match role {
        WorkRole::Update => content["state"] = json!(spec.state),
        WorkRole::Submission => {
            content["artifact"] = json!(spec.artifact);
            content["artifact_revision"] = json!(spec.artifact_revision);
        }
        _ => {}
    }
    typed(&content, tags, role)
}

/// Build an unsigned `buzz.wot` outcome label on one submission.
///
/// `e`, `p`, and `h` are derived from `target`: its id (with `origin` as the
/// relay hint), its signer, and its only channel. The relay re-derives and
/// checks the same values.
///
/// # Errors
///
/// Returns [`SdkError::InvalidInput`] when `target` is not a typed
/// submission with one channel, `claim` is not in [`OUTCOME_CLAIMS`],
/// `reason` is blank, or `criteria_checked` is empty or holds a blank entry.
pub fn build_work_label(
    target: &Event,
    origin: &str,
    claim: &str,
    reason: &str,
    criteria_checked: &[String],
    evidence_refs: &[String],
) -> Result<EventBuilder, SdkError> {
    let claim = OUTCOME_CLAIMS
        .into_iter()
        .find(|c| *c == claim)
        .ok_or_else(|| {
            SdkError::InvalidInput(format!(
                "{claim:?} is not one of {}",
                OUTCOME_CLAIMS.join(", ")
            ))
        })?;
    if !matches!(read(target), Message::Part(p) if p.role == WorkRole::Submission) {
        return Err(SdkError::InvalidInput(format!(
            "event {} is not a typed submission; outcomes judge an exact submitted revision",
            target.id
        )));
    }
    let channel = single(target, "h").ok_or_else(|| {
        SdkError::InvalidInput(format!("submission {} names no single channel", target.id))
    })?;
    let content = json!({
        "reason": reason,
        "criteria_checked": criteria_checked,
        "evidence_refs": evidence_refs,
    })
    .to_string();
    let map = object(&content).map_err(SdkError::InvalidInput)?;
    text(&map, "reason").map_err(SdkError::InvalidInput)?;
    texts(&map, "criteria_checked").map_err(SdkError::InvalidInput)?;
    // A self-judgment still names its target's signer: `p` is derived from
    // the target, so nostr's default self-tag stripping must not drop it.
    Ok(EventBuilder::new(Kind::Custom(1985), content)
        .tags([
            tag(["L", LABEL_NAMESPACE])?,
            tag(["l", claim, LABEL_NAMESPACE])?,
            tag(["e", &target.id.to_hex(), origin])?,
            tag(["p", &target.pubkey.to_hex()])?,
            tag(["h", channel])?,
        ])
        .allow_self_tagging())
}

/// Check `content` the way [`read`] will for `role`, then build the message.
fn typed(content: &Value, tags: Vec<nostr::Tag>, role: WorkRole) -> Result<EventBuilder, SdkError> {
    let content = content.to_string();
    let map = object(&content).map_err(SdkError::InvalidInput)?;
    let check = || -> Result<(), String> {
        match role {
            WorkRole::Request => {
                text(&map, "title")?;
                text(&map, "task_type")?;
                texts(&map, "acceptance_criteria")?;
                return Ok(());
            }
            WorkRole::Update => match text(&map, "state")?.as_str() {
                "working" | "blocked" => {}
                other => return Err(format!("state {other:?} is not working or blocked")),
            },
            WorkRole::Submission => {
                text(&map, "artifact")?;
                text(&map, "artifact_revision")?;
            }
            WorkRole::Commitment => {}
        }
        text(&map, "summary").map(drop)
    };
    check().map_err(SdkError::InvalidInput)?;
    Ok(EventBuilder::new(Kind::Custom(9), content)
        .tags(tags)
        .allow_self_tagging())
}

fn single<'a>(event: &'a Event, name: &'a str) -> Option<&'a str> {
    let mut values = tags(event, name).filter_map(|p| p.get(1));
    match (values.next(), values.next()) {
        (Some(v), None) => Some(v.as_str()),
        _ => None,
    }
}
