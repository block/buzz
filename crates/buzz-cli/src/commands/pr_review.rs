//! `buzz pr comment` and `buzz pr review`: the kind:1 notes Buzz Desktop reads
//! as pull-request comments, review requests, approvals and change requests.
//!
//! Desktop only counts a review note when its signer is trusted, so these
//! commands port that trust model and refuse to publish a note that clients
//! would ignore (`projectPullRequests.mjs`).

use std::collections::HashSet;

use buzz_core::kind::{
    KIND_GIT_PR_UPDATE, KIND_GIT_PULL_REQUEST, KIND_GIT_STATUS_CLOSED, KIND_GIT_STATUS_DRAFT,
    KIND_GIT_STATUS_MERGED, KIND_GIT_STATUS_OPEN,
};
use buzz_sdk::{GitCommentMeta, GitDiffSide, GitInlineAnchor, GitReviewDecision};
use nostr::Timestamp;

use super::git_thread::{
    fetch_bounded, fetch_root, fetch_thread_notes, next_created_at, Root, WireEvent,
};
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::{read_or_stdin, sdk_err, validate_hex64};

const WHAT: &str = "pull request";

/// A diff location for an inline comment.
pub struct InlineAnchorArgs {
    pub file: String,
    pub line: u64,
    pub side: GitDiffSide,
}

/// What `buzz pr review` publishes.
pub enum ReviewAction {
    Request(Vec<String>),
    Approve,
    RequestChanges,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrStatus {
    Open,
    Merged,
    Closed,
    Draft,
}

/// Allowed actors for a root: its author and the repo owner.
fn allowed_actors(root: &Root) -> HashSet<String> {
    HashSet::from([root.author.clone(), root.repo.owner.clone()])
}

/// Tip commit: the `c` tag of the newest update signed by the author or repo
/// owner, else of the root. An update without a `c` tag leaves no tip.
async fn tip_commit(client: &BuzzClient, root: &Root) -> Result<String, CliError> {
    let allowed = allowed_actors(root);
    let repo_address = format!("30617:{}:{}", root.repo.owner, root.repo.id);
    let updates = fetch_bounded(
        client,
        serde_json::json!({"kinds": [KIND_GIT_PR_UPDATE], "#a": [repo_address]}),
    )
    .await?;
    let mut latest: Option<&WireEvent> = None;
    for update in &updates {
        if update.kind == KIND_GIT_PR_UPDATE as u16
            && allowed.contains(&update.pubkey.to_ascii_lowercase())
            && update.tag("E") == Some(root.event.id.as_str())
            && latest.is_none_or(|current| update.created_at > current.created_at)
        {
            latest = Some(update);
        }
    }
    latest
        .unwrap_or(&root.event)
        .tag("c")
        .map(str::to_string)
        .ok_or_else(|| CliError::Other("the pull request has no tip commit to review".into()))
}

async fn current_status(client: &BuzzClient, root: &Root) -> Result<PrStatus, CliError> {
    let allowed = allowed_actors(root);
    let statuses = fetch_bounded(
        client,
        serde_json::json!({
            "kinds": [
                KIND_GIT_STATUS_OPEN,
                KIND_GIT_STATUS_MERGED,
                KIND_GIT_STATUS_CLOSED,
                KIND_GIT_STATUS_DRAFT,
            ],
            "#e": [root.event.id],
        }),
    )
    .await?;
    let mut latest: Option<&WireEvent> = None;
    for status in &statuses {
        if allowed.contains(&status.pubkey.to_ascii_lowercase())
            && status.references(&root.event.id)
            && latest.is_none_or(|current| status.created_at > current.created_at)
        {
            latest = Some(status);
        }
    }
    Ok(match latest.map(|status| u32::from(status.kind)) {
        Some(KIND_GIT_STATUS_OPEN) => PrStatus::Open,
        Some(KIND_GIT_STATUS_MERGED) => PrStatus::Merged,
        Some(KIND_GIT_STATUS_CLOSED) => PrStatus::Closed,
        Some(KIND_GIT_STATUS_DRAFT) => PrStatus::Draft,
        _ if root
            .event
            .tag_values("t")
            .any(|label| label.eq_ignore_ascii_case("draft")) =>
        {
            PrStatus::Draft
        }
        _ => PrStatus::Open,
    })
}

fn has_label(note: &WireEvent, wanted: &str) -> bool {
    note.tag_values("t")
        .any(|label| label.eq_ignore_ascii_case(wanted))
}

/// Requested reviewers: `p` tags on the root plus those of review requests
/// signed by the author or repo owner. The author is never a reviewer.
fn requested_reviewers(root: &Root, notes: &[WireEvent]) -> HashSet<String> {
    let allowed = allowed_actors(root);
    let mut reviewers: HashSet<String> = root.recipients.iter().cloned().collect();
    for note in notes {
        if has_label(note, "review-request") && allowed.contains(&note.pubkey.to_ascii_lowercase())
        {
            reviewers.extend(note.tag_values("p").map(str::to_ascii_lowercase));
        }
    }
    reviewers.remove(&root.author);
    reviewers
}

fn parse_extra(values: &[String]) -> Result<Vec<String>, CliError> {
    super::messages::normalize_explicit_mentions(values)
}

fn trimmed_body(content: &str) -> Result<String, CliError> {
    Ok(read_or_stdin(content)?.trim().to_string())
}

async fn publish(client: &BuzzClient, builder: nostr::EventBuilder) -> Result<(), CliError> {
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

/// `buzz pr comment`: a comment on a pull request, inline when `anchor` is set.
pub async fn cmd_pr_comment(
    client: &BuzzClient,
    pr: &str,
    content: &str,
    mentions: &[String],
    anchor: Option<InlineAnchorArgs>,
) -> Result<(), CliError> {
    validate_hex64(pr)?;
    let body = trimmed_body(content)?;
    if body.is_empty() {
        return Err(CliError::Usage("--content must not be empty".into()));
    }
    let mentions = parse_extra(mentions)?;
    let root = fetch_root(client, KIND_GIT_PULL_REQUEST as u16, pr, WHAT).await?;
    let anchor = match anchor {
        Some(anchor) => Some(GitInlineAnchor {
            commit: tip_commit(client, &root).await?,
            path: anchor.file,
            side: anchor.side,
            line: anchor.line,
        }),
        None => None,
    };
    let meta = GitCommentMeta {
        recipients: root.comment_recipients(&mentions),
        anchor,
    };
    let builder =
        buzz_sdk::build_git_comment(&root.repo, &root.event.id, &body, &meta).map_err(sdk_err)?;
    publish(client, builder).await
}

/// `buzz pr review`: request reviewers, approve, or request changes.
pub async fn cmd_pr_review(
    client: &BuzzClient,
    pr: &str,
    action: ReviewAction,
    content: Option<&str>,
) -> Result<(), CliError> {
    validate_hex64(pr)?;
    let body = content
        .map(trimmed_body)
        .transpose()?
        .filter(|body| !body.is_empty());
    let root = fetch_root(client, KIND_GIT_PULL_REQUEST as u16, pr, WHAT).await?;
    let signer = client.keys().public_key().to_hex();

    match action {
        ReviewAction::Request(reviewers) => {
            if !allowed_actors(&root).contains(&signer) {
                return Err(CliError::Usage(
                    "only the pull request author or the repo owner can request a review; clients ignore requests from anyone else".into(),
                ));
            }
            let reviewers = parse_extra(&reviewers)?;
            if reviewers.contains(&root.author) {
                return Err(CliError::Usage(
                    "the pull request author cannot be their own reviewer".into(),
                ));
            }
            let body = match body {
                Some(body) => body,
                None => format!(
                    "Requested a review from {}",
                    crate::commands::issues::pubkey_prefix_label(&reviewers)?
                ),
            };
            let builder =
                buzz_sdk::build_git_review_request(&root.repo, &root.event.id, &reviewers, &body)
                    .map_err(sdk_err)?;
            publish(client, builder).await
        }
        ReviewAction::Approve | ReviewAction::RequestChanges => {
            let (decision, default_body) = match action {
                ReviewAction::Approve => (GitReviewDecision::Approve, "Approved these changes"),
                _ => (GitReviewDecision::RequestChanges, "Requested changes"),
            };
            if signer == root.author {
                return Err(CliError::Usage(
                    "you cannot review your own pull request".into(),
                ));
            }
            let notes = fetch_thread_notes(client, &root.event.id).await?;
            let trusted =
                signer == root.repo.owner || requested_reviewers(&root, &notes).contains(&signer);
            if !trusted {
                return Err(CliError::Usage(
                    "you are not a requested reviewer or the repo owner; clients would ignore this review. Ask the author or owner to run `buzz pr review --request <your pubkey>` first".into(),
                ));
            }
            match current_status(client, &root).await? {
                PrStatus::Open | PrStatus::Draft => {}
                PrStatus::Merged | PrStatus::Closed => {
                    return Err(CliError::Usage(
                        "the pull request is merged or closed; only open or draft pull requests take reviews".into(),
                    ));
                }
            }
            let commit = tip_commit(client, &root).await?;
            let recipients = vec![root.repo.owner.clone(), root.author.clone()];
            let created_at = next_created_at(&notes, &signer)?;
            let builder = buzz_sdk::build_git_review_decision(
                &root.repo,
                &root.event.id,
                decision,
                body.as_deref().unwrap_or(default_body),
                &recipients,
                &commit,
            )
            .map_err(sdk_err)?
            .custom_created_at(Timestamp::from_secs(created_at));
            publish(client, builder).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::issues::cmd_issue_comment;
    use axum::{body::Bytes, extract::State, routing::post, Json, Router};
    use buzz_sdk::{GitIssueMeta, GitPullRequestMeta, GitRepoCoord, GitStatus, GitStatusMeta};
    use nostr::{EventBuilder, Keys};
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    #[derive(Default)]
    struct Relay {
        events: Vec<Value>,
        published: Vec<Value>,
    }
    type Shared = Arc<Mutex<Relay>>;

    fn matches(event: &Value, filter: &Value) -> bool {
        let in_list = |key: &str, value: &Value| {
            filter
                .get(key)
                .is_none_or(|list| list.as_array().is_some_and(|l| l.contains(value)))
        };
        let tag_ok = |name: &str| {
            filter.get(format!("#{name}")).is_none_or(|wanted| {
                event["tags"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tag| tag[0] == name && wanted.as_array().unwrap().contains(&tag[1]))
            })
        };
        in_list("kinds", &event["kind"])
            && in_list("ids", &event["id"])
            && in_list("authors", &event["pubkey"])
            && ["e", "E", "a"].into_iter().all(tag_ok)
    }

    async fn setup() -> (String, Shared) {
        let relay = Shared::default();
        let app = Router::new()
            .route(
                "/query",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let filters: Vec<Value> = serde_json::from_slice(&body).unwrap();
                    let relay = relay.lock().unwrap();
                    let found: Vec<_> = relay
                        .events
                        .iter()
                        .filter(|event| filters.iter().any(|f| matches(event, f)))
                        .cloned()
                        .collect();
                    Json(json!(found))
                }),
            )
            .route(
                "/events",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let event: Value = serde_json::from_slice(&body).unwrap();
                    let mut relay = relay.lock().unwrap();
                    relay.events.push(event.clone());
                    relay.published.push(event.clone());
                    Json(json!({"event_id": event["id"], "accepted": true, "message": ""}))
                }),
            )
            .with_state(relay.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), relay)
    }

    struct World {
        url: String,
        relay: Shared,
        owner: Keys,
        author: Keys,
        reviewer: Keys,
        stranger: Keys,
        repo: GitRepoCoord,
    }

    impl World {
        async fn new() -> Self {
            let (url, relay) = setup().await;
            let owner = Keys::generate();
            let repo = GitRepoCoord {
                owner: owner.public_key().to_hex(),
                id: "demo".into(),
            };
            Self {
                url,
                relay,
                owner,
                author: Keys::generate(),
                reviewer: Keys::generate(),
                stranger: Keys::generate(),
                repo,
            }
        }

        fn client(&self, keys: &Keys) -> BuzzClient {
            BuzzClient::new(self.url.clone(), keys.clone(), None, None).unwrap()
        }

        fn seed(&self, keys: &Keys, builder: EventBuilder, created_at: u64) -> String {
            let event = builder
                .custom_created_at(Timestamp::from_secs(created_at))
                .sign_with_keys(keys)
                .unwrap();
            self.relay
                .lock()
                .unwrap()
                .events
                .push(serde_json::to_value(&event).unwrap());
            event.id.to_hex()
        }

        fn seed_pr(&self, commit: &str) -> String {
            let meta = GitPullRequestMeta {
                subject: "Add thing".into(),
                commit: commit.into(),
                clone_urls: vec!["https://relay.example/git/demo".into()],
                ..Default::default()
            };
            self.seed(
                &self.author,
                buzz_sdk::build_git_pull_request(&self.repo, "body", &meta).unwrap(),
                1_000,
            )
        }

        fn seed_update(&self, keys: &Keys, pr: &str, commit: &str, created_at: u64) {
            let meta = buzz_sdk::GitPrUpdateMeta {
                pr_event: pr.into(),
                pr_author: self.author.public_key().to_hex(),
                commit: commit.into(),
                clone_urls: vec!["https://relay.example/git/demo".into()],
                ..Default::default()
            };
            self.seed(
                keys,
                buzz_sdk::build_git_pr_update(&self.repo, "", &meta).unwrap(),
                created_at,
            );
        }

        fn seed_status(&self, keys: &Keys, pr: &str, status: GitStatus, created_at: u64) {
            let meta = GitStatusMeta {
                root_event: pr.into(),
                repo: Some(GitRepoCoord {
                    owner: self.repo.owner.clone(),
                    id: self.repo.id.clone(),
                }),
                ..Default::default()
            };
            self.seed(
                keys,
                buzz_sdk::build_git_status(status, "", &meta).unwrap(),
                created_at,
            );
        }

        fn published(&self) -> Vec<Value> {
            self.relay.lock().unwrap().published.clone()
        }
    }

    fn pk(keys: &Keys) -> String {
        keys.public_key().to_hex()
    }

    fn tags(event: &Value) -> Vec<Vec<String>> {
        serde_json::from_value(event["tags"].clone()).unwrap()
    }

    fn row(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    const C1: &str = "1111111111111111111111111111111111111111";
    const C2: &str = "2222222222222222222222222222222222222222";
    const C3: &str = "3333333333333333333333333333333333333333";

    #[tokio::test]
    async fn review_journey_follows_desktop_trust_rules() {
        let w = World::new().await;
        let pr = w.seed_pr(C1);
        let (owner, author, reviewer) = (pk(&w.owner), pk(&w.author), pk(&w.reviewer));
        let repo_tag = row(&["a", &format!("30617:{owner}:demo")]);
        let root_tag = row(&["e", &pr, "", "root"]);

        let request = |who: &Keys, targets: &[String]| {
            let client = w.client(who);
            let targets = targets.to_vec();
            let pr = pr.clone();
            async move { cmd_pr_review(&client, &pr, ReviewAction::Request(targets), None).await }
        };
        let decide = |who: &Keys, action: ReviewAction, content: Option<&'static str>| {
            let client = w.client(who);
            let pr = pr.clone();
            async move { cmd_pr_review(&client, &pr, action, content).await }
        };

        assert!(matches!(
            request(&w.stranger, std::slice::from_ref(&reviewer)).await,
            Err(CliError::Usage(_))
        ));
        assert!(matches!(
            request(&w.author, std::slice::from_ref(&author)).await,
            Err(CliError::Usage(_))
        ));
        assert!(w.published().is_empty());

        request(&w.author, std::slice::from_ref(&reviewer))
            .await
            .unwrap();
        let published = w.published();
        assert_eq!(
            tags(&published[0]),
            vec![
                root_tag.clone(),
                repo_tag.clone(),
                row(&["p", &reviewer]),
                row(&["t", "review-request"]),
            ]
        );
        assert_eq!(published[0]["pubkey"], json!(author));

        for outsider in [&w.stranger, &w.author] {
            assert!(matches!(
                decide(outsider, ReviewAction::Approve, None).await,
                Err(CliError::Usage(_))
            ));
        }
        assert_eq!(w.published().len(), 1);

        decide(&w.reviewer, ReviewAction::Approve, None)
            .await
            .unwrap();
        let approval = w.published()[1].clone();
        assert_eq!(approval["content"], json!("Approved these changes"));
        assert_eq!(
            tags(&approval),
            vec![
                root_tag.clone(),
                repo_tag.clone(),
                row(&["p", &owner]),
                row(&["p", &author]),
                row(&["t", "approval"]),
                row(&["c", C1]),
            ]
        );

        w.seed_update(&w.author, &pr, C2, 5_000);
        w.seed_update(&w.stranger, &pr, C3, 6_000);
        decide(
            &w.reviewer,
            ReviewAction::RequestChanges,
            Some("  Handle the empty case \n"),
        )
        .await
        .unwrap();
        let changes = w.published()[2].clone();
        assert_eq!(changes["content"], json!("Handle the empty case"));
        assert_eq!(
            tags(&changes)[4..],
            [row(&["t", "changes-requested"]), row(&["c", C2])]
        );
        assert!(changes["created_at"].as_u64() > approval["created_at"].as_u64());

        decide(&w.owner, ReviewAction::Approve, None).await.unwrap();
        assert_eq!(w.published()[3]["pubkey"], json!(owner));

        w.seed_status(&w.stranger, &pr, GitStatus::Closed, 7_000);
        decide(&w.reviewer, ReviewAction::Approve, None)
            .await
            .unwrap();
        w.seed_status(&w.author, &pr, GitStatus::Closed, 8_000);
        assert!(matches!(
            decide(&w.reviewer, ReviewAction::Approve, None).await,
            Err(CliError::Usage(_))
        ));
        assert_eq!(w.published().len(), 5);
    }

    #[tokio::test]
    async fn owner_who_authored_the_pr_cannot_review_it() {
        let mut w = World::new().await;
        w.repo.owner = pk(&w.author);
        let pr = w.seed_pr(C1);
        let client = w.client(&w.author);
        for action in [ReviewAction::Approve, ReviewAction::RequestChanges] {
            let result = cmd_pr_review(&client, &pr, action, Some("x")).await;
            assert!(matches!(result, Err(CliError::Usage(_))));
        }
        assert!(w.published().is_empty());
    }

    #[tokio::test]
    async fn review_clock_follows_the_signers_latest_note() {
        let w = World::new().await;
        let pr = w.seed_pr(C1);
        let future = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 1_000;
        let prior = buzz_sdk::build_git_review_decision(
            &w.repo,
            &pr,
            GitReviewDecision::RequestChanges,
            "no",
            &[],
            C1,
        )
        .unwrap();
        w.seed(&w.owner, prior, future);
        let client = w.client(&w.owner);
        cmd_pr_review(&client, &pr, ReviewAction::Approve, None)
            .await
            .unwrap();
        assert_eq!(w.published()[0]["created_at"], json!(future + 1));
    }

    #[tokio::test]
    async fn comment_journey_builds_plain_and_inline_comments() {
        let w = World::new().await;
        let pr = w.seed_pr(C1);
        w.seed_update(&w.author, &pr, C2, 5_000);
        let (owner, author) = (pk(&w.owner), pk(&w.author));
        let mention = pk(&w.stranger);
        let client = w.client(&w.reviewer);

        assert!(matches!(
            cmd_pr_comment(&client, &pr, " \n", &[], None).await,
            Err(CliError::Usage(_))
        ));
        assert!(matches!(
            cmd_pr_comment(&client, &pr, "x", &["nope".into()], None).await,
            Err(CliError::Usage(_))
        ));
        let bad_anchor = InlineAnchorArgs {
            file: "../etc/passwd".into(),
            line: 1,
            side: GitDiffSide::New,
        };
        assert!(matches!(
            cmd_pr_comment(&client, &pr, "x", &[], Some(bad_anchor)).await,
            Err(CliError::Usage(_))
        ));
        assert!(matches!(
            cmd_pr_comment(&client, &"9".repeat(64), "x", &[], None).await,
            Err(CliError::NotFound(_))
        ));
        assert!(w.published().is_empty());

        cmd_pr_comment(
            &client,
            &pr,
            " Looks good ",
            std::slice::from_ref(&mention),
            None,
        )
        .await
        .unwrap();
        let plain = w.published()[0].clone();
        assert_eq!(plain["content"], json!("Looks good"));
        assert_eq!(
            tags(&plain),
            vec![
                row(&["e", &pr, "", "root"]),
                row(&["a", &format!("30617:{owner}:demo")]),
                row(&["p", &owner]),
                row(&["p", &author]),
                row(&["p", &mention]),
            ]
        );

        let anchor = InlineAnchorArgs {
            file: "src/lib.rs".into(),
            line: 42,
            side: GitDiffSide::Old,
        };
        cmd_pr_comment(&client, &pr, "Rename", &[], Some(anchor))
            .await
            .unwrap();
        assert_eq!(
            tags(&w.published()[1])[4..],
            [
                row(&["t", "inline-comment"]),
                row(&["c", C2]),
                row(&["file", "src/lib.rs"]),
                row(&["side", "old"]),
                row(&["line", "42"]),
            ]
        );
    }

    #[tokio::test]
    async fn issue_comment_follows_desktop_shape_and_clock() {
        let w = World::new().await;
        let issue = w.seed(
            &w.author,
            buzz_sdk::build_git_issue(&w.repo, "Bug", "details", &GitIssueMeta::default()).unwrap(),
            1_000,
        );
        let (owner, author) = (pk(&w.owner), pk(&w.author));
        let future = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 1_000;
        let earlier = buzz_sdk::build_git_comment(
            &w.repo,
            &issue,
            "first",
            &buzz_sdk::GitCommentMeta::default(),
        )
        .unwrap();
        w.seed(&w.reviewer, earlier, future);

        let client = w.client(&w.reviewer);
        assert!(matches!(
            cmd_issue_comment(&client, &issue, "", &[]).await,
            Err(CliError::Usage(_))
        ));
        cmd_issue_comment(&client, &issue, "second", &[])
            .await
            .unwrap();
        let comment = w.published()[0].clone();
        assert_eq!(comment["created_at"], json!(future + 1));
        assert_eq!(
            tags(&comment),
            vec![
                row(&["e", &issue, "", "root"]),
                row(&["a", &format!("30617:{owner}:demo")]),
                row(&["p", &owner]),
                row(&["p", &author]),
            ]
        );
    }
}
