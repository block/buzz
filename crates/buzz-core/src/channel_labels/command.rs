use std::collections::BTreeSet;

use nostr::Event;
use uuid::Uuid;

use super::{validate_value, LabelError, LabelSet, MAX_OPERATION_TAGS};
use crate::kind::{event_kind_u32, KIND_NIP29_CREATE_GROUP, KIND_NIP29_EDIT_METADATA};

/// A covered NIP-CL command. Parsing is not authorization or proof of application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelCommand {
    /// All h-tagged creates, even those with an empty initial label set.
    Create {
        /// Client-selected, non-nil channel UUID.
        channel: Uuid,
        /// Initial canonical set.
        labels: LabelSet,
    },
    /// A label-only incremental mutation.
    Mutate {
        /// Target channel UUID.
        channel: Uuid,
        /// Distinct additions (up to the raw operation limit).
        add: BTreeSet<String>,
        /// Distinct removals, disjoint from additions.
        remove: BTreeSet<String>,
    },
}

impl LabelCommand {
    /// Parse covered commands and reject misplaced label/snapshot tags on 9007/9002.
    ///
    /// `None` means an ordinary metadata command or a legacy unlabeled no-h
    /// create. This classification must run before generic event deduplication.
    pub fn parse(event: &Event) -> Result<Option<Self>, LabelError> {
        let kind = event_kind_u32(event);
        if !matches!(kind, KIND_NIP29_CREATE_GROUP | KIND_NIP29_EDIT_METADATA) {
            return Ok(None);
        }
        let create = kind == KIND_NIP29_CREATE_GROUP;
        let mut identifiers = Vec::new();
        let mut add = BTreeSet::new();
        let mut remove = BTreeSet::new();
        let mut raw_count = 0;
        for tag in event.tags.iter() {
            let parts = tag.as_slice();
            let name = parts[0].as_str();
            if matches!(name, "l" | "L" | "t")
                || (create && matches!(name, "add-label" | "remove-label"))
                || (!create && name == "label")
            {
                return Err(LabelError("misplaced label or snapshot tag"));
            }
            if name == "h" {
                identifiers.push(parts);
            }
            if matches!(name, "label" | "add-label" | "remove-label") {
                raw_count += 1;
                if raw_count > MAX_OPERATION_TAGS {
                    return Err(LabelError("too many raw label operations"));
                }
                if parts.len() != 2 {
                    return Err(LabelError("label operation requires exactly two elements"));
                }
                validate_value(&parts[1])?;
                if name == "remove-label" {
                    remove.insert(parts[1].clone());
                } else {
                    add.insert(parts[1].clone());
                }
            }
        }
        if raw_count == 0 && (!create || identifiers.is_empty()) {
            return Ok(None);
        }
        let channel = match identifiers.as_slice() {
            [parts] if parts.len() == 2 => Uuid::parse_str(&parts[1])
                .ok()
                .filter(|id| !id.is_nil())
                .ok_or(LabelError("h must identify a valid non-nil channel UUID"))?,
            _ => return Err(LabelError("command requires exactly one two-element h tag")),
        };
        if create {
            return Ok(Some(Self::Create {
                channel,
                labels: LabelSet::new(add)?,
            }));
        }
        // Unknown metadata cannot slip through the weaker topic/member branch.
        // Envelope/authentication tags have no metadata-changing semantics.
        if !event.content.is_empty()
            || event.tags.iter().any(|tag| {
                !matches!(
                    tag.as_slice()[0].as_str(),
                    "h" | "add-label"
                        | "remove-label"
                        | "auth"
                        | "client"
                        | "client-id"
                        | "nonce"
                        | "expiration"
                        | "-"
                )
            })
        {
            return Err(LabelError("label mutations must be label-only"));
        }
        if !add.is_disjoint(&remove) {
            return Err(LabelError("a label cannot be both added and removed"));
        }
        Ok(Some(Self::Mutate {
            channel,
            add,
            remove,
        }))
    }

    /// The channel named by the command's single validated `h` tag.
    pub fn channel(&self) -> Uuid {
        match self {
            Self::Create { channel, .. } | Self::Mutate { channel, .. } => *channel,
        }
    }
}
