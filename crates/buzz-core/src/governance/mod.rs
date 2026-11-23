//! Signed, whole-document governance commands and their strict wire schema.

mod document;
mod evidence;
pub use document::*;
pub use evidence::{Evidence, EvidenceBody};

use crate::kind::KIND_GOVERNANCE_COMMAND;
use nostr::{Event, EventBuilder, Kind, Tag};
use std::str::FromStr;
use uuid::Uuid;

/// Maximum signed policy content size, before parsing.
pub const MAX_DOCUMENT_BYTES: usize = 65_536;

/// A stable NIP-34 repository coordinate, independent of its channel binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoCoordinate {
    /// Original announcement author, lowercase hexadecimal.
    pub owner: String,
    /// Repository identifier.
    pub name: String,
}

impl FromStr for RepoCoordinate {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, String> {
        let mut parts = value.split(':');
        let kind = parts.next();
        let owner = parts.next().unwrap_or_default();
        let name = parts.next().unwrap_or_default();
        if kind != Some("30617")
            || !hex_key(owner)
            || name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || name.starts_with('.')
            || name.contains("..")
            || parts.next().is_some()
        {
            return Err("invalid repository coordinate".into());
        }
        Ok(Self {
            owner: owner.into(),
            name: name.into(),
        })
    }
}

impl std::fmt::Display for RepoCoordinate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "30617:{}:{}", self.owner, self.name)
    }
}

/// A document's stable address within a community.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentId {
    /// Community-wide requirements and label schema.
    Floor,
    /// Community's trusted check producers.
    Trusted,
    /// Accepted protected attributes.
    Labels(RepoCoordinate),
    /// Additional repository constraints.
    Rules(RepoCoordinate),
    /// Repository channel binding.
    Bind(RepoCoordinate),
}

impl DocumentId {
    /// Repository affected, if this is not a community document.
    pub fn repo(&self) -> Option<&RepoCoordinate> {
        match self {
            Self::Floor | Self::Trusted => None,
            Self::Labels(r) | Self::Rules(r) | Self::Bind(r) => Some(r),
        }
    }
}
impl FromStr for DocumentId {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, String> {
        match value {
            "floor" => return Ok(Self::Floor),
            "trusted" => return Ok(Self::Trusted),
            _ => {}
        }
        let (kind, rest) = value.split_once(':').ok_or("invalid document id")?;
        let repo = rest.parse()?;
        match kind {
            "labels" => Ok(Self::Labels(repo)),
            "rules" => Ok(Self::Rules(repo)),
            "bind" => Ok(Self::Bind(repo)),
            _ => Err("unsupported governance document".into()),
        }
    }
}
impl std::fmt::Display for DocumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Floor => f.write_str("floor"),
            Self::Trusted => f.write_str("trusted"),
            Self::Labels(r) => write!(f, "labels:{r}"),
            Self::Rules(r) => write!(f, "rules:{r}"),
            Self::Bind(r) => write!(f, "bind:{r}"),
        }
    }
}

/// The Git authority an owner signed for an agent in a NIP-OA Git grant: one
/// repository or all of them, up to one level, within an optional window.
/// Its fields are public: callers build one only from
/// `buzz_sdk::nip_oa::verify_git_grant`, a convention the type does not enforce.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitGrant {
    /// The owner who signed it.
    pub owner: nostr::PublicKey,
    /// The repository, or `None` for every repository (`repo=*`).
    pub repo: Option<RepoCoordinate>,
    /// The highest action allowed; each level includes the ones below it.
    pub level: GrantAction,
    /// `created_at>` bound, if signed.
    pub after: Option<u64>,
    /// `created_at<` bound, if signed.
    pub before: Option<u64>,
}

impl GitGrant {
    /// Whether this grant covers `action` on `repo`.
    pub fn allows(&self, repo: &RepoCoordinate, action: GrantAction) -> bool {
        self.repo.as_ref().is_none_or(|r| r == repo) && action <= self.level
    }

    /// The signed window `before - after` in seconds, when both are bounded
    /// and it is not empty.
    pub fn window(&self) -> Option<u64> {
        self.before?.checked_sub(self.after?).filter(|w| *w > 0)
    }
}

/// The exact accepted head a command may replace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpectedHead {
    /// Create only, never recreate a deleted initialized head.
    Absent,
    /// Replace this exact signed event.
    Event(String),
}
impl FromStr for ExpectedHead {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        if s == "absent" {
            Ok(Self::Absent)
        } else if hex_key(s) {
            Ok(Self::Event(s.into()))
        } else {
            Err("prev must be absent or a lowercase event ID".into())
        }
    }
}
impl std::fmt::Display for ExpectedHead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Absent => f.write_str("absent"),
            Self::Event(id) => f.write_str(id),
        }
    }
}

/// Parsed command; the signed event remains the audit source of truth.
#[derive(Clone, Debug)]
pub struct Command {
    /// Community explicitly bound by the signature (never tenant selection).
    pub community: Uuid,
    /// Target document.
    pub document: DocumentId,
    /// Compare-and-swap precondition.
    pub expected: ExpectedHead,
    /// Strict typed whole document.
    pub body: Document,
}
impl Command {
    /// Verify and parse a signed command, without assuming authority or freshness.
    pub fn parse(event: &Event) -> Result<Self, String> {
        if event.kind.as_u16() as u32 != KIND_GOVERNANCE_COMMAND {
            return Err("wrong command kind".into());
        }
        event.verify().map_err(|_| "invalid command signature")?;
        let community = singleton(event, "community")?
            .parse()
            .map_err(|_| "invalid community UUID")?;
        let document: DocumentId = singleton(event, "d")?.parse()?;
        let expected = singleton(event, "prev")?.parse()?;
        for tag in event.tags.iter() {
            let fields = tag.as_slice();
            let name = fields.first().map(String::as_str).unwrap_or_default();
            if !matches!(name, "community" | "d" | "prev" | "auth") {
                return Err(format!("unsupported command tag: {name}"));
            }
        }
        let body = Document::parse(&event.content, &document)?;
        Ok(Self {
            community,
            document,
            expected,
            body,
        })
    }
}

/// Build a command that the caller subsequently signs with its own identity.
pub fn command_builder(
    community: Uuid,
    document: &DocumentId,
    expected: &ExpectedHead,
    content: &str,
) -> Result<EventBuilder, String> {
    Document::parse(content, document)?;
    let tags = [
        ("community", community.to_string()),
        ("d", document.to_string()),
        ("prev", expected.to_string()),
    ]
    .into_iter()
    .map(|(name, value)| Tag::parse([name.to_string(), value]).map_err(|e| e.to_string()))
    .collect::<Result<Vec<_>, _>>()?;
    Ok(EventBuilder::new(Kind::Custom(KIND_GOVERNANCE_COMMAND as u16), content).tags(tags))
}

fn singleton<'a>(event: &'a Event, name: &str) -> Result<&'a str, String> {
    let mut tags = event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().is_some_and(|v| v == name));
    let tag = tags.next().ok_or_else(|| format!("missing {name}"))?;
    if tags.next().is_some() || tag.as_slice().len() != 2 {
        return Err(format!("duplicate or malformed {name}"));
    }
    Ok(&tag.as_slice()[1])
}

/// Whether a value is a canonical 32-byte lowercase hexadecimal key or ID.
pub fn hex_key(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests;
