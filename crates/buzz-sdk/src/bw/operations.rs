//! Shared, concrete NIP-BW issue operations.
//!
//! The operations own history projection, causal resume, signing order,
//! publication, exact readback, one same-event retry, and idempotency. Adapters
//! implement only the I/O port for relay, keys, clock, and Git readback.

use super::{record, Draft, Publication, RecordType, SdkError};
use buzz_core::bw::{parse_json, Consumer, Decision, Evidence, Trust};
use nostr::Event;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fmt, future::Future, pin::Pin};

/// Boxed future returned by an [`OperationPort`] method.
pub type PortFuture<'a, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

/// Raw, repository-scoped history returned by an adapter.
#[derive(Debug, Clone)]
pub struct RawHistory {
    /// Externally trusted repository identity.
    pub trust: Trust,
    /// Current external observations used by Core.
    pub evidence: Evidence,
    /// Observation time for the history snapshot.
    pub now: u64,
    /// Original signed event bytes.
    pub events: Vec<Vec<u8>>,
}

/// Public signing metadata. Private key material never enters the shared layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigningIdentity {
    /// Hex public key that will sign the candidate.
    pub pubkey: String,
    /// Extra event tags bound to this identity, such as NIP-OA authorization.
    pub event_tags: Vec<Vec<String>>,
}

/// Concrete I/O used by the shared issue operations.
pub trait OperationPort: Send {
    /// Adapter-specific failure type.
    type Error;

    /// Return the current Unix timestamp.
    fn now(&mut self) -> Result<u64, Self::Error>;

    /// Load a fresh complete repository history.
    fn load_history<'a>(&'a mut self, repo: &'a str) -> PortFuture<'a, RawHistory, Self::Error>;

    /// Resolve the default signer or the exact writer required by a lifecycle step.
    fn signing_identity<'a>(
        &'a mut self,
        required_pubkey: Option<&'a str>,
    ) -> PortFuture<'a, SigningIdentity, Self::Error>;

    /// Sign the exact candidate fields prepared by this module.
    fn sign<'a>(
        &'a mut self,
        candidate: &'a Value,
        required_pubkey: Option<&'a str>,
    ) -> PortFuture<'a, Event, Self::Error>;

    /// Submit an already signed event. An error means delivery is unknown.
    fn submit<'a>(&'a mut self, event: &'a Event) -> PortFuture<'a, (), Self::Error>;

    /// Read the exact event ID back from the relay.
    fn readback<'a>(&'a mut self, id: &'a str) -> PortFuture<'a, Option<Event>, Self::Error>;

    /// Observe the canonical remote head for a repository stream.
    fn remote_head<'a>(
        &'a mut self,
        repo: &'a str,
        stream: &'a str,
    ) -> PortFuture<'a, String, Self::Error>;
}

/// Failure from shared validation or adapter I/O.
#[derive(Debug)]
pub enum OperationError<E> {
    /// Stable BW refusal or typed input error.
    Refused(String),
    /// Concrete adapter failure.
    Port(E),
}

impl<E: fmt::Display> fmt::Display for OperationError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(message) => formatter.write_str(message),
            Self::Port(error) => error.fmt(formatter),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for OperationError<E> {}

/// Status of one concrete operation step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// A newly signed event was published and read back exactly.
    Published,
    /// An unambiguous matching event from fresh history was reused.
    Reused,
    /// The step does not apply to this request.
    NotRequired,
}

impl StepStatus {
    /// Stable string form used by CLI, Tauri, and tests.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Published => "published",
            Self::Reused => "reused",
            Self::NotRequired => "not_required",
        }
    }
}

/// One ordered step in a business operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OperationStep {
    /// Stable step name.
    pub name: String,
    /// Exact signed event ID when the step has one.
    pub event_id: Option<String>,
    /// Whether the event was published, reused, or unnecessary.
    pub status: StepStatus,
}

/// Typed result shared by all issue mutations.
#[derive(Debug, Clone, Serialize)]
pub struct OperationOutcome {
    /// Issue root event ID.
    pub issue: String,
    /// True when fresh history already contained the complete target state.
    pub already_completed: bool,
    /// Ordered causal steps considered by the operation.
    pub steps: Vec<OperationStep>,
    /// Core projection after the final fresh history read.
    pub projection: Value,
}

/// Read-only projection for one issue, computed from complete repository history.
#[derive(Debug, Clone, Serialize)]
pub struct IssueProjection {
    /// Issue root event ID.
    pub issue: String,
    /// Complete Core repository projection containing the requested aggregate.
    pub projection: Value,
}

/// Repository and issue coordinates shared by simple lifecycle operations.
#[derive(Debug, Clone, Deserialize)]
pub struct IssueRequest {
    /// NIP-34 repository coordinate.
    pub repo: String,
    /// NIP-34 issue root event ID.
    pub issue: String,
}

impl IssueRequest {
    /// Construct and normalize later at the operation boundary.
    pub fn new(repo: String, issue: String) -> Self {
        Self { repo, issue }
    }
}

/// Accept an issue and advance its state chain to backlog.
#[derive(Debug, Clone, Deserialize)]
pub struct AcceptToBacklogRequest {
    /// NIP-34 repository coordinate.
    pub repo: String,
    /// NIP-34 issue root event ID.
    pub issue: String,
    /// Use the current policy's matching single-task delegation.
    #[serde(default)]
    pub delegated: bool,
}

/// Select or replace the writer bound to an issue.
#[derive(Debug, Clone, Deserialize)]
pub struct AssignWriterRequest {
    /// NIP-34 repository coordinate.
    pub repo: String,
    /// NIP-34 issue root event ID.
    pub issue: String,
    /// Selected writer public key.
    pub writer: String,
}

/// Move or reset an issue to Ready.
#[derive(Debug, Clone, Deserialize)]
pub struct MoveToReadyRequest {
    /// NIP-34 repository coordinate.
    pub repo: String,
    /// NIP-34 issue root event ID.
    pub issue: String,
    /// Canonical Git stream bound into Ready.
    pub stream: String,
    /// Optional rejecting member-verdict used for rework.
    #[serde(default)]
    pub rework: Option<String>,
    /// Optional failed or aborted release set used for recovery.
    #[serde(default)]
    pub terminal_set: Option<String>,
}

/// Mark the selected writer's work implemented after canonical Git readback.
#[derive(Debug, Clone, Deserialize)]
pub struct MarkImplementedRequest {
    /// NIP-34 repository coordinate.
    pub repo: String,
    /// NIP-34 issue root event ID.
    pub issue: String,
    /// Optional expected commit; CLI supplies it, desktop may use observed head.
    #[serde(default)]
    pub commit: Option<String>,
    /// Honest test and limitation summary.
    pub tests: String,
}

#[derive(Debug)]
struct StoredEvent {
    wire: Value,
    decision: Decision,
}

struct History {
    consumer: Consumer,
    evidence: Evidence,
    records: BTreeMap<String, StoredEvent>,
}

impl History {
    fn from_raw(raw: RawHistory) -> Self {
        let RawHistory {
            trust,
            evidence,
            now,
            events,
        } = raw;
        let mut consumer = Consumer::new(trust, evidence.clone(), now);
        for event in &events {
            consumer.ingest(event);
        }
        let decisions = consumer.inspect_all();
        let mut records = BTreeMap::new();
        for (bytes, decision) in events.into_iter().zip(decisions) {
            let Ok(wire) = parse_json(&bytes) else {
                continue;
            };
            let Some(id) = wire["id"].as_str() else {
                continue;
            };
            let id = id.to_ascii_lowercase();
            match records.entry(id) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(StoredEvent { wire, decision });
                }
                std::collections::btree_map::Entry::Occupied(mut entry)
                    if decision.outcome == "accept" =>
                {
                    entry.insert(StoredEvent { wire, decision });
                }
                std::collections::btree_map::Entry::Occupied(_) => {}
            }
        }
        Self {
            consumer,
            evidence,
            records,
        }
    }

    fn projection(&self) -> Value {
        self.consumer.projection()
    }

    fn event(&self, id: &str) -> Option<&StoredEvent> {
        self.records.get(&id.to_ascii_lowercase())
    }

    fn ensure_issue<E>(&self, repo: &str, issue: &str) -> Result<(), OperationError<E>> {
        let root = self
            .event(issue)
            .ok_or_else(|| refused("bw:pending:references:missing-reference"))?;
        if root.wire["kind"] != 1621
            || tag(&root.wire, "a") != Some(repo)
            || root.decision.outcome != "accept"
        {
            return Err(refused(decision_code(&root.decision)));
        }
        Ok(())
    }

    fn current_state(&self, issue: &str) -> Option<(String, Value)> {
        let projection = self.consumer.projection();
        let id = projection["issue_state_id"][issue].as_str()?.to_owned();
        let event = self.event(&id)?;
        Some((event.wire["id"].as_str()?.to_owned(), event.wire.clone()))
    }

    fn chain_head<E>(
        &self,
        issue: &str,
        record_type: &str,
    ) -> Result<Option<Value>, OperationError<E>> {
        for record in self.records.values().filter(|record| {
            event_type(&record.wire) == record_type && event_issue(&record.wire) == Some(issue)
        }) {
            if record.decision.outcome == "conflict" {
                return Err(refused(decision_code(&record.decision)));
            }
        }
        let records = self
            .records
            .values()
            .filter(|record| {
                record.decision.outcome == "accept"
                    && event_type(&record.wire) == record_type
                    && event_issue(&record.wire) == Some(issue)
            })
            .map(|record| &record.wire)
            .collect::<Vec<_>>();
        let heads = records
            .iter()
            .filter(|candidate| {
                let id = candidate["id"].as_str().unwrap_or_default();
                !records.iter().any(|other| previous(other) == Some(id))
            })
            .copied()
            .collect::<Vec<_>>();
        match heads.as_slice() {
            [] => Ok(None),
            [head] => Ok(Some((*head).clone())),
            _ => Err(refused("bw:conflict:causality:fork")),
        }
    }

    fn ready_rebind_is_compatible(&self, state: &Value, assignment: &Value) -> bool {
        let Some(unassignment_id) = previous(assignment) else {
            return true;
        };
        let Some(previous_state_id) = previous(state) else {
            return false;
        };
        let Some(previous_state) = self.event(previous_state_id) else {
            return false;
        };
        let Ok(previous_body) = body(&previous_state.wire) else {
            return false;
        };
        if previous_body["state"] != "ready" {
            return true;
        }

        let Some(unassignment) = self.event(unassignment_id) else {
            return false;
        };
        let Some(old_assignment_id) = previous(&unassignment.wire) else {
            return false;
        };
        let Some(old_assignment) = self.event(old_assignment_id) else {
            return false;
        };
        if unassignment.decision.outcome != "accept"
            || old_assignment.decision.outcome != "accept"
            || tag(&unassignment.wire, "t") != Some("unassignment")
            || tag(&old_assignment.wire, "t") != Some("assignment")
            || tag(&unassignment.wire, "p") != tag(&old_assignment.wire, "p")
            || previous_body["assignment"] != old_assignment_id
        {
            return false;
        }

        let Ok(current_body) = body(state) else {
            return false;
        };
        ["stream", "update", "rework", "terminal_set"]
            .into_iter()
            .all(|field| {
                current_body.get(field).unwrap_or(&Value::Null)
                    == previous_body.get(field).unwrap_or(&Value::Null)
            })
    }

    fn state_ancestor(&self, issue: &str, wanted: &str) -> Option<Value> {
        let (mut id, _) = self.current_state(issue)?;
        loop {
            let event = &self.event(&id)?.wire;
            if body(event).ok()?["state"] == wanted {
                return Some(event.clone());
            }
            id = previous(event)?.to_owned();
        }
    }
}

fn refused<E>(message: impl Into<String>) -> OperationError<E> {
    OperationError::Refused(message.into())
}

fn decision_code(decision: &Decision) -> String {
    format!(
        "bw:{}:{}:{}",
        decision.outcome, decision.stage, decision.code
    )
}

fn map_sdk<E>(error: SdkError) -> OperationError<E> {
    match error {
        SdkError::InvalidInput(code) => refused(code),
        other => refused(other.to_string()),
    }
}

fn normalized_hex<E>(value: &str, label: &str) -> Result<String, OperationError<E>> {
    let value = value.trim().to_ascii_lowercase();
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(refused(format!(
            "{label} must be a 64-character hex string"
        )));
    }
    Ok(value)
}

fn normalized_commit<E>(value: &str) -> Result<String, OperationError<E>> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(refused("commit must be a 40-character Git SHA-1"));
    }
    Ok(value.to_ascii_lowercase())
}

fn normalized_request<E>(request: IssueRequest) -> Result<IssueRequest, OperationError<E>> {
    let issue = normalized_hex(&request.issue, "issue")?;
    let mut parts = request.repo.splitn(3, ':');
    let (Some("30617"), Some(owner), Some(repo_id)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(refused("invalid BW repository coordinate"));
    };
    let owner = normalized_hex(owner, "repository owner")?;
    if repo_id.trim().is_empty() || repo_id.chars().any(char::is_control) {
        return Err(refused("invalid BW repository id"));
    }
    Ok(IssueRequest {
        repo: format!("30617:{owner}:{repo_id}"),
        issue,
    })
}

fn tag<'a>(event: &'a Value, name: &str) -> Option<&'a str> {
    event["tags"]
        .as_array()?
        .iter()
        .find(|tag| tag[0] == name)?[1]
        .as_str()
}

fn event_type(event: &Value) -> &str {
    if event["kind"] == 1 && matches!(tag(event, "t"), Some("assignment" | "unassignment")) {
        "assignment"
    } else if event["kind"] == 1621 {
        "root"
    } else {
        tag(event, "record").unwrap_or("")
    }
}

fn event_issue(event: &Value) -> Option<&str> {
    if event["kind"] == 1 {
        tag(event, "e")
    } else {
        tag(event, "issue")
    }
}

fn previous(event: &Value) -> Option<&str> {
    tag(
        event,
        if event["kind"] == 1 {
            "prior"
        } else {
            "previous"
        },
    )
}

fn body(event: &Value) -> Result<Value, String> {
    event["content"]
        .as_str()
        .ok_or_else(|| "bw:shape:content".to_string())
        .and_then(|content| parse_json(content.as_bytes()).map_err(|error| error.to_string()))
}

async fn load<P: OperationPort>(
    port: &mut P,
    repo: &str,
) -> Result<History, OperationError<P::Error>> {
    let raw = port
        .load_history(repo)
        .await
        .map_err(OperationError::Port)?;
    if raw.trust.repo != repo {
        return Err(refused("bw:reject:references:repository"));
    }
    Ok(History::from_raw(raw))
}

async fn identity<P: OperationPort>(
    port: &mut P,
    required: Option<&str>,
) -> Result<SigningIdentity, OperationError<P::Error>> {
    let identity = port
        .signing_identity(required)
        .await
        .map_err(OperationError::Port)?;
    let pubkey = normalized_hex(&identity.pubkey, "signer")?;
    if required.is_some_and(|required| !required.eq_ignore_ascii_case(&pubkey)) {
        return Err(refused("bw:reject:role:unauthorized"));
    }
    Ok(SigningIdentity { pubkey, ..identity })
}

async fn publish_draft<P: OperationPort>(
    port: &mut P,
    history: &mut History,
    draft: Draft,
    identity: &SigningIdentity,
    required_signer: Option<&str>,
    created_at: u64,
) -> Result<(String, Value), OperationError<P::Error>> {
    history
        .consumer
        .observe(history.evidence.clone(), created_at);
    let dry = draft.dry_run().map_err(map_sdk)?;
    let candidate = json!({
        "pubkey": identity.pubkey,
        "created_at": created_at,
        "kind": dry["kind"],
        "tags": dry["tags"],
        "content": dry["content"],
    });
    let publication =
        Publication::prepare(&history.consumer, candidate.clone()).map_err(map_sdk)?;
    let signed = port
        .sign(&candidate, required_signer)
        .await
        .map_err(OperationError::Port)?;
    publication.seal(&signed).map_err(map_sdk)?;

    let mut last_error = None;
    for _ in 0..2 {
        if let Err(error) = port.submit(&signed).await {
            last_error = Some(error);
        }
        match port.readback(publication.event_id()).await {
            Ok(Some(observed)) => {
                publication
                    .confirm(&mut history.consumer, &signed, Some(&observed))
                    .map_err(map_sdk)?;
                return Ok((publication.event_id().to_owned(), history.projection()));
            }
            Ok(None) => {}
            Err(error) => last_error = Some(error),
        }
    }
    match last_error {
        Some(error) => Err(OperationError::Port(error)),
        None => Err(refused("bw:readback:missing")),
    }
}

struct RecordInput<'a> {
    repo: &'a str,
    issue: &'a str,
    required_signer: Option<&'a str>,
    record_type: RecordType,
    extra_tags: Vec<Vec<String>>,
    content: Value,
    delegated: bool,
}

async fn publish_record<P: OperationPort>(
    port: &mut P,
    history: &mut History,
    mut input: RecordInput<'_>,
) -> Result<(String, Value), OperationError<P::Error>> {
    let identity = identity(port, input.required_signer).await?;
    let created_at = port.now().map_err(OperationError::Port)?;
    history
        .consumer
        .observe(history.evidence.clone(), created_at);
    let activation = history
        .consumer
        .activation()
        .map_err(|error| refused(error.to_string()))?;
    let mut tags = vec![
        vec!["a".to_owned(), input.repo.to_owned()],
        vec!["policy".to_owned(), activation.policy.clone()],
        vec!["issue".to_owned(), input.issue.to_owned()],
    ];
    if input.delegated {
        tags.push(vec!["delegation".to_owned(), activation.policy]);
    }
    tags.append(&mut input.extra_tags);
    tags.extend(identity.event_tags.iter().cloned());
    let draft = record(input.record_type, tags, input.content).map_err(map_sdk)?;
    publish_draft(
        port,
        history,
        draft,
        &identity,
        input.required_signer,
        created_at,
    )
    .await
}

async fn publish_assignment<P: OperationPort>(
    port: &mut P,
    history: &mut History,
    repo: &str,
    issue: &str,
    writer: &str,
    operation: &str,
    prior: Option<&str>,
) -> Result<(String, Value), OperationError<P::Error>> {
    let identity = identity(port, None).await?;
    let created_at = port.now().map_err(OperationError::Port)?;
    history
        .consumer
        .observe(history.evidence.clone(), created_at);
    let mut tags = vec![
        vec![
            "e".to_owned(),
            issue.to_owned(),
            String::new(),
            "root".to_owned(),
        ],
        vec!["a".to_owned(), repo.to_owned()],
        vec!["p".to_owned(), writer.to_owned()],
        vec!["t".to_owned(), operation.to_owned()],
    ];
    if let Some(prior) = prior {
        tags.push(vec!["prior".to_owned(), prior.to_owned()]);
    }
    tags.extend(identity.event_tags.iter().cloned());
    let content = if operation == "assignment" {
        "Assigned this issue"
    } else {
        "Unassigned this issue"
    };
    let draft = Draft::new(1, json!(tags), content.to_owned()).map_err(map_sdk)?;
    publish_draft(port, history, draft, &identity, None, created_at).await
}

fn step(name: &str, event_id: Option<&str>, status: StepStatus) -> OperationStep {
    OperationStep {
        name: name.to_owned(),
        event_id: event_id.map(str::to_owned),
        status,
    }
}

fn completed(steps: &[OperationStep]) -> bool {
    !steps
        .iter()
        .any(|operation| operation.status == StepStatus::Published)
}

async fn finish<P: OperationPort>(
    port: &mut P,
    repo: &str,
    issue: String,
    steps: Vec<OperationStep>,
) -> Result<OperationOutcome, OperationError<P::Error>> {
    let history = load(port, repo).await?;
    history.ensure_issue(repo, &issue)?;
    Ok(OperationOutcome {
        issue,
        already_completed: completed(&steps),
        steps,
        projection: history.projection(),
    })
}

/// Read one issue projection from a fresh complete repository history.
pub async fn get_issue_projection<P: OperationPort>(
    port: &mut P,
    request: IssueRequest,
) -> Result<IssueProjection, OperationError<P::Error>> {
    let request = normalized_request(request)?;
    let history = load(port, &request.repo).await?;
    history.ensure_issue(&request.repo, &request.issue)?;
    Ok(IssueProjection {
        issue: request.issue,
        projection: history.projection(),
    })
}

/// Enroll an issue in triage, retrying the same signed event at most once.
pub async fn enroll_issue<P: OperationPort>(
    port: &mut P,
    request: IssueRequest,
) -> Result<OperationOutcome, OperationError<P::Error>> {
    let request = normalized_request(request)?;
    let mut history = load(port, &request.repo).await?;
    history.ensure_issue(&request.repo, &request.issue)?;
    if let Some((event_id, _)) = history.current_state(&request.issue) {
        return finish(
            port,
            &request.repo,
            request.issue,
            vec![step("enroll", Some(&event_id), StepStatus::Reused)],
        )
        .await;
    }
    let (event_id, _) = publish_record(
        port,
        &mut history,
        RecordInput {
            repo: &request.repo,
            issue: &request.issue,
            required_signer: None,
            record_type: RecordType::IssueState,
            extra_tags: vec![],
            content: json!({"state":"triage"}),
            delegated: false,
        },
    )
    .await?;
    finish(
        port,
        &request.repo,
        request.issue,
        vec![step("enroll", Some(&event_id), StepStatus::Published)],
    )
    .await
}

/// Accept an enrolled issue and ensure the causal backlog state exists.
pub async fn accept_to_backlog<P: OperationPort>(
    port: &mut P,
    request: AcceptToBacklogRequest,
) -> Result<OperationOutcome, OperationError<P::Error>> {
    let common = normalized_request(IssueRequest::new(request.repo, request.issue))?;
    let mut history = load(port, &common.repo).await?;
    history.ensure_issue(&common.repo, &common.issue)?;
    let mut steps = Vec::new();

    let current_state = history
        .current_state(&common.issue)
        .ok_or_else(|| refused("bw:reject:causality:not-enrolled"))?;
    let current_body = body(&current_state.1).map_err(refused)?;
    let accept = history.chain_head(&common.issue, "triage-action")?;
    let accept_id = if let Some(accept) = accept {
        let accept_body = body(&accept).map_err(refused)?;
        if accept_body["action"] == "accept" {
            let event_id = accept["id"].as_str().unwrap_or_default().to_owned();
            steps.push(step("accept", Some(&event_id), StepStatus::Reused));
            event_id
        } else if matches!(
            accept_body["action"].as_str(),
            Some("decline" | "duplicate")
        ) {
            return Err(refused("bw:reject:causality:triage-terminal"));
        } else {
            let previous = accept["id"]
                .as_str()
                .map(|id| vec!["previous".to_owned(), id.to_owned()]);
            let (event_id, _) = publish_record(
                port,
                &mut history,
                RecordInput {
                    repo: &common.repo,
                    issue: &common.issue,
                    required_signer: None,
                    record_type: RecordType::TriageAction,
                    extra_tags: previous.into_iter().collect(),
                    content: json!({"action":"accept"}),
                    delegated: request.delegated,
                },
            )
            .await?;
            steps.push(step("accept", Some(&event_id), StepStatus::Published));
            event_id
        }
    } else if current_body["state"] == "triage" {
        let (event_id, _) = publish_record(
            port,
            &mut history,
            RecordInput {
                repo: &common.repo,
                issue: &common.issue,
                required_signer: None,
                record_type: RecordType::TriageAction,
                extra_tags: vec![],
                content: json!({"action":"accept"}),
                delegated: request.delegated,
            },
        )
        .await?;
        steps.push(step("accept", Some(&event_id), StepStatus::Published));
        event_id
    } else {
        return Err(refused("bw:reject:causality:triage-state"));
    };

    history = load(port, &common.repo).await?;
    history.ensure_issue(&common.repo, &common.issue)?;
    let (state_id, state_event) = history
        .current_state(&common.issue)
        .ok_or_else(|| refused("bw:reject:causality:not-enrolled"))?;
    let state_body = body(&state_event).map_err(refused)?;
    if state_body["state"] == "triage" {
        let (event_id, _) = publish_record(
            port,
            &mut history,
            RecordInput {
                repo: &common.repo,
                issue: &common.issue,
                required_signer: None,
                record_type: RecordType::IssueState,
                extra_tags: vec![vec!["previous".to_owned(), state_id.to_owned()]],
                content: json!({"state":"backlog","triage":accept_id}),
                delegated: false,
            },
        )
        .await?;
        steps.push(step("backlog", Some(&event_id), StepStatus::Published));
    } else {
        let backlog = history
            .state_ancestor(&common.issue, "backlog")
            .ok_or_else(|| refused("bw:reject:causality:state-transition"))?;
        let backlog_body = body(&backlog).map_err(refused)?;
        if backlog_body["triage"] != accept_id {
            return Err(refused("bw:reject:causality:triage-reference"));
        }
        steps.push(step("backlog", backlog["id"].as_str(), StepStatus::Reused));
    }

    finish(port, &common.repo, common.issue, steps).await
}

/// Select a writer, resuming an unassignment/assignment/Ready-rebind chain.
pub async fn assign_writer<P: OperationPort>(
    port: &mut P,
    request: AssignWriterRequest,
) -> Result<OperationOutcome, OperationError<P::Error>> {
    let common = normalized_request(IssueRequest::new(request.repo, request.issue))?;
    let writer = normalized_hex(&request.writer, "writer")?;
    let mut history = load(port, &common.repo).await?;
    history.ensure_issue(&common.repo, &common.issue)?;
    let (_, current_state) = history
        .current_state(&common.issue)
        .ok_or_else(|| refused("bw:reject:causality:not-enrolled"))?;
    let current_state = body(&current_state).map_err(refused)?;
    if !matches!(current_state["state"].as_str(), Some("backlog" | "ready")) {
        return Err(refused("bw:reject:causality:assignment-state"));
    }
    let mut steps = Vec::new();
    let mut head = history.chain_head(&common.issue, "assignment")?;

    if let Some(current) = head.clone() {
        let operation = tag(&current, "t").unwrap_or_default();
        let current_writer = tag(&current, "p").unwrap_or_default();
        if operation == "assignment" && current_writer != writer {
            let prior = current["id"].as_str().unwrap_or_default();
            let (event_id, _) = publish_assignment(
                port,
                &mut history,
                &common.repo,
                &common.issue,
                current_writer,
                "unassignment",
                Some(prior),
            )
            .await?;
            steps.push(step("unassignment", Some(&event_id), StepStatus::Published));
            history = load(port, &common.repo).await?;
            head = history.chain_head(&common.issue, "assignment")?;
        } else if operation == "unassignment" {
            steps.push(step(
                "unassignment",
                current["id"].as_str(),
                StepStatus::Reused,
            ));
        } else {
            steps.push(step("unassignment", None, StepStatus::NotRequired));
        }
    } else {
        steps.push(step("unassignment", None, StepStatus::NotRequired));
    }

    let assignment_id = match head {
        Some(current)
            if tag(&current, "t") == Some("assignment")
                && tag(&current, "p") == Some(writer.as_str()) =>
        {
            let event_id = current["id"].as_str().unwrap_or_default().to_owned();
            steps.push(step("assignment", Some(&event_id), StepStatus::Reused));
            event_id
        }
        Some(current) if tag(&current, "t") == Some("unassignment") => {
            let prior = current["id"].as_str().unwrap_or_default();
            let (event_id, _) = publish_assignment(
                port,
                &mut history,
                &common.repo,
                &common.issue,
                &writer,
                "assignment",
                Some(prior),
            )
            .await?;
            steps.push(step("assignment", Some(&event_id), StepStatus::Published));
            event_id
        }
        None => {
            let (event_id, _) = publish_assignment(
                port,
                &mut history,
                &common.repo,
                &common.issue,
                &writer,
                "assignment",
                None,
            )
            .await?;
            steps.push(step("assignment", Some(&event_id), StepStatus::Published));
            event_id
        }
        Some(_) => return Err(refused("bw:conflict:causality:fork")),
    };

    history = load(port, &common.repo).await?;
    let (state_id, state_event) = history
        .current_state(&common.issue)
        .ok_or_else(|| refused("bw:reject:causality:not-enrolled"))?;
    let state_body = body(&state_event).map_err(refused)?;
    if state_body["state"] == "ready" && state_body["assignment"] != assignment_id {
        let stream = state_body["stream"]
            .as_str()
            .ok_or_else(|| refused("bw:reject:causality:ready-fields"))?;
        let update = state_body["update"]
            .as_str()
            .ok_or_else(|| refused("bw:reject:causality:ready-fields"))?;
        let mut content = json!({
            "state":"ready",
            "stream":stream,
            "assignment":assignment_id,
            "update":update,
            "rework":state_body.get("rework").cloned().unwrap_or(Value::Null),
        });
        if let Some(terminal_set) = state_body.get("terminal_set") {
            content["terminal_set"] = terminal_set.clone();
        }
        let (event_id, _) = publish_record(
            port,
            &mut history,
            RecordInput {
                repo: &common.repo,
                issue: &common.issue,
                required_signer: None,
                record_type: RecordType::IssueState,
                extra_tags: vec![vec!["previous".to_owned(), state_id.to_owned()]],
                content,
                delegated: false,
            },
        )
        .await?;
        steps.push(step("ready_rebind", Some(&event_id), StepStatus::Published));
    } else if state_body["state"] == "ready" {
        let assignment = history
            .event(&assignment_id)
            .ok_or_else(|| refused("bw:reject:causality:assignment-head"))?;
        if !history.ready_rebind_is_compatible(&state_event, &assignment.wire) {
            return Err(refused("bw:reject:causality:ready-rebind"));
        }
        steps.push(step("ready_rebind", Some(&state_id), StepStatus::Reused));
    } else {
        steps.push(step("ready_rebind", None, StepStatus::NotRequired));
    }

    finish(port, &common.repo, common.issue, steps).await
}

/// Move an accepted issue to Ready using current update and assignment heads.
pub async fn move_to_ready<P: OperationPort>(
    port: &mut P,
    request: MoveToReadyRequest,
) -> Result<OperationOutcome, OperationError<P::Error>> {
    let common = normalized_request(IssueRequest::new(request.repo, request.issue))?;
    let stream = request.stream.trim().to_owned();
    if stream.is_empty() || stream.chars().any(char::is_control) {
        return Err(refused("invalid BW stream"));
    }
    let mut history = load(port, &common.repo).await?;
    history.ensure_issue(&common.repo, &common.issue)?;
    let mut steps = Vec::new();
    let (_, state_event) = history
        .current_state(&common.issue)
        .ok_or_else(|| refused("bw:reject:causality:not-enrolled"))?;
    if body(&state_event).map_err(refused)?["state"] == "triage" {
        let accept = history
            .chain_head(&common.issue, "triage-action")?
            .ok_or_else(|| refused("bw:reject:causality:triage-state"))?;
        if body(&accept).map_err(refused)?["action"] != "accept" {
            return Err(refused("bw:reject:causality:triage-state"));
        }
        let accepted = accept_to_backlog(
            port,
            AcceptToBacklogRequest {
                repo: common.repo.clone(),
                issue: common.issue.clone(),
                delegated: false,
            },
        )
        .await?;
        steps.extend(accepted.steps);
        history = load(port, &common.repo).await?;
    }

    let update = history
        .chain_head(&common.issue, "issue-update")?
        .ok_or_else(|| refused("bw:pending:references:missing-reference"))?;
    let update_id = update["id"].as_str().unwrap_or_default().to_owned();
    let assignment = history.chain_head(&common.issue, "assignment")?;
    let assignment_id = assignment
        .filter(|event| tag(event, "t") == Some("assignment"))
        .and_then(|event| event["id"].as_str().map(str::to_owned));
    let (state_id, state_event) = history
        .current_state(&common.issue)
        .ok_or_else(|| refused("bw:reject:causality:not-enrolled"))?;
    let state_body = body(&state_event).map_err(refused)?;
    if state_body["state"] == "ready"
        && state_body["stream"] == stream
        && state_body["assignment"] == json!(assignment_id)
        && state_body["update"] == update_id
        && state_body.get("rework").unwrap_or(&Value::Null) == &json!(request.rework)
        && request
            .terminal_set
            .as_deref()
            .is_none_or(|terminal| state_body["terminal_set"].as_str() == Some(terminal))
    {
        steps.push(step("ready", Some(&state_id), StepStatus::Reused));
        return finish(port, &common.repo, common.issue, steps).await;
    }

    let mut content = json!({
        "state":"ready",
        "stream":stream,
        "assignment":assignment_id,
        "update":update_id,
        "rework":request.rework,
    });
    if let Some(terminal_set) = request.terminal_set {
        content["terminal_set"] = json!(terminal_set);
    }
    let (event_id, _) = publish_record(
        port,
        &mut history,
        RecordInput {
            repo: &common.repo,
            issue: &common.issue,
            required_signer: None,
            record_type: RecordType::IssueState,
            extra_tags: vec![vec!["previous".to_owned(), state_id.to_owned()]],
            content,
            delegated: false,
        },
    )
    .await?;
    steps.push(step("ready", Some(&event_id), StepStatus::Published));
    finish(port, &common.repo, common.issue, steps).await
}

/// Let the selected writer move Ready to In Development.
pub async fn start_development<P: OperationPort>(
    port: &mut P,
    request: IssueRequest,
) -> Result<OperationOutcome, OperationError<P::Error>> {
    let request = normalized_request(request)?;
    let mut history = load(port, &request.repo).await?;
    history.ensure_issue(&request.repo, &request.issue)?;
    let (state_id, state_event) = history
        .current_state(&request.issue)
        .ok_or_else(|| refused("bw:reject:causality:not-enrolled"))?;
    let state_body = body(&state_event).map_err(refused)?;
    if matches!(
        state_body["state"].as_str(),
        Some("in-development" | "implemented")
    ) {
        let existing = history
            .state_ancestor(&request.issue, "in-development")
            .ok_or_else(|| refused("bw:reject:causality:state-transition"))?;
        return finish(
            port,
            &request.repo,
            request.issue,
            vec![step(
                "start_development",
                existing["id"].as_str(),
                StepStatus::Reused,
            )],
        )
        .await;
    }
    if state_body["state"] != "ready" {
        return Err(refused("BW issue is not ready"));
    }
    let assignment_id = state_body["assignment"]
        .as_str()
        .ok_or_else(|| refused("ready issue has no selected writer"))?;
    let assignment = history
        .chain_head(&request.issue, "assignment")?
        .filter(|event| event["id"].as_str() == Some(assignment_id))
        .ok_or_else(|| refused("bw:reject:causality:assignment-head"))?;
    let writer =
        tag(&assignment, "p").ok_or_else(|| refused("selected BW assignment has no writer"))?;
    let stream = state_body["stream"]
        .as_str()
        .ok_or_else(|| refused("ready issue has no stream"))?;
    let (event_id, _) = publish_record(
        port,
        &mut history,
        RecordInput {
            repo: &request.repo,
            issue: &request.issue,
            required_signer: Some(writer),
            record_type: RecordType::IssueState,
            extra_tags: vec![vec!["previous".to_owned(), state_id.to_owned()]],
            content: json!({
                "state":"in-development",
                "stream":stream,
                "assignment":assignment_id,
            }),
            delegated: false,
        },
    )
    .await?;
    finish(
        port,
        &request.repo,
        request.issue,
        vec![step(
            "start_development",
            Some(&event_id),
            StepStatus::Published,
        )],
    )
    .await
}

/// Let the selected writer mark its pushed canonical stream head implemented.
pub async fn mark_implemented<P: OperationPort>(
    port: &mut P,
    request: MarkImplementedRequest,
) -> Result<OperationOutcome, OperationError<P::Error>> {
    let common = normalized_request(IssueRequest::new(request.repo, request.issue))?;
    let expected_commit = request
        .commit
        .as_deref()
        .map(normalized_commit)
        .transpose()?;
    let tests = request.tests.trim().to_owned();
    if tests.is_empty() {
        return Err(refused(
            "an honest tests and limitations summary is required",
        ));
    }
    let mut history = load(port, &common.repo).await?;
    history.ensure_issue(&common.repo, &common.issue)?;
    let (state_id, state_event) = history
        .current_state(&common.issue)
        .ok_or_else(|| refused("bw:reject:causality:not-enrolled"))?;
    let state_body = body(&state_event).map_err(refused)?;
    if state_body["state"] == "implemented" {
        if expected_commit
            .as_deref()
            .is_some_and(|expected| state_body["commit"] != expected)
        {
            return Err(refused("implemented commit differs from requested commit"));
        }
        return finish(
            port,
            &common.repo,
            common.issue,
            vec![step(
                "mark_implemented",
                Some(&state_id),
                StepStatus::Reused,
            )],
        )
        .await;
    }
    if state_body["state"] != "in-development" {
        return Err(refused("BW issue is not in development"));
    }
    let stream = state_body["stream"]
        .as_str()
        .ok_or_else(|| refused("in-development issue has no stream"))?
        .to_owned();
    let assignment_id = state_body["assignment"]
        .as_str()
        .ok_or_else(|| refused("in-development issue has no selected writer"))?
        .to_owned();
    let assignment = history
        .event(&assignment_id)
        .filter(|record| record.decision.outcome == "accept")
        .ok_or_else(|| refused("bw:reject:causality:assignment-head"))?;
    let writer = tag(&assignment.wire, "p")
        .ok_or_else(|| refused("selected BW assignment has no writer"))?
        .to_owned();
    let remote_head = port
        .remote_head(&common.repo, &stream)
        .await
        .map_err(OperationError::Port)?;
    let remote_head = normalized_commit(&remote_head)?;
    if expected_commit
        .as_deref()
        .is_some_and(|expected| expected != remote_head)
    {
        return Err(refused(format!(
            "pushed commit mismatch: remote {stream} is {remote_head}, not {}",
            expected_commit.unwrap_or_default()
        )));
    }
    let created_at = port.now().map_err(OperationError::Port)?;
    let remote_readback = json!({
        "repo":common.repo,
        "stream":stream,
        "head":remote_head,
        "observed_at":created_at,
    });
    history.evidence.git_readbacks.push(remote_readback.clone());
    history
        .consumer
        .observe(history.evidence.clone(), created_at);
    let identity = identity(port, Some(&writer)).await?;
    let activation = history
        .consumer
        .activation()
        .map_err(|error| refused(error.to_string()))?;
    let mut tags = vec![
        vec!["a".to_owned(), common.repo.clone()],
        vec!["policy".to_owned(), activation.policy],
        vec!["issue".to_owned(), common.issue.clone()],
        vec!["previous".to_owned(), state_id.to_owned()],
    ];
    tags.extend(identity.event_tags.iter().cloned());
    let draft = record(
        RecordType::IssueState,
        tags,
        json!({
            "state":"implemented",
            "stream":stream,
            "assignment":assignment_id,
            "commit":remote_head,
            "tests":tests,
            "remote_readback":remote_readback,
        }),
    )
    .map_err(map_sdk)?;
    let (event_id, _) = publish_draft(
        port,
        &mut history,
        draft,
        &identity,
        Some(&writer),
        created_at,
    )
    .await?;
    finish(
        port,
        &common.repo,
        common.issue,
        vec![step(
            "mark_implemented",
            Some(&event_id),
            StepStatus::Published,
        )],
    )
    .await
}
