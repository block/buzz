//! The swarm file: one YAML document holding the owner identity, the defaults
//! every agent inherits, and the per-agent overrides.
//!
//! Defaults and agent entries share a single schema (`Spec`). They are merged
//! as YAML mappings before typing, so a new knob is added in exactly one place
//! and can never be forgotten in a hand-written merge function.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use serde_yaml::{Mapping, Value};
use zeroize::Zeroizing;

use crate::env::Env;

pub const DEFAULT_CONFIG_FILE: &str = "buzz-swarm.yaml";

/// Top-level document.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct SwarmFile {
    pub owner: Option<Owner>,
    /// Paths in the file are relative to its directory, independent of cwd.
    #[serde(skip)]
    pub directory: PathBuf,
    /// Settings every agent inherits. Merged under each agent entry.
    #[serde(default)]
    pub defaults: Mapping,
    /// One entry per agent identity. Each becomes one supervised `buzz-acp`.
    pub agents: Vec<Value>,
}

/// The human whose community membership the agents borrow.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct Owner {
    pub nsec: SecretRef,
    /// NIP-OA delegation conditions, e.g. `kind=1&created_at<1913957000`.
    /// Empty (the default) delegates until the owner retires the agent key.
    #[serde(default)]
    pub conditions: String,
}

/// Where a value comes from — an agent key, or an entry under `env:`. Literals
/// are supported for convenience; `env:` and `file:` keep the secret out of the
/// config file entirely.
#[derive(Clone)]
pub enum SecretRef {
    Literal(String),
    Env(String),
    File(PathBuf),
}

impl fmt::Debug for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretRef(<redacted>)")
    }
}

/// Keep source errors actionable without echoing secret values.
impl<'de> Deserialize<'de> for SecretRef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(SecretRefVisitor)
    }
}

struct SecretRefVisitor;

impl<'de> serde::de::Visitor<'de> for SecretRefVisitor {
    type Value = SecretRef;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a value, `{env: VAR}`, or `{file: PATH}`")
    }

    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(SecretRef::Literal(value.to_owned()))
    }

    fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut source: Option<SecretRef> = None;
        while let Some(key) = map.next_key::<String>()? {
            let next = match key.as_str() {
                "env" => SecretRef::Env(map.next_value()?),
                "file" => SecretRef::File(map.next_value()?),
                other => {
                    return Err(serde::de::Error::custom(format!(
                        "unknown source `{other}` — expected `env` or `file`"
                    )))
                }
            };
            if source.replace(next).is_some() {
                return Err(serde::de::Error::custom(
                    "set either `env:` or `file:`, not both",
                ));
            }
        }
        source.ok_or_else(|| {
            serde::de::Error::custom("expected a value, `{env: VAR}`, or `{file: PATH}`")
        })
    }
}

impl SecretRef {
    /// Resolve without logging values. Literal and environment values are exact;
    /// files trim surrounding whitespace for secret-manager mounted credentials.
    pub fn resolve(&self, env: &Env) -> Result<Secret> {
        let value = match self {
            Self::Literal(value) => Zeroizing::new(value.clone()),
            Self::Env(var) => {
                let value = env
                    .get(var)
                    .with_context(|| format!("environment variable `{var}` is not set"))?;
                Zeroizing::new(value.to_owned())
            }
            Self::File(path) => {
                let path = expand_tilde(path, env);
                let contents = Zeroizing::new(
                    std::fs::read_to_string(&path)
                        .with_context(|| format!("reading secret file {}", path.display()))?,
                );
                Zeroizing::new(contents.trim().to_owned())
            }
        };
        Ok(Secret(value))
    }

    fn resolve_path(&mut self, directory: &Path, env: &Env) {
        if let Self::File(path) = self {
            *path = directory.join(expand_tilde(path, env));
        }
    }

    pub fn env_key(&self) -> Option<&str> {
        match self {
            Self::Env(key) => Some(key),
            _ => None,
        }
    }
}

/// Secret material that never reaches a log line by accident.
#[derive(Clone)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Swarm settings. Runtime-specific options use the existing `BUZZ_*`
/// environment contract through `env`, rather than a second configuration API.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct Spec {
    // Identity — agent entries only.
    pub name: Option<String>,
    pub nsec: Option<SecretRef>,
    /// Pre-signed NIP-OA tag for this identity; avoids sending the owner key
    /// to the machine running the swarm.
    pub auth_tag: Option<SecretRef>,
    pub enabled: Option<bool>,

    pub relays: Option<Vec<String>>,
    pub workdir: Option<PathBuf>,
    pub harness: Option<String>,
    pub restart: Option<Restart>,
    pub max_restarts: Option<u32>,

    /// Extra environment for the harness and everything below it. Keys the
    /// swarm owns are rejected.
    #[serde(default)]
    pub env: BTreeMap<String, SecretRef>,
}

/// What to do when a harness process exits.
///
/// `on-failure` is the default because a Buzz agent is built to end itself —
/// it says goodbye and exits 0 after `exit_after_inactivity`, or when its owner
/// tells it to stop. Restarting a clean exit would fight that lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Restart {
    Never,
    #[default]
    OnFailure,
    Always,
}

impl SwarmFile {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading swarm file {}", path.display()))?;
        let mut file = Self::parse(&text)?;
        file.directory = std::path::absolute(path)?
            .parent()
            .context("swarm file has no parent directory")?
            .to_path_buf();
        Ok(file)
    }

    pub fn parse(text: &str) -> Result<Self> {
        serde_yaml::from_str(text).context("parsing swarm file")
    }

    pub fn owner(&self, env: &Env) -> Option<Owner> {
        self.owner.as_ref().map(|owner| {
            let mut nsec = owner.nsec.clone();
            nsec.resolve_path(&self.directory, env);
            Owner {
                nsec,
                conditions: owner.conditions.clone(),
            }
        })
    }

    pub fn resolved_specs(&self, env: &Env) -> Result<Vec<Spec>> {
        let mut specs = self.specs()?;
        for spec in &mut specs {
            for source in spec
                .nsec
                .iter_mut()
                .chain(spec.auth_tag.iter_mut())
                .chain(spec.env.values_mut())
            {
                source.resolve_path(&self.directory, env);
            }
            if let Some(path) = &mut spec.workdir {
                *path = self.directory.join(expand_tilde(path, env));
            }
            if let Some(command) = &mut spec.harness {
                if command.contains('/') || command.starts_with('~') {
                    *command = self
                        .directory
                        .join(expand_tilde(Path::new(command), env))
                        .to_string_lossy()
                        .into_owned();
                }
            }
        }
        Ok(specs)
    }

    /// `defaults:` on its own, including values every agent overrides.
    pub fn defaults(&self) -> Result<Spec> {
        serde_yaml::from_value(Value::Mapping(self.defaults.clone())).context("parsing `defaults:`")
    }

    /// Type-check `defaults:` on its own, then produce one fully merged `Spec`
    /// per agent entry (agent keys win; `env:` maps merge key-wise).
    pub fn specs(&self) -> Result<Vec<Spec>> {
        let defaults = self.defaults()?;
        for (field, present) in [
            ("name", defaults.name.is_some()),
            ("nsec", defaults.nsec.is_some()),
            ("auth_tag", defaults.auth_tag.is_some()),
            ("enabled", defaults.enabled.is_some()),
        ] {
            if present {
                bail!("`defaults:` must not set `{field}` — it belongs to an agent entry");
            }
        }

        let mut specs = Vec::with_capacity(self.agents.len());
        for (index, entry) in self.agents.iter().enumerate() {
            let Value::Mapping(entry) = entry else {
                bail!("agents[{index}]: each entry must be a mapping with a `name`");
            };
            let label = entry
                .get(Value::from("name"))
                .and_then(Value::as_str)
                .map(|name| format!("agents[{index}] (`{name}`)"))
                .unwrap_or_else(|| format!("agents[{index}]"));
            let merged = merge_mappings(&self.defaults, entry);
            let spec: Spec = serde_yaml::from_value(Value::Mapping(merged))
                .with_context(|| format!("parsing {label}"))?;
            specs.push(spec);
        }
        Ok(specs)
    }
}

impl fmt::Debug for SwarmFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SwarmFile")
            .field("agents", &self.agents.len())
            .finish_non_exhaustive()
    }
}

/// Only `env` merges key-wise. A secret source is replaced as a whole.
fn merge_mappings(base: &Mapping, over: &Mapping) -> Mapping {
    let mut merged = base.clone();
    for (key, value) in over {
        match (merged.get(key), value) {
            (Some(Value::Mapping(base_map)), Value::Mapping(over_map))
                if key.as_str() == Some("env") =>
            {
                let mut nested = base_map.clone();
                nested.extend(over_map.clone());
                merged.insert(key.clone(), Value::Mapping(nested));
            }
            _ => {
                merged.insert(key.clone(), value.clone());
            }
        }
    }
    merged
}

/// Names also determine the default key filenames.
pub fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')),
        "agent name must be 1–64 ASCII letters, digits, hyphens, or underscores"
    );
    Ok(())
}

pub fn expand_tilde(path: &Path, env: &Env) -> PathBuf {
    let Ok(rest) = path.strip_prefix("~") else {
        return path.to_path_buf();
    };
    match env.get("HOME") {
        Some(home) if !home.is_empty() => Path::new(home).join(rest),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests;
