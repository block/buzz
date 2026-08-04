use crate::config::ProviderConfig;
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Output, Stdio};

const OUTPUT_LIMIT: usize = 64 * 1024;

pub struct FlyCli {
    binary: String,
}

impl FlyCli {
    pub fn discover() -> Result<Self, String> {
        for candidate in ["fly", "flyctl"] {
            if Command::new(candidate)
                .arg("version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
            {
                return Ok(Self {
                    binary: candidate.to_string(),
                });
            }
        }
        Err("Fly.io CLI not found on PATH. Install it with `brew install flyctl`, then run `fly auth login`.".to_string())
    }

    pub fn require_auth(&self) -> Result<(), String> {
        self.run(&["auth", "whoami"]).map(|_| ()).map_err(|_| {
            "Fly.io is not authenticated. Run `fly auth login` in a terminal and retry Start."
                .to_string()
        })
    }

    pub fn ensure_app(&self, app: &str, organization: &str) -> Result<(), String> {
        if self.run(&["status", "--app", app, "--json"]).is_ok() {
            return Ok(());
        }
        self.run(&[
            "apps",
            "create",
            app,
            "--org",
            organization,
            "--yes",
            "--json",
        ])
        .map(|_| ())
        .map_err(|error| format!("could not create Fly app {app:?}: {error}"))
    }

    pub fn import_secrets(&self, app: &str, env: &BTreeMap<String, String>) -> Result<(), String> {
        let mut input = String::new();
        for (name, value) in env {
            let quoted = serde_json::to_string(value)
                .map_err(|error| format!("could not encode secret {name}: {error}"))?;
            input.push_str(name);
            input.push('=');
            input.push_str(&quoted);
            input.push('\n');
        }
        self.run_with_stdin(&["secrets", "import", "--app", app, "--stage"], &input)
            .map(|_| ())
            .map_err(|_| {
                "could not import Fly app secrets; no secret values were included in this error. Check `fly auth whoami` and the Fly dashboard, then retry Start."
                    .to_string()
            })
    }

    pub fn remove_legacy_mcp_secrets(&self, app: &str) -> Result<(), String> {
        let output = self.run(&["secrets", "list", "--app", app, "--json"])?;
        let names = legacy_mcp_secret_names(&output)?;
        if names.is_empty() {
            return Ok(());
        }

        let mut args = vec!["secrets", "unset", "--app", app, "--stage"];
        args.extend(names.iter().map(String::as_str));
        self.run(&args).map(|_| ()).map_err(|_| {
            "could not remove legacy agent-owned MCP secrets from the Fly app; no secret values were included in this error. Retry Start before deploying this agent."
                .to_string()
        })
    }

    pub fn ensure_volume(&self, app: &str, config: &ProviderConfig) -> Result<String, String> {
        let output = self.run(&["volumes", "list", "--app", app, "--json"])?;
        let value: serde_json::Value = serde_json::from_str(&output)
            .map_err(|error| format!("could not parse `fly volumes list --json`: {error}"))?;
        let volumes = value
            .as_array()
            .ok_or_else(|| "`fly volumes list --json` did not return an array".to_string())?;
        let matches: Vec<&serde_json::Value> = volumes
            .iter()
            .filter(|volume| json_string(volume, &["name", "Name"]) == Some("agent_data"))
            .collect();
        if matches.len() > 1 {
            return Err(format!(
                "Fly app {app:?} has multiple volumes named agent_data; refusing an ambiguous mount"
            ));
        }
        if let Some(volume) = matches.first() {
            let id = json_string(volume, &["id", "ID"])
                .ok_or_else(|| "existing agent_data volume has no id".to_string())?;
            let region = json_string(volume, &["region", "Region"]);
            if region != Some(config.region.as_str()) {
                return Err(format!(
                    "existing agent_data volume is in region {:?}, but provider_config.region is {:?}",
                    region.unwrap_or("unknown"),
                    config.region
                ));
            }
            return Ok(id.to_string());
        }

        let output = self.run(&[
            "volumes",
            "create",
            "agent_data",
            "--app",
            app,
            "--region",
            &config.region,
            "--size",
            &config.volume_gb.to_string(),
            "--scheduled-snapshots",
            "--snapshot-retention",
            "5",
            "--yes",
            "--json",
        ])?;
        let value: serde_json::Value = serde_json::from_str(&output)
            .map_err(|error| format!("could not parse created Fly volume: {error}"))?;
        json_string(&value, &["id", "ID"])
            .map(str::to_string)
            .ok_or_else(|| "Fly created a volume but returned no id".to_string())
    }

    pub fn reconcile_machine(
        &self,
        app: &str,
        pubkey: &str,
        volume_id: &str,
        config: &ProviderConfig,
    ) -> Result<String, String> {
        let output = self.run(&["machine", "list", "--app", app, "--json"])?;
        let value: serde_json::Value = serde_json::from_str(&output)
            .map_err(|error| format!("could not parse `fly machine list --json`: {error}"))?;
        let machines = value
            .as_array()
            .ok_or_else(|| "`fly machine list --json` did not return an array".to_string())?;
        if machines.len() > 1 {
            return Err(format!(
                "Fly app {app:?} contains {} Machines; expected at most one per agent",
                machines.len()
            ));
        }
        if let Some(machine) = machines.first() {
            let id = json_string(machine, &["id", "ID"])
                .ok_or_else(|| "existing Fly Machine has no id".to_string())?;
            let state = json_string(machine, &["state", "State"]);
            let recorded = machine
                .pointer("/config/metadata/buzz-agent-pubkey")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    machine
                        .pointer("/Config/metadata/buzz-agent-pubkey")
                        .and_then(serde_json::Value::as_str)
                });
            if recorded != Some(pubkey) {
                return Err(format!(
                    "existing Machine {id:?} is not marked for this agent pubkey; refusing to overwrite it"
                ));
            }
            self.run(&[
                "machine",
                "update",
                id,
                "--app",
                app,
                "--image",
                &config.image,
                "--vm-size",
                &config.vm_size,
                "--vm-memory",
                &config.memory_mb.to_string(),
                "--restart",
                "on-failure",
                "--autostop=off",
                "--command",
                "",
                "--skip-health-checks",
                "--yes",
            ])?;
            if !matches!(state, Some("started" | "starting")) {
                self.run(&["machine", "start", id, "--app", app])?;
            }
            return Ok(id.to_string());
        }

        self.run(&[
            "machine",
            "run",
            &config.image,
            "--app",
            app,
            "--region",
            &config.region,
            "--name",
            "agent",
            "--volume",
            &format!("{volume_id}:/home/agent"),
            "--restart",
            "on-failure",
            "--autostop=off",
            "--vm-size",
            &config.vm_size,
            "--vm-memory",
            &config.memory_mb.to_string(),
            "--metadata",
            &format!("buzz-agent-pubkey={pubkey}"),
            "--metadata",
            "buzz-provider=buzz-backend-fly",
            "--detach",
        ])?;

        let output = self.run(&["machine", "list", "--app", app, "--json"])?;
        let value: serde_json::Value = serde_json::from_str(&output)
            .map_err(|error| format!("could not parse created Fly Machine: {error}"))?;
        let machines = value
            .as_array()
            .ok_or_else(|| "created Fly Machine list was not an array".to_string())?;
        if machines.len() != 1 {
            return Err(format!(
                "Fly Machine creation returned {}, expected exactly one",
                machines.len()
            ));
        }
        json_string(&machines[0], &["id", "ID"])
            .map(str::to_string)
            .ok_or_else(|| "created Fly Machine has no id".to_string())
    }

    fn run(&self, args: &[&str]) -> Result<String, String> {
        let output = Command::new(&self.binary)
            .args(args)
            .output()
            .map_err(|error| format!("could not execute {}: {error}", self.binary))?;
        checked_output(output)
    }

    fn run_with_stdin(&self, args: &[&str], input: &str) -> Result<String, String> {
        let mut child = Command::new(&self.binary)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("could not execute {}: {error}", self.binary))?;
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "could not open Fly CLI stdin".to_string())?;
        stdin
            .write_all(input.as_bytes())
            .map_err(|error| format!("could not write Fly CLI stdin: {error}"))?;
        let output = child
            .wait_with_output()
            .map_err(|error| format!("could not wait for Fly CLI: {error}"))?;
        checked_output(output)
    }
}

fn checked_output(output: Output) -> Result<String, String> {
    let stdout = bounded(&output.stdout);
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = bounded(&output.stderr);
    let message = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    Err(format!(
        "Fly CLI exited with {}: {}",
        output.status,
        message.trim()
    ))
}

fn bounded(bytes: &[u8]) -> String {
    let end = bytes.len().min(OUTPUT_LIMIT);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn json_string<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
}

fn is_legacy_mcp_secret_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("BUZZ_ACP_MCP_SERVERS")
        || name
            .get(..9)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("BUZZ_MCP_"))
}

fn legacy_mcp_secret_names(output: &str) -> Result<Vec<String>, String> {
    let value: serde_json::Value = serde_json::from_str(output)
        .map_err(|error| format!("could not parse `fly secrets list --json`: {error}"))?;
    let secrets = value
        .as_array()
        .ok_or_else(|| "`fly secrets list --json` did not return an array".to_string())?;
    let mut names: Vec<String> = secrets
        .iter()
        .filter_map(|secret| json_string(secret, &["name", "Name"]))
        .filter(|name| is_legacy_mcp_secret_name(name))
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_output_caps_provider_messages() {
        let bytes = vec![b'x'; OUTPUT_LIMIT + 100];
        assert_eq!(bounded(&bytes).len(), OUTPUT_LIMIT);
    }

    #[test]
    fn json_string_accepts_fly_field_casing() {
        let lower = serde_json::json!({"id":"vol_1"});
        let upper = serde_json::json!({"ID":"vol_2"});
        assert_eq!(json_string(&lower, &["id", "ID"]), Some("vol_1"));
        assert_eq!(json_string(&upper, &["id", "ID"]), Some("vol_2"));
    }

    #[test]
    fn legacy_mcp_secret_names_selects_only_retired_agent_owned_keys() {
        let output = serde_json::json!([
            {"Name":"BUZZ_ACP_MCP_SERVERS","Digest":"one"},
            {"name":"BUZZ_MCP_CRM_AUTH_HEADER","digest":"two"},
            {"name":"OPENAI_COMPAT_API_KEY","digest":"three"}
        ])
        .to_string();
        assert_eq!(
            legacy_mcp_secret_names(&output).unwrap(),
            ["BUZZ_ACP_MCP_SERVERS", "BUZZ_MCP_CRM_AUTH_HEADER"]
        );
    }

    #[test]
    fn legacy_mcp_secret_names_is_case_insensitive_and_deduplicates() {
        let output = r#"[{"name":"buzz_mcp_crm_token"},{"Name":"buzz_mcp_crm_token"}]"#;
        assert_eq!(
            legacy_mcp_secret_names(output).unwrap(),
            ["buzz_mcp_crm_token"]
        );
    }
}
