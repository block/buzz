//! Opt-in, event-driven native agent-job delegation.
//!
//! The model never polls. A terminal assistant response is either a typed
//! `BUZZ_ACTION_V1` control record or, for a manager, the final owner-facing
//! response. Buzz publishes the corresponding durable event after the ACP turn
//! has ended.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use buzz_core::agent_job::{
    EvidenceRef, JobBudget, JobRequest, JobResult, JobStatus, JobTerminalStatus, ResultContract,
    AGENT_JOB_VERSION,
};
use buzz_core::kind::{
    KIND_JOB_ACCEPTED, KIND_JOB_CANCEL, KIND_JOB_ERROR, KIND_JOB_REQUEST, KIND_JOB_RESULT,
};
use chrono::Utc;
use nostr::{Event, EventId, PublicKey};
use serde::{Deserialize, Deserializer};
use uuid::Uuid;

use crate::queue::{parse_thread_tags, FlushBatch, ThreadTags};
use crate::relay::RestClient;

const ACTION_PREFIX: &str = "BUZZ_ACTION_V1";
const DEFAULT_DEADLINE_SECS: u64 = 900;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 8192;
const MANAGER_CONTROL_PROMPT: &str = concat!(
    "[NATIVE_JOB_CONTROL_V1]\n",
    "To delegate, end this turn with exactly one record: BUZZ_ACTION_V1 followed by JSON ",
    "{\"op\":\"delegate_task\",\"role\":\"research|builder|qa|screenshots|memory|git|translation\",",
    "\"objective\":\"one bounded task\",\"evidence_refs\":[],\"result_contract\":",
    "{\"kind\":\"evidence_packet|build_result|qa_result|screenshot_manifest|memory_result|git_readiness|translation_manifest\",",
    "\"required\":[]},\"constraints\":[],\"max_model_calls\":3}. ",
    "The host sends it, ends this turn, suspends you, and wakes you with only the compact ",
    "specialist result as the next prompt. Never use Buzz tools to delegate, poll, wait, ",
    "check presence, send progress, or deliver the final response. Do not inspect facts ",
    "assigned to Research or reread artifacts yourself. Preserve a specialist's exact ",
    "evidence refs unchanged in downstream tasks unless that specialist explicitly marks ",
    "a ref invalid. Treat evidence references—especially atlas-memory:// URIs—as opaque ",
    "machine strings: copy them byte-for-byte from the result, never retype, shorten repeated ",
    "words, normalize, or reconstruct them. Copy every artifact reference byte-for-byte into ",
    "downstream tasks and owner responses; never abbreviate, normalize, or retype an artifact ",
    "basename. Screenshots receives only one exact product route, approved evidence refs, ",
    "capture requirements, and an ATLAS-owned output directory. Memory receives only structured ",
    "verified findings or exact evidence IDs; route a current Memory reference directly without ",
    "asking Research to recheck it. Git receives one exact worktree path, expected branch/HEAD, ",
    "requested file scope, and whether a read-only remote check is allowed; it only inspects and ",
    "prepares commands for later human approval. After every READY Builder result, delegate one ",
    "bounded QA check with the same evidence refs and artifact scope before presenting work as ",
    "ready. On QA PASS, answer the owner. On QA FAIL, allow at most one bounded Builder correction ",
    "and one QA recheck; if that recheck fails, escalate the exact failures to the owner. Never ",
    "create an open-ended Builder/QA loop. The host publishes ordinary final assistant content ",
    "to the owner. If no delegation is needed, answer the owner normally. For an opt-in hybrid ",
    "Helpdesk article task, delegate Memory once with one HYBRID_WORKFLOW_V1:{json} constraint. ",
    "Use these exact JSON keys: kind=helpdesk_article_v1, stage=memory, evidence_id, worktree ",
    "(an absolute path), target_file (a repository-relative path), expected_branch, expected_head, one exact batched ",
    "research_command, research_requirements, preserve_screenshot_placeholders=true, ",
    "allow_human_reviewed=false, and reasoning_qa=true. The host performs the predictable ",
    "transitions after Memory; do not delegate later hybrid stages yourself. Put only the ",
    "specific product facts or questions Research must answer in research_requirements; never ",
    "put Builder, QA, Git, preservation, publication, or workflow instructions there.",
);

/// Native-job runtime role for an opted-in harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeJobRole {
    /// Receives owner requests, delegates, then presents the final result.
    Manager,
    /// Receives one bounded job and returns one terminal result.
    Worker,
}

/// Default-off native-job configuration resolved from the managed-agent env.
#[derive(Debug, Clone)]
pub struct NativeJobsConfig {
    /// Whether the native job path is enabled.
    pub enabled: bool,
    /// Harness behavior when enabled.
    pub role: Option<NativeJobRole>,
    /// Logical role to exact worker pubkey, used only by a manager.
    pub delegation_targets: HashMap<String, String>,
    /// Logical worker role (`research`, `builder`, `qa`, `screenshots`,
    /// `memory`, `git`, or `translation`) for worker harnesses.
    pub worker_role: Option<String>,
    /// Exact manager pubkey accepted by a worker harness.
    pub delegator: Option<String>,
    /// Opt-in deterministic execution and sequential Helpdesk routing.
    pub hybrid_enabled: bool,
    /// Authoritative ATLAS Memory index used by model-free lookup work.
    pub hybrid_memory_index: Option<PathBuf>,
    /// Filesystem roots deterministic workers may inspect.
    pub hybrid_allowed_roots: Vec<PathBuf>,
    /// Default wall-clock deadline assigned to new jobs.
    pub deadline: Duration,
    /// Process-local duplicate suppression for replayed task events.
    seen_requests: Arc<Mutex<HashSet<Uuid>>>,
    seen_terminals: Arc<Mutex<HashSet<Uuid>>>,
    seen_cancels: Arc<Mutex<HashSet<Uuid>>>,
}

impl Default for NativeJobsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            role: None,
            delegation_targets: HashMap::new(),
            worker_role: None,
            delegator: None,
            hybrid_enabled: false,
            hybrid_memory_index: None,
            hybrid_allowed_roots: vec![],
            deadline: Duration::from_secs(DEFAULT_DEADLINE_SECS),
            seen_requests: Arc::new(Mutex::new(HashSet::new())),
            seen_terminals: Arc::new(Mutex::new(HashSet::new())),
            seen_cancels: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

impl NativeJobsConfig {
    #[cfg(test)]
    pub(crate) fn enabled_for_test() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }

    /// Resolve native-job settings from environment variables.
    pub fn from_env() -> anyhow::Result<Self> {
        if std::env::var("BUZZ_ACP_NATIVE_JOBS").as_deref() != Ok("true") {
            return Ok(Self::default());
        }
        let role = std::env::var("BUZZ_ACP_NATIVE_JOB_ROLE")?;
        let (role, worker_role) = match role.trim().to_ascii_lowercase().as_str() {
            "manager" => (NativeJobRole::Manager, None),
            "research" => (NativeJobRole::Worker, Some("research".to_string())),
            "builder" => (NativeJobRole::Worker, Some("builder".to_string())),
            "qa" => (NativeJobRole::Worker, Some("qa".to_string())),
            "screenshots" => (NativeJobRole::Worker, Some("screenshots".to_string())),
            "memory" => (NativeJobRole::Worker, Some("memory".to_string())),
            "git" => (NativeJobRole::Worker, Some("git".to_string())),
            "translation" => (NativeJobRole::Worker, Some("translation".to_string())),
            _ => {
                anyhow::bail!(
                    "BUZZ_ACP_NATIVE_JOB_ROLE must be manager, research, builder, qa, screenshots, memory, git, or translation"
                )
            }
        };
        let raw_targets =
            std::env::var("BUZZ_ACP_DELEGATION_TARGETS").unwrap_or_else(|_| "{}".to_string());
        let raw_delegation_targets: HashMap<String, String> = serde_json::from_str(&raw_targets)
            .map_err(|error| {
                anyhow::anyhow!("BUZZ_ACP_DELEGATION_TARGETS must be a JSON object: {error}")
            })?;
        let mut delegation_targets = HashMap::new();
        for (name, pubkey) in raw_delegation_targets {
            let name = name.trim().to_ascii_lowercase();
            let pubkey = pubkey.trim().to_ascii_lowercase();
            PublicKey::from_hex(&pubkey).map_err(|_| {
                anyhow::anyhow!("delegation target {name:?} is not a valid public key")
            })?;
            delegation_targets.insert(name, pubkey);
        }
        if role == NativeJobRole::Manager
            && !(delegation_targets.contains_key("research")
                && delegation_targets.contains_key("builder"))
        {
            anyhow::bail!("native manager requires research and builder delegation targets");
        }
        let delegator = if role == NativeJobRole::Worker {
            let pubkey = std::env::var("BUZZ_ACP_NATIVE_JOB_DELEGATOR")?
                .trim()
                .to_ascii_lowercase();
            PublicKey::from_hex(&pubkey).map_err(|_| {
                anyhow::anyhow!("native worker delegator is not a valid public key")
            })?;
            Some(pubkey)
        } else {
            None
        };
        let deadline_secs = std::env::var("BUZZ_ACP_NATIVE_JOB_DEADLINE_SECS")
            .ok()
            .map(|value| value.parse::<u64>())
            .transpose()?
            .unwrap_or(DEFAULT_DEADLINE_SECS)
            .clamp(30, 86_400);
        let hybrid_enabled = std::env::var("BUZZ_ACP_NATIVE_JOB_HYBRID").as_deref() == Ok("true");
        let hybrid_memory_index = std::env::var_os("BUZZ_ACP_HYBRID_MEMORY_INDEX")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let hybrid_allowed_roots: Vec<PathBuf> = std::env::var_os("BUZZ_ACP_HYBRID_ALLOWED_ROOTS")
            .map(|value| std::env::split_paths(&value).collect())
            .unwrap_or_default();
        if hybrid_enabled && role == NativeJobRole::Worker {
            match worker_role.as_deref() {
                Some("memory") if hybrid_memory_index.is_none() => {
                    anyhow::bail!("hybrid memory worker requires BUZZ_ACP_HYBRID_MEMORY_INDEX")
                }
                Some("git" | "qa") if hybrid_allowed_roots.is_empty() => {
                    anyhow::bail!("hybrid git/qa worker requires BUZZ_ACP_HYBRID_ALLOWED_ROOTS")
                }
                _ => {}
            }
        }
        if hybrid_enabled
            && role == NativeJobRole::Manager
            && !["memory", "research", "builder", "qa", "git"]
                .iter()
                .all(|target| delegation_targets.contains_key(*target))
        {
            anyhow::bail!("hybrid manager requires memory, research, builder, qa, and git targets");
        }
        if hybrid_enabled && role == NativeJobRole::Manager && hybrid_allowed_roots.is_empty() {
            anyhow::bail!("hybrid manager requires BUZZ_ACP_HYBRID_ALLOWED_ROOTS");
        }
        Ok(Self {
            enabled: true,
            role: Some(role),
            delegation_targets,
            worker_role,
            delegator,
            hybrid_enabled,
            hybrid_memory_index,
            hybrid_allowed_roots,
            deadline: Duration::from_secs(deadline_secs),
            seen_requests: Arc::new(Mutex::new(HashSet::new())),
            seen_terminals: Arc::new(Mutex::new(HashSet::new())),
            seen_cancels: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    /// Event kinds added to the ordinary mention subscription when enabled.
    pub fn subscription_kinds(&self) -> &'static [u32] {
        if self.enabled {
            &[
                buzz_core::kind::KIND_JOB_REQUEST,
                buzz_core::kind::KIND_JOB_ACCEPTED,
                buzz_core::kind::KIND_JOB_PROGRESS,
                buzz_core::kind::KIND_JOB_RESULT,
                buzz_core::kind::KIND_JOB_CANCEL,
                buzz_core::kind::KIND_JOB_ERROR,
            ]
        } else {
            &[]
        }
    }

    /// Authorize and atomically claim one inbound native event.
    ///
    /// This role gate runs before queueing, so an unrelated same-owner agent
    /// cannot spend an ATLAS model call. Replayed task IDs are also dropped.
    pub fn claim_inbound_event(&self, event: &Event) -> bool {
        if !self.enabled {
            return true;
        }
        let kind = event.kind.as_u16() as u32;
        let author = event.pubkey.to_hex();
        match (self.role, kind) {
            (Some(NativeJobRole::Worker), KIND_JOB_REQUEST) => {
                if self.delegator.as_deref() != Some(author.as_str()) {
                    return false;
                }
                let Ok(request) = buzz_core::agent_job::parse_job_request(&event.content) else {
                    return false;
                };
                if self.worker_role.as_deref() != Some(request.assigned_role.as_str()) {
                    return false;
                }
                claim_task(&self.seen_requests, request.task_id)
            }
            (Some(NativeJobRole::Worker), KIND_JOB_CANCEL) => {
                if self.delegator.as_deref() != Some(author.as_str()) {
                    return false;
                }
                buzz_core::agent_job::parse_job_cancel(&event.content)
                    .is_ok_and(|cancel| claim_task(&self.seen_cancels, cancel.task_id))
            }
            (Some(NativeJobRole::Manager), KIND_JOB_RESULT | KIND_JOB_ERROR) => {
                if !self
                    .delegation_targets
                    .values()
                    .any(|pubkey| pubkey == &author)
                {
                    return false;
                }
                buzz_core::agent_job::parse_job_result(&event.content)
                    .is_ok_and(|result| claim_task(&self.seen_terminals, result.task_id))
            }
            _ => false,
        }
    }

    /// Compact host instruction appended to an opted-in manager's ordinary
    /// owner prompt. Native callback prompts already carry their own contract.
    pub fn manager_control_prompt(&self) -> Option<&'static str> {
        (self.enabled && self.role == Some(NativeJobRole::Manager))
            .then_some(MANAGER_CONTROL_PROMPT)
    }
}

fn claim_task(tasks: &Mutex<HashSet<Uuid>>, task_id: Uuid) -> bool {
    let Ok(mut tasks) = tasks.lock() else {
        return false;
    };
    if tasks.len() >= 4096 {
        tasks.clear();
    }
    tasks.insert(task_id)
}

/// Typed terminal output emitted by native-job agents.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeAction {
    /// Delegate exactly one sequential child task.
    #[serde(rename = "delegate_task")]
    Delegate {
        /// Configured logical role.
        role: String,
        /// One bounded objective.
        objective: String,
        /// Durable evidence references to reuse.
        #[serde(default, deserialize_with = "deserialize_evidence_refs")]
        evidence_refs: Vec<EvidenceRef>,
        /// Expected result type.
        result_contract: ResultContract,
        /// Task-specific restrictions.
        #[serde(default)]
        constraints: Vec<String>,
        /// Advisory model-call ceiling.
        #[serde(default)]
        max_model_calls: Option<u32>,
    },
    /// Complete the assigned task.
    #[serde(rename = "complete_task")]
    Complete {
        /// Assigned task identifier.
        task_id: Uuid,
        /// At most five concise result bullets.
        #[serde(default, deserialize_with = "deserialize_compact_strings")]
        summary: Vec<String>,
        /// Durable artifact references.
        #[serde(default, deserialize_with = "deserialize_compact_strings")]
        artifacts: Vec<String>,
        /// Evidence supporting the result.
        #[serde(default, deserialize_with = "deserialize_evidence_refs")]
        evidence_refs: Vec<EvidenceRef>,
        /// Checks performed.
        #[serde(default, deserialize_with = "deserialize_compact_strings")]
        checks: Vec<String>,
        /// Unresolved facts.
        #[serde(default, deserialize_with = "deserialize_compact_strings")]
        gaps: Vec<String>,
    },
    /// Report a real blocker without retry chatter.
    #[serde(rename = "block_task")]
    Block {
        /// Assigned task identifier.
        task_id: Uuid,
        /// Concise blocker summary.
        #[serde(deserialize_with = "deserialize_compact_strings")]
        summary: Vec<String>,
        /// Unresolved facts or decisions.
        #[serde(default, deserialize_with = "deserialize_compact_strings")]
        gaps: Vec<String>,
    },
    /// Report terminal failure.
    #[serde(rename = "fail_task")]
    Fail {
        /// Assigned task identifier.
        task_id: Uuid,
        /// Concise failure summary.
        #[serde(deserialize_with = "deserialize_compact_strings")]
        summary: Vec<String>,
        /// Checks attempted before failure.
        #[serde(default, deserialize_with = "deserialize_compact_strings")]
        checks: Vec<String>,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ActionEvidenceRef {
    Structured(EvidenceRef),
    Compact(String),
}

fn deserialize_evidence_refs<'de, D>(deserializer: D) -> Result<Vec<EvidenceRef>, D::Error>
where
    D: Deserializer<'de>,
{
    Vec::<ActionEvidenceRef>::deserialize(deserializer).map(|references| {
        references
            .into_iter()
            .map(|reference| match reference {
                ActionEvidenceRef::Structured(reference) => reference,
                ActionEvidenceRef::Compact(uri) => EvidenceRef {
                    uri,
                    revision: None,
                    section: None,
                },
            })
            .collect()
    })
}

fn deserialize_compact_strings<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Vec::<serde_json::Value>::deserialize(deserializer).map(|values| {
        values
            .into_iter()
            .map(|value| match value {
                serde_json::Value::String(value) => value,
                value => value.to_string(),
            })
            .collect()
    })
}

/// Parse an exact terminal native action. Ordinary prose returns `Ok(None)`.
pub fn parse_terminal_action(output: &str) -> anyhow::Result<Option<NativeAction>> {
    let trimmed = output.trim();
    let Some(remainder) = trimmed.strip_prefix(ACTION_PREFIX) else {
        return Ok(None);
    };
    if !remainder
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_whitespace())
    {
        anyhow::bail!("BUZZ_ACTION_V1 must be followed by whitespace and one JSON record");
    }
    let json = remainder.trim_start_matches(|character: char| character.is_ascii_whitespace());
    if json.contains(ACTION_PREFIX) {
        anyhow::bail!("terminal output contains multiple native actions");
    }
    let action = serde_json::from_str(json)
        .map_err(|error| anyhow::anyhow!("invalid BUZZ_ACTION_V1: {error}"))?;
    Ok(Some(action))
}

fn translation_terminal_action(output: &str, request: &JobRequest) -> anyhow::Result<NativeAction> {
    let mut lines = output.trim().lines();
    let verdict = lines.next().unwrap_or_default().trim();
    let finding = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    if finding.len() > 1024 {
        anyhow::bail!("translation QA finding exceeds 1024 characters");
    }
    if finding.contains(ACTION_PREFIX) || output.contains('{') || output.contains('}') {
        anyhow::bail!("translation QA must not construct a native-job envelope");
    }

    let manifest_refs: Vec<String> = request
        .evidence_refs
        .iter()
        .filter(|reference| reference.section.as_deref() == Some("TranslationManifest"))
        .map(|reference| reference.uri.clone())
        .collect();
    if manifest_refs.len() != 1 {
        anyhow::bail!("translation QA requires exactly one TranslationManifest reference");
    }

    match verdict {
        "PASS" if finding.is_empty() => Ok(NativeAction::Complete {
            task_id: request.task_id,
            summary: vec!["PASS".into()],
            artifacts: manifest_refs,
            evidence_refs: request.evidence_refs.clone(),
            checks: vec!["Bounded linguistic QA passed".into()],
            gaps: vec![],
        }),
        "REVIEW_NEEDED" if !finding.is_empty() => Ok(NativeAction::Complete {
            task_id: request.task_id,
            summary: vec!["REVIEW_NEEDED".into()],
            artifacts: manifest_refs,
            evidence_refs: request.evidence_refs.clone(),
            checks: vec!["Deterministic validation passed before linguistic QA".into()],
            gaps: vec![finding],
        }),
        "FAIL" if !finding.is_empty() => Ok(NativeAction::Fail {
            task_id: request.task_id,
            summary: vec!["FAIL".into()],
            checks: vec![finding],
        }),
        "PASS" => anyhow::bail!("PASS must not include a finding"),
        "REVIEW_NEEDED" | "FAIL" => {
            anyhow::bail!("{verdict} requires one bounded finding")
        }
        _ => anyhow::bail!("translation QA verdict must be PASS, REVIEW_NEEDED, or FAIL"),
    }
}

pub(crate) fn event_tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        (parts.first().map(String::as_str) == Some(name))
            .then(|| parts.get(1).map(String::as_str))
            .flatten()
    })
}

fn native_event(batch: &FlushBatch) -> Option<&Event> {
    batch
        .events
        .iter()
        .rev()
        .map(|event| &event.event)
        .find(|event| {
            matches!(
                event.kind.as_u16() as u32,
                KIND_JOB_REQUEST | KIND_JOB_RESULT | KIND_JOB_ERROR
            )
        })
}

/// Whether a batch is a request or terminal native-job turn.
pub fn is_native_job_batch(batch: &FlushBatch) -> bool {
    native_event(batch).is_some()
}

fn root_task_id(batch: &FlushBatch) -> anyhow::Result<String> {
    if let Some(root) = native_event(batch).and_then(|event| event_tag(event, "root")) {
        return Ok(root.to_string());
    }
    let last = &batch
        .events
        .last()
        .ok_or_else(|| anyhow::anyhow!("native job batch is empty"))?
        .event;
    Ok(parse_thread_tags(last)
        .root_event_id
        .unwrap_or_else(|| last.id.to_hex()))
}

fn worker_request(batch: &FlushBatch) -> anyhow::Result<(&Event, JobRequest)> {
    let event = native_event(batch)
        .filter(|event| event.kind.as_u16() as u32 == KIND_JOB_REQUEST)
        .ok_or_else(|| anyhow::anyhow!("worker terminal action has no job request"))?;
    Ok((
        event,
        buzz_core::agent_job::parse_job_request(&event.content)?,
    ))
}

/// Build a compact prompt for a native job event, bypassing conversation fetch.
pub fn compact_prompt(batch: &FlushBatch) -> Option<String> {
    let event = native_event(batch)?;
    match event.kind.as_u16() as u32 {
        KIND_JOB_REQUEST => {
            let request = buzz_core::agent_job::parse_job_request(&event.content).ok()?;
            let hybrid_context = crate::hybrid::worker_context(&request)
                .map(|context| format!("\n{context}"))
                .unwrap_or_default();
            let efficiency = match request.assigned_role.as_str() {
                "research" => "Efficiency: use the exact evidence refs/resolver first. One fresh authoritative result is sufficient. Do not broadly discover files or verify the same fact twice unless that result reports stale, conflict, or error.",
                "builder" => "Efficiency: the envelope is complete. Do not load the full Helpdesk manual, unrelated skills/maps, or independently research supplied evidence. Use one scoped inspection, one edit, and one batched validation where practical. Never remove or rewrite existing placeholders, screenshot markers, frontmatter, metadata, or unrelated content unless explicitly required.",
                "qa" => "Efficiency: independently validate only the assigned artifact against the supplied evidence refs and applicable rules. Do not redo Research or use Git history/file discovery to locate evidence. Run the literal authoritative resolver command supplied in evidence refs exactly once and batch it with artifact/status/diff checks in one tool operation. If that command is missing, report a precise evidence gap instead of discovering it. Report exact PASS or FAIL findings. Keep summary to one PASS/FAIL string (the native contract permits at most five); put details in checks. A second tool operation requires a precise conflict or evidence gap. Read-only except your own ATLAS runtime/output directory.",
                "screenshots" => "Efficiency: treat Manager-supplied evidence refs as approved and resolved; never open their source files. Resolve the deferred guarded Playwright namespace with exactly one narrow tool-search call for browser_navigate and browser_take_screenshot (include browser_resize only when the requested dimensions differ from the attached 1280x720 viewport); never use shell/process discovery or a second tool search. Navigate once to the exact assigned Synup product route, then capture it. Do not browse for alternatives, use recovery/helper navigation, or change product data. Inspect the image returned by browser_take_screenshot directly; do not reread it with view_image or take a separate accessibility snapshot. If the named US demo fixture is absent, block immediately. Otherwise save only in the assigned ATLAS Screenshots output directory, run file type/dimensions/size/SHA-256 in one batched command, and return one compact screenshot_manifest terminal result. Emit no intermediate commentary: the terminal BUZZ_ACTION_V1 record must be your first and only assistant content. Never edit Helpdesk content.",
                "memory" => "Efficiency: use only the structured findings or exact evidence IDs in the task. Promote only a reusable verified claim with a source revision/date; reject transient task status, narrative, or unsupported candidates. Run the supplied deterministic memory resolver/index command once, preferably as one batch. No broad repository scans, source rereads, transcript storage, or independent fact research. If an identical current finding exists, return its exact evidence reference without rewriting. Supersede only when the candidate explicitly names the current reference and carries a newer verified source date. Write only under atlas/evidence/memory/ and return one compact memory_result terminal record with exact references. Emit no acknowledgement or intermediate commentary.",
                "git" => "Efficiency: inspect only the exact assigned worktree and scope. Run the supplied deterministic read-only Git inspector exactly once; use its existing tracking refs unless the task explicitly allows its read-only ls-remote check. Do not run repository-wide discovery or a second Git command. Never fetch, commit, stage, push, merge, rebase, reset, checkout, clean, delete, or edit. Return one compact git_readiness result. Summary must contain exactly one READY or BLOCKED string; put branch, HEAD, state, changed files, scope verdict, diff-check result, ahead/behind, blockers, and exact unexecuted future commit/push commands in checks. Emit no acknowledgement or intermediate commentary.",
                "translation" => "Efficiency: perform only bounded linguistic judgment on the exact source and generated locale references in the packet. The deterministic host already owns eligibility, hashing, translation memory, provider calls, batching, retries, writes, Markdown safety, protected terms, links, routes, SEO/build validation, usage accounting, reporting, and tracker output. Do not run the pipeline, translate content, discover repositories, edit files, publish, or delegate. Use only the relevant glossary subset and policy supplied in this task.",
                _ => "Efficiency: use only the task envelope and its exact references; avoid discovery and duplicate verification.",
            };
            let terminal_contract = if request.assigned_role == "translation" {
                "The host owns the native-job terminal envelope. Return exactly PASS on one line, or REVIEW_NEEDED/FAIL on the first line followed by one bounded finding on the remaining line. Return no JSON, prose preface, Markdown, acknowledgement, progress, polling, or delegation."
            } else {
                "This native callback overrides the legacy ATLAS_RESULT_V1/message-return instruction. Your final assistant content must contain only BUZZ_ACTION_V1 followed by one JSON record—no prose or Markdown. Complete shape: {\"op\":\"complete_task\",\"task_id\":\"TASK_ID\",\"summary\":[],\"artifacts\":[],\"evidence_refs\":[],\"checks\":[],\"gaps\":[]}. Blocked shape: {\"op\":\"block_task\",\"task_id\":\"TASK_ID\",\"summary\":[],\"gaps\":[]}. Failed shape: {\"op\":\"fail_task\",\"task_id\":\"TASK_ID\",\"summary\":[],\"checks\":[]}. Do not add fields to these shapes. Do not send a Buzz message and do not poll."
            }
            .replace("TASK_ID", &request.task_id.to_string());
            Some(format!(
                "[NATIVE_JOB_V1]\nTask: {}\nRole: {}\nObjective: {}\nEvidence refs: {}\nConstraints: {}\nResult contract: {}\n{}{}\n\n{}",
                request.task_id,
                request.assigned_role,
                request.objective,
                serde_json::to_string(&request.evidence_refs).ok()?,
                serde_json::to_string(&request.constraints).ok()?,
                serde_json::to_string(&request.result_contract).ok()?,
                efficiency,
                hybrid_context,
                terminal_contract,
            ))
        }
        KIND_JOB_RESULT | KIND_JOB_ERROR => {
            let result = buzz_core::agent_job::parse_job_result(&event.content).ok()?;
            let public_artifacts: Vec<_> = result
                .artifacts
                .iter()
                .filter(|value| !crate::hybrid::is_workflow_marker(value))
                .collect();
            Some(format!(
                "[SPECIALIST_RESULT_V1]\nTask: {}\nStatus: {:?}\nSummary: {}\nArtifacts: {}\nEvidence refs: {}\nChecks: {}\nGaps: {}\n\nReturn the concise final owner response or follow this delegation contract for the next sequential specialist:\n{}",
                result.task_id,
                result.status,
                serde_json::to_string(&result.summary).ok()?,
                serde_json::to_string(&public_artifacts).ok()?,
                serde_json::to_string(&result.evidence_refs).ok()?,
                serde_json::to_string(&result.checks).ok()?,
                serde_json::to_string(&result.gaps).ok()?,
                MANAGER_CONTROL_PROMPT,
            ))
        }
        _ => None,
    }
}

/// Publish a host-generated accepted/running event before the worker prompt.
pub async fn publish_accepted(config: &NativeJobsConfig, rest: &RestClient, batch: &FlushBatch) {
    if !config.enabled || config.role != Some(NativeJobRole::Worker) {
        return;
    }
    let Ok((request_event, request)) = worker_request(batch) else {
        return;
    };
    if config.worker_role.as_deref() != Some(request.assigned_role.as_str()) {
        return;
    }
    let status = JobStatus {
        v: AGENT_JOB_VERSION,
        task_id: request.task_id,
        status: "running".into(),
        detail: Some(format!("{} running", request.assigned_role)),
    };
    let builder = match buzz_sdk::build_agent_job_status(
        KIND_JOB_ACCEPTED,
        batch.channel_id,
        &request_event.pubkey.to_hex(),
        &request.root_task_id,
        &request_event.id.to_hex(),
        &status,
    ) {
        Ok(builder) => builder,
        Err(error) => {
            tracing::warn!("native job accepted build failed: {error}");
            return;
        }
    };
    if let Err(error) = submit_builder(rest, builder, "accepted").await {
        tracing::warn!("native job accepted publish failed: {error}");
    }
}

/// Effect of consuming a completed native-job ACP response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeTurnEffect {
    /// Whether native mode consumed/published the assistant response.
    pub handled: bool,
    /// Whether the task-local worker session should be discarded.
    pub invalidate_session: bool,
}

impl NativeTurnEffect {
    fn ignored() -> Self {
        Self {
            handled: false,
            invalidate_session: false,
        }
    }
}

/// Consume a successful turn's terminal output and publish its durable effect.
pub async fn handle_terminal_output(
    config: &NativeJobsConfig,
    rest: &RestClient,
    batch: &FlushBatch,
    output: &str,
) -> anyhow::Result<NativeTurnEffect> {
    if !config.enabled {
        return Ok(NativeTurnEffect::ignored());
    }
    let action = if config.role == Some(NativeJobRole::Worker)
        && config.worker_role.as_deref() == Some("translation")
    {
        let (_, request) = worker_request(batch)?;
        Some(translation_terminal_action(output, &request)?)
    } else {
        parse_terminal_action(output)?
    };
    match (config.role, action) {
        (
            Some(NativeJobRole::Manager),
            Some(NativeAction::Delegate {
                role,
                objective,
                evidence_refs,
                result_contract,
                constraints,
                max_model_calls,
            }),
        ) => {
            let role = role.trim().to_ascii_lowercase();
            let assignee = config
                .delegation_targets
                .get(&role)
                .ok_or_else(|| anyhow::anyhow!("no native delegation target for role {role:?}"))?;
            let parent_task_id = native_event(batch)
                .and_then(|event| buzz_core::agent_job::parse_job_result(&event.content).ok())
                .map(|result| result.task_id);
            let request = JobRequest {
                v: AGENT_JOB_VERSION,
                task_id: Uuid::new_v4(),
                root_task_id: root_task_id(batch)?,
                parent_task_id,
                assigned_role: role,
                objective,
                evidence_refs,
                result_contract,
                constraints,
                budget: JobBudget {
                    deadline_at: (Utc::now()
                        + chrono::Duration::from_std(config.deadline)
                            .unwrap_or_else(|_| chrono::Duration::minutes(15)))
                    .to_rfc3339(),
                    max_model_calls,
                    max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
                },
                attempt: 1,
            };
            let builder = buzz_sdk::build_agent_job_request(
                batch.channel_id,
                assignee,
                &rest.keys.public_key().to_hex(),
                &request,
            )?;
            submit_builder(rest, builder, "request").await?;
            Ok(NativeTurnEffect {
                handled: true,
                // Suspend the Manager session so it retains the owner's plan;
                // the next turn adds only the compact specialist result.
                invalidate_session: false,
            })
        }
        (Some(NativeJobRole::Manager), None) => {
            if output.trim().is_empty() {
                anyhow::bail!("native manager returned empty final output");
            }
            let thread_tags = owner_thread_tags(batch);
            crate::pool::post_failure_notice(rest, batch.channel_id, &thread_tags, output.trim())
                .await;
            Ok(NativeTurnEffect {
                handled: true,
                invalidate_session: true,
            })
        }
        (Some(NativeJobRole::Worker), Some(action)) => {
            let (request_event, request) = worker_request(batch)?;
            if config.worker_role.as_deref() != Some(request.assigned_role.as_str()) {
                anyhow::bail!("job assigned to a different worker role");
            }
            let (task_id, status, mut summary, mut artifacts, evidence_refs, mut checks, gaps) =
                match action {
                    NativeAction::Complete {
                        task_id,
                        summary,
                        artifacts,
                        evidence_refs,
                        checks,
                        gaps,
                    } => (
                        task_id,
                        JobTerminalStatus::Completed,
                        summary,
                        artifacts,
                        evidence_refs,
                        checks,
                        gaps,
                    ),
                    NativeAction::Block {
                        task_id,
                        summary,
                        gaps,
                    } => (
                        task_id,
                        JobTerminalStatus::Blocked,
                        summary,
                        vec![],
                        vec![],
                        vec![],
                        gaps,
                    ),
                    NativeAction::Fail {
                        task_id,
                        summary,
                        checks,
                    } => (
                        task_id,
                        JobTerminalStatus::Failed,
                        summary,
                        vec![],
                        vec![],
                        checks,
                        vec![],
                    ),
                    NativeAction::Delegate { .. } => {
                        anyhow::bail!("native worker may not delegate")
                    }
                };
            if task_id != request.task_id {
                anyhow::bail!("worker result task_id does not match request");
            }
            if summary.len() > 5 {
                checks.extend(
                    summary
                        .drain(5..)
                        .map(|line| format!("Additional result detail: {line}")),
                );
            }
            for line in &mut summary {
                if line.len() > 1024 {
                    *line = line.chars().take(1024).collect();
                }
            }
            crate::hybrid::carry_workflow_marker(&request, &mut artifacts);
            let result = JobResult {
                v: AGENT_JOB_VERSION,
                task_id,
                status,
                summary,
                artifacts,
                evidence_refs,
                checks,
                gaps,
            };
            let kind = if status == JobTerminalStatus::Completed {
                KIND_JOB_RESULT
            } else {
                KIND_JOB_ERROR
            };
            let builder = buzz_sdk::build_agent_job_result(
                kind,
                batch.channel_id,
                &request_event.pubkey.to_hex(),
                &request.root_task_id,
                &request_event.id.to_hex(),
                &result,
            )?;
            submit_builder(rest, builder, "terminal").await?;
            Ok(NativeTurnEffect {
                handled: true,
                invalidate_session: true,
            })
        }
        (Some(NativeJobRole::Worker), None) => {
            anyhow::bail!("native worker must return a terminal BUZZ_ACTION_V1 record")
        }
        _ => Ok(NativeTurnEffect::ignored()),
    }
}

/// Publish a terminal contract error for a worker so its manager resumes once.
pub async fn publish_contract_error(
    config: &NativeJobsConfig,
    rest: &RestClient,
    batch: &FlushBatch,
    error: &str,
) {
    if !config.enabled || config.role != Some(NativeJobRole::Worker) {
        return;
    }
    let Ok((request_event, request)) = worker_request(batch) else {
        return;
    };
    let result = JobResult {
        v: AGENT_JOB_VERSION,
        task_id: request.task_id,
        status: JobTerminalStatus::Failed,
        summary: vec![format!("Worker result contract failed: {error}")],
        artifacts: vec![],
        evidence_refs: vec![],
        checks: vec![],
        gaps: vec![],
    };
    let builder = match buzz_sdk::build_agent_job_result(
        KIND_JOB_ERROR,
        batch.channel_id,
        &request_event.pubkey.to_hex(),
        &request.root_task_id,
        &request_event.id.to_hex(),
        &result,
    ) {
        Ok(builder) => builder,
        Err(build_error) => {
            tracing::warn!("native job contract error build failed: {build_error}");
            return;
        }
    };
    if let Err(publish_error) = submit_builder(rest, builder, "contract-error").await {
        tracing::warn!("native job contract error publish failed: {publish_error}");
    }
}

/// Surface a terminal-processing failure without spending another model call.
///
/// Workers publish a typed job error so the delegating manager resumes. A
/// manager failure is posted to the owner thread because there is no parent
/// job event to wake.
pub async fn publish_turn_error(
    config: &NativeJobsConfig,
    rest: &RestClient,
    batch: &FlushBatch,
    error: &str,
) {
    match config.role {
        Some(NativeJobRole::Worker) => {
            publish_contract_error(config, rest, batch, error).await;
        }
        Some(NativeJobRole::Manager) => {
            let thread_tags = owner_thread_tags(batch);
            let notice = format!("ATLAS delegation failed safely: {error}");
            crate::pool::post_failure_notice(rest, batch.channel_id, &thread_tags, &notice).await;
        }
        None => {}
    }
}

/// Convert a validated worker cancellation into a terminal event immediately.
///
/// The worker model is cancelled separately by the harness. Publishing here
/// wakes the manager without waiting for another worker inference.
pub async fn publish_cancelled(
    config: &NativeJobsConfig,
    rest: &RestClient,
    channel_id: Uuid,
    cancel_event: &Event,
) -> anyhow::Result<()> {
    if config.role != Some(NativeJobRole::Worker) {
        anyhow::bail!("only a native worker may consume a job cancellation");
    }
    let cancel = buzz_core::agent_job::parse_job_cancel(&cancel_event.content)?;
    let root = event_tag(cancel_event, "root")
        .ok_or_else(|| anyhow::anyhow!("job cancellation has no root tag"))?;
    let request = event_tag(cancel_event, "request")
        .ok_or_else(|| anyhow::anyhow!("job cancellation has no request tag"))?;
    let result = JobResult {
        v: AGENT_JOB_VERSION,
        task_id: cancel.task_id,
        status: JobTerminalStatus::Cancelled,
        summary: vec![format!("Job cancelled: {}", cancel.reason)],
        artifacts: vec![],
        evidence_refs: vec![],
        checks: vec![],
        gaps: vec![],
    };
    let builder = buzz_sdk::build_agent_job_result(
        KIND_JOB_ERROR,
        channel_id,
        &cancel_event.pubkey.to_hex(),
        root,
        request,
        &result,
    )?;
    submit_builder(rest, builder, "cancelled").await
}

fn owner_thread_tags(batch: &FlushBatch) -> ThreadTags {
    if let Some(root) = native_event(batch).and_then(|event| event_tag(event, "root")) {
        return ThreadTags {
            root_event_id: EventId::from_hex(root).ok().map(|id| id.to_hex()),
            parent_event_id: EventId::from_hex(root).ok().map(|id| id.to_hex()),
            mentioned_pubkeys: vec![],
        };
    }
    batch
        .events
        .last()
        .map(|event| parse_thread_tags(&event.event))
        .unwrap_or_default()
}

pub(crate) async fn submit_builder(
    rest: &RestClient,
    builder: nostr::EventBuilder,
    label: &str,
) -> anyhow::Result<()> {
    let event = builder
        .sign_with_keys(&rest.keys)
        .map_err(|error| anyhow::anyhow!("native job {label} sign failed: {error}"))?;
    match tokio::time::timeout(Duration::from_secs(5), rest.submit_event(&event)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) => Err(anyhow::anyhow!(
            "native job {label} publish failed: {error}"
        )),
        Err(_) => Err(anyhow::anyhow!("native job {label} publish timed out")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker_config(manager: &nostr::Keys) -> NativeJobsConfig {
        NativeJobsConfig {
            enabled: true,
            role: Some(NativeJobRole::Worker),
            worker_role: Some("research".into()),
            delegator: Some(manager.public_key().to_hex()),
            ..NativeJobsConfig::default()
        }
    }

    fn request_event(manager: &nostr::Keys, role: &str, task_id: Uuid) -> Event {
        let request = JobRequest {
            v: AGENT_JOB_VERSION,
            task_id,
            root_task_id: "owner-event".into(),
            parent_task_id: None,
            assigned_role: role.into(),
            objective: "Verify one fact".into(),
            evidence_refs: vec![],
            result_contract: ResultContract {
                kind: "evidence_packet".into(),
                required: vec!["summary".into()],
            },
            constraints: vec!["read-only".into()],
            budget: JobBudget {
                deadline_at: "2026-08-21T12:00:00Z".into(),
                max_model_calls: Some(3),
                max_output_bytes: 8192,
            },
            attempt: 1,
        };
        buzz_sdk::build_agent_job_request(
            Uuid::new_v4(),
            &nostr::Keys::generate().public_key().to_hex(),
            &manager.public_key().to_hex(),
            &request,
        )
        .unwrap()
        .sign_with_keys(manager)
        .unwrap()
    }

    fn translation_request(task_id: Uuid) -> JobRequest {
        JobRequest {
            v: AGENT_JOB_VERSION,
            task_id,
            root_task_id: "owner-event".into(),
            parent_task_id: None,
            assigned_role: "translation".into(),
            objective: "Judge one bounded es/fr translation".into(),
            evidence_refs: vec![EvidenceRef {
                uri: "atlas-translation://acceptance/manifest.json".into(),
                revision: Some("source-sha".into()),
                section: Some("TranslationManifest".into()),
            }],
            result_contract: ResultContract {
                kind: "translation_manifest".into(),
                required: vec!["validation_result".into()],
            },
            constraints: vec!["No file or provider mutation".into()],
            budget: JobBudget {
                deadline_at: "2026-08-21T12:00:00Z".into(),
                max_model_calls: Some(1),
                max_output_bytes: 8192,
            },
            attempt: 1,
        }
    }

    #[test]
    fn native_jobs_default_off() {
        let config = NativeJobsConfig::default();
        assert!(!config.enabled);
        assert!(config.subscription_kinds().is_empty());
    }

    #[test]
    fn manager_contract_makes_host_responsible_for_delivery() {
        assert!(MANAGER_CONTROL_PROMPT.contains("ends this turn"));
        assert!(MANAGER_CONTROL_PROMPT
            .contains("only the compact specialist result as the next prompt"));
        assert!(MANAGER_CONTROL_PROMPT.contains("host publishes ordinary final assistant content"));
        assert!(MANAGER_CONTROL_PROMPT.contains("Never use Buzz tools"));
        assert!(MANAGER_CONTROL_PROMPT.contains("Do not inspect facts assigned to Research"));
        assert!(MANAGER_CONTROL_PROMPT.contains("After every READY Builder result"));
        assert!(MANAGER_CONTROL_PROMPT.contains("at most one bounded Builder correction"));
        assert!(MANAGER_CONTROL_PROMPT.contains("Never create an open-ended Builder/QA loop"));
        assert!(MANAGER_CONTROL_PROMPT.contains("Copy every artifact reference byte-for-byte"));
        assert!(MANAGER_CONTROL_PROMPT.contains("atlas-memory:// URIs"));
        assert!(MANAGER_CONTROL_PROMPT.contains("opaque machine strings"));
        assert!(MANAGER_CONTROL_PROMPT.contains("research|builder|qa|screenshots|memory|git"));
        assert!(MANAGER_CONTROL_PROMPT.contains("git|translation"));
        assert!(MANAGER_CONTROL_PROMPT.contains("translation_manifest"));
        assert!(MANAGER_CONTROL_PROMPT.contains("it only inspects and prepares commands"));
    }

    #[test]
    fn worker_prompt_carries_role_specific_efficiency_contract() {
        let manager = nostr::Keys::generate();
        let request = request_event(&manager, "research", Uuid::new_v4());
        let batch = FlushBatch {
            channel_id: Uuid::new_v4(),
            events: vec![crate::queue::BatchEvent {
                event: request,
                received_at: std::time::Instant::now(),
                prompt_tag: "native-job".into(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };
        let prompt = compact_prompt(&batch).unwrap();
        assert!(prompt.contains("exact evidence refs/resolver first"));
        assert!(prompt.contains("Do not broadly discover files"));

        let qa_request = request_event(&manager, "qa", Uuid::new_v4());
        let qa_batch = FlushBatch {
            channel_id: Uuid::new_v4(),
            events: vec![crate::queue::BatchEvent {
                event: qa_request,
                received_at: std::time::Instant::now(),
                prompt_tag: "native-job".into(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };
        let qa_prompt = compact_prompt(&qa_batch).unwrap();
        assert!(qa_prompt.contains("Do not redo Research"));
        assert!(qa_prompt.contains("literal authoritative resolver command"));
        assert!(qa_prompt.contains("Read-only except your own ATLAS runtime/output directory"));

        let screenshot_request = request_event(&manager, "screenshots", Uuid::new_v4());
        let screenshot_batch = FlushBatch {
            channel_id: Uuid::new_v4(),
            events: vec![crate::queue::BatchEvent {
                event: screenshot_request,
                received_at: std::time::Instant::now(),
                prompt_tag: "native-job".into(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };
        let screenshot_prompt = compact_prompt(&screenshot_batch).unwrap();
        assert!(screenshot_prompt.contains("exact assigned Synup product route"));
        assert!(screenshot_prompt.contains("evidence refs as approved and resolved"));
        assert!(screenshot_prompt.contains("exactly one narrow tool-search call"));
        assert!(screenshot_prompt.contains("never use shell/process discovery"));
        assert!(screenshot_prompt.contains("do not reread it with view_image"));
        assert!(screenshot_prompt.contains("one batched command"));
        assert!(screenshot_prompt.contains("Do not browse for alternatives"));
        assert!(screenshot_prompt.contains("first and only assistant content"));
        assert!(screenshot_prompt.contains("screenshot_manifest"));

        let memory_request = request_event(&manager, "memory", Uuid::new_v4());
        let memory_batch = FlushBatch {
            channel_id: Uuid::new_v4(),
            events: vec![crate::queue::BatchEvent {
                event: memory_request,
                received_at: std::time::Instant::now(),
                prompt_tag: "native-job".into(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };
        let memory_prompt = compact_prompt(&memory_batch).unwrap();
        assert!(memory_prompt.contains("deterministic memory resolver"));
        assert!(memory_prompt.contains("No broad repository scans"));
        assert!(memory_prompt.contains("reject transient task status"));
        assert!(memory_prompt.contains("memory_result"));

        let git_request = request_event(&manager, "git", Uuid::new_v4());
        let git_batch = FlushBatch {
            channel_id: Uuid::new_v4(),
            events: vec![crate::queue::BatchEvent {
                event: git_request,
                received_at: std::time::Instant::now(),
                prompt_tag: "native-job".into(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };
        let git_prompt = compact_prompt(&git_batch).unwrap();
        assert!(git_prompt.contains("deterministic read-only Git inspector"));
        assert!(git_prompt.contains("read-only ls-remote check"));
        assert!(git_prompt.contains("Never fetch, commit, stage, push"));
        assert!(git_prompt.contains("Summary must contain exactly one"));
        assert!(git_prompt.contains("git_readiness"));

        let translation_request = request_event(&manager, "translation", Uuid::new_v4());
        let translation_batch = FlushBatch {
            channel_id: Uuid::new_v4(),
            events: vec![crate::queue::BatchEvent {
                event: translation_request,
                received_at: std::time::Instant::now(),
                prompt_tag: "native-job".into(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };
        let translation_prompt = compact_prompt(&translation_batch).unwrap();
        assert!(translation_prompt.contains("bounded linguistic judgment"));
        assert!(translation_prompt.contains("deterministic host already owns eligibility"));
        assert!(translation_prompt.contains("host owns the native-job terminal envelope"));
        assert!(!translation_prompt
            .contains("Your final assistant content must contain only BUZZ_ACTION_V1"));
    }

    #[test]
    fn translation_judgment_is_host_wrapped() {
        let task_id = Uuid::new_v4();
        let request = translation_request(task_id);

        let pass = translation_terminal_action("PASS", &request).unwrap();
        assert!(matches!(
            pass,
            NativeAction::Complete {
                task_id: returned,
                summary,
                artifacts,
                gaps,
                ..
            } if returned == task_id
                && summary == ["PASS"]
                && artifacts == ["atlas-translation://acceptance/manifest.json"]
                && gaps.is_empty()
        ));

        let review = translation_terminal_action(
            "REVIEW_NEEDED\nOwner must approve the localized product term.",
            &request,
        )
        .unwrap();
        assert!(matches!(
            review,
            NativeAction::Complete { summary, gaps, .. }
                if summary == ["REVIEW_NEEDED"] && gaps.len() == 1
        ));

        let fail = translation_terminal_action(
            "FAIL\nThe French sentence changes the approved product claim.",
            &request,
        )
        .unwrap();
        assert!(matches!(
            fail,
            NativeAction::Fail { summary, checks, .. }
                if summary == ["FAIL"] && checks.len() == 1
        ));
    }

    #[test]
    fn translation_judgment_rejects_protocol_json_and_ambiguous_results() {
        let request = translation_request(Uuid::new_v4());
        assert!(translation_terminal_action("PASS\nunexpected finding", &request).is_err());
        assert!(translation_terminal_action("REVIEW_NEEDED", &request).is_err());
        assert!(translation_terminal_action("MAYBE", &request).is_err());
        assert!(translation_terminal_action("PASS\nBUZZ_ACTION_V1 {}", &request).is_err());

        let mut missing_manifest = request;
        missing_manifest.evidence_refs.clear();
        assert!(translation_terminal_action("PASS", &missing_manifest).is_err());
    }

    #[test]
    fn terminal_delegate_is_strict_and_typed() {
        let action = parse_terminal_action(
            "BUZZ_ACTION_V1\n{\"op\":\"delegate_task\",\"role\":\"research\",\"objective\":\"Verify X\",\"evidence_refs\":[],\"result_contract\":{\"kind\":\"evidence_packet\",\"required\":[\"summary\"]},\"constraints\":[\"read-only\"],\"max_model_calls\":3}",
        )
        .unwrap();
        assert!(matches!(
            action,
            Some(NativeAction::Delegate { role, .. }) if role == "research"
        ));
        assert_eq!(
            parse_terminal_action("ordinary final response").unwrap(),
            None
        );
    }

    #[test]
    fn terminal_action_accepts_acp_single_space_separator() {
        let action = parse_terminal_action(
            "BUZZ_ACTION_V1 {\"op\":\"delegate_task\",\"role\":\"research\",\"objective\":\"Verify X\",\"evidence_refs\":[],\"result_contract\":{\"kind\":\"evidence_packet\",\"required\":[\"summary\"]},\"constraints\":[],\"max_model_calls\":3}",
        )
        .unwrap();
        assert!(matches!(
            action,
            Some(NativeAction::Delegate { role, .. }) if role == "research"
        ));
    }

    #[test]
    fn terminal_action_accepts_compact_evidence_reference() {
        let action = parse_terminal_action(
            "BUZZ_ACTION_V1 {\"op\":\"delegate_task\",\"role\":\"research\",\"objective\":\"Verify X\",\"evidence_refs\":[\"maps/STATUS.md § census\"],\"result_contract\":{\"kind\":\"evidence_packet\",\"required\":[\"summary\"]},\"constraints\":[],\"max_model_calls\":3}",
        )
        .unwrap();
        assert!(matches!(
            action,
            Some(NativeAction::Delegate { evidence_refs, .. })
                if evidence_refs == vec![EvidenceRef {
                    uri: "maps/STATUS.md § census".into(),
                    revision: None,
                    section: None,
                }]
        ));
    }

    #[test]
    fn terminal_action_compacts_structured_artifacts_and_checks() {
        let task_id = Uuid::new_v4();
        let output = format!(
            "BUZZ_ACTION_V1 {{\"op\":\"complete_task\",\"task_id\":\"{task_id}\",\"summary\":[\"done\"],\"artifacts\":[{{\"kind\":\"evidence_packet\"}}],\"evidence_refs\":[],\"checks\":[{{\"name\":\"census\",\"result\":\"pass\"}}],\"gaps\":[]}}"
        );
        let action = parse_terminal_action(&output).unwrap();
        assert!(matches!(
            action,
            Some(NativeAction::Complete { artifacts, checks, .. })
                if artifacts == ["{\"kind\":\"evidence_packet\"}"]
                    && checks == ["{\"name\":\"census\",\"result\":\"pass\"}"]
        ));
    }

    #[test]
    fn malformed_action_does_not_fall_back_to_chat() {
        assert!(parse_terminal_action("BUZZ_ACTION_V1\n{bad-json}").is_err());
    }

    #[test]
    fn worker_claims_only_its_manager_role_and_first_task_delivery() {
        let manager = nostr::Keys::generate();
        let impostor = nostr::Keys::generate();
        let config = worker_config(&manager);
        let task_id = Uuid::new_v4();
        let request = request_event(&manager, "research", task_id);
        assert!(config.claim_inbound_event(&request));
        assert!(
            !config.claim_inbound_event(&request),
            "duplicate task is dropped"
        );
        assert!(!config.claim_inbound_event(&request_event(&manager, "builder", Uuid::new_v4())));
        assert!(!config.claim_inbound_event(&request_event(&impostor, "research", Uuid::new_v4())));
    }

    #[test]
    fn qa_worker_claims_only_qa_requests() {
        let manager = nostr::Keys::generate();
        let mut config = worker_config(&manager);
        config.worker_role = Some("qa".into());
        assert!(config.claim_inbound_event(&request_event(&manager, "qa", Uuid::new_v4())));
        assert!(!config.claim_inbound_event(&request_event(&manager, "builder", Uuid::new_v4())));
    }

    #[test]
    fn screenshots_worker_claims_only_screenshot_requests() {
        let manager = nostr::Keys::generate();
        let mut config = worker_config(&manager);
        config.worker_role = Some("screenshots".into());
        assert!(config.claim_inbound_event(&request_event(
            &manager,
            "screenshots",
            Uuid::new_v4()
        )));
        assert!(!config.claim_inbound_event(&request_event(&manager, "builder", Uuid::new_v4())));
    }

    #[test]
    fn memory_worker_claims_only_memory_requests() {
        let manager = nostr::Keys::generate();
        let mut config = worker_config(&manager);
        config.worker_role = Some("memory".into());
        assert!(config.claim_inbound_event(&request_event(&manager, "memory", Uuid::new_v4())));
        assert!(!config.claim_inbound_event(&request_event(&manager, "research", Uuid::new_v4())));
    }

    #[test]
    fn git_worker_claims_only_git_requests() {
        let manager = nostr::Keys::generate();
        let mut config = worker_config(&manager);
        config.worker_role = Some("git".into());
        assert!(config.claim_inbound_event(&request_event(&manager, "git", Uuid::new_v4())));
        assert!(!config.claim_inbound_event(&request_event(&manager, "builder", Uuid::new_v4())));
    }

    #[test]
    fn translation_worker_claims_only_translation_requests_once() {
        let manager = nostr::Keys::generate();
        let mut config = worker_config(&manager);
        config.worker_role = Some("translation".into());
        let request = request_event(&manager, "translation", Uuid::new_v4());
        assert!(config.claim_inbound_event(&request));
        assert!(!config.claim_inbound_event(&request));
        assert!(!config.claim_inbound_event(&request_event(&manager, "qa", Uuid::new_v4())));
    }
}
