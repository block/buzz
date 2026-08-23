//! Opt-in deterministic execution for a single sequential Helpdesk workflow.
//!
//! This module deliberately does not introduce a second job protocol. Workflow
//! state is a private artifact carried by the existing kinds 43001-43006, and
//! every host worker emits the same typed `JobResult` as a model worker.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;

use buzz_core::agent_job::{
    EvidenceRef, JobBudget, JobRequest, JobResult, JobStatus, JobTerminalStatus, ResultContract,
    AGENT_JOB_VERSION,
};
use buzz_core::kind::{KIND_JOB_ACCEPTED, KIND_JOB_ERROR, KIND_JOB_REQUEST, KIND_JOB_RESULT};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use uuid::Uuid;

use crate::jobs::{event_tag, submit_builder, NativeJobRole, NativeJobsConfig};
use crate::relay::{BuzzEvent, RestClient};

const WORKFLOW_PREFIX: &str = "HYBRID_WORKFLOW_V1:";
const MAX_OUTPUT_BYTES: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Stage {
    Memory,
    Research,
    Builder,
    StructuralQa,
    ReasoningQa,
    Git,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HelpdeskWorkflow {
    kind: String,
    stage: Stage,
    evidence_id: String,
    #[serde(alias = "absolute_worktree")]
    worktree: PathBuf,
    #[serde(alias = "repository-relative target_file")]
    target_file: PathBuf,
    expected_branch: String,
    expected_head: String,
    research_command: String,
    #[serde(default)]
    research_requirements: Vec<String>,
    #[serde(default = "default_true")]
    preserve_screenshot_placeholders: bool,
    #[serde(default)]
    allow_human_reviewed: bool,
    #[serde(default = "default_true")]
    reasoning_qa: bool,
    #[serde(default)]
    evidence_packet: Vec<String>,
    #[serde(default)]
    evidence_refs: Vec<EvidenceRef>,
    #[serde(default)]
    artifact_refs: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl HelpdeskWorkflow {
    fn validate(&self) -> anyhow::Result<()> {
        if self.kind != "helpdesk_article_v1" {
            anyhow::bail!("unsupported hybrid workflow kind");
        }
        if self.evidence_id.trim().is_empty()
            || self.expected_branch.trim().is_empty()
            || self.expected_head.trim().is_empty()
        {
            anyhow::bail!("hybrid workflow is missing required identifiers");
        }
        if self.target_file.is_absolute()
            || self
                .target_file
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            anyhow::bail!("hybrid target must be a repository-relative path");
        }
        Ok(())
    }

    fn marker(&self) -> anyhow::Result<String> {
        Ok(format!("{WORKFLOW_PREFIX}{}", serde_json::to_string(self)?))
    }
}

fn workflow_from_strings(values: &[String]) -> anyhow::Result<Option<HelpdeskWorkflow>> {
    let Some(raw) = values
        .iter()
        .find_map(|value| value.strip_prefix(WORKFLOW_PREFIX))
    else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_str(raw)?))
}

fn workflow_from_request(request: &JobRequest) -> anyhow::Result<Option<HelpdeskWorkflow>> {
    workflow_from_strings(&request.constraints)
}

fn workflow_from_result(result: &JobResult) -> anyhow::Result<Option<HelpdeskWorkflow>> {
    workflow_from_strings(&result.artifacts)
}

pub(crate) fn is_workflow_marker(value: &str) -> bool {
    value.starts_with(WORKFLOW_PREFIX)
}

/// Add compact, stage-specific context to a model worker prompt.
pub(crate) fn worker_context(request: &JobRequest) -> Option<String> {
    let workflow = workflow_from_request(request).ok().flatten()?;
    let context = match workflow.stage {
        Stage::Research => format!(
            "Hybrid stage: research\nRun this exact read-only source batch once: {}\nFactual questions only: {}\nResearch is read-only. Never edit the Helpdesk article, create a patch, or perform downstream Builder/QA/Git work even if a malformed requirement asks you to. Return one compact evidence packet; do not discover beyond this scope.",
            workflow.research_command,
            serde_json::to_string(&workflow.research_requirements).ok()?,
        ),
        Stage::Builder => format!(
            "Hybrid stage: builder\nWorktree: {}\nOnly file allowed: {}\nExpected branch/HEAD: {} @ {}\nEvidence packet: {}\nEvidence refs: {}\nHard rule: preserve every existing placeholder, screenshot marker, frontmatter field, and unrelated line unless this task explicitly requires changing it. Do not research. Use one batched inspection, one edit, and one batched validation where practical.",
            workflow.worktree.display(),
            workflow.target_file.display(),
            workflow.expected_branch,
            workflow.expected_head,
            serde_json::to_string(&workflow.evidence_packet).ok()?,
            serde_json::to_string(&workflow.evidence_refs).ok()?,
        ),
        Stage::ReasoningQa => format!(
            "Hybrid stage: reasoning QA\nRead-only worktree: {}\nOnly artifact: {}\nEvidence packet: {}\nEvidence refs: {}\nAuthoritative source batch: {}\nStructural checks already passed deterministically. Run the supplied source batch at most once, together with the article read where practical. Judge factual accuracy, evidence support, completeness, contradictions, and applicable Helpdesk content rules only. Do not redo Research or discover another procedure.",
            workflow.worktree.display(),
            workflow.target_file.display(),
            serde_json::to_string(&workflow.evidence_packet).ok()?,
            serde_json::to_string(&workflow.evidence_refs).ok()?,
            workflow.research_command,
        ),
        _ => return None,
    };
    Some(context)
}

/// Preserve private routing state when a model worker emits its typed result.
pub(crate) fn carry_workflow_marker(request: &JobRequest, artifacts: &mut Vec<String>) {
    if let Some(marker) = request
        .constraints
        .iter()
        .find(|value| is_workflow_marker(value))
    {
        artifacts.retain(|value| !is_workflow_marker(value));
        artifacts.push(marker.clone());
    }
}

/// Consume a routine native request without starting an ACP/model turn.
pub(crate) async fn try_handle_deterministic_request(
    config: &NativeJobsConfig,
    rest: &RestClient,
    inbound: &BuzzEvent,
) -> anyhow::Result<bool> {
    if !config.hybrid_enabled
        || config.role != Some(NativeJobRole::Worker)
        || inbound.event.kind.as_u16() as u32 != KIND_JOB_REQUEST
    {
        return Ok(false);
    }
    let request = buzz_core::agent_job::parse_job_request(&inbound.event.content)?;
    let workflow = match workflow_from_request(&request) {
        Ok(Some(workflow)) => workflow,
        Ok(None) => return Ok(false),
        Err(error) => {
            publish_deterministic_contract_failure(rest, inbound, &request, &error.to_string())
                .await?;
            return Ok(true);
        }
    };
    if let Err(error) = workflow.validate() {
        publish_deterministic_contract_failure(rest, inbound, &request, &error.to_string()).await?;
        return Ok(true);
    }
    let deterministic = matches!(
        (request.assigned_role.as_str(), workflow.stage),
        ("memory", Stage::Memory) | ("qa", Stage::StructuralQa) | ("git", Stage::Git)
    );
    if !deterministic {
        return Ok(false);
    }

    publish_status(config, rest, inbound, &request).await?;
    let result = match workflow.stage {
        Stage::Memory => memory_lookup(config, &request, workflow).await,
        Stage::StructuralQa => structural_qa(config, &request, workflow).await,
        Stage::Git => git_readiness(config, &request, workflow).await,
        _ => unreachable!("deterministic stages checked above"),
    };
    let result = result.unwrap_or_else(|error| JobResult {
        v: AGENT_JOB_VERSION,
        task_id: request.task_id,
        status: JobTerminalStatus::Failed,
        summary: vec![format!(
            "Deterministic {} failed safely",
            request.assigned_role
        )],
        artifacts: request
            .constraints
            .iter()
            .filter(|value| is_workflow_marker(value))
            .cloned()
            .collect(),
        evidence_refs: vec![],
        checks: vec![error.to_string()],
        gaps: vec![],
    });
    publish_result(rest, inbound, &request, &result).await?;
    tracing::info!(
        task_id = %request.task_id,
        role = %request.assigned_role,
        status = ?result.status,
        "hybrid deterministic job completed without a model turn"
    );
    Ok(true)
}

async fn publish_deterministic_contract_failure(
    rest: &RestClient,
    inbound: &BuzzEvent,
    request: &JobRequest,
    error: &str,
) -> anyhow::Result<()> {
    let result = JobResult {
        v: AGENT_JOB_VERSION,
        task_id: request.task_id,
        status: JobTerminalStatus::Failed,
        summary: vec![format!(
            "Deterministic {} contract failed safely",
            request.assigned_role
        )],
        artifacts: vec![],
        evidence_refs: vec![],
        checks: vec![error.to_string()],
        gaps: vec![],
    };
    publish_result(rest, inbound, request, &result).await
}

/// Route an obvious successful stage directly to the next native worker.
/// Returns true when the Manager must stay suspended.
pub(crate) async fn try_route_manager_result(
    config: &NativeJobsConfig,
    rest: &RestClient,
    inbound: &BuzzEvent,
) -> anyhow::Result<bool> {
    if !config.hybrid_enabled
        || config.role != Some(NativeJobRole::Manager)
        || !matches!(
            inbound.event.kind.as_u16() as u32,
            KIND_JOB_RESULT | KIND_JOB_ERROR
        )
    {
        return Ok(false);
    }
    let result = buzz_core::agent_job::parse_job_result(&inbound.event.content)?;
    let Some(mut workflow) = workflow_from_result(&result)? else {
        return Ok(false);
    };
    workflow.validate()?;
    allowed_worktree(config, &workflow.worktree)?;
    if result.status != JobTerminalStatus::Completed {
        return Ok(false);
    }

    let next = match workflow.stage {
        Stage::Memory => {
            if result.evidence_refs.is_empty() {
                workflow.stage = Stage::Research;
                (
                    "research",
                    "Collect the exact missing source evidence for the assigned Helpdesk article",
                )
            } else {
                workflow.evidence_refs = result.evidence_refs.clone();
                workflow.evidence_packet = result.summary.clone();
                workflow.stage = Stage::Builder;
                (
                    "builder",
                    "Update only the assigned Helpdesk article from current approved evidence",
                )
            }
        }
        Stage::Research => {
            workflow.evidence_packet = result
                .summary
                .iter()
                .chain(result.checks.iter())
                .chain(result.gaps.iter())
                .take(16)
                .cloned()
                .collect();
            workflow.evidence_refs = result.evidence_refs.clone();
            workflow.stage = Stage::Builder;
            (
                "builder",
                "Update only the assigned Helpdesk article from the supplied evidence packet",
            )
        }
        Stage::Builder => {
            workflow.artifact_refs = result
                .artifacts
                .iter()
                .filter(|value| !is_workflow_marker(value))
                .cloned()
                .collect();
            if workflow.artifact_refs.is_empty() {
                workflow
                    .artifact_refs
                    .push(workflow.target_file.display().to_string());
            }
            workflow.stage = Stage::StructuralQa;
            (
                "qa",
                "Run deterministic structural Helpdesk validation on the assigned article",
            )
        }
        Stage::StructuralQa => {
            if workflow.reasoning_qa {
                workflow.stage = Stage::ReasoningQa;
                (
                    "qa",
                    "Independently judge the assigned article against its approved evidence",
                )
            } else {
                workflow.stage = Stage::Git;
                (
                    "git",
                    "Inspect the assigned worktree for read-only Git readiness",
                )
            }
        }
        Stage::ReasoningQa => {
            workflow.stage = Stage::Git;
            (
                "git",
                "Inspect the QA-passed worktree for read-only Git readiness",
            )
        }
        Stage::Git => return Ok(false),
    };
    publish_next_request(config, rest, inbound, &result, workflow, next.0, next.1).await?;
    Ok(true)
}

async fn publish_status(
    _config: &NativeJobsConfig,
    rest: &RestClient,
    inbound: &BuzzEvent,
    request: &JobRequest,
) -> anyhow::Result<()> {
    let status = JobStatus {
        v: AGENT_JOB_VERSION,
        task_id: request.task_id,
        status: "running".into(),
        detail: Some(format!(
            "{} running (deterministic host worker)",
            request.assigned_role
        )),
    };
    let builder = buzz_sdk::build_agent_job_status(
        KIND_JOB_ACCEPTED,
        inbound.channel_id,
        &inbound.event.pubkey.to_hex(),
        &request.root_task_id,
        &inbound.event.id.to_hex(),
        &status,
    )?;
    submit_builder(rest, builder, "hybrid-accepted").await
}

async fn publish_result(
    rest: &RestClient,
    inbound: &BuzzEvent,
    request: &JobRequest,
    result: &JobResult,
) -> anyhow::Result<()> {
    let kind = if result.status == JobTerminalStatus::Completed {
        KIND_JOB_RESULT
    } else {
        KIND_JOB_ERROR
    };
    let builder = buzz_sdk::build_agent_job_result(
        kind,
        inbound.channel_id,
        &inbound.event.pubkey.to_hex(),
        &request.root_task_id,
        &inbound.event.id.to_hex(),
        result,
    )?;
    submit_builder(rest, builder, "hybrid-terminal").await
}

async fn publish_next_request(
    config: &NativeJobsConfig,
    rest: &RestClient,
    inbound: &BuzzEvent,
    previous: &JobResult,
    workflow: HelpdeskWorkflow,
    role: &str,
    objective: &str,
) -> anyhow::Result<()> {
    let assignee = config
        .delegation_targets
        .get(role)
        .ok_or_else(|| anyhow::anyhow!("hybrid manager has no {role} target"))?;
    let root = event_tag(&inbound.event, "root")
        .ok_or_else(|| anyhow::anyhow!("hybrid result has no root tag"))?;
    let marker = workflow.marker()?;
    let request = JobRequest {
        v: AGENT_JOB_VERSION,
        task_id: Uuid::new_v4(),
        root_task_id: root.to_string(),
        parent_task_id: Some(previous.task_id),
        assigned_role: role.to_string(),
        objective: objective.to_string(),
        evidence_refs: workflow.evidence_refs.clone(),
        result_contract: ResultContract {
            kind: match role {
                "research" => "evidence_packet",
                "builder" => "build_result",
                "qa" => "qa_result",
                "git" => "git_readiness",
                _ => "job_result",
            }
            .into(),
            required: vec!["summary".into(), "checks".into()],
        },
        constraints: vec![marker],
        budget: JobBudget {
            deadline_at: (Utc::now()
                + chrono::Duration::from_std(config.deadline)
                    .unwrap_or_else(|_| chrono::Duration::minutes(15)))
            .to_rfc3339(),
            max_model_calls: match role {
                "research" => Some(3),
                "builder" => Some(4),
                "qa" => Some(3),
                _ => Some(0),
            },
            max_output_bytes: MAX_OUTPUT_BYTES,
        },
        attempt: 1,
    };
    let builder = buzz_sdk::build_agent_job_request(
        inbound.channel_id,
        assignee,
        &rest.keys.public_key().to_hex(),
        &request,
    )?;
    submit_builder(rest, builder, "hybrid-route").await
}

#[derive(Deserialize)]
struct MemoryIndex {
    records: Vec<MemoryRecord>,
}

#[derive(Deserialize)]
struct MemoryRecord {
    evidence_id: String,
    evidence_ref: String,
    summary: String,
    status: String,
    freshness: String,
    #[serde(default)]
    superseded_by: Option<String>,
    source: MemorySource,
}

#[derive(Deserialize)]
struct MemorySource {
    revision: String,
}

async fn memory_lookup(
    config: &NativeJobsConfig,
    request: &JobRequest,
    workflow: HelpdeskWorkflow,
) -> anyhow::Result<JobResult> {
    let path = config
        .hybrid_memory_index
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("hybrid Memory index is not configured"))?;
    let bytes = fs::read(path)?;
    let index: MemoryIndex = serde_json::from_slice(&bytes)?;
    let matches: Vec<_> = index
        .records
        .iter()
        .filter(|record| {
            record.evidence_id == workflow.evidence_id
                && record.status == "authoritative"
                && record.freshness == "current"
                && record.superseded_by.is_none()
        })
        .collect();
    if matches.len() > 1 {
        anyhow::bail!("Memory index has duplicate authoritative current records");
    }
    let marker = workflow.marker()?;
    let Some(record) = matches.first() else {
        return Ok(JobResult {
            v: AGENT_JOB_VERSION,
            task_id: request.task_id,
            status: JobTerminalStatus::Completed,
            summary: vec![format!("MISS: {}", workflow.evidence_id)],
            artifacts: vec![marker],
            evidence_refs: vec![],
            checks: vec!["Authoritative Memory index checked once".into()],
            gaps: vec![],
        });
    };
    Ok(JobResult {
        v: AGENT_JOB_VERSION,
        task_id: request.task_id,
        status: JobTerminalStatus::Completed,
        summary: vec![format!(
            "HIT: {} — {}",
            workflow.evidence_id, record.summary
        )],
        artifacts: vec![marker],
        evidence_refs: vec![EvidenceRef {
            uri: record.evidence_ref.clone(),
            revision: Some(record.source.revision.clone()),
            section: None,
        }],
        checks: vec!["Record is authoritative, current, unique, and not superseded".into()],
        gaps: vec![],
    })
}

async fn structural_qa(
    config: &NativeJobsConfig,
    request: &JobRequest,
    workflow: HelpdeskWorkflow,
) -> anyhow::Result<JobResult> {
    let worktree = allowed_worktree(config, &workflow.worktree)?;
    let target = worktree.join(&workflow.target_file);
    let current = fs::read_to_string(&target)?;
    let baseline = git_output(
        &worktree,
        &["show", &format!("HEAD:{}", workflow.target_file.display())],
    )
    .await?;
    let status = git_output(
        &worktree,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )
    .await?;
    let diff_check = git_command(&worktree, &["diff", "--check"]).await?;
    let mut failures = Vec::new();

    let changed = parse_status_paths(&status);
    if changed != BTreeSet::from([workflow.target_file.display().to_string()]) {
        failures.push(format!("unexpected changed-file scope: {changed:?}"));
    }
    if !diff_check.status.success() {
        failures.push(format!(
            "git diff --check failed: {}",
            stderr_text(&diff_check)
        ));
    }
    if frontmatter(&baseline) != frontmatter(&current) {
        failures.push("frontmatter changed unexpectedly".into());
    }
    for heading in [
        "## Answer",
        "## Key facts",
        "## Steps",
        "## Questions people ask",
        "## Next",
    ] {
        if !current.contains(heading) {
            failures.push(format!("required heading missing: {heading}"));
        }
    }
    if workflow.preserve_screenshot_placeholders {
        let before = screenshot_placeholders(&baseline);
        let after = screenshot_placeholders(&current);
        if before != after {
            failures.push(format!(
                "screenshot placeholders changed (expected {}, found {})",
                before.len(),
                after.len()
            ));
        }
    }
    if !workflow.allow_human_reviewed && current.contains("HUMAN REVIEWED") {
        failures.push("HUMAN REVIEWED marker is not allowed".into());
    }
    if let Some(pattern) = privacy_pattern(&current) {
        failures.push(format!("possible private/customer data pattern: {pattern}"));
    }
    if let Some(link) = malformed_root_link(&current) {
        failures.push(format!("malformed root-relative Markdown link: {link}"));
    }

    let marker = workflow.marker()?;
    let passed = failures.is_empty();
    Ok(JobResult {
        v: AGENT_JOB_VERSION,
        task_id: request.task_id,
        status: if passed {
            JobTerminalStatus::Completed
        } else {
            JobTerminalStatus::Failed
        },
        summary: vec![if passed {
            "STRUCTURAL QA PASS".into()
        } else {
            "STRUCTURAL QA FAIL".into()
        }],
        artifacts: workflow
            .artifact_refs
            .iter()
            .cloned()
            .chain(std::iter::once(marker))
            .collect(),
        evidence_refs: workflow.evidence_refs,
        checks: if passed {
            vec![
                "frontmatter and required structure preserved".into(),
                "screenshot placeholders preserved".into(),
                "changed-file scope and git diff --check passed".into(),
                "HUMAN REVIEWED/privacy/link static guards passed".into(),
            ]
        } else {
            failures
        },
        gaps: vec![],
    })
}

async fn git_readiness(
    config: &NativeJobsConfig,
    request: &JobRequest,
    workflow: HelpdeskWorkflow,
) -> anyhow::Result<JobResult> {
    let worktree = allowed_worktree(config, &workflow.worktree)?;
    let branch = git_output(&worktree, &["branch", "--show-current"]).await?;
    let head = git_output(&worktree, &["rev-parse", "HEAD"]).await?;
    let status = git_output(
        &worktree,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )
    .await?;
    let diff_check = git_command(&worktree, &["diff", "--check"]).await?;
    let changed = parse_status_paths(&status);
    let expected = BTreeSet::from([workflow.target_file.display().to_string()]);
    let mut blockers = Vec::new();
    if branch.trim() != workflow.expected_branch {
        blockers.push(format!("branch mismatch: {}", branch.trim()));
    }
    if head.trim() != workflow.expected_head {
        blockers.push(format!("HEAD mismatch: {}", head.trim()));
    }
    if changed != expected {
        blockers.push(format!("changed-file scope mismatch: {changed:?}"));
    }
    if !diff_check.status.success() {
        blockers.push(format!(
            "git diff --check failed: {}",
            stderr_text(&diff_check)
        ));
    }
    let ahead_behind = match git_command(
        &worktree,
        &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"],
    )
    .await
    {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => "unavailable (no fetch performed)".into(),
    };
    let marker = workflow.marker()?;
    let ready = blockers.is_empty();
    Ok(JobResult {
        v: AGENT_JOB_VERSION,
        task_id: request.task_id,
        status: JobTerminalStatus::Completed,
        summary: vec![if ready {
            "GIT READY".into()
        } else {
            "GIT BLOCKED".into()
        }],
        artifacts: workflow
            .artifact_refs
            .iter()
            .cloned()
            .chain(std::iter::once(marker))
            .collect(),
        evidence_refs: workflow.evidence_refs,
        checks: vec![
            format!("branch: {}", branch.trim()),
            format!("HEAD: {}", head.trim()),
            format!("changed files: {changed:?}"),
            format!(
                "diff check: {}",
                if diff_check.status.success() {
                    "PASS"
                } else {
                    "FAIL"
                }
            ),
            format!("upstream behind/ahead: {ahead_behind}"),
            format!("scope verdict: {}", if ready { "READY" } else { "BLOCKED" }),
        ]
        .into_iter()
        .chain(blockers)
        .collect(),
        gaps: vec![],
    })
}

fn allowed_worktree(config: &NativeJobsConfig, requested: &Path) -> anyhow::Result<PathBuf> {
    let requested = requested.canonicalize()?;
    let allowed = config
        .hybrid_allowed_roots
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| requested == root || requested.starts_with(&root));
    if !allowed {
        anyhow::bail!("hybrid worktree is outside configured read-only/write scope");
    }
    Ok(requested)
}

async fn git_output(worktree: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = git_command(worktree, args).await?;
    if !output.status.success() {
        anyhow::bail!("git {} failed: {}", args.join(" "), stderr_text(&output));
    }
    Ok(String::from_utf8(output.stdout)?)
}

async fn git_command(worktree: &Path, args: &[&str]) -> anyhow::Result<std::process::Output> {
    Ok(Command::new("git")
        .arg("--no-optional-locks")
        .args(args)
        .current_dir(worktree)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_COUNT", "2")
        .env("GIT_CONFIG_KEY_0", "maintenance.auto")
        .env("GIT_CONFIG_VALUE_0", "false")
        .env("GIT_CONFIG_KEY_1", "gc.auto")
        .env("GIT_CONFIG_VALUE_1", "0")
        .stdin(Stdio::null())
        .output()
        .await?)
}

fn parse_status_paths(status: &str) -> BTreeSet<String> {
    status
        .lines()
        .filter_map(|line| line.get(3..))
        .map(|path| path.rsplit(" -> ").next().unwrap_or(path).to_string())
        .collect()
}

fn frontmatter(value: &str) -> Option<&str> {
    value
        .strip_prefix("---\n")
        .and_then(|rest| rest.find("\n---\n").map(|end| &rest[..end]))
}

fn screenshot_placeholders(value: &str) -> Vec<&str> {
    value
        .lines()
        .filter(|line| line.contains("[SCREENSHOT PLACEHOLDER:"))
        .collect()
}

fn privacy_pattern(value: &str) -> Option<&'static str> {
    let lower = value.to_ascii_lowercase();
    for (needle, name) in [
        ("-----begin private key-----", "private key"),
        ("api_key=", "API key assignment"),
        ("access_token=", "access token assignment"),
        ("customer password", "customer password"),
    ] {
        if lower.contains(needle) {
            return Some(name);
        }
    }
    None
}

fn malformed_root_link(value: &str) -> Option<&str> {
    value
        .lines()
        .find(|line| line.contains("](/ ") || line.contains("](// ") || line.contains("](/# "))
}

fn stderr_text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn config(root: &Path, index: &Path) -> NativeJobsConfig {
        let mut config = NativeJobsConfig::enabled_for_test();
        config.role = Some(NativeJobRole::Worker);
        config.hybrid_enabled = true;
        config.hybrid_memory_index = Some(index.to_path_buf());
        config.hybrid_allowed_roots = vec![root.to_path_buf()];
        config
    }

    fn workflow(root: &Path, stage: Stage) -> HelpdeskWorkflow {
        HelpdeskWorkflow {
            kind: "helpdesk_article_v1".into(),
            stage,
            evidence_id: "asset-evidence".into(),
            worktree: root.into(),
            target_file: "content/data/find-assets.md".into(),
            expected_branch: "trial".into(),
            expected_head: String::new(),
            research_command: "rg exact".into(),
            research_requirements: vec!["fact".into()],
            preserve_screenshot_placeholders: true,
            allow_human_reviewed: false,
            reasoning_qa: true,
            evidence_packet: vec![],
            evidence_refs: vec![],
            artifact_refs: vec![],
        }
    }

    async fn fixture() -> (TempDir, PathBuf, String) {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("content/data")).unwrap();
        fs::write(
            root.join("content/data/find-assets.md"),
            "---\ntitle: Find assets\ndraft: true\n---\n## Answer\nTODO\n## Key facts\nTODO\n## Steps\n1. TODO\n   [SCREENSHOT PLACEHOLDER: keep]\n## Questions people ask\nTODO\n## Next\nTODO\n",
        )
        .unwrap();
        let index = root.join("memory.json");
        fs::write(
            &index,
            r#"{"records":[{"evidence_id":"asset-evidence","evidence_ref":"atlas-memory://asset-evidence@r1","summary":"current","status":"authoritative","freshness":"current","superseded_by":null,"source":{"revision":"r1"}}]}"#,
        )
        .unwrap();
        for args in [
            vec!["init", "-b", "trial"],
            vec!["config", "user.email", "atlas@example.test"],
            vec!["config", "user.name", "ATLAS Test"],
            vec!["add", "content/data/find-assets.md"],
            vec!["add", "memory.json"],
            vec!["commit", "-m", "baseline"],
        ] {
            let output = Command::new("git")
                .args(["-c", "commit.gpgsign=false"])
                .args(&args)
                .current_dir(root)
                .output()
                .await
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let head = git_output(root, &["rev-parse", "HEAD"])
            .await
            .unwrap()
            .trim()
            .to_string();
        (temp, index, head)
    }

    #[tokio::test]
    async fn memory_lookup_returns_current_hit_and_miss_without_mutation() {
        let (temp, index, head) = fixture().await;
        let mut hit = workflow(temp.path(), Stage::Memory);
        hit.expected_head = head;
        let request = test_request(&hit);
        let result = memory_lookup(&config(temp.path(), &index), &request, hit)
            .await
            .unwrap();
        assert_eq!(
            result.status,
            JobTerminalStatus::Completed,
            "{:?}",
            result.checks
        );
        assert_eq!(result.evidence_refs.len(), 1);

        let mut miss = workflow(temp.path(), Stage::Memory);
        miss.expected_head = request.root_task_id;
        miss.evidence_id = "missing".into();
        let request = test_request(&miss);
        let result = memory_lookup(&config(temp.path(), &index), &request, miss)
            .await
            .unwrap();
        assert!(result.evidence_refs.is_empty());
        assert!(result.summary[0].starts_with("MISS:"));
    }

    #[test]
    fn workflow_parser_accepts_explicit_path_aliases_without_relaxing_schema() {
        let marker = concat!(
            "HYBRID_WORKFLOW_V1:{\"kind\":\"helpdesk_article_v1\",\"stage\":\"memory\",",
            "\"evidence_id\":\"e\",\"absolute_worktree\":\"/tmp/worktree\",",
            "\"repository-relative target_file\":\"content/a.md\",\"expected_branch\":\"b\",",
            "\"expected_head\":\"h\",\"research_command\":\"rg exact\"}"
        );
        let parsed = workflow_from_strings(&[marker.into()]).unwrap().unwrap();
        assert_eq!(parsed.worktree, PathBuf::from("/tmp/worktree"));
        assert_eq!(parsed.target_file, PathBuf::from("content/a.md"));
    }

    #[tokio::test]
    async fn structural_qa_passes_and_rejects_placeholder_or_scope_changes() {
        let (temp, index, head) = fixture().await;
        let path = temp.path().join("content/data/find-assets.md");
        let baseline = fs::read_to_string(&path).unwrap();
        fs::write(&path, baseline.replacen("TODO", "Verified", 1)).unwrap();
        let mut plan = workflow(temp.path(), Stage::StructuralQa);
        plan.expected_head = head;
        let request = test_request(&plan);
        let result = structural_qa(&config(temp.path(), &index), &request, plan.clone())
            .await
            .unwrap();
        assert_eq!(
            result.status,
            JobTerminalStatus::Completed,
            "{:?}",
            result.checks
        );

        fs::write(
            &path,
            baseline.replace("   [SCREENSHOT PLACEHOLDER: keep]\n", ""),
        )
        .unwrap();
        let result = structural_qa(&config(temp.path(), &index), &request, plan.clone())
            .await
            .unwrap();
        assert_eq!(result.status, JobTerminalStatus::Failed);
        assert!(result
            .checks
            .iter()
            .any(|check| check.contains("screenshot")));

        fs::write(&path, &baseline).unwrap();
        fs::write(temp.path().join("extra.txt"), "unexpected\n").unwrap();
        let result = structural_qa(&config(temp.path(), &index), &request, plan)
            .await
            .unwrap();
        assert_eq!(result.status, JobTerminalStatus::Failed);
        assert!(result.checks.iter().any(|check| check.contains("scope")));
    }

    #[tokio::test]
    async fn git_readiness_is_read_only_and_reports_ready_or_blocked() {
        let (temp, index, head) = fixture().await;
        let path = temp.path().join("content/data/find-assets.md");
        let before_index = fs::read(temp.path().join(".git/index")).unwrap();
        let baseline = fs::read_to_string(&path).unwrap();
        fs::write(&path, baseline.replacen("TODO", "Verified", 1)).unwrap();
        let mut plan = workflow(temp.path(), Stage::Git);
        plan.expected_head = head.clone();
        let request = test_request(&plan);
        let result = git_readiness(&config(temp.path(), &index), &request, plan.clone())
            .await
            .unwrap();
        assert_eq!(result.summary, vec!["GIT READY"], "{:?}", result.checks);
        assert_eq!(
            git_output(temp.path(), &["rev-parse", "HEAD"])
                .await
                .unwrap()
                .trim(),
            head
        );
        assert_eq!(
            fs::read(temp.path().join(".git/index")).unwrap(),
            before_index
        );

        plan.expected_head = "deadbeef".into();
        let request = test_request(&plan);
        let result = git_readiness(&config(temp.path(), &index), &request, plan)
            .await
            .unwrap();
        assert_eq!(result.summary, vec!["GIT BLOCKED"]);
    }

    fn test_request(workflow: &HelpdeskWorkflow) -> JobRequest {
        JobRequest {
            v: AGENT_JOB_VERSION,
            task_id: Uuid::new_v4(),
            root_task_id: workflow.expected_head.clone(),
            parent_task_id: None,
            assigned_role: match workflow.stage {
                Stage::Memory => "memory",
                Stage::Research => "research",
                Stage::Builder => "builder",
                Stage::StructuralQa | Stage::ReasoningQa => "qa",
                Stage::Git => "git",
            }
            .into(),
            objective: "test".into(),
            evidence_refs: vec![],
            result_contract: ResultContract {
                kind: "test".into(),
                required: vec![],
            },
            constraints: vec![workflow.marker().unwrap()],
            budget: JobBudget {
                deadline_at: Utc::now().to_rfc3339(),
                max_model_calls: Some(0),
                max_output_bytes: MAX_OUTPUT_BYTES,
            },
            attempt: 1,
        }
    }
}
