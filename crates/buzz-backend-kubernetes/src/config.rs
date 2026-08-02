//! `provider_config` parsing and the `info` config schema
//! (spec §`provider_config` v1 fields, `docs/remote-agents.md:1384-1389`).
//!
//! Nine fields, all optional except `image` (v1 ships no baked default —
//! §Image). No credential field exists, by I2: cluster auth comes from ambient
//! kubeconfig resolution and nothing else (`:196-198`).

use crate::image::{self, ImageRef};

/// Resource requests and limits (§Pod shape: 1cpu/2Gi → 2cpu/4Gi, all four
/// configurable — `cargo build` in an agent workspace makes 500m/1Gi
/// unrealistic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resources {
    pub cpu_request: String,
    pub memory_request: String,
    pub cpu_limit: String,
    pub memory_limit: String,
}

impl Default for Resources {
    fn default() -> Self {
        Self {
            cpu_request: "1".into(),
            memory_request: "2Gi".into(),
            cpu_limit: "2".into(),
            memory_limit: "4Gi".into(),
        }
    }
}

/// Default inactivity budget: the I5 opt-in (§Auto-Stop). The config field and
/// `BUZZ_ACP_EXIT_AFTER_INACTIVITY` are one knob, not two.
pub const DEFAULT_INACTIVITY_SECONDS: u64 = 7200;

/// Fixed nonzero UID/GID for the agent container (§Pod shape hardening).
pub const RUN_AS_UID: i64 = 10001;
pub const RUN_AS_GID: i64 = 10001;

/// Writable workspace root; also `HOME` and the harness's cwd
/// (§Working directory).
pub const WORKSPACE_PATH: &str = "/home/agent";

/// `terminationGracePeriodSeconds` — a declared budget, not a derived sum
/// (§Pod shape). Kubernetes' default 30s would SIGKILL the harness mid-drain.
pub const TERMINATION_GRACE_SECONDS: i64 = 60;

/// The only restart policy v1 ships. `OnFailure` is double-gated on the
/// harness exit-code contract *and* a crash-loop classification row the state
/// machine does not have (`:1121-1139`); until both land the provider refuses
/// the combination rather than shipping against an undefended convention.
pub const RESTART_POLICY: &str = "Never";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfig {
    /// kubeconfig context; `None` uses the current context.
    pub context: Option<String>,
    pub namespace: String,
    pub image: ImageRef,
    pub resources: Resources,
    /// `None` when `inactivity_seconds` was 0 — refused in v1, see [`parse`].
    pub inactivity_seconds: Option<u64>,
    pub service_account: Option<String>,
}

/// Read an optional non-empty string field. Rejects non-string scalars rather
/// than stringifying them, so a mistyped field is named at the boundary.
fn optional_string(cfg: &serde_json::Value, field: &str) -> Result<Option<String>, String> {
    match cfg.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s.trim().to_string())),
        Some(other) => Err(format!(
            "provider_config.{field} must be a string, got {other}"
        )),
    }
}

/// Read an optional unsigned integer. The desktop's form omits blank numeric
/// fields rather than sending `""`, but a hand-crafted payload may send a
/// numeric string — accept both, refuse anything else.
fn optional_u64(cfg: &serde_json::Value, field: &str) -> Result<Option<u64>, String> {
    match cfg.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(n)) => n.as_u64().map(Some).ok_or_else(|| {
            format!("provider_config.{field} must be a non-negative integer, got {n}")
        }),
        Some(serde_json::Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(serde_json::Value::String(s)) => s.trim().parse::<u64>().map(Some).map_err(|_| {
            format!("provider_config.{field} must be a non-negative integer, got {s:?}")
        }),
        Some(other) => Err(format!(
            "provider_config.{field} must be a non-negative integer, got {other}"
        )),
    }
}

/// A Kubernetes namespace name: RFC 1123 label, ≤63 chars. Validated here so a
/// typo fails with a named field instead of an apiserver rejection partway
/// through a deploy.
fn valid_namespace(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name.ends_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn parse(cfg: &serde_json::Value) -> Result<ProviderConfig, String> {
    if !cfg.is_object() && !cfg.is_null() {
        return Err("provider_config must be a JSON object".to_string());
    }

    let namespace = optional_string(cfg, "namespace")?.ok_or_else(|| {
        "provider_config.namespace is required: the info schema supplies a \
         generated default, so an empty value means the form was cleared"
            .to_string()
    })?;
    if !valid_namespace(&namespace) {
        return Err(format!(
            "provider_config.namespace {namespace:?} is not a valid Kubernetes \
             namespace (lowercase alphanumerics and '-', ≤63 characters)"
        ));
    }

    let image = image::parse(optional_string(cfg, "image")?.unwrap_or_default().as_str())?;

    let defaults = Resources::default();
    let resources = Resources {
        cpu_request: optional_string(cfg, "cpu_request")?.unwrap_or(defaults.cpu_request),
        memory_request: optional_string(cfg, "memory_request")?.unwrap_or(defaults.memory_request),
        cpu_limit: optional_string(cfg, "cpu_limit")?.unwrap_or(defaults.cpu_limit),
        memory_limit: optional_string(cfg, "memory_limit")?.unwrap_or(defaults.memory_limit),
    };

    // `inactivity_seconds: 0` is a legal, blessed value in the spec (§Auto-Stop)
    // meaning "no auto-stop" — but it selects `restartPolicy: OnFailure`, which
    // §Pod shape forbids until the harness exit-code contract is pinned AND the
    // state machine gains a crash-loop row. Refusing the *combination* is what
    // the spec asks for; silently downgrading to `Never` would ship an
    // indefinite agent that dies on its first crash.
    let inactivity_seconds = match optional_u64(cfg, "inactivity_seconds")? {
        None => Some(DEFAULT_INACTIVITY_SECONDS),
        Some(0) => {
            return Err(
                "provider_config.inactivity_seconds: 0 (indefinite lifetime) is not \
                 supported in this version: it requires restartPolicy OnFailure, \
                 which is gated on the harness exit-code contract. Set a positive \
                 number of seconds."
                    .to_string(),
            )
        }
        Some(n) => Some(n),
    };

    Ok(ProviderConfig {
        context: optional_string(cfg, "context")?,
        namespace,
        image,
        resources,
        inactivity_seconds,
        service_account: optional_string(cfg, "service_account")?,
    })
}

/// A fresh `buzz-agents-<rand6>` namespace default.
///
/// Computed per `info` call, which is how "random default" is satisfied with
/// zero UI changes: the schema's `default` prefills the form (§K8s Namespace).
pub fn generated_namespace() -> String {
    use rand::RngExt;
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();
    let suffix: String = (0..6)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect();
    format!("buzz-agents-{suffix}")
}

/// The `config_schema` returned by `info`. Drives the UI form:
/// `properties[*].default` prefill, scalar coercion, `required` gating
/// (`:407-411`).
pub fn config_schema() -> serde_json::Value {
    let defaults = Resources::default();
    serde_json::json!({
        "type": "object",
        "properties": {
            "context": {
                "type": "string",
                "title": "Kubeconfig context",
                "description": "Context from your kubeconfig. Leave empty to use the current context."
            },
            "namespace": {
                "type": "string",
                "title": "Namespace",
                "description": "Created if it does not exist.",
                "default": generated_namespace()
            },
            "image": {
                "type": "string",
                "title": "Agent image",
                "description": "Digest-pinned image containing the buzz-acp runtime ABI, e.g. ghcr.io/block/buzz-sprig@sha256:<digest>. Tags are not accepted: this pod holds the agent's private key."
            },
            "cpu_request": {
                "type": "string", "title": "CPU request", "default": defaults.cpu_request
            },
            "memory_request": {
                "type": "string", "title": "Memory request", "default": defaults.memory_request
            },
            "cpu_limit": {
                "type": "string", "title": "CPU limit", "default": defaults.cpu_limit
            },
            "memory_limit": {
                "type": "string", "title": "Memory limit", "default": defaults.memory_limit
            },
            "inactivity_seconds": {
                "type": "number",
                "title": "Stop after inactivity (seconds)",
                "description": "The agent exits after this long with no work, and can be started again at any time.",
                "default": DEFAULT_INACTIVITY_SECONDS
            },
            "service_account": {
                "type": "string",
                "title": "Service account",
                "description": "Scheduling/RBAC identity only. No API token is mounted."
            }
        },
        "required": ["namespace", "image"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest_ref() -> String {
        format!("ghcr.io/block/buzz-sprig@sha256:{}", "a".repeat(64))
    }

    fn minimal() -> serde_json::Value {
        serde_json::json!({"namespace": "buzz-agents-abc123", "image": digest_ref()})
    }

    #[test]
    fn applies_spec_defaults() {
        let c = parse(&minimal()).unwrap();
        assert_eq!(c.resources, Resources::default());
        assert_eq!(c.resources.cpu_request, "1");
        assert_eq!(c.resources.memory_request, "2Gi");
        assert_eq!(c.resources.cpu_limit, "2");
        assert_eq!(c.resources.memory_limit, "4Gi");
        assert_eq!(c.inactivity_seconds, Some(DEFAULT_INACTIVITY_SECONDS));
        assert_eq!(c.context, None);
        assert_eq!(c.service_account, None);
    }

    #[test]
    fn all_four_resources_are_configurable() {
        let mut cfg = minimal();
        cfg["cpu_request"] = "500m".into();
        cfg["memory_request"] = "1Gi".into();
        cfg["cpu_limit"] = "4".into();
        cfg["memory_limit"] = "8Gi".into();
        let c = parse(&cfg).unwrap();
        assert_eq!(
            c.resources,
            Resources {
                cpu_request: "500m".into(),
                memory_request: "1Gi".into(),
                cpu_limit: "4".into(),
                memory_limit: "8Gi".into(),
            }
        );
    }

    /// The desktop's form omits blank numeric fields; a hand-crafted payload
    /// may send a numeric string. Both must mean the same thing.
    #[test]
    fn inactivity_accepts_number_string_and_omission() {
        let mut cfg = minimal();
        cfg["inactivity_seconds"] = serde_json::json!(300);
        assert_eq!(parse(&cfg).unwrap().inactivity_seconds, Some(300));

        cfg["inactivity_seconds"] = serde_json::json!("300");
        assert_eq!(parse(&cfg).unwrap().inactivity_seconds, Some(300));

        cfg["inactivity_seconds"] = serde_json::json!("");
        assert_eq!(
            parse(&cfg).unwrap().inactivity_seconds,
            Some(DEFAULT_INACTIVITY_SECONDS)
        );
    }

    /// Indefinite lifetime selects `OnFailure`, which is gated. Refuse rather
    /// than silently downgrade — a downgraded agent dies on its first crash
    /// while the user believes they asked for indefinite.
    #[test]
    fn refuses_indefinite_lifetime() {
        let mut cfg = minimal();
        cfg["inactivity_seconds"] = serde_json::json!(0);
        let err = parse(&cfg).unwrap_err();
        assert!(err.contains("inactivity_seconds"), "got: {err}");
        assert!(
            err.contains("OnFailure"),
            "error should name the gate: {err}"
        );
    }

    #[test]
    fn rejects_negative_and_non_numeric_inactivity() {
        for bad in [
            serde_json::json!(-1),
            serde_json::json!(1.5),
            serde_json::json!("soon"),
            serde_json::json!(true),
        ] {
            let mut cfg = minimal();
            cfg["inactivity_seconds"] = bad.clone();
            assert!(parse(&cfg).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn image_is_required_and_must_be_digest_pinned() {
        let mut cfg = minimal();
        cfg.as_object_mut().unwrap().remove("image");
        assert!(parse(&cfg).unwrap_err().contains("provider_config.image"));

        cfg["image"] = "ghcr.io/block/buzz-sprig:latest".into();
        assert!(parse(&cfg).unwrap_err().contains("digest-pinned"));
    }

    #[test]
    fn rejects_invalid_namespace_names() {
        for bad in [
            "",
            "Buzz-Agents",
            "-leading",
            "trailing-",
            "has_underscore",
            &"n".repeat(64),
        ] {
            let mut cfg = minimal();
            cfg["namespace"] = bad.into();
            assert!(parse(&cfg).is_err(), "accepted namespace {bad:?}");
        }
    }

    /// I2 corollary: there is no config path for cluster credentials, so a
    /// caller that tries to supply one gets no effect from it. Asserting the
    /// parsed struct has no such field is the closest a test can get to
    /// "the type makes it impossible".
    #[test]
    fn credential_fields_have_no_effect() {
        let mut cfg = minimal();
        cfg["token"] = "hunter2".into();
        cfg["client_key"] = "hunter2".into();
        let c = parse(&cfg).unwrap();
        let rendered = format!("{c:?}");
        assert!(
            !rendered.contains("hunter2"),
            "config absorbed a credential: {rendered}"
        );
    }

    #[test]
    fn mistyped_string_fields_are_named() {
        let mut cfg = minimal();
        cfg["namespace"] = serde_json::json!(42);
        assert!(parse(&cfg)
            .unwrap_err()
            .contains("provider_config.namespace"));
    }

    #[test]
    fn generated_namespaces_are_fresh_and_valid() {
        let a = generated_namespace();
        let b = generated_namespace();
        assert_ne!(a, b, "namespace default is not random");
        assert!(valid_namespace(&a), "{a} is not a valid namespace");
        assert!(a.starts_with("buzz-agents-"));
        assert_eq!(a.len(), "buzz-agents-".len() + 6);
    }

    /// The schema's own namespace default must be a value the parser accepts —
    /// otherwise the UI prefills a form that fails on submit.
    #[test]
    fn schema_default_namespace_round_trips_through_parse() {
        let schema = config_schema();
        let default = schema["properties"]["namespace"]["default"]
            .as_str()
            .unwrap();
        let cfg = serde_json::json!({"namespace": default, "image": digest_ref()});
        assert_eq!(parse(&cfg).unwrap().namespace, default);
    }

    /// Nine fields exactly (§`provider_config` v1 fields). The cap is 20; the
    /// count is pinned so a field added without a spec change is caught here.
    #[test]
    fn schema_declares_exactly_the_nine_v1_fields() {
        let schema = config_schema();
        let props = schema["properties"].as_object().unwrap();
        let mut keys: Vec<&str> = props.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "context",
                "cpu_limit",
                "cpu_request",
                "image",
                "inactivity_seconds",
                "memory_limit",
                "memory_request",
                "namespace",
                "service_account"
            ]
        );
        assert_eq!(
            schema["required"],
            serde_json::json!(["namespace", "image"])
        );
    }

    /// I2's key lint rejects any field whose word-split contains
    /// secret|password|token|key|credential. A schema field tripping it would
    /// make every deploy fail validation desktop-side (`:185-198`).
    #[test]
    fn no_schema_field_trips_the_i2_key_lint() {
        const BANNED: [&str; 5] = ["secret", "password", "token", "key", "credential"];
        let schema = config_schema();
        for field in schema["properties"].as_object().unwrap().keys() {
            for word in field.split(['_', '-']) {
                assert!(
                    !BANNED.contains(&word),
                    "field {field:?} contains I2-banned word {word:?}"
                );
            }
        }
    }
}
