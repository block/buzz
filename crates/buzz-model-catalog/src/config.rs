//! Minimal provider config for model discovery.
//!
//! This is the surviving slice of the pre-goose `buzz-agent/src/config.rs`
//! (2,709 lines). Everything else in that file existed to *implement* provider
//! configuration for the agent loop — provider enums, base URLs, model-name
//! normalization, OpenAI upgrade rules — and goose owns all of it now.
//!
//! What remains is only what Databricks model discovery needs, with the exact
//! names `desktop/src-tauri` matches on
//! (`commands/agent_models.rs:755-785`).

/// Provider families the desktop model picker can discover models for.
///
/// Names preserved verbatim: the desktop constructs these variants directly.
/// Reasoning/thinking effort level for providers that support it.
///
/// Retained in this crate because the model-capability manifest
/// ([`crate::model_capabilities`]) is typed in terms of it, and the desktop
/// model picker reads capabilities without linking goose. The request-path
/// mapping that used to live alongside it went with `buzz-agent`'s own HTTP
/// transport — goose owns the wire now, so only the vocabulary remains.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Provider {
    Anthropic,
    #[default]
    OpenAi,
    /// Databricks model serving (`api/2.0/serving-endpoints`).
    Databricks,
    /// Databricks AI Gateway v2.
    DatabricksV2,
}

/// Optional visibility filter for the Databricks model catalog.
///
/// Each comma-separated pattern is trimmed and matched against the complete,
/// case-sensitive model id. Only `*` (zero or more characters) and `?` (one
/// character) have wildcard semantics; all other characters are literals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabricksModelFilter {
    patterns: Vec<String>,
}

impl DatabricksModelFilter {
    /// Parse `DATABRICKS_MODEL_FILTER`-style input.
    ///
    /// Unset or whitespace-only input disables filtering. A nonblank value must
    /// contain at least one nonblank comma-separated pattern.
    pub fn parse(raw: Option<&str>) -> Result<Option<Self>, String> {
        let Some(raw) = raw else {
            return Ok(None);
        };

        if raw.trim().is_empty() {
            return Ok(None);
        }

        let patterns: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|pattern| !pattern.is_empty())
            .map(str::to_owned)
            .collect();
        if patterns.is_empty() {
            return Err(
                "config: DATABRICKS_MODEL_FILTER must contain at least one nonblank pattern".into(),
            );
        }

        Ok(Some(Self { patterns }))
    }

    /// Return whether the complete model id matches at least one pattern.
    pub fn matches(&self, model_id: &str) -> bool {
        self.patterns
            .iter()
            .any(|pattern| glob_matches(pattern, model_id))
    }
}

/// Match one full-string `*`/`?` pattern without treating any other character
/// as syntax. The inputs are converted to Unicode scalar values so `?` means
/// one character rather than one UTF-8 byte.
fn glob_matches(pattern: &str, value: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let value: Vec<char> = value.chars().collect();
    let mut pattern_index = 0;
    let mut value_index = 0;
    let mut star_index = None;
    let mut star_value_index = 0;

    while value_index < value.len() {
        match pattern.get(pattern_index) {
            Some('?') => {
                pattern_index += 1;
                value_index += 1;
            }
            Some('*') => {
                star_index = Some(pattern_index);
                star_value_index = value_index;
                pattern_index += 1;
            }
            Some(character) if *character == value[value_index] => {
                pattern_index += 1;
                value_index += 1;
            }
            _ if star_index.is_some() => {
                if let Some(star_index) = star_index {
                    pattern_index = star_index + 1;
                }
                star_value_index += 1;
                value_index = star_value_index;
            }
            _ => return false,
        }
    }

    while matches!(pattern.get(pattern_index), Some('*')) {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

/// Which OpenAI-family HTTP API to call. Set via `OPENAI_COMPAT_API`
/// (`auto|chat|responses`); ignored when `provider = Anthropic`. `Auto`
/// picks Responses for `*.openai.com`, Chat Completions otherwise, and
/// permits a one-shot chat→responses upgrade on a "use /v1/responses"
/// provider error.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OpenAiApi {
    Chat,
    Responses,
    Auto,
}

/// Credentials and host for a discovery call.
#[derive(Debug, Clone)]
pub struct Config {
    pub provider: Provider,
    /// Static bearer. Empty means "try the PKCE cache, but never open a
    /// browser" — see [`crate::catalog::discover_databricks_models`].
    pub api_key: String,
    /// Provider host, e.g. `DATABRICKS_HOST`.
    pub base_url: String,
    /// Optional model-id visibility filter.
    pub databricks_model_filter: Option<DatabricksModelFilter>,
}

impl Config {
    /// Signature preserved from the pre-goose crate — the desktop calls this
    /// directly (`commands/agent_models.rs:785`).
    pub fn for_discovery(
        provider: Provider,
        api_key: String,
        base_url: String,
        databricks_model_filter: Option<DatabricksModelFilter>,
    ) -> Self {
        Self {
            provider,
            api_key,
            base_url,
            databricks_model_filter,
        }
    }
}
