//! GitHub webhooks for channels that follow a repository.
//!
//! A channel member follows a repository with a `buzz.github.hook` artifact
//! whose content carries the webhook secret, NIP-44 encrypted from the
//! member's key to the relay. GitHub delivers to `/hooks/github/{hook}`; the
//! relay keeps one relay-signed `buzz.github.pull` artifact per pull request in
//! that channel, rooted on a relay-signed announcement, and posts merges,
//! closes and reviews in its thread. Every delivery acts with the hook
//! author's current authority in the channel.
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use buzz_core::{CommunityId, StoredEvent, TenantContext};
use buzz_db::artifact::ArtifactOutcome;
use buzz_db::RelayArtifactState;
use buzz_workflow::action_sink::ActionSink as _;
use hmac::{digest::KeyInit, Hmac, Mac};
use nostr::{EventBuilder, Kind, Tag};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

pub(crate) const HOOK_TYPE: &str = "buzz.github.hook";
pub(crate) const PULL_TYPE: &str = "buzz.github.pull";
const WRITE_ATTEMPTS: usize = 3;
const STATUS_MATCHES: i64 = 20;
const LOCK_WAIT: Duration = Duration::from_secs(8);

type Reply = (StatusCode, Json<Value>);

fn done(status: StatusCode, message: &str) -> Reply {
    (status, Json(json!({ "status": message })))
}

fn not_found() -> Reply {
    super::not_found("hook not found")
}

/// `POST /hooks/github/{hook}`. Until the signature verifies, every failure
/// answers like a missing hook.
pub(crate) async fn github_webhook(
    State(state): State<Arc<AppState>>,
    Path(hook): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Reply {
    let Ok(hook_id) = buzz_core::artifact::canonical_uuid(&hook) else {
        return not_found();
    };
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let Ok(tenant) = crate::tenant::bind_community(&*state, raw_host).await else {
        return not_found();
    };
    let Some(hook) = load_hook(&state, tenant.community(), hook_id).await else {
        return not_found();
    };
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_signature(&hook.secret, &body, signature) {
        tracing::warn!(hook = %hook_id, "github webhook: invalid signature");
        return not_found();
    }
    let event = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if event == "ping" {
        return done(StatusCode::OK, "pong");
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
        return super::api_error(StatusCode::BAD_REQUEST, "invalid JSON body");
    };
    let repo = str_at(&payload, "/repository/full_name").to_ascii_lowercase();
    if repo != hook.repo {
        return done(StatusCode::ACCEPTED, "repository not followed by this hook");
    }
    match author_may_post(&state, tenant.community(), hook.channel, &hook.author).await {
        Ok(true) => {}
        Ok(false) => {
            return super::api_error(
                StatusCode::FORBIDDEN,
                "the hook's author can no longer post in its channel",
            )
        }
        Err(e) => {
            tracing::error!(hook = %hook_id, "github webhook: authority lookup failed: {e}");
            return super::api_error(StatusCode::SERVICE_UNAVAILABLE, "try again later");
        }
    }
    let writer = Writer {
        state: &state,
        tenant: &tenant,
        hook: &hook,
    };
    let result = match event {
        "pull_request" => writer.pull_request(&payload).await,
        "pull_request_review" => writer.review(&payload).await,
        "check_suite" => {
            let key = str_at(&payload, "/check_suite/app/slug");
            writer
                .check(
                    &payload,
                    "/check_suite",
                    if key.is_empty() { "checks" } else { key },
                )
                .await
        }
        "check_run" => {
            writer
                .check(&payload, "/check_run", str_at(&payload, "/check_run/name"))
                .await
        }
        "status" => writer.status(&payload).await,
        _ => return done(StatusCode::ACCEPTED, "event ignored"),
    };
    match result {
        Ok(()) => done(StatusCode::OK, "ok"),
        Err(e) => {
            tracing::error!(hook = %hook_id, event, "github webhook: {e}");
            super::api_error(StatusCode::SERVICE_UNAVAILABLE, "try again later")
        }
    }
}

struct Hook {
    id: Uuid,
    channel: Uuid,
    author: nostr::PublicKey,
    repo: String,
    secret: String,
}

#[derive(Deserialize)]
struct HookContent {
    repo: String,
    secret: String,
}

async fn load_hook(state: &AppState, community: CommunityId, id: Uuid) -> Option<Hook> {
    let (head, deleted) = state.db.relay_artifact_head(community, id).await.ok()??;
    if deleted || tag_value(&head, "type") != Some(HOOK_TYPE) {
        return None;
    }
    let content: HookContent = serde_json::from_str(&head.event.content).ok()?;
    let secret = nostr::nips::nip44::decrypt(
        state.relay_keypair.secret_key(),
        &head.event.pubkey,
        &content.secret,
    )
    .ok()?;
    if secret.is_empty() {
        return None;
    }
    Some(Hook {
        id,
        channel: head.channel_id?,
        author: head.event.pubkey,
        repo: content.repo.to_ascii_lowercase(),
        secret,
    })
}

fn verify_signature(secret: &str, body: &[u8], header: &str) -> bool {
    let Some(signature) = header
        .strip_prefix("sha256=")
        .and_then(|hex_sig| hex::decode(hex_sig).ok())
    else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&signature).is_ok()
}

async fn author_may_post(
    state: &AppState,
    community: CommunityId,
    channel: Uuid,
    author: &nostr::PublicKey,
) -> Result<bool, buzz_db::DbError> {
    let pubkey = author.to_bytes().to_vec();
    let record = match state
        .db
        .get_channel_for_event_write(community, channel)
        .await
    {
        Ok(record) => record,
        Err(buzz_db::DbError::ChannelNotFound(_) | buzz_db::DbError::NotFound(_)) => {
            return Ok(false)
        }
        Err(e) => return Err(e),
    };
    if record.archived_at.is_some() {
        return Ok(false);
    }
    let restriction = state.restriction_state_cached(community, &pubkey).await?;
    if restriction.banned
        || restriction
            .muted_until
            .is_some_and(|until| until > chrono::Utc::now())
    {
        return Ok(false);
    }
    let is_member = state.is_member_cached(community, channel, &pubkey).await?;
    crate::handlers::ingest::may_post(state, community, channel, &pubkey, &record, is_member).await
}

fn str_at<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value.pointer(pointer).and_then(Value::as_str).unwrap_or("")
}

fn tag_value<'a>(stored: &'a StoredEvent, name: &str) -> Option<&'a str> {
    stored
        .event
        .tags
        .iter()
        .map(|t| t.as_slice())
        .find(|t| t.len() >= 2 && t[0] == name)
        .map(|t| t[1].as_str())
}

/// A pull request as the card shows it: `buzz.github.pull` content v1.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub(crate) struct Pull {
    pub(crate) version: u8,
    pub(crate) url: String,
    pub(crate) repo: String,
    pub(crate) number: u64,
    pub(crate) title: String,
    pub(crate) author: String,
    pub(crate) state: String,
    pub(crate) base: String,
    pub(crate) head: String,
    pub(crate) head_sha: String,
    pub(crate) updated_at: String,
    pub(crate) additions: u64,
    pub(crate) deletions: u64,
    pub(crate) changed_files: u64,
    pub(crate) checks: Checks,
    pub(crate) reviews: BTreeMap<String, String>,
    pub(crate) requested_reviewers: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub(crate) struct Checks {
    pub(crate) state: String,
    pub(crate) runs: BTreeMap<String, String>,
}

fn pull_state(pr: &Value) -> &'static str {
    if pr.get("merged").and_then(Value::as_bool) == Some(true)
        || pr.get("merged_at").is_some_and(|v| !v.is_null())
    {
        "merged"
    } else if str_at(pr, "/state") == "closed" {
        "closed"
    } else if pr.get("draft").and_then(Value::as_bool) == Some(true) {
        "draft"
    } else {
        "open"
    }
}

/// Fold a `pull_request` payload's PR into `prev`. A payload older than the
/// stored one keeps the stored fields, so out-of-order deliveries never
/// regress the card; a new head SHA starts its checks over.
fn apply_pull_request(prev: Option<&Pull>, pr: &Value, repo: &str) -> Pull {
    let updated_at = str_at(pr, "/updated_at");
    if let Some(prev) =
        prev.filter(|p| !p.updated_at.is_empty() && p.updated_at.as_str() > updated_at)
    {
        return prev.clone();
    }
    let mut next = prev.cloned().unwrap_or_default();
    let head_sha = str_at(pr, "/head/sha").to_owned();
    if next.head_sha != head_sha {
        next.checks = Checks::default();
    }
    let count = |field: &str| pr.get(field).and_then(Value::as_u64).unwrap_or(0);
    let mut requested: Vec<String> = pr
        .get("requested_reviewers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| r.get("login").and_then(Value::as_str).map(str::to_owned))
        .collect();
    requested.extend(
        pr.get("requested_teams")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|t| t.get("slug").and_then(Value::as_str))
            .map(|slug| format!("team:{slug}")),
    );
    next = Pull {
        version: 1,
        url: str_at(pr, "/html_url").to_owned(),
        repo: repo.to_owned(),
        number: pr.get("number").and_then(Value::as_u64).unwrap_or(0),
        title: str_at(pr, "/title").to_owned(),
        author: str_at(pr, "/user/login").to_owned(),
        state: pull_state(pr).to_owned(),
        base: str_at(pr, "/base/ref").to_owned(),
        head: str_at(pr, "/head/ref").to_owned(),
        head_sha,
        updated_at: updated_at.to_owned(),
        additions: count("additions"),
        deletions: count("deletions"),
        changed_files: count("changed_files"),
        requested_reviewers: requested,
        ..next
    };
    next.checks.state = checks_state(&next.checks.runs).to_owned();
    next
}

/// GitHub's own rule: a reviewer's latest approval or change request stands
/// until they submit another or it is dismissed; comments don't replace it.
fn apply_review(pull: &mut Pull, login: &str, action: &str, review_state: &str) -> bool {
    let next = if action == "dismissed" {
        "dismissed".to_owned()
    } else {
        review_state.to_ascii_lowercase()
    };
    let current = pull.reviews.get(login).map(String::as_str);
    if next == "commented" && matches!(current, Some("approved" | "changes_requested")) {
        return false;
    }
    if current == Some(next.as_str()) {
        return false;
    }
    pull.reviews.insert(login.to_owned(), next);
    pull.requested_reviewers.retain(|r| r != login);
    true
}

fn apply_check(pull: &mut Pull, sha: &str, key: &str, result: &str) -> bool {
    if key.is_empty() || pull.head_sha != sha {
        return false;
    }
    if pull.checks.runs.get(key).map(String::as_str) == Some(result) {
        return false;
    }
    pull.checks.runs.insert(key.to_owned(), result.to_owned());
    pull.checks.state = checks_state(&pull.checks.runs).to_owned();
    true
}

fn checks_state(runs: &BTreeMap<String, String>) -> &'static str {
    if runs.is_empty() {
        return "none";
    }
    let mut pending = false;
    for result in runs.values() {
        match result.as_str() {
            "failure" | "error" | "cancelled" | "timed_out" | "action_required"
            | "startup_failure" => return "failure",
            "success" | "neutral" | "skipped" => {}
            _ => pending = true,
        }
    }
    if pending {
        "pending"
    } else {
        "success"
    }
}

/// What a check delivery reports: the conclusion once completed, otherwise
/// that the run is still pending.
fn check_result(run: &Value) -> String {
    match str_at(run, "/status") {
        "completed" => {
            let conclusion = str_at(run, "/conclusion");
            if conclusion.is_empty() {
                "neutral".to_owned()
            } else {
                conclusion.to_owned()
            }
        }
        _ => "pending".to_owned(),
    }
}

/// A login or title shown in relay text, unable to resolve as a mention.
fn plain(text: &str) -> String {
    text.replace('@', "@\u{2060}")
}

fn transition_reply(prev: &str, next: &str, actor: &str) -> Option<String> {
    let actor = plain(actor);
    match (prev, next) {
        (p, "merged") if p != "merged" => Some(format!("Merged by {actor}")),
        (p, "closed") if p != "closed" && p != "merged" => Some(format!("Closed by {actor}")),
        ("closed", "open" | "draft") => Some(format!("Reopened by {actor}")),
        ("draft", "open") => Some("Ready for review".to_owned()),
        _ => None,
    }
}

fn review_reply(login: &str, state: &str) -> Option<String> {
    let login = plain(login);
    match state {
        "approved" => Some(format!("{login} approved")),
        "changes_requested" => Some(format!("{login} requested changes")),
        _ => None,
    }
}

fn truncate(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

struct Writer<'a> {
    state: &'a Arc<AppState>,
    tenant: &'a TenantContext,
    hook: &'a Hook,
}

/// The relay's own card for one pull request, whatever members did to the
/// artifact since.
enum Loaded {
    Missing,
    /// A moderator redacted it, or the relay never wrote it: leave it alone.
    Retired,
    Card(Box<Card>),
}

struct Card {
    head: RelayArtifactState,
    pull: Pull,
    root: Option<String>,
}

/// A stored change: the pull request before it and the thread to reply in.
struct Changed {
    prev: Option<Pull>,
    root: String,
}

impl Writer<'_> {
    /// Relay-derived, so no one else can claim a pull request's identity first.
    fn pull_id(&self, url: &str) -> Uuid {
        let mut key = Sha256::new();
        key.update(b"buzz-github-pull-v1\0");
        key.update(self.state.relay_keypair.secret_key().as_secret_bytes());
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&key.finalize()).expect("HMAC accepts any key length");
        mac.update(self.tenant.community().as_uuid().as_bytes());
        mac.update(b"\0");
        mac.update(self.hook.id.as_bytes());
        mac.update(b"\0");
        mac.update(url.as_bytes());
        let digest = mac.finalize().into_bytes();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        uuid::Builder::from_custom_bytes(bytes).into_uuid()
    }

    /// One delivery at a time per pull request, across relay instances.
    async fn lock(&self, id: Uuid) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, String> {
        let deadline = tokio::time::Instant::now() + LOCK_WAIT;
        loop {
            let lock = self
                .state
                .db
                .try_relay_artifact_lock(self.tenant.community(), id)
                .await
                .map_err(|e| format!("artifact lock: {e}"))?;
            if let Some(lock) = lock {
                return Ok(lock);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("pull request stayed locked".into());
            }
            let jitter = rand::random::<u64>() % 100;
            tokio::time::sleep(Duration::from_millis(25 + jitter)).await;
        }
    }

    async fn load(&self, id: Uuid) -> Result<Loaded, String> {
        let head = self
            .state
            .db
            .relay_artifact_state(
                self.tenant.community(),
                id,
                self.state.relay_keypair.public_key().as_bytes(),
            )
            .await
            .map_err(|e| format!("artifact read: {e}"))?;
        let Some(head) = head else {
            return Ok(Loaded::Missing);
        };
        let Some(own) = head.own.as_ref().filter(|_| !head.redacted) else {
            return Ok(Loaded::Retired);
        };
        let Ok(pull) = serde_json::from_str::<Pull>(&own.event.content) else {
            return Ok(Loaded::Retired);
        };
        let root = tag_value(own, "root").map(str::to_owned);
        Ok(Loaded::Card(Box::new(Card { head, pull, root })))
    }

    /// The head is no longer the relay's latest revision in the hook's channel.
    fn tampered(&self, head: &RelayArtifactState) -> bool {
        !head.authored
            || head.deleted
            || head.channel != self.hook.channel
            || head
                .own
                .as_ref()
                .map(|own| own.event.id.as_bytes().as_slice())
                != Some(head.head.as_slice())
    }

    async fn post(&self, text: &str, reply_to: Option<&str>) -> Result<String, String> {
        crate::workflow_sink::RelayActionSink::new(self.state)
            .send_message(
                self.tenant.community(),
                &self.hook.channel.to_string(),
                text,
                "",
                &self.hook.author.to_hex(),
                reply_to,
            )
            .await
            .map_err(|e| format!("post message: {e}"))
    }

    /// Store `pull` as the next revision after `head`, restoring or moving it
    /// back when a member deleted or moved it. `Ok(false)` means nothing new
    /// was stored and the caller should read again.
    async fn write(
        &self,
        id: Uuid,
        head: Option<&RelayArtifactState>,
        root: Option<&str>,
        pull: &Pull,
    ) -> Result<bool, String> {
        let id_text = id.to_string();
        let channel = self.hook.channel.to_string();
        let hook = self.hook.id.to_string();
        let title = format!("#{} · {}", pull.number, pull.title);
        let prev = head.map(|h| hex::encode(&h.head));
        let (op, source) = match head {
            None => ("create", None),
            Some(h) if h.deleted => ("restore", None),
            Some(h) if h.channel != self.hook.channel => ("move", Some(h.channel)),
            Some(_) => ("update", None),
        };
        let mut tags = vec![
            vec!["ar", "1"],
            vec!["d", &id_text],
            vec!["h", &channel],
            vec!["type", PULL_TYPE],
            vec!["title", truncate(&title, 512)],
            vec!["op", op],
            vec!["r", &pull.url],
            vec!["repo", &pull.repo],
            vec!["hook", &hook],
            vec!["sha", &pull.head_sha],
            vec!["state", &pull.state],
        ];
        if let Some(root) = root {
            tags.push(vec!["root", root]);
        }
        if let Some(prev) = &prev {
            tags.push(vec!["prev", prev]);
        }
        let tags = tags
            .into_iter()
            .map(Tag::parse)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("artifact tag: {e}"))?;
        let content = serde_json::to_string(pull).map_err(|e| e.to_string())?;
        let event = EventBuilder::new(Kind::Custom(45010), content)
            .tags(tags)
            .sign_with_keys(&self.state.relay_keypair)
            .map_err(|e| format!("artifact signing: {e}"))?;
        let env = buzz_core::artifact::validate(&event).map_err(|e| format!("artifact: {e}"))?;
        let outcome = self
            .state
            .db
            .accept_artifact(
                self.tenant.community(),
                &event,
                &env,
                source,
                &self.state.relay_keypair,
            )
            .await
            .map_err(|e| format!("artifact storage: {e}"))?;
        match outcome {
            ArtifactOutcome::Accepted(events) => {
                for stored in &events {
                    crate::handlers::artifact::publish(self.state, self.tenant, stored).await;
                }
                Ok(true)
            }
            ArtifactOutcome::Duplicate | ArtifactOutcome::Conflict(_) => Ok(false),
            ArtifactOutcome::Rejected(reason) => Err(format!("artifact rejected: {reason}")),
        }
    }

    /// Under the pull request's lock, store what `change` makes of the card,
    /// repairing member edits on the way. `change` answers `None` when it
    /// changes nothing. `create` lets a delivery start the card and its
    /// announcement.
    async fn change(
        &self,
        url: &str,
        create: bool,
        change: impl Fn(Option<&Pull>) -> Option<Pull>,
    ) -> Result<Option<Changed>, String> {
        let id = self.pull_id(url);
        let _lock = self.lock(id).await?;
        for _ in 0..WRITE_ATTEMPTS {
            let (head, prev, root) = match self.load(id).await? {
                Loaded::Retired => return Ok(None),
                Loaded::Missing if !create => return Ok(None),
                Loaded::Missing => (None, None, None),
                Loaded::Card(card) => (Some(card.head), Some(card.pull), card.root),
            };
            let next = change(prev.as_ref());
            let changed = next.is_some();
            let repair = head.as_ref().is_some_and(|h| self.tampered(h));
            let Some(pull) = next.or_else(|| prev.clone()) else {
                return Ok(None);
            };
            if (changed || repair)
                && !self
                    .write(id, head.as_ref(), root.as_deref(), &pull)
                    .await?
            {
                continue;
            }
            let root = match root {
                Some(root) => root,
                // The card exists before its announcement, so a lost race or a
                // failed write never leaves an orphan message.
                None => self.announce(id, &pull).await?,
            };
            return Ok(changed.then_some(Changed { prev, root }));
        }
        Err("artifact head kept changing".into())
    }

    async fn announce(&self, id: Uuid, pull: &Pull) -> Result<String, String> {
        let text = format!("#{} · {}\n{}", pull.number, plain(&pull.title), pull.url);
        let root = self.post(&text, None).await?;
        for _ in 0..WRITE_ATTEMPTS {
            let Loaded::Card(card) = self.load(id).await? else {
                return Ok(root);
            };
            if self
                .write(id, Some(&card.head), Some(&root), &card.pull)
                .await?
            {
                return Ok(root);
            }
        }
        Err("artifact head kept changing".into())
    }

    async fn pull_request(&self, payload: &Value) -> Result<(), String> {
        let Some(pr) = payload.get("pull_request") else {
            return Ok(());
        };
        let url = str_at(pr, "/html_url");
        if url.is_empty() {
            return Ok(());
        }
        let action = str_at(payload, "/action");
        let changed = self
            .change(url, action != "closed", |prev| {
                let next = apply_pull_request(prev, pr, &self.hook.repo);
                (prev != Some(&next)).then_some(next)
            })
            .await?;
        let Some(Changed {
            prev: Some(prev),
            root,
        }) = changed
        else {
            return Ok(());
        };
        let state = pull_state(pr);
        let merged_by = str_at(pr, "/merged_by/login");
        let actor = if state == "merged" && !merged_by.is_empty() {
            merged_by
        } else {
            str_at(payload, "/sender/login")
        };
        if let Some(message) = transition_reply(&prev.state, state, actor) {
            self.post(&message, Some(&root)).await?;
        }
        Ok(())
    }

    async fn review(&self, payload: &Value) -> Result<(), String> {
        let url = str_at(payload, "/pull_request/html_url");
        let login = str_at(payload, "/review/user/login");
        if url.is_empty() || login.is_empty() {
            return Ok(());
        }
        let action = str_at(payload, "/action");
        let review_state = str_at(payload, "/review/state").to_ascii_lowercase();
        let changed = self
            .change(url, false, |prev| {
                let mut pull = prev?.clone();
                apply_review(&mut pull, login, action, &review_state).then_some(pull)
            })
            .await?;
        if let (Some(changed), Some(message)) = (changed, review_reply(login, &review_state)) {
            if action == "submitted" {
                self.post(&message, Some(&changed.root)).await?;
            }
        }
        Ok(())
    }

    async fn check(&self, payload: &Value, run_at: &str, key: &str) -> Result<(), String> {
        let Some(run) = payload.pointer(run_at) else {
            return Ok(());
        };
        let sha = str_at(run, "/head_sha");
        let base = str_at(payload, "/repository/html_url");
        let mut urls: Vec<String> = run
            .get("pull_requests")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|pr| pr.get("number").and_then(Value::as_u64))
            .map(|number| format!("{base}/pull/{number}"))
            .collect();
        // GitHub omits pull requests from forks.
        if urls.is_empty() {
            urls = self.urls_at(sha).await?;
        }
        self.record_check(&urls, sha, key, &check_result(run)).await
    }

    async fn status(&self, payload: &Value) -> Result<(), String> {
        let sha = str_at(payload, "/sha");
        let context = str_at(payload, "/context");
        let result = match str_at(payload, "/state") {
            "success" => "success",
            "failure" => "failure",
            "error" => "error",
            _ => "pending",
        };
        let urls = self.urls_at(sha).await?;
        self.record_check(&urls, sha, context, result).await
    }

    async fn record_check(
        &self,
        urls: &[String],
        sha: &str,
        key: &str,
        result: &str,
    ) -> Result<(), String> {
        if sha.is_empty() || key.is_empty() {
            return Ok(());
        }
        for url in urls {
            self.change(url, false, |prev| {
                let mut pull = prev?.clone();
                apply_check(&mut pull, sha, key, result).then_some(pull)
            })
            .await?;
        }
        Ok(())
    }

    /// This hook's cards whose head commit is `sha`.
    async fn urls_at(&self, sha: &str) -> Result<Vec<String>, String> {
        if sha.is_empty() {
            return Ok(Vec::new());
        }
        let hook = self.hook.id.to_string();
        let heads = self
            .state
            .db
            .relay_artifact_heads_tagged(
                self.tenant.community(),
                self.hook.channel,
                PULL_TYPE,
                self.state.relay_keypair.public_key().as_bytes(),
                &[("sha", sha), ("hook", &hook)],
                STATUS_MATCHES,
            )
            .await
            .map_err(|e| format!("artifact read: {e}"))?;
        Ok(heads
            .iter()
            .filter_map(|head| tag_value(head, "r").map(str::to_owned))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(updated_at: &str, sha: &str) -> Value {
        json!({
            "html_url": "https://github.com/o/r/pull/7",
            "number": 7,
            "title": "Fix @alice's bug",
            "user": {"login": "bob"},
            "state": "open",
            "draft": false,
            "merged": false,
            "base": {"ref": "main"},
            "head": {"ref": "fix", "sha": sha},
            "updated_at": updated_at,
            "additions": 3,
            "deletions": 1,
            "changed_files": 2,
            "requested_reviewers": [{"login": "carol"}],
            "requested_teams": [{"slug": "docs"}]
        })
    }

    #[test]
    fn signature_verifies_only_the_exact_body_and_secret() {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"s3cret").unwrap();
        mac.update(b"{}");
        let header = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(verify_signature("s3cret", b"{}", &header));
        assert!(!verify_signature("other", b"{}", &header));
        assert!(!verify_signature("s3cret", b"{ }", &header));
        assert!(!verify_signature(
            "s3cret",
            b"{}",
            header.trim_start_matches("sha256=")
        ));
        assert!(!verify_signature("s3cret", b"{}", "sha256=zz"));
    }

    #[test]
    fn pull_request_fields_come_from_the_payload() {
        let pull = apply_pull_request(None, &pr("2026-11-04T10:00:00Z", "a1"), "o/r");
        assert_eq!(pull.version, 1);
        assert_eq!(pull.url, "https://github.com/o/r/pull/7");
        assert_eq!((pull.number, pull.state.as_str()), (7, "open"));
        assert_eq!(pull.requested_reviewers, vec!["carol", "team:docs"]);
        assert_eq!(pull.checks.state, "none");
        assert_eq!(
            (pull.additions, pull.deletions, pull.changed_files),
            (3, 1, 2)
        );
    }

    #[test]
    fn pull_state_prefers_merged_then_closed_then_draft() {
        let mut payload = pr("t", "a");
        payload["draft"] = json!(true);
        assert_eq!(pull_state(&payload), "draft");
        payload["state"] = json!("closed");
        assert_eq!(pull_state(&payload), "closed");
        payload["merged_at"] = json!("2026-11-04T11:00:00Z");
        assert_eq!(pull_state(&payload), "merged");
    }

    #[test]
    fn an_older_delivery_does_not_regress_the_card() {
        let newer = apply_pull_request(None, &pr("2026-11-04T11:00:00Z", "a1"), "o/r");
        let mut older = pr("2026-11-04T10:00:00Z", "a1");
        older["title"] = json!("Old title");
        assert_eq!(apply_pull_request(Some(&newer), &older, "o/r"), newer);
    }

    #[test]
    fn a_new_head_starts_checks_over_and_keeps_reviews() {
        let mut pull = apply_pull_request(None, &pr("2026-11-04T10:00:00Z", "a1"), "o/r");
        assert!(apply_check(&mut pull, "a1", "ci", "success"));
        assert!(apply_review(&mut pull, "carol", "submitted", "APPROVED"));
        let next = apply_pull_request(Some(&pull), &pr("2026-11-04T11:00:00Z", "b2"), "o/r");
        assert_eq!(
            next.checks,
            Checks {
                state: "none".into(),
                runs: BTreeMap::new()
            }
        );
        assert_eq!(
            next.reviews.get("carol").map(String::as_str),
            Some("approved")
        );
    }

    #[test]
    fn checks_only_count_for_the_current_head() {
        let mut pull = apply_pull_request(None, &pr("t", "a1"), "o/r");
        assert!(!apply_check(&mut pull, "old", "ci", "failure"));
        assert!(apply_check(&mut pull, "a1", "ci", "pending"));
        assert_eq!(pull.checks.state, "pending");
        assert!(apply_check(&mut pull, "a1", "lint", "failure"));
        assert_eq!(pull.checks.state, "failure");
        assert!(apply_check(&mut pull, "a1", "lint", "success"));
        assert!(apply_check(&mut pull, "a1", "ci", "success"));
        assert_eq!(pull.checks.state, "success");
        assert!(!apply_check(&mut pull, "a1", "ci", "success"));
    }

    #[test]
    fn check_results_read_status_then_conclusion() {
        assert_eq!(check_result(&json!({"status": "in_progress"})), "pending");
        assert_eq!(
            check_result(&json!({"status": "completed", "conclusion": "timed_out"})),
            "timed_out"
        );
        assert_eq!(check_result(&json!({"status": "completed"})), "neutral");
    }

    #[test]
    fn a_comment_does_not_replace_an_approval_but_a_dismissal_does() {
        let mut pull = Pull {
            requested_reviewers: vec!["carol".into()],
            ..Pull::default()
        };
        assert!(apply_review(&mut pull, "carol", "submitted", "APPROVED"));
        assert!(pull.requested_reviewers.is_empty());
        assert!(!apply_review(&mut pull, "carol", "submitted", "COMMENTED"));
        assert!(!apply_review(&mut pull, "carol", "submitted", "approved"));
        assert!(apply_review(&mut pull, "carol", "dismissed", "dismissed"));
        assert_eq!(
            pull.reviews.get("carol").map(String::as_str),
            Some("dismissed")
        );
    }

    #[test]
    fn replies_name_each_transition_once() {
        assert_eq!(
            transition_reply("open", "merged", "dan").as_deref(),
            Some("Merged by dan")
        );
        assert_eq!(transition_reply("merged", "merged", "dan"), None);
        assert_eq!(
            transition_reply("draft", "closed", "dan").as_deref(),
            Some("Closed by dan")
        );
        assert_eq!(transition_reply("merged", "closed", "dan"), None);
        assert_eq!(
            transition_reply("closed", "open", "dan").as_deref(),
            Some("Reopened by dan")
        );
        assert_eq!(
            transition_reply("draft", "open", "dan").as_deref(),
            Some("Ready for review")
        );
        assert_eq!(transition_reply("open", "draft", "dan"), None);
    }

    #[test]
    fn relay_text_cannot_mention() {
        assert_eq!(plain("@alice"), "@\u{2060}alice");
        assert_eq!(
            review_reply("@x", "approved").as_deref(),
            Some("@\u{2060}x approved")
        );
        assert_eq!(review_reply("x", "commented"), None);
    }

    #[test]
    fn titles_truncate_on_a_char_boundary() {
        assert_eq!(truncate("héllo", 2), "h");
        assert_eq!(truncate("hello", 9), "hello");
    }
}

#[cfg(test)]
#[path = "github_hooks_postgres_tests.rs"]
mod postgres_tests;
