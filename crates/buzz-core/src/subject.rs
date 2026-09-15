//! Conversation subjects: domain state independent of its conversation home.
//!
//! Domain values decoded from subject declarations. Transport tags,
//! authorization, and transactional relationship validation are separate from
//! structural validation.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A typed thing discussed in a channel or thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Subject {
    /// An outcome grouping repository-independent tasks.
    Project {
        /// Human-readable project name.
        title: String,
    },
    /// A unit of work belonging to a project, not a repository.
    Task {
        /// Human-readable task title.
        title: String,
        /// Stable Buzz project identity.
        project_id: Uuid,
        /// Optional parent task in the same project.
        parent_task_id: Option<Uuid>,
        /// Task workflow state.
        status: TaskStatus,
    },
    /// A Buzz repository whose Git representation is NIP-34.
    Repository {
        /// Human-readable repository name.
        title: String,
        /// Address of the upstream repository announcement, not an event ID.
        nip34_coordinate: String,
    },
    /// A branch incarnation, distinct from a ref-state snapshot.
    Branch {
        /// Stable Buzz repository identity.
        repository_id: Uuid,
        /// Full Git branch ref, for example `refs/heads/fix-login`.
        ref_name: String,
    },
}

/// Initial task workflow vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Work has not started.
    Open,
    /// Work is underway.
    InProgress,
    /// Work is complete.
    Done,
    /// Work was abandoned.
    Cancelled,
}

/// A conversation location. Threads inherit their channel's access rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Conversation {
    /// Canonical home for a project or repository.
    Channel {
        /// Authorization channel.
        channel_id: Uuid,
    },
    /// Canonical discussion for a task or branch.
    Thread {
        /// Authorization channel, not necessarily the branch repository's channel.
        channel_id: Uuid,
        /// Stable conversation anchor, independent of metadata revisions.
        root_event_id: nostr::EventId,
    },
}

/// Internal structural input. Relationships are decoded from event tags,
/// rather than duplicated in a second JSON wire representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectDeclaration {
    /// Stable identity shared by all revisions.
    pub id: Uuid,
    /// Canonical conversation location.
    pub conversation: Conversation,
    /// Type-specific state.
    pub subject: Subject,
}

/// A malformed declaration. Existence and permission errors belong to the relay.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid conversation subject: {0}")]
pub struct SubjectError(pub &'static str);

impl SubjectDeclaration {
    /// Validate shape only. This does not establish acceptance or authority.
    pub fn validate(&self) -> Result<(), SubjectError> {
        non_nil(self.id)?;
        let channel_id = match self.conversation {
            Conversation::Channel { channel_id } => channel_id,
            Conversation::Thread { channel_id, .. } => channel_id,
        };
        non_nil(channel_id)?;
        match (&self.subject, &self.conversation) {
            (
                Subject::Project { .. } | Subject::Repository { .. },
                Conversation::Channel { .. },
            )
            | (Subject::Task { .. } | Subject::Branch { .. }, Conversation::Thread { .. }) => {}
            _ => {
                return Err(SubjectError(
                    "subject type does not match conversation type",
                ))
            }
        }
        match &self.subject {
            Subject::Project { title } => validate_title(title),
            Subject::Task {
                title,
                project_id,
                parent_task_id,
                ..
            } => {
                validate_title(title)?;
                non_nil(*project_id)?;
                if *project_id == self.id {
                    return Err(SubjectError("task cannot be its own project"));
                }
                if let Some(parent) = parent_task_id {
                    non_nil(*parent)?;
                    if *parent == self.id {
                        return Err(SubjectError("task cannot parent itself"));
                    }
                }
                Ok(())
            }
            Subject::Repository {
                title,
                nip34_coordinate,
            } => {
                validate_title(title)?;
                validate_repository_coordinate(nip34_coordinate)
            }
            Subject::Branch {
                repository_id,
                ref_name,
            } => {
                non_nil(*repository_id)?;
                if *repository_id == self.id {
                    return Err(SubjectError("branch cannot be its own repository"));
                }
                validate_branch_ref(ref_name)
            }
        }
    }
}

fn non_nil(id: Uuid) -> Result<(), SubjectError> {
    if id.is_nil() {
        Err(SubjectError("nil identity"))
    } else {
        Ok(())
    }
}

fn validate_title(title: &str) -> Result<(), SubjectError> {
    if title.trim().is_empty() || title.len() > 512 {
        return Err(SubjectError(
            "title must contain text and be at most 512 bytes",
        ));
    }
    Ok(())
}

/// Validate a NIP-34 repository address without resolving it.
/// The identifier may contain colons; only the first two delimiters are structural.
pub fn validate_repository_coordinate(value: &str) -> Result<(), SubjectError> {
    let mut parts = value.splitn(3, ':');
    if parts.next() != Some("30617") {
        return Err(SubjectError("expected a kind-30617 repository coordinate"));
    }
    let key = parts
        .next()
        .ok_or(SubjectError("missing repository author"))?;
    if key.len() != 64
        || !key
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        || nostr::PublicKey::from_hex(key).is_err()
    {
        return Err(SubjectError(
            "repository author must be a lowercase hex public key",
        ));
    }
    if parts.next().is_none() {
        return Err(SubjectError("missing repository identifier"));
    }
    Ok(())
}

/// Validate a fully-qualified branch ref using Git's ref-name restrictions.
pub fn validate_branch_ref(value: &str) -> Result<(), SubjectError> {
    let Some(name) = value.strip_prefix("refs/heads/") else {
        return Err(SubjectError("branch ref must start with refs/heads/"));
    };
    if name.is_empty()
        || value.ends_with('.')
        || value.contains("..")
        || value.contains("@{")
        || value
            .bytes()
            .any(|c| c <= b' ' || c == 127 || b"~^:?*[\\".contains(&c))
        || value
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.') || part.ends_with(".lock"))
    {
        return Err(SubjectError("invalid Git branch ref"));
    }
    Ok(())
}
