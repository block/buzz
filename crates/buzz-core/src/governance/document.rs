use super::{hex_key, DocumentId, MAX_DOCUMENT_BYTES};
use crate::git_perms::RefPattern;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Complete versioned governance document. Unknown fields and variants are errors.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Document {
    /// Wire schema version; currently 1.
    pub version: u32,
    /// Document-specific content.
    pub body: Body,
}

/// Typed governance state; no permissive extension bag.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Body {
    /// Community-wide floor and bounded delegation settings.
    Floor {
        /// All applicable requirements are conjunctive.
        rules: Vec<Rule>,
        /// Protected label schema and defaults for new repos.
        labels: BTreeMap<String, LabelDefinition>,
        /// Longest window, in seconds, an agent's Git grant may sign.
        max_grant_seconds: u64,
    },
    /// Trusted check names and allowed producer keys.
    Trusted {
        /// Each named check has an explicit set of signer keys.
        checks: BTreeMap<String, BTreeSet<String>>,
    },
    /// Complete protected-label assignments.
    Labels {
        /// Values validated against the current floor at apply time.
        values: BTreeMap<String, String>,
    },
    /// Repository requirements, added to the floor and legacy protections.
    Rules {
        /// All matching rules must pass.
        rules: Vec<Rule>,
    },
    /// Repository's accepted channel binding.
    Bind {
        /// Both old and new channel admin authority are required.
        channel: Uuid,
    },
}

/// The levels of a Git grant, lowest first; each includes those below it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GrantAction {
    /// Read repository content.
    Read,
    /// Submit ref updates.
    Push,
    /// Manage repository configuration.
    Manage,
}

/// Allowed protected-label values and optional creation default.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LabelDefinition {
    /// Closed value set.
    pub values: BTreeSet<String>,
    /// Applied before a new repository becomes writable.
    pub default: Option<String>,
}

/// One source-qualified conjunctive policy rule.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Stable identifier within this document.
    pub id: String,
    /// Full Git ref patterns, using the existing parser.
    pub refs: Vec<String>,
    /// Optional conjunction of protected-label equalities; floor only.
    pub labels: BTreeMap<String, String>,
    /// Every listed requirement applies.
    pub require: Vec<Requirement>,
}

/// Supported requirement vocabulary. Unknown names deny at parse time.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Requirement {
    /// Require the actor to be a channel admin.
    AdminPush,
    /// Deny non-fast-forward changes.
    NoForce,
    /// Deny deletion.
    NoDelete,
    /// Require distinct independent human reviewers.
    Review {
        /// Positive number of distinct principals.
        count: u32,
        /// Optional additional channel whose current members may review.
        group: Option<Uuid>,
    },
    /// Whether approvals must have observed the current target tip.
    FreshTarget {
        /// Explicit true/false; absent configuration defaults to strict.
        required: bool,
    },
    /// Require a latest trusted passing check for the exact candidate.
    Check {
        /// Name in the community trusted-check document.
        name: String,
    },
    /// Read owners from the old target tree.
    Codeowners,
}

impl Document {
    /// Parse and validate a complete document for this exact address.
    pub fn parse(content: &str, id: &DocumentId) -> Result<Self, String> {
        if content.len() > MAX_DOCUMENT_BYTES {
            return Err("document too large".into());
        }
        // A duplicate map key must not select different semantics in two clients.
        let mut decoder = serde_json::Deserializer::from_str(content);
        let _: UniqueJson =
            serde::Deserialize::deserialize(&mut decoder).map_err(|e| e.to_string())?;
        decoder.end().map_err(|e| e.to_string())?;
        let doc: Self = serde_json::from_str(content).map_err(|e| e.to_string())?;
        if doc.version != 1 {
            return Err("unsupported document version".into());
        }
        match (&doc.body, id) {
            (
                Body::Floor {
                    rules,
                    labels,
                    max_grant_seconds,
                },
                DocumentId::Floor,
            ) => {
                validate_rules(rules, true)?;
                if *max_grant_seconds == 0 || *max_grant_seconds > 31_536_000 {
                    return Err("grant lifetime must be between 1 second and 1 year".into());
                }
                if labels.len() > 128 {
                    return Err("too many label definitions".into());
                }
                for (name, def) in labels {
                    name_ok(name)?;
                    if def.values.is_empty() || def.values.len() > 128 {
                        return Err("invalid label enum".into());
                    }
                    for value in &def.values {
                        name_ok(value)?;
                    }
                    if def
                        .default
                        .as_ref()
                        .is_some_and(|v| !def.values.contains(v))
                    {
                        return Err("label default outside enum".into());
                    }
                }
                for rule in rules {
                    for (name, value) in &rule.labels {
                        if !labels.get(name).is_some_and(|d| d.values.contains(value)) {
                            return Err("rule selects an undefined label value".into());
                        }
                    }
                }
            }
            (Body::Trusted { checks }, DocumentId::Trusted) => {
                if checks.len() > 128 {
                    return Err("too many trusted checks".into());
                }
                for (name, keys) in checks {
                    name_ok(name)?;
                    if keys.is_empty() || keys.len() > 128 || keys.iter().any(|k| !hex_key(k)) {
                        return Err("invalid trusted check keys".into());
                    }
                }
            }
            (Body::Rules { rules }, DocumentId::Rules(_)) => validate_rules(rules, false)?,
            (Body::Labels { values }, DocumentId::Labels(_)) => {
                if values.len() > 128 {
                    return Err("too many labels".into());
                }
                for (name, value) in values {
                    name_ok(name)?;
                    name_ok(value)?;
                }
            }
            (Body::Bind { .. }, DocumentId::Bind(_)) => {}
            _ => return Err("document body does not match its address".into()),
        }
        Ok(doc)
    }
}
fn name_ok(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
    {
        Err("invalid policy identifier".into())
    } else {
        Ok(())
    }
}
fn validate_rules(rules: &[Rule], floor: bool) -> Result<(), String> {
    if rules.len() > 128 {
        return Err("too many rules".into());
    }
    let mut ids = BTreeSet::new();
    for rule in rules {
        name_ok(&rule.id)?;
        if !ids.insert(&rule.id)
            || rule.refs.is_empty()
            || rule.refs.len() > 32
            || rule.require.is_empty()
            || rule.require.len() > 32
        {
            return Err("invalid or duplicate rule".into());
        }
        if !floor && !rule.labels.is_empty() {
            return Err("repo rules cannot select labels".into());
        }
        for pattern in &rule.refs {
            RefPattern::parse(pattern).map_err(|e| e.to_string())?;
        }
        let mut freshness = false;
        for req in &rule.require {
            match req {
                Requirement::Review { count, .. } if *count == 0 || *count > 128 => {
                    return Err("invalid review count".into())
                }
                Requirement::Check { name } => name_ok(name)?,
                Requirement::FreshTarget { .. } => {
                    if freshness {
                        return Err("duplicate freshness setting".into());
                    }
                    freshness = true;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

// Validate duplicate object keys recursively before typed deserialization. JSON
// numbers and arrays carry no semantics here; the typed schema handles those.
pub(super) struct UniqueJson;
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON without duplicate keys")
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> Result<UniqueJson, A::Error> {
                while a.next_element::<UniqueJson>()?.is_some() {}
                Ok(UniqueJson)
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut a: A,
            ) -> Result<UniqueJson, A::Error> {
                let mut keys = BTreeSet::new();
                while let Some(key) = a.next_key::<String>()? {
                    if !keys.insert(key) {
                        return Err(serde::de::Error::custom("duplicate JSON object key"));
                    }
                    a.next_value::<UniqueJson>()?;
                }
                Ok(UniqueJson)
            }
        }
        d.deserialize_any(Visitor)
    }
}
