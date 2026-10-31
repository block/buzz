//! Shared reads for commenting on a git issue or pull request: the root
//! event, its repo coordinate, and the signer's comment clock.

use buzz_core::kind::KIND_TEXT_NOTE;
use buzz_sdk::GitRepoCoord;
use serde::Deserialize;

use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::validate_hex64;

/// Upper bound for every exhaustive thread read. Reaching it is an error,
/// never a silently shortened thread.
pub(crate) const THREAD_READ_BOUND: u32 = 2_000;

/// An event as returned by `/query`, which strips signatures.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WireEvent {
    pub id: String,
    pub kind: u16,
    pub pubkey: String,
    pub created_at: u64,
    pub tags: Vec<Vec<String>>,
}

impl WireEvent {
    /// First value of the first tag named `name`.
    pub fn tag<'a>(&'a self, name: &'a str) -> Option<&'a str> {
        self.tag_values(name).next()
    }

    /// Values of every non-empty tag named `name`.
    pub fn tag_values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> {
        self.tags
            .iter()
            .filter_map(move |tag| match tag.as_slice() {
                [n, value, ..] if n == name && !value.is_empty() => Some(value.as_str()),
                _ => None,
            })
    }

    /// Whether an `e` or `E` tag points at `root`.
    pub fn references(&self, root: &str) -> bool {
        self.tags.iter().any(|tag| {
            matches!(tag.as_slice(), [n, value, ..] if (n == "e" || n == "E") && value == root)
        })
    }
}

/// An issue or pull request root plus the facts every comment on it needs.
pub(crate) struct Root {
    pub event: WireEvent,
    /// Lowercase author pubkey.
    pub author: String,
    /// Repo the root belongs to, from its first `a` tag.
    pub repo: GitRepoCoord,
    /// Lowercase `p`-tag pubkeys on the root, in order.
    pub recipients: Vec<String>,
}

fn is_hex64(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn parse_repo_address(address: &str) -> Option<GitRepoCoord> {
    let mut parts = address.splitn(3, ':');
    let (Some("30617"), Some(owner), Some(id)) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    (is_hex64(owner) && crate::validate::validate_repo_id(id).is_ok()).then(|| GitRepoCoord {
        owner: owner.to_ascii_lowercase(),
        id: id.to_string(),
    })
}

/// Fetch a root event of `kind` by id. `what` names it in errors.
pub(crate) async fn fetch_root(
    client: &BuzzClient,
    kind: u16,
    id: &str,
    what: &str,
) -> Result<Root, CliError> {
    validate_hex64(id)?;
    let id = id.to_ascii_lowercase();
    let raw = client
        .query(&serde_json::json!({"kinds": [kind], "ids": [id], "limit": 1}))
        .await?;
    let events: Vec<WireEvent> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse {what} query: {e}")))?;
    let event = events
        .into_iter()
        .find(|event| event.kind == kind && event.id.eq_ignore_ascii_case(&id))
        .ok_or_else(|| CliError::NotFound(format!("{what} {id} not found")))?;
    if !is_hex64(&event.pubkey) {
        return Err(CliError::Other(format!("{what} has an invalid author")));
    }
    let repo = event
        .tag("a")
        .and_then(parse_repo_address)
        .ok_or_else(|| CliError::Other(format!("{what} has no valid repo address (`a` tag)")))?;
    let recipients = event
        .tag_values("p")
        .filter(|pk| is_hex64(pk))
        .map(str::to_ascii_lowercase)
        .collect();
    Ok(Root {
        author: event.pubkey.to_ascii_lowercase(),
        event,
        repo,
        recipients,
    })
}

impl Root {
    /// Recipients of a comment: repo owner, root author, the root's own `p`
    /// tags, then `extra`. The SDK builder drops duplicates.
    pub fn comment_recipients(&self, extra: &[String]) -> Vec<String> {
        [self.repo.owner.clone(), self.author.clone()]
            .into_iter()
            .chain(self.recipients.iter().cloned())
            .chain(extra.iter().cloned())
            .collect()
    }
}

/// Every text note under `root` (all authors), failing past the read bound.
pub(crate) async fn fetch_thread_notes(
    client: &BuzzClient,
    root: &str,
) -> Result<Vec<WireEvent>, CliError> {
    fetch_bounded(
        client,
        serde_json::json!({"kinds": [KIND_TEXT_NOTE], "#e": [root]}),
    )
    .await
}

pub(crate) async fn fetch_bounded(
    client: &BuzzClient,
    filter: serde_json::Value,
) -> Result<Vec<WireEvent>, CliError> {
    client
        .query_all_bounded(filter, THREAD_READ_BOUND)
        .await?
        .into_iter()
        .map(|event| {
            serde_json::from_value(event)
                .map_err(|e| CliError::Other(format!("failed to parse event: {e}")))
        })
        .collect()
}

/// Timestamp for a new note from `signer`: now, but after every earlier note
/// the signer left in the same thread, so desktop orders them as written.
pub(crate) fn next_created_at(notes: &[WireEvent], signer: &str) -> Result<u64, CliError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| CliError::Other(format!("read system clock: {e}")))?
        .as_secs();
    let latest = notes
        .iter()
        .filter(|note| note.pubkey.eq_ignore_ascii_case(signer))
        .map(|note| note.created_at)
        .max();
    Ok(latest.map_or(now, |latest| now.max(latest.saturating_add(1))))
}
