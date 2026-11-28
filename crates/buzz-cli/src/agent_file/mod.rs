//! `AGENT.md`: a local agent's configuration as a file you can keep in Git.
//!
//! The front matter holds the profile, settings, channels and wakes. The body
//! holds the persona prompt, then one `## When <wake ids>` section per Interest
//! with that Interest's instructions.

pub mod codec;
pub mod document;
pub mod format;

#[cfg(test)]
mod tests;

pub use codec::{changed_fields, check_name, parse, render, slug_name, CodecError, ParsedFile};
pub use document::{AgentDocument, ReadResult};

/// What an import would do to one agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Create,
    Update(Vec<String>),
    NoChange,
}

/// A dry run: the action and a unified diff of the two canonical files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub action: Action,
    pub diff: String,
}

impl Plan {
    /// The header line, then the diff.
    pub fn report(&self, name: &str) -> String {
        let header = match &self.action {
            Action::Create => format!("CREATE {name}"),
            Action::Update(fields) => format!("UPDATE {name}: {}", fields.join(", ")),
            Action::NoChange => format!("NO_CHANGE {name}"),
        };
        if self.diff.is_empty() {
            format!("{header}\n")
        } else {
            format!("{header}\n{}", self.diff)
        }
    }
}

/// Compares a file's document against the live agent's, if there is one.
///
/// Both sides go through the canonical renderer, so comments, key order and
/// whitespace in the file don't show up as changes.
pub fn plan(
    live: Option<&AgentDocument>,
    desired: &AgentDocument,
    name: &str,
) -> Result<Plan, CodecError> {
    let new_text = render(desired, name)?;
    let label = |side: &str| format!("{side}/{name}/AGENT.md");
    let Some(live) = live else {
        let diff = diffy::DiffOptions::new()
            .set_original_filename("/dev/null")
            .set_modified_filename(label("desired"))
            .create_patch("", &new_text)
            .to_string();
        return Ok(Plan {
            action: Action::Create,
            diff,
        });
    };
    let old_text = render(live, name)?;
    // Read the live agent back through the file, so both sides lose the same
    // whitespace before they are compared.
    let normalized = parse(&old_text)?.document;
    let fields = changed_fields(&normalized, desired);
    if fields.is_empty() {
        return Ok(Plan {
            action: Action::NoChange,
            diff: String::new(),
        });
    }
    let diff = diffy::DiffOptions::new()
        .set_original_filename(label("live"))
        .set_modified_filename(label("desired"))
        .create_patch(&old_text, &new_text)
        .to_string();
    Ok(Plan {
        action: Action::Update(fields),
        diff,
    })
}
