//! Entity holon R3 — self-location for managed agent bodies.
//!
//! Every local Desktop spawn learns where it is (host · role · surface · DNA)
//! so it cannot hallucinate another machine's workspace. Values are public-safe
//! (no full home paths in the prompt block by default).

use std::collections::BTreeMap;

/// Env keys written by Desktop at spawn (entity holon R3).
pub(crate) const ENV_HOST_ID: &str = "BUZZ_HOST_ID";
pub(crate) const ENV_HOST_ROLE: &str = "BUZZ_HOST_ROLE";
pub(crate) const ENV_SURFACE_KIND: &str = "BUZZ_SURFACE_KIND";
pub(crate) const ENV_SURFACE_ID: &str = "BUZZ_SURFACE_ID";
pub(crate) const ENV_BIRTH_CERT: &str = "BUZZ_BIRTH_CERT_ID";
pub(crate) const ENV_BODY_ID: &str = "BUZZ_BODY_ID";
pub(crate) const ENV_PLACE_BLOCK: &str = "BUZZ_PLACE_PROOF_PROMPT";

/// Compact prompt appendix — token-wise, once per body.
const PLACE_MARKER: &str = "## Self-location (this body only)";

#[derive(Debug, Clone)]
pub(crate) struct SelfLocation {
    pub host_id: String,
    pub host_role: String,
    pub surface_kind: String,
    pub surface_id: String,
    pub birth_cert_id: String,
    pub body_id: String,
    pub legal_name: String,
}

impl SelfLocation {
    /// Desktop-local ACP body on the machine running this Desktop.
    pub(crate) fn for_desktop_agent(pubkey: &str, legal_name: &str, start_nonce: &str) -> Self {
        let host_id = std::env::var("BUZZ_HOST_ID")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(detect_host_id);
        let host_role = std::env::var("BUZZ_HOST_ROLE")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "desktop".into());
        let surface_kind = "desktop-local".to_string();
        let surface_id = format!(
            "bind:desktop:{}:{}",
            short_hex(pubkey),
            &start_nonce[..start_nonce.len().min(8)]
        );
        let body_id = format!(
            "desktop-{}-{}",
            short_hex(pubkey),
            &start_nonce[..start_nonce.len().min(12)]
        );
        Self {
            host_id,
            host_role,
            surface_kind,
            surface_id,
            birth_cert_id: pubkey.to_ascii_lowercase(),
            body_id,
            legal_name: legal_name.to_string(),
        }
    }

    pub(crate) fn env_map(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert(ENV_HOST_ID.into(), self.host_id.clone());
        m.insert(ENV_HOST_ROLE.into(), self.host_role.clone());
        m.insert(ENV_SURFACE_KIND.into(), self.surface_kind.clone());
        m.insert(ENV_SURFACE_ID.into(), self.surface_id.clone());
        m.insert(ENV_BIRTH_CERT.into(), self.birth_cert_id.clone());
        m.insert(ENV_BODY_ID.into(), self.body_id.clone());
        m.insert(ENV_PLACE_BLOCK.into(), self.prompt_block());
        m
    }

    pub(crate) fn prompt_block(&self) -> String {
        format!(
            "{PLACE_MARKER}\n\
- legal_name: {name}\n\
- birth_cert (DNA): {dna}\n\
- body_id: {body}\n\
- host_id: {host}\n\
- host_role: {role}\n\
- surface_kind: {skind}\n\
- surface_id: {sid}\n\
\n\
You are **this body on this host only**. Do not claim another machine's \
workspace, files, or uptime. A second process with the same DNA elsewhere is \
a different body — refuse to act as if you were that place. Prefer place-safe \
tools; if a path is not on this surface, say so.\n\
(Public place only — full disk paths are not required for self-knowledge.)",
            name = self.legal_name,
            dna = self.birth_cert_id,
            body = self.body_id,
            host = self.host_id,
            role = self.host_role,
            skind = self.surface_kind,
            sid = self.surface_id,
        )
    }

    /// Append place block to an existing system prompt (once).
    pub(crate) fn append_to_prompt(&self, existing: Option<&str>) -> String {
        let block = self.prompt_block();
        match existing {
            Some(p) if p.contains(PLACE_MARKER) => p.to_string(),
            Some(p) if !p.trim().is_empty() => format!("{}\n\n{block}", p.trim_end()),
            _ => block,
        }
    }

    /// Stamp place env + system prompt on the spawn command (entity holon R3).
    pub(crate) fn apply_to_command(
        &self,
        command: &mut std::process::Command,
        existing_prompt: Option<&str>,
    ) {
        for (key, value) in self.env_map() {
            command.env(key, value);
        }
        command.env(
            "BUZZ_ACP_SYSTEM_PROMPT",
            self.append_to_prompt(existing_prompt),
        );
    }
}

/// Stamp Desktop ownership markers + self-location; returns start nonce.
/// Keeps `spawn_agent_child` under the desktop file-size ratchet.
pub(crate) fn stamp_desktop_spawn_identity(
    command: &mut std::process::Command,
    instance_id: &str,
    pubkey: &str,
    display_name: Option<&str>,
    name: &str,
    existing_prompt: Option<&str>,
) -> String {
    let start_nonce = uuid::Uuid::new_v4().simple().to_string();
    command
        .env("BUZZ_MANAGED_AGENT", instance_id)
        .env("BUZZ_MANAGED_AGENT_START_NONCE", &start_nonce);
    let legal = display_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(name);
    SelfLocation::for_desktop_agent(pubkey, legal, &start_nonce)
        .apply_to_command(command, existing_prompt);
    start_nonce
}

fn short_hex(pubkey: &str) -> String {
    let p = pubkey.trim().to_ascii_lowercase();
    if p.len() >= 8 {
        p[..8].to_string()
    } else {
        p
    }
}

fn detect_host_id() -> String {
    if let Ok(h) = std::fs::read_to_string("/etc/hostname") {
        let t = h.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "desktop-host".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_once() {
        let loc = SelfLocation {
            host_id: "host".into(),
            host_role: "desktop".into(),
            surface_kind: "desktop-local".into(),
            surface_id: "bind:x".into(),
            birth_cert_id: "aa".repeat(32),
            body_id: "body-1".into(),
            legal_name: "Home-Fizz".into(),
        };
        let once = loc.append_to_prompt(Some("Be helpful."));
        assert!(once.contains("Be helpful."));
        assert!(once.contains(PLACE_MARKER));
        let twice = loc.append_to_prompt(Some(&once));
        assert_eq!(twice.matches(PLACE_MARKER).count(), 1);
    }

    #[test]
    fn env_has_birth_cert() {
        let loc = SelfLocation::for_desktop_agent(&"bb".repeat(32), "Fizz", "nonce12345678");
        let env = loc.env_map();
        assert_eq!(
            env.get(ENV_BIRTH_CERT).map(String::as_str),
            Some(&*"bb".repeat(32))
        );
        assert_eq!(
            env.get(ENV_SURFACE_KIND).map(String::as_str),
            Some("desktop-local")
        );
        assert!(!env.get(ENV_SURFACE_ID).unwrap().contains('/'));
    }
}
