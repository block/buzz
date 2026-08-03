use std::path::PathBuf;

pub const DEFAULT_REGION: &str = "ams";
pub const DEFAULT_IMAGE: &str = "registry.fly.io/buzz-agent-runtime-anneday:pilot-20260803";
pub const DEFAULT_VM_SIZE: &str = "shared-cpu-1x";
pub const DEFAULT_MEMORY_MB: u64 = 1024;
pub const DEFAULT_VOLUME_GB: u64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfig {
    pub organization: String,
    pub region: String,
    pub image: String,
    pub vm_size: String,
    pub memory_mb: u64,
    pub volume_gb: u64,
    pub app_prefix: String,
    pub mcp_profile_path: Option<PathBuf>,
}

fn optional_string(value: &serde_json::Value, field: &str) -> Result<Option<String>, String> {
    match value.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value.trim().to_string())),
        Some(other) => Err(format!(
            "provider_config.{field} must be a string, got {other}"
        )),
    }
}

fn optional_u64(value: &serde_json::Value, field: &str) -> Result<Option<u64>, String> {
    match value.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(value)) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("provider_config.{field} must be a non-negative integer")),
        Some(serde_json::Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(serde_json::Value::String(value)) => value
            .trim()
            .parse::<u64>()
            .map(Some)
            .map_err(|_| format!("provider_config.{field} must be a non-negative integer")),
        Some(other) => Err(format!(
            "provider_config.{field} must be a non-negative integer, got {other}"
        )),
    }
}

fn valid_slug(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && value
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn normalize_digest_image(value: &str) -> Result<String, String> {
    let value = value.trim();
    // `fly machine run` 0.4.77 resolves digest references, then rejects the
    // resolved identifier. Use the unique, reviewed release tag for the
    // first-party image; arbitrary images must still be digest-pinned.
    if value == DEFAULT_IMAGE {
        return Ok(value.to_string());
    }
    let Some((name_and_tag, digest)) = value.split_once('@') else {
        return Err(
            "provider_config.image must be digest-pinned (name@sha256:<64 lowercase hex>)"
                .to_string(),
        );
    };
    if digest.matches(':').count() != 1 {
        return Err("provider_config.image has an invalid digest".to_string());
    }
    let Some(hex) = digest.strip_prefix("sha256:") else {
        return Err("provider_config.image digest must use sha256".to_string());
    };
    if hex.len() != 64
        || !hex
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    {
        return Err(
            "provider_config.image digest must contain 64 lowercase hexadecimal characters"
                .to_string(),
        );
    }
    let name = match name_and_tag.rfind(':') {
        Some(position) if !name_and_tag[position + 1..].contains('/') => &name_and_tag[..position],
        _ => name_and_tag,
    };
    if name.is_empty() || name.contains('@') {
        return Err("provider_config.image has no valid repository name".to_string());
    }
    Ok(format!("{name}@sha256:{hex}"))
}

pub fn parse(value: &serde_json::Value) -> Result<ProviderConfig, String> {
    if !value.is_object() && !value.is_null() {
        return Err("provider_config must be a JSON object".to_string());
    }
    let organization =
        optional_string(value, "organization")?.unwrap_or_else(|| "personal".to_string());
    if !valid_slug(&organization, 63) {
        return Err(format!(
            "provider_config.organization {organization:?} is not a valid Fly.io organization slug"
        ));
    }
    let region = optional_string(value, "region")?.unwrap_or_else(|| DEFAULT_REGION.to_string());
    if !valid_slug(&region, 16) {
        return Err(format!(
            "provider_config.region {region:?} must be a lowercase Fly.io region code"
        ));
    }
    let image = normalize_digest_image(
        optional_string(value, "image")?
            .unwrap_or_else(|| DEFAULT_IMAGE.to_string())
            .as_str(),
    )?;
    let vm_size = optional_string(value, "vm_size")?.unwrap_or_else(|| DEFAULT_VM_SIZE.to_string());
    if !valid_slug(&vm_size, 32) {
        return Err(format!(
            "provider_config.vm_size {vm_size:?} is not a valid Fly.io VM size"
        ));
    }
    let memory_mb = optional_u64(value, "memory_mb")?.unwrap_or(DEFAULT_MEMORY_MB);
    if !(512..=32768).contains(&memory_mb) {
        return Err("provider_config.memory_mb must be between 512 and 32768".to_string());
    }
    let volume_gb = optional_u64(value, "volume_gb")?.unwrap_or(DEFAULT_VOLUME_GB);
    if !(1..=500).contains(&volume_gb) {
        return Err("provider_config.volume_gb must be between 1 and 500".to_string());
    }
    let app_prefix =
        optional_string(value, "app_prefix")?.unwrap_or_else(|| "buzz-agent".to_string());
    if !valid_slug(&app_prefix, 20) {
        return Err(
            "provider_config.app_prefix must be 1-20 lowercase letters, digits or '-'".to_string(),
        );
    }
    let mcp_profile_path = optional_string(value, "mcp_profile_path")?
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                Ok(path)
            } else {
                Err("provider_config.mcp_profile_path must be absolute so Desktop and the provider resolve the same file".to_string())
            }
        })
        .transpose()?;

    Ok(ProviderConfig {
        organization,
        region,
        image,
        vm_size,
        memory_mb,
        volume_gb,
        app_prefix,
        mcp_profile_path,
    })
}

pub fn config_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "organization": {
                "type": "string",
                "title": "Fly organization",
                "description": "Organization slug from `fly orgs list`.",
                "default": "personal"
            },
            "region": {
                "type": "string",
                "title": "Fly region",
                "default": DEFAULT_REGION
            },
            "image": {
                "type": "string",
                "title": "Agent image",
                "description": "Version-pinned first-party image, or a digest-pinned custom image, containing buzz-acp and the selected ACP runtime.",
                "default": DEFAULT_IMAGE
            },
            "vm_size": {
                "type": "string",
                "title": "VM size",
                "default": DEFAULT_VM_SIZE
            },
            "memory_mb": {
                "type": "number",
                "title": "Memory (MB)",
                "default": DEFAULT_MEMORY_MB
            },
            "volume_gb": {
                "type": "number",
                "title": "Persistent volume (GB)",
                "default": DEFAULT_VOLUME_GB
            },
            "app_prefix": {
                "type": "string",
                "title": "App name prefix",
                "default": "buzz-agent"
            },
            "mcp_profile_path": {
                "type": "string",
                "title": "MCP profile file",
                "description": "Optional absolute path to a JSON MCP profile. Account credentials remain in the agent environment and are referenced with inherit_env."
            }
        },
        "required": ["organization", "region", "image"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_small_and_version_pinned() {
        let config = parse(&serde_json::json!({})).unwrap();
        assert_eq!(config.region, "ams");
        assert_eq!(config.memory_mb, 1024);
        assert_eq!(config.volume_gb, 5);
        assert_eq!(config.image, DEFAULT_IMAGE);
    }

    #[test]
    fn rejects_tag_only_images() {
        let error = parse(&serde_json::json!({"image":"example/image:latest"})).unwrap_err();
        assert!(error.contains("digest-pinned"));
    }

    #[test]
    fn requires_absolute_mcp_profile_path() {
        let error = parse(&serde_json::json!({"mcp_profile_path":"relative.json"})).unwrap_err();
        assert!(error.contains("must be absolute"));
    }
}
