use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use buzz_core::kind::{
    KIND_GIT_STATUS_CLOSED, KIND_GIT_STATUS_DRAFT, KIND_GIT_STATUS_MERGED, KIND_GIT_STATUS_OPEN,
};

use crate::client::BuzzClient;
use crate::commands::with_git_provenance;
use crate::error::CliError;
use crate::validate::{read_or_stdin, sdk_err, validate_hex64, validate_repo_id};
use buzz_sdk::{GitIssueMeta, GitRepoCoord, GitStatusMeta};
use fs2::FileExt;
use nostr::{EventBuilder, Kind, Tag, Timestamp};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const ISSUE_ASSIGNMENT_LABEL: &str = "assignment";
const ISSUE_UNASSIGNMENT_LABEL: &str = "unassignment";

/// NIP-34 issue/patch status-transition kinds (open/resolved/closed/draft).
const ISSUE_STATUS_KINDS: [u32; 4] = [
    KIND_GIT_STATUS_OPEN,
    KIND_GIT_STATUS_MERGED,
    KIND_GIT_STATUS_CLOSED,
    KIND_GIT_STATUS_DRAFT,
];

/// Relay filter for the status-transition events (kind 1630–1633) of the
/// given issue event ids. Status events reference their issue via an `e` tag,
/// so `#e` is the complete read path regardless of whether the status setter
/// passed repo coordinates (the `a` tag is optional on status events).
fn issue_status_filter(issue_ids: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "kinds": ISSUE_STATUS_KINDS,
        "#e": issue_ids,
        "limit": 1000
    })
}

fn status_event_created_at(event: &serde_json::Value) -> i64 {
    event
        .get("created_at")
        .and_then(|c| c.as_i64())
        .unwrap_or(0)
}

/// Reorder a flat relay response: issue events (kind 1621) first, then their
/// status events (kind 1630–1633) chronologically. This is the read path for
/// `buzz issues status --content` — without it, status-change commentary is
/// accepted by the relay but invisible to every CLI consumer.
fn order_issue_with_status_events(raw: &str) -> Result<String, CliError> {
    let events: Vec<serde_json::Value> = serde_json::from_str(raw)
        .map_err(|e| CliError::Other(format!("failed to parse issue response: {e}")))?;
    let mut issues: Vec<serde_json::Value> = Vec::new();
    let mut statuses: Vec<serde_json::Value> = Vec::new();
    for event in events {
        match event.get("kind").and_then(|k| k.as_u64()) {
            Some(1621) => issues.push(event),
            Some(kind) if ISSUE_STATUS_KINDS.contains(&(kind as u32)) => statuses.push(event),
            _ => {}
        }
    }
    statuses.sort_by_key(status_event_created_at);
    issues.append(&mut statuses);
    serde_json::to_string(&issues)
        .map_err(|e| CliError::Other(format!("failed to serialize issue response: {e}")))
}

fn assignment_note_label(assignees: &[String], label: Option<&str>) -> Result<String, CliError> {
    if let Some(label) = label {
        let label = label.trim();
        if label.is_empty() || label.chars().count() > 128 {
            return Err(CliError::Usage(
                "--label must be between 1 and 128 characters".into(),
            ));
        }
        return Ok(label.to_string());
    }
    let prefixes = assignees
        .iter()
        .map(|assignee| format!("{}…", assignee.chars().take(8).collect::<String>()))
        .collect::<Vec<_>>();
    for included in (0..=prefixes.len()).rev() {
        let omitted = prefixes.len() - included;
        let mut generated = prefixes[..included].join(", ");
        if omitted > 0 {
            let suffix = format!("{omitted} other{}", if omitted == 1 { "" } else { "s" });
            if !generated.is_empty() {
                generated.push_str(", and ");
            }
            generated.push_str(&suffix);
        }
        if !generated.is_empty() && generated.chars().count() <= 128 {
            return Ok(generated);
        }
    }
    Err(CliError::Usage(
        "Unable to generate an assignee label between 1 and 128 characters".into(),
    ))
}

#[derive(Clone, Copy)]
enum IssueAssignmentOperation {
    Assign,
    Unassign,
}

#[derive(Debug)]
struct AssignmentEvent {
    id: String,
    pubkey: String,
    created_at: u64,
    tags: Vec<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct AssignmentQueryEvent {
    id: String,
    kind: u16,
    pubkey: String,
    created_at: u64,
    tags: Vec<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize)]
struct IssueCommentQueryEvent {
    id: String,
    kind: u16,
    pubkey: String,
    content: String,
    tags: Vec<Vec<String>>,
}

fn parse_verified_issue_comment(
    value: serde_json::Value,
) -> Result<IssueCommentQueryEvent, CliError> {
    let signed: nostr::Event = serde_json::from_value(value.clone())
        .map_err(|error| CliError::Other(format!("invalid signed issue comment: {error}")))?;
    signed
        .verify()
        .map_err(|error| CliError::Other(format!("invalid issue comment signature: {error}")))?;
    serde_json::from_value(value)
        .map_err(|error| CliError::Other(format!("invalid issue comment response: {error}")))
}

#[derive(Debug, PartialEq, Eq)]
enum ReviewCommentResolution {
    Create,
    Reuse(String),
}

struct ReviewCommentLock {
    _file: File,
}

fn acquire_review_comment_lock(
    directory: &Path,
    review_id: &str,
) -> Result<ReviewCommentLock, CliError> {
    let digest = hex::encode(Sha256::digest(review_id.as_bytes()));
    let path = directory.join(format!("buzz-issue-review-{digest}.lock"));
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|error| {
        CliError::Other(format!(
            "cannot open review comment lock at {}: {error}",
            path.display()
        ))
    })?;
    file.try_lock_exclusive().map_err(|error| {
        CliError::Other(format!(
            "review comment lock is busy at {}: {error}",
            path.display()
        ))
    })?;
    file.set_len(0)
        .map_err(|error| CliError::Other(format!("cannot reset review comment lock: {error}")))?;
    if let Err(error) = writeln!(file, "{}", std::process::id()) {
        return Err(CliError::Other(format!(
            "cannot initialize review comment lock: {error}"
        )));
    }
    Ok(ReviewCommentLock { _file: file })
}

fn canonical_repo_coord(repo_coord: &str) -> String {
    let mut repo_parts = repo_coord.splitn(3, ':');
    format!(
        "{}:{}:{}",
        repo_parts.next().unwrap_or_default(),
        repo_parts.next().unwrap_or_default().to_ascii_lowercase(),
        repo_parts.next().unwrap_or_default()
    )
}

fn build_review_comment_query(issue_id: &str, repo_coord: &str, author: &str) -> serde_json::Value {
    serde_json::json!({
        "kinds": [1],
        "authors": [author.to_ascii_lowercase()],
        "#e": [issue_id.to_ascii_lowercase()],
        "#a": [canonical_repo_coord(repo_coord)],
        "writer_consistent": true,
    })
}

fn build_review_comment_event(
    issue_id: &str,
    repo_coord: &str,
    review_id: &str,
    content: &str,
    recipients: &[String],
) -> Result<EventBuilder, CliError> {
    let issue_id = issue_id.to_ascii_lowercase();
    let repo_coord = canonical_repo_coord(repo_coord);
    let mut tags = vec![
        Tag::parse(["e", issue_id.as_str(), "", "root"])
            .map_err(|error| CliError::Other(format!("invalid issue root tag: {error}")))?,
        Tag::parse(["a", repo_coord.as_str()])
            .map_err(|error| CliError::Other(format!("invalid repository tag: {error}")))?,
        Tag::parse(["review-id", review_id])
            .map_err(|error| CliError::Other(format!("invalid review-id tag: {error}")))?,
    ];
    let mut recipients = recipients
        .iter()
        .map(|recipient| recipient.to_ascii_lowercase())
        .collect::<Vec<_>>();
    recipients.sort();
    recipients.dedup();
    for recipient in recipients {
        tags.push(
            Tag::parse(["p", recipient.as_str()])
                .map_err(|error| CliError::Other(format!("invalid recipient tag: {error}")))?,
        );
    }
    Ok(EventBuilder::new(Kind::TextNote, content).tags(tags))
}

fn resolve_review_comment(
    events: &[IssueCommentQueryEvent],
    issue_id: &str,
    repo_coord: &str,
    author: &str,
    review_id: &str,
    content: &str,
    recipients: &[String],
) -> Result<ReviewCommentResolution, CliError> {
    let matches = events
        .iter()
        .filter(|event| {
            event.tags.iter().any(|tag| {
                matches!(tag.as_slice(), [name, value, ..] if name == "review-id" && value == review_id)
            })
        })
        .collect::<Vec<_>>();
    let Some(event) = matches.first() else {
        return Ok(ReviewCommentResolution::Create);
    };
    if matches.len() != 1 {
        return Err(CliError::Other(format!(
            "multiple issue comments found for Review-ID {review_id}"
        )));
    }

    let root_matches = event.tags.iter().any(|tag| {
        matches!(tag.as_slice(), [name, value, relay, marker, ..]
            if name == "e" && value == issue_id && relay.is_empty() && marker == "root")
    });
    let repo_matches = event.tags.iter().any(
        |tag| matches!(tag.as_slice(), [name, value, ..] if name == "a" && value == repo_coord),
    );
    let actual_recipients = event
        .tags
        .iter()
        .filter_map(|tag| match tag.as_slice() {
            [name, value, ..] if name == "p" => Some(value.to_ascii_lowercase()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let expected_recipients = recipients
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    if event.kind != 1
        || !event.pubkey.eq_ignore_ascii_case(author)
        || event.content != content
        || !root_matches
        || !repo_matches
        || actual_recipients != expected_recipients
    {
        return Err(CliError::Other(format!(
            "issue comment for Review-ID {review_id} does not match the requested handoff"
        )));
    }
    Ok(ReviewCommentResolution::Reuse(event.id.clone()))
}

impl From<&AssignmentQueryEvent> for AssignmentEvent {
    fn from(event: &AssignmentQueryEvent) -> Self {
        Self {
            id: event.id.clone(),
            pubkey: event.pubkey.clone(),
            created_at: event.created_at,
            tags: event.tags.clone(),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct AssignmentState {
    assignees: HashSet<String>,
    heads: HashMap<String, String>,
}

#[derive(Debug)]
struct ParsedAssignmentOperation {
    id: String,
    is_assignment: bool,
    pubkeys: Vec<String>,
    prior: Option<String>,
}

fn tag_values<'a>(event: &'a AssignmentEvent, name: &str) -> Vec<&'a str> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            if tag.first().map(String::as_str) != Some(name) {
                return None;
            }
            tag.get(1)
                .map(String::as_str)
                .filter(|value| !value.is_empty())
        })
        .collect()
}

fn has_root_tag(event: &AssignmentEvent, issue_id: &str) -> bool {
    event.tags.iter().any(|tag| {
        matches!(
            tag.as_slice(),
            [name, value, ..] if name == "e" && value == issue_id
        )
    })
}

fn is_hex64(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn apply_assignment_operation(state: &mut AssignmentState, operation: ParsedAssignmentOperation) {
    if let Some(prior) = operation.prior.as_ref() {
        let Some(target) = operation.pubkeys.first() else {
            return;
        };
        if state.heads.get(target) != Some(prior) {
            return;
        }
    }
    for pubkey in operation.pubkeys {
        if operation.is_assignment {
            state.assignees.insert(pubkey.clone());
        } else {
            state.assignees.remove(&pubkey);
        }
        state.heads.insert(pubkey, operation.id.clone());
    }
}

fn reduce_assignment_operations(
    issue_id: &str,
    issue_author: &str,
    repo_owner: &str,
    events: &[AssignmentEvent],
) -> AssignmentState {
    let issue_author = issue_author.to_ascii_lowercase();
    let repo_owner = repo_owner.to_ascii_lowercase();
    let mut events = events.iter().collect::<Vec<_>>();
    events.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut uncaused_self_operations = Vec::new();
    let mut authoritative_operations = Vec::new();
    let mut causal_self_operations = Vec::new();
    for event in events {
        if !has_root_tag(event, issue_id) {
            continue;
        }
        let labels = tag_values(event, "t");
        let is_assignment = labels.contains(&ISSUE_ASSIGNMENT_LABEL);
        let is_unassignment = labels.contains(&ISSUE_UNASSIGNMENT_LABEL);
        if is_assignment == is_unassignment {
            continue;
        }
        let signer = event.pubkey.to_ascii_lowercase();
        let pubkeys = tag_values(event, "p")
            .into_iter()
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        let is_authoritative = signer == issue_author || signer == repo_owner;
        let is_self_operation = pubkeys.len() == 1 && pubkeys[0] == signer;
        if !is_authoritative && !is_self_operation {
            continue;
        }

        let mut operation = ParsedAssignmentOperation {
            id: event.id.to_ascii_lowercase(),
            is_assignment,
            pubkeys,
            prior: None,
        };
        if is_authoritative {
            authoritative_operations.push(operation);
            continue;
        }

        let prior_tags = event
            .tags
            .iter()
            .filter(|tag| tag.first().map(String::as_str) == Some("prior"))
            .collect::<Vec<_>>();
        if prior_tags.is_empty() {
            uncaused_self_operations.push(operation);
        } else if prior_tags.len() == 1 && prior_tags[0].get(1).is_some_and(|prior| is_hex64(prior))
        {
            operation.prior = prior_tags[0].get(1).map(|prior| prior.to_ascii_lowercase());
            causal_self_operations.push(operation);
        }
    }

    let mut state = AssignmentState::default();
    for operation in uncaused_self_operations
        .into_iter()
        .chain(authoritative_operations)
        .chain(causal_self_operations)
    {
        apply_assignment_operation(&mut state, operation);
    }
    state
}

struct IssueAssignmentContext {
    created_at: u64,
    prior: Option<String>,
}

impl IssueAssignmentOperation {
    fn content(self, label: &str) -> String {
        match self {
            Self::Assign => format!("Assigned this issue to {label}"),
            Self::Unassign => format!("Unassigned {label} from this issue"),
        }
    }
}

pub async fn cmd_issue_comment(
    client: &BuzzClient,
    issue: &str,
    repo_owner: &str,
    repo_id: &str,
    content: &str,
    review_id: &str,
    recipients: &[String],
) -> Result<(), CliError> {
    validate_hex64(issue)?;
    validate_hex64(repo_owner)?;
    validate_repo_id(repo_id)?;
    for recipient in recipients {
        validate_hex64(recipient)?;
    }
    if review_id.is_empty()
        || review_id.len() > 256
        || review_id
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err(CliError::Usage(
            "--review-id must contain 1..256 non-whitespace characters".into(),
        ));
    }
    let content = read_or_stdin(content)?;
    if content.is_empty() || content.len() > 10_000 {
        return Err(CliError::Usage(
            "--content must contain 1..10000 UTF-8 bytes".into(),
        ));
    }

    let author = client.keys().public_key().to_hex();
    let issue = issue.to_ascii_lowercase();
    let recipients = recipients
        .iter()
        .map(|recipient| recipient.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let repo_coord = format!("30617:{}:{repo_id}", repo_owner.to_ascii_lowercase());
    let lock_directory = dirs::cache_dir()
        .ok_or_else(|| CliError::Other("cannot resolve private cache directory".into()))?
        .join("buzz")
        .join("locks");
    std::fs::create_dir_all(&lock_directory).map_err(|error| {
        CliError::Other(format!("cannot create review lock directory: {error}"))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&lock_directory, std::fs::Permissions::from_mode(0o700)).map_err(
            |error| CliError::Other(format!("cannot secure review lock directory: {error}")),
        )?;
    }
    let lock_scope = format!("{author}:{issue}:{repo_coord}:{review_id}");
    let _lock = acquire_review_comment_lock(&lock_directory, &lock_scope)?;
    let filter = build_review_comment_query(&issue, &repo_coord, &author);
    let before = client
        .query_all(filter.clone())
        .await?
        .into_iter()
        .map(parse_verified_issue_comment)
        .collect::<Result<Vec<IssueCommentQueryEvent>, _>>()?;
    if let ReviewCommentResolution::Reuse(event_id) = resolve_review_comment(
        &before,
        &issue,
        &repo_coord,
        &author,
        review_id,
        &content,
        &recipients,
    )? {
        println!(
            "{}",
            serde_json::json!({"event_id": event_id, "reused": true})
        );
        return Ok(());
    }

    let event = client.sign_event(build_review_comment_event(
        &issue,
        &repo_coord,
        review_id,
        &content,
        &recipients,
    )?)?;
    let submitted_event_id = event.id.to_hex();
    let submit_result = client.submit_event(event).await;

    let after = client
        .query_all(filter)
        .await?
        .into_iter()
        .map(parse_verified_issue_comment)
        .collect::<Result<Vec<IssueCommentQueryEvent>, _>>()?;
    match resolve_review_comment(
        &after,
        &issue,
        &repo_coord,
        &author,
        review_id,
        &content,
        &recipients,
    )? {
        ReviewCommentResolution::Reuse(event_id) => {
            println!(
                "{}",
                serde_json::json!({
                    "event_id": event_id,
                    "published_event_id": submitted_event_id,
                    "reused": event_id != submitted_event_id
                })
            );
            Ok(())
        }
        ReviewCommentResolution::Create => match submit_result {
            Err(error) => Err(error),
            Ok(response) => Err(CliError::Other(format!(
                "relay accepted no independently readable issue comment: {response}"
            ))),
        },
    }
}

pub async fn cmd_create_issue(
    client: &BuzzClient,
    repo_owner: &str,
    repo_id: &str,
    subject: &str,
    content: &str,
    labels: &[String],
    to: &[String],
) -> Result<(), CliError> {
    validate_hex64(repo_owner)?;
    validate_repo_id(repo_id)?;
    let body = read_or_stdin(content)?;

    let meta = GitIssueMeta {
        labels: labels.to_vec(),
        recipients: to.to_vec(),
    };

    let repo = GitRepoCoord {
        owner: repo_owner.to_string(),
        id: repo_id.to_string(),
    };

    let builder = with_git_provenance(
        buzz_sdk::build_git_issue(&repo, subject, &body, &meta).map_err(sdk_err)?,
    )?;
    let event = client.sign_event(builder)?;
    let event_id = event.id.to_hex();
    let resp = client.submit_event(event).await?;
    // `link` renders as a rich preview card in Buzz Desktop when included in
    // a chat message — agents announce issues with it (see base_prompt.md).
    let link = crate::links::issue_link(&event_id, repo_owner, repo_id);
    crate::client::print_create_response(&resp, "link", &link);
    Ok(())
}

/// Publish an issue assignment: a kind:1 comment on the issue whose `p`
/// tags are the assignees, labeled `t: assignment` (same event shape the
/// Desktop app writes). Clients trust it when signed by the issue author
/// or repo owner, or when it is a self-assignment.
pub async fn cmd_assign_issue(
    client: &BuzzClient,
    issue: &str,
    repo_owner: &str,
    repo_id: &str,
    assignees: &[String],
    label: Option<&str>,
) -> Result<(), CliError> {
    publish_issue_assignment_operation(
        client,
        issue,
        repo_owner,
        repo_id,
        assignees,
        label,
        IssueAssignmentOperation::Assign,
    )
    .await
}

/// Publish an issue unassignment with the same trust rules as assignment.
pub async fn cmd_unassign_issue(
    client: &BuzzClient,
    issue: &str,
    repo_owner: &str,
    repo_id: &str,
    assignees: &[String],
    label: Option<&str>,
) -> Result<(), CliError> {
    publish_issue_assignment_operation(
        client,
        issue,
        repo_owner,
        repo_id,
        assignees,
        label,
        IssueAssignmentOperation::Unassign,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn publish_issue_assignment_operation(
    client: &BuzzClient,
    issue: &str,
    repo_owner: &str,
    repo_id: &str,
    assignees: &[String],
    label: Option<&str>,
    operation: IssueAssignmentOperation,
) -> Result<(), CliError> {
    validate_hex64(issue)?;
    validate_hex64(repo_owner)?;
    validate_repo_id(repo_id)?;
    for assignee in assignees {
        validate_hex64(assignee)?;
    }

    let label = assignment_note_label(assignees, label)?;
    let content = operation.content(&label);
    let repo = GitRepoCoord {
        owner: repo_owner.to_string(),
        id: repo_id.to_string(),
    };
    let signer = client.keys().public_key().to_hex();
    let is_self_service = assignees.len() == 1
        && assignees[0].eq_ignore_ascii_case(&signer)
        && !signer.eq_ignore_ascii_case(repo_owner);
    let context = issue_assignment_context(client, issue, &repo, &signer, is_self_service).await?;
    let builder = match (operation, is_self_service) {
        (IssueAssignmentOperation::Assign, true) => {
            buzz_sdk::build_git_issue_assignment_with_prior(
                &repo,
                issue,
                assignees,
                &content,
                context.prior.as_deref(),
            )
        }
        (IssueAssignmentOperation::Unassign, true) => {
            buzz_sdk::build_git_issue_unassignment_with_prior(
                &repo,
                issue,
                assignees,
                &content,
                context.prior.as_deref(),
            )
        }
        (IssueAssignmentOperation::Assign, false) => {
            buzz_sdk::build_git_issue_assignment(&repo, issue, assignees, &content)
        }
        (IssueAssignmentOperation::Unassign, false) => {
            buzz_sdk::build_git_issue_unassignment(&repo, issue, assignees, &content)
        }
    }
    .map(|builder| builder.custom_created_at(Timestamp::from_secs(context.created_at)));
    let event = client.sign_event(builder.map_err(sdk_err)?)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

fn build_issue_assignment_queries(issue: &str, signer: &str) -> [serde_json::Value; 3] {
    let root_filter = serde_json::json!({
        "kinds": [1621],
        "ids": [issue],
        "limit": 1,
        "writer_consistent": true,
    });
    let assignment_filter = serde_json::json!({
        "kinds": [1],
        "#e": [issue],
        "#t": [ISSUE_ASSIGNMENT_LABEL, ISSUE_UNASSIGNMENT_LABEL],
        "limit": 500,
        "writer_consistent": true,
    });
    let signer_comment_filter = serde_json::json!({
        "kinds": [1],
        "#e": [issue],
        "authors": [signer],
        "limit": 1,
        "writer_consistent": true,
    });
    [root_filter, assignment_filter, signer_comment_filter]
}

async fn issue_assignment_context(
    client: &BuzzClient,
    issue: &str,
    repo: &GitRepoCoord,
    signer: &str,
    include_prior: bool,
) -> Result<IssueAssignmentContext, CliError> {
    let filters = build_issue_assignment_queries(issue, signer);
    let response = client.query_multi(&filters).await?;
    // CLI read responses intentionally omit signatures, so deserialize only
    // the event fields needed for assignment reduction.
    let events = serde_json::from_str::<Vec<AssignmentQueryEvent>>(&response)
        .map_err(|error| CliError::Other(format!("parse issue assignment context: {error}")))?;
    let root = events
        .iter()
        .find(|event| event.kind == 1621 && event.id == issue)
        .ok_or_else(|| CliError::Other("issue root was not returned by the relay".into()))?;
    let expected_repo = format!("30617:{}:{}", repo.owner.to_ascii_lowercase(), repo.id);
    let root_matches_repo = root.tags.iter().any(|tag| {
        tag.as_slice().first().map(String::as_str) == Some("a")
            && tag.as_slice().get(1) == Some(&expected_repo)
    });
    if !root_matches_repo {
        return Err(CliError::Other(
            "issue root does not match the requested repository".into(),
        ));
    }

    let comments = events
        .iter()
        .filter(|event| event.kind == 1)
        .map(AssignmentEvent::from)
        .collect::<Vec<_>>();
    let latest = comments
        .iter()
        .filter(|event| event.pubkey.eq_ignore_ascii_case(signer))
        .map(|event| event.created_at)
        .max()
        .unwrap_or(0);
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| CliError::Other(format!("read system clock: {error}")))?
        .as_secs()
        .max(latest.saturating_add(1));
    let prior = include_prior
        .then(|| {
            reduce_assignment_operations(issue, &root.pubkey, &repo.owner, &comments)
                .heads
                .get(&signer.to_ascii_lowercase())
                .cloned()
        })
        .flatten();
    Ok(IssueAssignmentContext { created_at, prior })
}

pub async fn cmd_get_issue(client: &BuzzClient, event: &str) -> Result<(), CliError> {
    validate_hex64(event)?;
    let issue_filter = serde_json::json!({
        "kinds": [1621],
        "ids": [event]
    });
    let status_filter = issue_status_filter(&[event]);
    let resp = client.query_multi(&[issue_filter, status_filter]).await?;
    println!("{}", order_issue_with_status_events(&resp)?);
    Ok(())
}

pub async fn cmd_list_issues(
    client: &BuzzClient,
    repo_owner: &str,
    repo_id: &str,
    author: Option<&str>,
    label: Option<&str>,
    limit: Option<u32>,
) -> Result<(), CliError> {
    validate_hex64(repo_owner)?;
    validate_repo_id(repo_id)?;

    let a_value = format!("30617:{repo_owner}:{repo_id}");
    let mut filter = serde_json::json!({
        "kinds": [1621],
        "#a": [a_value]
    });

    if let Some(pk) = author {
        validate_hex64(pk)?;
        filter["authors"] = serde_json::json!([pk]);
    }
    if let Some(l) = label {
        filter["#t"] = serde_json::json!([l]);
    }
    if let Some(n) = limit {
        filter["limit"] = serde_json::json!(n);
    }

    let resp = client.query(&filter).await?;
    let issues: Vec<serde_json::Value> = serde_json::from_str(&resp)
        .map_err(|e| CliError::Other(format!("failed to parse issue list: {e}")))?;
    let issue_ids: Vec<&str> = issues
        .iter()
        .filter_map(|issue| issue.get("id").and_then(|id| id.as_str()))
        .collect();
    if issue_ids.is_empty() {
        println!("{resp}");
        return Ok(());
    }
    let status_resp = client.query(&issue_status_filter(&issue_ids)).await?;
    let mut statuses: Vec<serde_json::Value> = serde_json::from_str(&status_resp)
        .map_err(|e| CliError::Other(format!("failed to parse status events: {e}")))?;
    statuses.sort_by_key(status_event_created_at);
    let mut merged = issues;
    merged.append(&mut statuses);
    println!(
        "{}",
        serde_json::to_string(&merged)
            .map_err(|e| CliError::Other(format!("failed to serialize issue list: {e}")))?
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn cmd_issue_status(
    client: &BuzzClient,
    issue: &str,
    status: &str,
    content: Option<&str>,
    repo_owner: Option<&str>,
    repo_id: Option<&str>,
    euc: Option<&str>,
    to: &[String],
) -> Result<(), CliError> {
    validate_hex64(issue)?;
    let status = crate::commands::patches::parse_status(status)?;
    let body = match content {
        Some(c) => read_or_stdin(c)?,
        None => String::new(),
    };

    let repo = match (repo_owner, repo_id) {
        (Some(owner), Some(id)) => {
            validate_hex64(owner)?;
            validate_repo_id(id)?;
            Some(GitRepoCoord {
                owner: owner.to_string(),
                id: id.to_string(),
            })
        }
        (None, None) => None,
        _ => {
            return Err(CliError::Usage(
                "--repo-owner and --repo-id must be given together".into(),
            ))
        }
    };

    // Mirrors `buzz patches status`: default a `p` tag to the repo owner
    // for discoverability, plus a `--to` escape hatch for the issue author
    // or anyone else who should be notified of the status change.
    let mut recipients = Vec::new();
    if let Some(ref repo) = repo {
        recipients.push(repo.owner.clone());
    }
    for recipient in to {
        validate_hex64(recipient)?;
        if !recipients.contains(recipient) {
            recipients.push(recipient.clone());
        }
    }

    let meta = GitStatusMeta {
        root_event: issue.to_string(),
        accepted_revision_root: None,
        repo,
        euc: euc.map(str::to_string),
        recipients,
        applied_patches: vec![],
        merge_commit: None,
        applied_as_commits: vec![],
    };

    let builder =
        with_git_provenance(buzz_sdk::build_git_status(status, &body, &meta).map_err(sdk_err)?)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

pub async fn dispatch(cmd: crate::IssuesCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::{IssueBwCmd, IssuesCmd};
    match cmd {
        IssuesCmd::Bw(IssueBwCmd::Get {
            issue,
            repo_owner,
            repo_id,
        }) => {
            crate::commands::bw::get_issue_projection(client, &issue, &repo_owner, &repo_id).await
        }
        IssuesCmd::Bw(IssueBwCmd::Enroll {
            issue,
            repo_owner,
            repo_id,
        }) => crate::commands::bw::enroll_issue(client, &issue, &repo_owner, &repo_id).await,
        IssuesCmd::Bw(IssueBwCmd::Accept {
            issue,
            repo_owner,
            repo_id,
            delegated,
        }) => {
            crate::commands::bw::accept_issue(client, &issue, &repo_owner, &repo_id, delegated)
                .await
        }
        IssuesCmd::Bw(IssueBwCmd::AssignWriter {
            issue,
            repo_owner,
            repo_id,
            writer,
        }) => {
            crate::commands::bw::assign_writer(client, &issue, &repo_owner, &repo_id, &writer).await
        }
        IssuesCmd::Bw(IssueBwCmd::MoveToReady {
            issue,
            repo_owner,
            repo_id,
            stream,
            rework,
            terminal_set,
        }) => {
            crate::commands::bw::move_to_ready(
                client,
                &issue,
                &repo_owner,
                &repo_id,
                &stream,
                rework,
                terminal_set,
            )
            .await
        }
        IssuesCmd::Bw(IssueBwCmd::StartDevelopment {
            issue,
            repo_owner,
            repo_id,
        }) => {
            crate::commands::bw::issue_bw_start_development(client, &issue, &repo_owner, &repo_id)
                .await
        }
        IssuesCmd::Bw(IssueBwCmd::MarkImplemented {
            issue,
            repo_owner,
            repo_id,
            commit,
            tests,
        }) => {
            let tests = read_or_stdin(&tests)?;
            crate::commands::bw::issue_bw_mark_implemented(
                client,
                &issue,
                &repo_owner,
                &repo_id,
                &commit,
                &tests,
            )
            .await
        }
        IssuesCmd::Comment {
            issue,
            repo_owner,
            repo_id,
            content,
            review_id,
            to,
        } => {
            cmd_issue_comment(
                client,
                &issue,
                &repo_owner,
                &repo_id,
                &content,
                &review_id,
                &to,
            )
            .await
        }
        IssuesCmd::Create {
            repo_owner,
            repo_id,
            title,
            content,
            label,
            to,
        } => cmd_create_issue(client, &repo_owner, &repo_id, &title, &content, &label, &to).await,
        IssuesCmd::Get { event } => cmd_get_issue(client, &event).await,
        IssuesCmd::List {
            repo_owner,
            repo_id,
            author,
            label,
            limit,
        } => {
            cmd_list_issues(
                client,
                &repo_owner,
                &repo_id,
                author.as_deref(),
                label.as_deref(),
                limit,
            )
            .await
        }
        IssuesCmd::StartDevelopment {
            issue,
            repo_owner,
            repo_id,
        } => crate::commands::bw::start_development(client, &issue, &repo_owner, &repo_id).await,
        IssuesCmd::MarkImplemented {
            issue,
            repo_owner,
            repo_id,
            commit,
            tests,
        } => {
            let tests = read_or_stdin(&tests)?;
            crate::commands::bw::mark_implemented(
                client,
                &issue,
                &repo_owner,
                &repo_id,
                &commit,
                &tests,
            )
            .await
        }
        IssuesCmd::Status {
            issue,
            status,
            content,
            repo_owner,
            repo_id,
            euc,
            to,
        } => {
            cmd_issue_status(
                client,
                &issue,
                &status,
                content.as_deref(),
                repo_owner.as_deref(),
                repo_id.as_deref(),
                euc.as_deref(),
                &to,
            )
            .await
        }
        IssuesCmd::Assign {
            issue,
            repo_owner,
            repo_id,
            assignee,
            label,
        } => {
            cmd_assign_issue(
                client,
                &issue,
                &repo_owner,
                &repo_id,
                &assignee,
                label.as_deref(),
            )
            .await
        }
        IssuesCmd::Unassign {
            issue,
            repo_owner,
            repo_id,
            assignee,
            label,
        } => {
            cmd_unassign_issue(
                client,
                &issue,
                &repo_owner,
                &repo_id,
                &assignee,
                label.as_deref(),
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::{
        acquire_review_comment_lock, assignment_note_label, build_issue_assignment_queries,
        build_review_comment_event, build_review_comment_query, parse_verified_issue_comment,
        reduce_assignment_operations, resolve_review_comment, AssignmentEvent,
        AssignmentQueryEvent, IssueCommentQueryEvent, ReviewCommentResolution,
        ISSUE_ASSIGNMENT_LABEL, ISSUE_UNASSIGNMENT_LABEL,
    };
    use nostr::{EventBuilder, Keys};

    const ISSUE: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    const AUTHOR: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const OWNER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const VOLUNTEER: &str = "5555555555555555555555555555555555555555555555555555555555555555";

    #[test]
    fn issue_comment_readback_rejects_tampered_signed_events() {
        let signed = EventBuilder::text_note("review handoff")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let mut event = serde_json::to_value(signed).unwrap();
        assert!(parse_verified_issue_comment(event.clone()).is_ok());

        event["content"] = serde_json::json!("tampered");
        assert!(parse_verified_issue_comment(event).is_err());
    }

    #[test]
    fn review_comment_lock_serializes_the_same_review_id() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire_review_comment_lock(dir.path(), "board:task:run-42").unwrap();
        assert!(acquire_review_comment_lock(dir.path(), "board:task:run-42").is_err());
        drop(first);
        acquire_review_comment_lock(dir.path(), "board:task:run-42").unwrap();
    }

    #[test]
    fn stale_review_comment_lock_file_does_not_block_retry() {
        let dir = tempfile::tempdir().unwrap();
        let scope = "board:task:run-42";
        let digest = Sha256::digest(scope.as_bytes());
        let path = dir
            .path()
            .join(format!("buzz-issue-review-{}.lock", hex::encode(digest)));
        std::fs::write(path, "stale-owner").unwrap();

        acquire_review_comment_lock(dir.path(), scope).unwrap();
    }

    #[test]
    fn review_comment_event_has_issue_root_repo_and_technical_recipient_tags() {
        let keys = nostr::Keys::generate();
        let coord = format!("30617:{OWNER}:demo");
        let uppercase_coord = format!("30617:{}:demo", OWNER.to_ascii_uppercase());
        let event = build_review_comment_event(
            &ISSUE.to_ascii_uppercase(),
            &uppercase_coord,
            "board:task:run-42",
            "Testziel: Start\nReview-ID: board:task:run-42",
            &[VOLUNTEER.to_ascii_uppercase()],
        )
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
        let tags = event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect::<Vec<_>>();

        assert_eq!(event.kind, nostr::Kind::TextNote);
        assert!(tags.contains(&vec!["e".into(), ISSUE.into(), "".into(), "root".into()]));
        assert!(tags.contains(&vec!["a".into(), coord]));
        assert!(tags.contains(&vec!["review-id".into(), "board:task:run-42".into()]));
        assert!(tags.contains(&vec!["p".into(), VOLUNTEER.into()]));
    }

    #[test]
    fn review_comment_query_is_canonical_writer_consistent_and_unbounded() {
        assert_eq!(
            build_review_comment_query(
                &ISSUE.to_ascii_uppercase(),
                &format!("30617:{}:demo", OWNER.to_ascii_uppercase()),
                &AUTHOR.to_ascii_uppercase(),
            ),
            serde_json::json!({
                "kinds": [1],
                "authors": [AUTHOR],
                "#e": [ISSUE],
                "#a": [format!("30617:{OWNER}:demo")],
                "writer_consistent": true,
            })
        );
    }

    #[test]
    fn review_comment_ensure_reuses_exact_event_and_rejects_drift_or_duplicates() {
        let content = "Testziel: Start\nReview-ID: board:task:run-42";
        let exact = IssueCommentQueryEvent {
            id: "1".repeat(64),
            kind: 1,
            pubkey: AUTHOR.into(),
            content: content.into(),
            tags: vec![
                vec!["e".into(), ISSUE.into(), "".into(), "root".into()],
                vec!["a".into(), format!("30617:{OWNER}:demo")],
                vec!["review-id".into(), "board:task:run-42".into()],
                vec!["p".into(), VOLUNTEER.into()],
            ],
        };

        assert_eq!(
            resolve_review_comment(
                std::slice::from_ref(&exact),
                ISSUE,
                &format!("30617:{OWNER}:demo"),
                AUTHOR,
                "board:task:run-42",
                content,
                &[VOLUNTEER.to_string()],
            )
            .unwrap(),
            ReviewCommentResolution::Reuse(exact.id.clone())
        );

        let mut drifted = exact.clone();
        drifted.content.push_str("\nchanged");
        assert!(resolve_review_comment(
            &[drifted],
            ISSUE,
            &format!("30617:{OWNER}:demo"),
            AUTHOR,
            "board:task:run-42",
            content,
            &[VOLUNTEER.to_string()],
        )
        .is_err());
        assert!(resolve_review_comment(
            &[exact.clone(), exact],
            ISSUE,
            &format!("30617:{OWNER}:demo"),
            AUTHOR,
            "board:task:run-42",
            content,
            &[VOLUNTEER.to_string()],
        )
        .is_err());
    }

    fn assignment_event(
        pubkey: &str,
        id: &str,
        assignment: bool,
        created_at: u64,
        prior: Option<&str>,
    ) -> AssignmentEvent {
        let mut tags = vec![
            vec!["e".into(), ISSUE.into(), "".into(), "root".into()],
            vec!["p".into(), VOLUNTEER.into()],
            vec![
                "t".into(),
                if assignment {
                    ISSUE_ASSIGNMENT_LABEL.into()
                } else {
                    ISSUE_UNASSIGNMENT_LABEL.into()
                },
            ],
        ];
        if let Some(prior) = prior {
            tags.push(vec!["prior".into(), prior.into()]);
        }
        AssignmentEvent {
            id: id.into(),
            pubkey: pubkey.into(),
            created_at,
            tags,
        }
    }

    #[test]
    fn assignment_note_label_enforces_desktop_length_limit() {
        let assignees = vec!["a".repeat(64)];
        assert_eq!(
            assignment_note_label(&assignees, Some(" Thomas ")).unwrap(),
            "Thomas"
        );
        assert!(assignment_note_label(&assignees, Some("")).is_err());
        assert!(assignment_note_label(&assignees, Some(&"x".repeat(129))).is_err());
        assert_eq!(
            assignment_note_label(&assignees, None).unwrap(),
            "aaaaaaaa…"
        );
        let many_assignees = (0..50)
            .map(|index| format!("{index:064x}"))
            .collect::<Vec<_>>();
        let generated = assignment_note_label(&many_assignees, None).unwrap();
        assert!(generated.chars().count() <= 128);
        assert!(generated.contains("others"));
    }

    #[test]
    fn assignment_query_event_accepts_sig_stripped_cli_reads() {
        let event = serde_json::from_value::<AssignmentQueryEvent>(serde_json::json!({
            "id": "1".repeat(64),
            "kind": 1,
            "pubkey": VOLUNTEER,
            "created_at": 200,
            "tags": [["e", ISSUE, "", "root"]]
        }))
        .unwrap();

        assert_eq!(event.kind, 1);
        assert_eq!(event.pubkey, VOLUNTEER);
    }

    #[test]
    fn assignment_context_queries_are_writer_consistent() {
        let queries = build_issue_assignment_queries(ISSUE, VOLUNTEER);

        assert_eq!(queries.len(), 3);
        assert!(queries
            .iter()
            .all(|query| query["writer_consistent"] == serde_json::json!(true)));
    }

    #[test]
    fn uncaused_future_self_operations_lose_to_authority() {
        let owner_unassign = "1".repeat(64);
        let state = reduce_assignment_operations(
            ISSUE,
            AUTHOR,
            OWNER,
            &[
                assignment_event(VOLUNTEER, &"2".repeat(64), true, 1_000, None),
                assignment_event(OWNER, &owner_unassign, false, 200, None),
            ],
        );
        assert!(!state.assignees.contains(VOLUNTEER));
        assert_eq!(state.heads.get(VOLUNTEER), Some(&owner_unassign));

        let owner_assign = "3".repeat(64);
        let state = reduce_assignment_operations(
            ISSUE,
            AUTHOR,
            OWNER,
            &[
                assignment_event(VOLUNTEER, &"4".repeat(64), false, 1_000, None),
                assignment_event(OWNER, &owner_assign, true, 200, None),
            ],
        );
        assert!(state.assignees.contains(VOLUNTEER));
        assert_eq!(state.heads.get(VOLUNTEER), Some(&owner_assign));
    }

    #[test]
    fn causal_self_operations_can_follow_authority() {
        let owner_assign = "5".repeat(64);
        let self_unassign = "6".repeat(64);
        let state = reduce_assignment_operations(
            ISSUE,
            AUTHOR,
            OWNER,
            &[
                assignment_event(OWNER, &owner_assign, true, 200, None),
                assignment_event(VOLUNTEER, &self_unassign, false, 300, Some(&owner_assign)),
            ],
        );
        assert!(!state.assignees.contains(VOLUNTEER));
        assert_eq!(state.heads.get(VOLUNTEER), Some(&self_unassign));

        let owner_unassign = "7".repeat(64);
        let self_assign = "8".repeat(64);
        let state = reduce_assignment_operations(
            ISSUE,
            AUTHOR,
            OWNER,
            &[
                assignment_event(OWNER, &owner_unassign, false, 200, None),
                assignment_event(VOLUNTEER, &self_assign, true, 300, Some(&owner_unassign)),
            ],
        );
        assert!(state.assignees.contains(VOLUNTEER));
        assert_eq!(state.heads.get(VOLUNTEER), Some(&self_assign));
    }

    #[test]
    fn stale_causal_self_operation_is_ignored() {
        let initial_assign = "9".repeat(64);
        let owner_unassign = "a".repeat(64);
        let state = reduce_assignment_operations(
            ISSUE,
            AUTHOR,
            OWNER,
            &[
                assignment_event(OWNER, &initial_assign, true, 100, None),
                assignment_event(OWNER, &owner_unassign, false, 200, None),
                assignment_event(VOLUNTEER, &"c".repeat(64), true, 300, Some(&initial_assign)),
            ],
        );

        assert!(!state.assignees.contains(VOLUNTEER));
        assert_eq!(state.heads.get(VOLUNTEER), Some(&owner_unassign));
    }
}
