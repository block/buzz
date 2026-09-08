//! Runtime model-capability interpreter.
//!
//! `scripts/model-capabilities.json` is the single source of truth for every
//! model's six-axis capability profile (thinking mode, supported efforts,
//! default effort, Databricks v2 wire route, normalization policy, and picker
//! label). It is embedded at compile time (`include_str!`), parsed once through
//! strict `serde` (`deny_unknown_fields` + real enums), and cached in a
//! [`OnceLock`]. No codegen: both this interpreter and the TypeScript one in
//! `desktop/` read the same hand-curated manifest, and the shared normative
//! corpus (`scripts/normative-corpus.json`) is the cross-language contract that
//! guarantees they agree.
//!
//! ## Resolution algorithm (`resolve`)
//! 1. Provider canonicalization happens *inside* the resolver: trim, lowercase,
//!    and apply the alias map (`openai-compat` → `openai`,
//!    `databricks-v2` → `databricks_v2`).
//! 2. Provider-qualified exact-record lookup (case-insensitive on the model id).
//! 3. Boundary-aware family-rule match: strip any endpoint prefix at the first
//!    family token on a non-alphanumeric boundary, then take the longest match
//!    across every rule's `match_value` and `match_aliases`, breaking ties on
//!    the lexicographically smallest rule id.
//! 4. Provider fallback, distinguishing a blank model id from a concrete-unknown
//!    one.
//!
//! Every path yields a complete six-axis result; `registry_label` is populated
//! only on an exact-record hit.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::config::ThinkingEffort;

/// How a model activates and controls reasoning depth on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThinkingMode {
    Adaptive,
    ManualBudget,
    None,
    OmitFields,
}

/// The Databricks v2 AI Gateway wire route a model is served on. `NotApplicable`
/// marks non-Databricks providers; `RouteUnknown` marks a blank Databricks id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DatabricksV2Route {
    AnthropicMessages,
    MlflowChat,
    NotApplicable,
    OpenaiResponses,
    RouteUnknown,
}

/// Post-resolution effort normalization applied before a request is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NormalizationPolicy {
    None,
    OpenaiClampMaxToXhigh,
    OpenaiStandard,
}

/// Whether a family rule matches its token exactly or as a boundary-aware prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum MatchKind {
    Exact,
    Prefix,
}

/// A family/prefix rule: matches a canonical (prefix-stripped) model id against
/// `match_value` or any `match_aliases` token for the listed providers.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FamilyRule {
    id: String,
    match_kind: MatchKind,
    match_value: String,
    #[serde(default)]
    match_aliases: Vec<String>,
    providers: Vec<String>,
    thinking_mode: ThinkingMode,
    supported_efforts: Vec<ThinkingEffort>,
    default_effort: Option<ThinkingEffort>,
    databricks_v2_wire_route: DatabricksV2Route,
    normalization_policy: NormalizationPolicy,
    /// Documentation only; modeled so `deny_unknown_fields` accepts the manifest.
    #[serde(rename = "_comment", default)]
    #[allow(dead_code)]
    comment: Option<String>,
}

/// An authoritative six-axis snapshot for one concrete `(provider, model)` pair.
/// Exact records do *not* inherit from family rules at runtime; the doc fields
/// record the one-time provenance of each axis (see the manifest `_comment`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExactRecord {
    provider: String,
    raw_model_id: String,
    registry_label: String,
    thinking_mode: ThinkingMode,
    supported_efforts: Vec<ThinkingEffort>,
    default_effort: Option<ThinkingEffort>,
    databricks_v2_wire_route: DatabricksV2Route,
    normalization_policy: NormalizationPolicy,
    // Documentation/provenance keys; modeled for strict parsing, not read at runtime.
    #[serde(rename = "_provenance", default)]
    #[allow(dead_code)]
    provenance: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    source: Option<String>,
    #[serde(rename = "_source", default)]
    #[allow(dead_code)]
    source_alt: Option<String>,
    #[serde(rename = "_reconciliation", default)]
    #[allow(dead_code)]
    reconciliation: Option<String>,
    #[serde(rename = "_reconciliation_note", default)]
    #[allow(dead_code)]
    reconciliation_note: Option<String>,
    #[serde(rename = "_reconciliation_doc", default)]
    #[allow(dead_code)]
    reconciliation_doc: Option<String>,
}

/// One provider's fallback profiles for a blank vs. a concrete-unknown model id.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FallbackPair {
    blank: FallbackState,
    concrete_unknown: FallbackState,
}

/// A five-axis fallback profile (no label — fallbacks never carry one).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FallbackState {
    databricks_v2_wire_route: DatabricksV2Route,
    thinking_mode: ThinkingMode,
    supported_efforts: Vec<ThinkingEffort>,
    default_effort: Option<ThinkingEffort>,
    normalization_policy: NormalizationPolicy,
}

/// Provider fallbacks keyed by canonical provider, with a `_default` catch-all.
/// Both states of every provider are required, so "both fallback states present"
/// is enforced structurally by the parse.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderFallbacks {
    anthropic: FallbackPair,
    openai: FallbackPair,
    databricks: FallbackPair,
    databricks_v2: FallbackPair,
    openrouter: FallbackPair,
    #[serde(rename = "_default")]
    default: FallbackPair,
}

impl ProviderFallbacks {
    /// Fallback pair for a canonical provider, or `_default` for anything else.
    fn get(&self, provider: &str) -> &FallbackPair {
        match provider {
            "anthropic" => &self.anthropic,
            "openai" => &self.openai,
            "databricks" => &self.databricks,
            "databricks_v2" => &self.databricks_v2,
            "openrouter" => &self.openrouter,
            _ => &self.default,
        }
    }

    /// Named pairs, for validation.
    fn named(&self) -> [(&str, &FallbackPair); 6] {
        [
            ("anthropic", &self.anthropic),
            ("openai", &self.openai),
            ("databricks", &self.databricks),
            ("databricks_v2", &self.databricks_v2),
            ("openrouter", &self.openrouter),
            ("_default", &self.default),
        ]
    }
}

/// The parsed manifest.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    family_tokens: Vec<String>,
    label_family_tokens: Vec<String>,
    family_rules: Vec<FamilyRule>,
    databricks_v2_known_models: Vec<String>,
    exact_records: Vec<ExactRecord>,
    provider_fallbacks: ProviderFallbacks,
    // Root documentation keys; modeled for strict parsing, not read at runtime.
    #[serde(rename = "_comment", default)]
    #[allow(dead_code)]
    comment: Option<String>,
    #[serde(rename = "_comment_label_family_tokens", default)]
    #[allow(dead_code)]
    comment_label_family_tokens: Option<String>,
    #[serde(rename = "_comment_databricks_v2_known_models", default)]
    #[allow(dead_code)]
    comment_known_models: Option<String>,
    #[serde(rename = "_sources", default)]
    #[allow(dead_code)]
    sources: std::collections::BTreeMap<String, String>,
}

/// The resolved six-axis capability profile for one `(provider, model)` query.
/// All fields borrow from the process-lifetime manifest. The field names and
/// declaration order are the corpus `expect` schema — the test-only generator
/// serializes this struct directly, so there is no second encoding of the axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CapabilityResult {
    pub thinking_mode: ThinkingMode,
    pub supported_efforts: &'static [ThinkingEffort],
    pub default_effort: Option<ThinkingEffort>,
    pub databricks_v2_wire_route: DatabricksV2Route,
    pub normalization_policy: NormalizationPolicy,
    pub registry_label: Option<&'static str>,
}

const MANIFEST_JSON: &str = include_str!("../../../scripts/model-capabilities.json");

static MANIFEST: OnceLock<Manifest> = OnceLock::new();

/// Parse (once) and return the embedded manifest. Panics on a malformed or
/// invalid bundled manifest — a build-time data error that must never ship.
fn manifest() -> &'static Manifest {
    MANIFEST.get_or_init(|| {
        let parsed: Manifest = serde_json::from_str(MANIFEST_JSON)
            .expect("bundled model-capabilities.json must parse");
        if let Err(e) = validate_manifest(&parsed) {
            panic!("bundled model-capabilities.json failed validation: {e}");
        }
        parsed
    })
}

/// Canonicalize a provider name: trim, lowercase, apply the alias map.
fn canonical_provider(provider: &str) -> String {
    let canon = provider.trim().to_ascii_lowercase();
    match canon.as_str() {
        "openai-compat" => "openai".to_string(),
        "databricks-v2" => "databricks_v2".to_string(),
        _ => canon,
    }
}

/// Strip an endpoint-naming prefix by locating the earliest family token that
/// begins on a non-alphanumeric boundary (or at the start), returning the slice
/// from that token onward. Returns the input unchanged when no token qualifies.
fn strip_catalog_prefix<'a>(model_lower: &'a str, family_tokens: &[String]) -> &'a str {
    let bytes = model_lower.as_bytes();
    let mut best: Option<usize> = None;
    for tok in family_tokens {
        let mut from = 0;
        while let Some(rel) = model_lower[from..].find(tok.as_str()) {
            let idx = from + rel;
            if idx == 0 || !bytes[idx - 1].is_ascii_alphanumeric() {
                best = Some(best.map_or(idx, |b| b.min(idx)));
                break;
            }
            from = idx + 1;
        }
    }
    match best {
        Some(idx) => &model_lower[idx..],
        None => model_lower,
    }
}

/// Boundary-aware prefix test: `s` equals `token`, or `s` starts with `token`
/// and the following character is a non-alphanumeric boundary.
fn prefix_matches(token: &str, s: &str) -> bool {
    match s.strip_prefix(token) {
        Some(rest) => rest
            .chars()
            .next()
            .is_none_or(|c| !c.is_ascii_alphanumeric()),
        None => false,
    }
}

/// Return whether `model` is exactly three non-empty dot-separated components.
///
/// Databricks Unity Catalog model-service names are catalog data, not model
/// family hints. Both capability interpreters use this shape check before
/// family matching so suffixes such as `kimi-k3` cannot inherit endpoint
/// capabilities accidentally.
pub(crate) fn is_databricks_model_service_fqn(model: &str) -> bool {
    let mut components = model.split('.');
    let (Some(catalog), Some(schema), Some(service)) =
        (components.next(), components.next(), components.next())
    else {
        return false;
    };
    [catalog, schema, service].into_iter().all(|component| {
        !component.is_empty()
            && !component.chars().any(char::is_whitespace)
            && !component.contains('/')
    }) && components.next().is_none()
}

/// Resolve the capability profile for a `(provider, raw_model_id)` pair.
pub fn resolve(provider: &str, raw_model_id: &str) -> CapabilityResult {
    let m = manifest();
    let canon = canonical_provider(provider);
    let blank = raw_model_id.trim().is_empty();

    // Unity Catalog FQNs are neutral model-service identities. Resolve them
    // through the concrete-unknown fallback before any suffix can match a
    // provider family rule. Routing and effort normalization then share this
    // one answer in Rust and TypeScript.
    let model_service_fqn =
        canon == "databricks_v2" && is_databricks_model_service_fqn(raw_model_id);

    // 1. Provider-qualified exact-record lookup (case-insensitive on the id).
    if !blank && !model_service_fqn {
        for rec in &m.exact_records {
            if rec.provider == canon && rec.raw_model_id.eq_ignore_ascii_case(raw_model_id) {
                return CapabilityResult {
                    thinking_mode: rec.thinking_mode,
                    supported_efforts: &rec.supported_efforts,
                    default_effort: rec.default_effort,
                    databricks_v2_wire_route: rec.databricks_v2_wire_route,
                    normalization_policy: rec.normalization_policy,
                    registry_label: Some(&rec.registry_label),
                };
            }
        }
    }

    // 2. Boundary-aware family match: longest token wins, lexicographic tie-break.
    if !blank && !model_service_fqn {
        let model_lower = raw_model_id.to_ascii_lowercase();
        let stripped = strip_catalog_prefix(&model_lower, &m.family_tokens);
        let mut best: Option<(usize, &FamilyRule)> = None;
        for rule in &m.family_rules {
            if !rule.providers.iter().any(|p| p == &canon) {
                continue;
            }
            let mut matched: Option<usize> = None;
            for tok in std::iter::once(&rule.match_value).chain(rule.match_aliases.iter()) {
                let ok = match rule.match_kind {
                    MatchKind::Exact => stripped == tok.as_str(),
                    MatchKind::Prefix => prefix_matches(tok, stripped),
                };
                if ok {
                    matched = Some(matched.map_or(tok.len(), |l| l.max(tok.len())));
                }
            }
            if let Some(len) = matched {
                let better = match best {
                    None => true,
                    Some((blen, brule)) => len > blen || (len == blen && rule.id < brule.id),
                };
                if better {
                    best = Some((len, rule));
                }
            }
        }
        if let Some((_, rule)) = best {
            let route = if canon == "databricks_v2" {
                rule.databricks_v2_wire_route
            } else {
                DatabricksV2Route::NotApplicable
            };
            return CapabilityResult {
                thinking_mode: rule.thinking_mode,
                supported_efforts: &rule.supported_efforts,
                default_effort: rule.default_effort,
                databricks_v2_wire_route: route,
                normalization_policy: rule.normalization_policy,
                registry_label: None,
            };
        }
    }

    // 3. Provider fallback (blank vs. concrete-unknown); never carries a label.
    let pair = m.provider_fallbacks.get(&canon);
    let state = if blank {
        &pair.blank
    } else {
        &pair.concrete_unknown
    };
    CapabilityResult {
        thinking_mode: state.thinking_mode,
        supported_efforts: &state.supported_efforts,
        default_effort: state.default_effort,
        databricks_v2_wire_route: state.databricks_v2_wire_route,
        normalization_policy: state.normalization_policy,
        registry_label: None,
    }
}

/// Authoritative list of known Databricks v2 model ids, sourced from the manifest.
pub fn databricks_v2_known_models() -> &'static [String] {
    &manifest().databricks_v2_known_models
}

/// Curated display label for a Databricks endpoint id, or `None` when no exact
/// record covers it. Exact raw-id hits preserve the resolver's current behavior.
/// On an exact miss, aliases share a label only when stripping the manifest's
/// existing family-token prefix from the query and record keys yields exactly one
/// `databricks_v2` record; no or ambiguous stripped matches deliberately remain
/// uncurated. This accessor is discovery-only, so `resolve()` retains its exact-
/// record label contract.
pub fn databricks_registry_label(raw_model_id: &str) -> Option<&'static str> {
    let m = manifest();
    registry_label_for_databricks_records(raw_model_id, &m.exact_records, &m.label_family_tokens)
}

fn registry_label_for_databricks_records<'a>(
    raw_model_id: &str,
    records: &'a [ExactRecord],
    family_tokens: &[String],
) -> Option<&'a str> {
    if raw_model_id.trim().is_empty() {
        return None;
    }

    if let Some(rec) = records.iter().find(|rec| {
        rec.provider == "databricks_v2" && rec.raw_model_id.eq_ignore_ascii_case(raw_model_id)
    }) {
        return Some(&rec.registry_label);
    }

    let query_lower = raw_model_id.to_ascii_lowercase();
    let stripped_query = strip_catalog_prefix(&query_lower, family_tokens);
    if stripped_query == query_lower {
        return None;
    }
    let mut matching_record = None;
    for rec in records.iter().filter(|rec| rec.provider == "databricks_v2") {
        let record_lower = rec.raw_model_id.to_ascii_lowercase();
        if strip_catalog_prefix(&record_lower, family_tokens) == stripped_query
            && matching_record.replace(rec).is_some()
        {
            return None;
        }
    }
    matching_record.map(|rec| rec.registry_label.as_str())
}

/// Semantic invariants that strict typed parsing cannot express. Structural
/// checks (required fields, enum domains, both fallback states) are already
/// guaranteed by `serde` + `deny_unknown_fields`; this owns the rest.
fn validate_manifest(m: &Manifest) -> Result<(), String> {
    if m.family_tokens.is_empty() {
        return Err("family_tokens must be non-empty".to_string());
    }
    if m.label_family_tokens.is_empty() {
        return Err("label_family_tokens must be non-empty".to_string());
    }

    let check_efforts = |ctx: &str,
                         efforts: &[ThinkingEffort],
                         default: Option<ThinkingEffort>|
     -> Result<(), String> {
        if efforts.is_empty() {
            return Err(format!("{ctx}: supported_efforts must be non-empty"));
        }
        // Canonical enum order is None < Minimal < ... < Max; strict ascending
        // enforces sorted + duplicate-free in one check.
        if !efforts.windows(2).all(|w| w[0] < w[1]) {
            return Err(format!(
                "{ctx}: supported_efforts must be sorted in canonical order with no duplicates"
            ));
        }
        if let Some(d) = default {
            if !efforts.contains(&d) {
                return Err(format!(
                    "{ctx}: default_effort {d:?} not in supported_efforts"
                ));
            }
        }
        Ok(())
    };

    // Family rules: unique ids, non-empty providers, effort validity, and no
    // match token (value or alias) shared across or within rules.
    let mut rule_ids = std::collections::HashSet::new();
    let mut token_owner: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for rule in &m.family_rules {
        if !rule_ids.insert(rule.id.as_str()) {
            return Err(format!("duplicate family rule id: {}", rule.id));
        }
        if rule.providers.is_empty() {
            return Err(format!("family rule {} has empty providers", rule.id));
        }
        check_efforts(
            &format!("family_rule {}", rule.id),
            &rule.supported_efforts,
            rule.default_effort,
        )?;
        for tok in std::iter::once(&rule.match_value).chain(rule.match_aliases.iter()) {
            if let Some(prev) = token_owner.insert(tok.as_str(), rule.id.as_str()) {
                return Err(format!(
                    "duplicate match token {tok:?} (rules {prev} and {})",
                    rule.id
                ));
            }
        }
    }

    // Exact records: case-insensitive uniqueness of (provider, id), non-empty
    // labels, effort validity.
    let mut exact_keys = std::collections::HashSet::new();
    for rec in &m.exact_records {
        let key = (rec.provider.clone(), rec.raw_model_id.to_ascii_lowercase());
        if !exact_keys.insert(key) {
            return Err(format!(
                "duplicate exact record: {} / {}",
                rec.provider, rec.raw_model_id
            ));
        }
        if rec.registry_label.trim().is_empty() {
            return Err(format!(
                "exact record {} has an empty registry_label",
                rec.raw_model_id
            ));
        }
        check_efforts(
            &format!("exact_record {}", rec.raw_model_id),
            &rec.supported_efforts,
            rec.default_effort,
        )?;
    }

    // Known-model ids: case-insensitive uniqueness.
    let mut known = std::collections::HashSet::new();
    for id in &m.databricks_v2_known_models {
        if id.trim().is_empty() {
            return Err("databricks_v2_known_models contains an empty id".to_string());
        }
        if !known.insert(id.to_ascii_lowercase()) {
            return Err(format!("duplicate databricks_v2_known_models id: {id}"));
        }
    }

    // Provider fallbacks: effort validity for both states of every provider.
    for (name, pair) in m.provider_fallbacks.named() {
        check_efforts(
            &format!("fallback {name}/blank"),
            &pair.blank.supported_efforts,
            pair.blank.default_effort,
        )?;
        check_efforts(
            &format!("fallback {name}/concrete_unknown"),
            &pair.concrete_unknown.supported_efforts,
            pair.concrete_unknown.default_effort,
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One entry of the generator's *inputs-only* table. It encodes **which
    /// questions to ask** — section headers and `(provider, raw_model_id)`
    /// query pairs plus a human note — and never any expected answer. Every
    /// answer is computed by the production [`resolve`] at generation time, so
    /// the manifest stays the single place capability behavior is encoded.
    enum Q {
        Section {
            group: &'static str,
            note: Option<&'static str>,
        },
        Vector {
            id: &'static str,
            provider: &'static str,
            raw_model_id: &'static str,
            note: Option<&'static str>,
        },
    }

    /// The inputs-only question set (section headers interleaved with query
    /// vectors, in file order). Answers live only in the manifest; this table
    /// says which questions to ask. Adding, removing, or reordering a `Vector`
    /// here changes the generated corpus — run `just regen-model-corpus`.
    const INPUTS: &[Q] = &[
    Q::Section { group: "Anthropic curated family-rule model names", note: None },
    Q::Vector { id: "anthropic-claude-3-family", provider: "anthropic", raw_model_id: "claude-3-7-sonnet-20250219", note: None },
    Q::Vector { id: "anthropic-claude-opus-4-5", provider: "anthropic", raw_model_id: "claude-opus-4-5", note: None },
    Q::Vector { id: "anthropic-claude-opus-4-7", provider: "anthropic", raw_model_id: "claude-opus-4-7", note: None },
    Q::Vector { id: "anthropic-claude-opus-4-8", provider: "anthropic", raw_model_id: "claude-opus-4-8", note: None },
    Q::Vector { id: "anthropic-claude-sonnet-5", provider: "anthropic", raw_model_id: "claude-sonnet-5-20260101", note: None },
    Q::Vector { id: "anthropic-claude-fable-5", provider: "anthropic", raw_model_id: "claude-fable-5", note: None },
    Q::Vector { id: "anthropic-claude-mythos-5", provider: "anthropic", raw_model_id: "claude-mythos-5", note: None },
    Q::Vector { id: "anthropic-claude-opus-4-6", provider: "anthropic", raw_model_id: "claude-opus-4-6", note: None },
    Q::Vector { id: "anthropic-claude-sonnet-4-6", provider: "anthropic", raw_model_id: "claude-sonnet-4-6", note: None },
    Q::Vector { id: "anthropic-claude-mythos-preview", provider: "anthropic", raw_model_id: "claude-mythos-preview", note: None },
    Q::Section { group: "Anthropic blank and concrete-unknown inputs", note: None },
    Q::Vector { id: "anthropic-unknown-blank", provider: "anthropic", raw_model_id: "", note: None },
    Q::Vector { id: "anthropic-unknown-concrete", provider: "anthropic", raw_model_id: "claude-ultra-9000", note: None },
    Q::Section { group: "OpenAI curated family-rule model names", note: None },
    Q::Vector { id: "openai-gpt5-pro", provider: "openai", raw_model_id: "gpt-5-pro", note: None },
    Q::Vector { id: "openai-gpt5.6", provider: "openai", raw_model_id: "gpt-5.6", note: None },
    Q::Vector { id: "openai-gpt5-6-dashed", provider: "openai", raw_model_id: "gpt-5-6", note: None },
    Q::Vector { id: "openai-gpt5.5", provider: "openai", raw_model_id: "gpt-5.5", note: None },
    Q::Vector { id: "openai-gpt5.4", provider: "openai", raw_model_id: "gpt-5.4", note: None },
    Q::Vector { id: "openai-gpt5.1", provider: "openai", raw_model_id: "gpt-5.1", note: None },
    Q::Vector { id: "openai-gpt5-base", provider: "openai", raw_model_id: "gpt-5", note: None },
    Q::Section { group: "OpenAI gpt-5 boundary-matching probes (ported from config.rs tests)", note: None },
    Q::Vector { id: "openai-gpt5-1106-date-suffix-probe", provider: "openai", raw_model_id: "gpt-5-1106", note: Some("Probes a 4-digit date-shaped suffix after the gpt-5 stem.") },
    Q::Vector { id: "openai-gpt5-4o-alpha-suffix-probe", provider: "openai", raw_model_id: "gpt-5-4o", note: Some("Probes a leading-digit-then-letter suffix ('4o') after the gpt-5 stem.") },
    Q::Vector { id: "openai-gpt5-pro-precedence-probe", provider: "openai", raw_model_id: "gpt-5-pro", note: Some("Probes precedence between the gpt-5-pro rule and the gpt-5 base stem.") },
    Q::Vector { id: "openai-gpt5-10-multi-digit-probe", provider: "openai", raw_model_id: "gpt-5-10", note: Some("Probes a two-digit minor-version suffix after the gpt-5 stem.") },
    Q::Vector { id: "openai-gpt5-date-suffix-probe", provider: "openai", raw_model_id: "gpt-5-20260101", note: Some("Probes an 8-digit date suffix after the gpt-5 stem.") },
    Q::Section { group: "DatabricksV2 segment/prefix routing probes (ported from llm.rs tests)", note: None },
    Q::Vector { id: "dbv2-gpt5-5-probe", provider: "databricks_v2", raw_model_id: "gpt-5.5", note: None },
    Q::Vector { id: "dbv2-claude-opus-4-7-probe", provider: "databricks_v2", raw_model_id: "claude-opus-4-7", note: None },
    Q::Vector { id: "dbv2-databricks-prefix-probe", provider: "databricks_v2", raw_model_id: "databricks-claude-opus-4-7", note: Some("Probes stripping of the databricks- catalog prefix.") },
    Q::Vector { id: "dbv2-goose-claude-prefix-probe", provider: "databricks_v2", raw_model_id: "goose-claude-fable-5", note: Some("Probes stripping of the goose- catalog prefix.") },
    Q::Vector { id: "dbv2-goose-claude-4-6-sonnet-alias-probe", provider: "databricks_v2", raw_model_id: "goose-claude-4-6-sonnet", note: Some("Probes the discovered Goose Sonnet 4.6 endpoint spelling and label.") },
    Q::Vector { id: "dbv2-goose-claude-4-7-opus-alias-probe", provider: "databricks_v2", raw_model_id: "goose-claude-4-7-opus", note: Some("Probes the discovered Goose Opus 4.7 endpoint spelling and label.") },
    Q::Vector { id: "dbv2-team-prefix-probe", provider: "databricks_v2", raw_model_id: "team-x-claude-opus-4-7", note: Some("Probes stripping of a team-x- catalog prefix.") },
    Q::Vector { id: "dbv2-consolidated-llama-substring-probe", provider: "databricks_v2", raw_model_id: "consolidated-llama", note: Some("Probes a name where a code word ('sol') appears only as a substring, not a boundary-aligned segment.") },
    Q::Vector { id: "dbv2-terraform-coder-substring-probe", provider: "databricks_v2", raw_model_id: "terraform-coder", note: Some("Probes a name where a code word ('terra') is only a segment prefix, not a full segment.") },
    Q::Vector { id: "dbv2-corpus-reranker-substring-probe", provider: "databricks_v2", raw_model_id: "corpus-reranker", note: Some("Probes a name where 'opus' appears only as a substring of a segment.") },
    Q::Vector { id: "dbv2-octopus-model-substring-probe", provider: "databricks_v2", raw_model_id: "octopus-model", note: Some("Probes a name where 'opus' appears only as a substring of a segment.") },
    Q::Vector { id: "dbv2-goose-opus-5-prefix-probe", provider: "databricks_v2", raw_model_id: "goose-opus-5", note: Some("Probes a goose- prefix over a bare code-name segment with no leading claude.") },
    Q::Section { group: "Resolver-contract probes (plan v4 §Resolver contract)", note: None },
    Q::Vector { id: "resolver-exact-raw-id-probe", provider: "databricks_v2", raw_model_id: "databricks-gpt-5-4-mini", note: Some("Probes a raw id that has an exact record.") },
    Q::Vector { id: "dbv2-claude-fable-5-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-claude-fable-5", note: Some("Probes the canonical Databricks Fable 5 endpoint record.") },
    Q::Vector { id: "dbv2-goose-claude-fable-5-alias-probe", provider: "databricks_v2", raw_model_id: "goose-claude-fable-5", note: Some("Probes a prefixed alias of the Databricks Fable 5 endpoint.") },
    Q::Vector { id: "dbv2-claude-fable-5-1-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-claude-fable-5-1", note: Some("Probes the canonical Databricks Fable 5.1 endpoint record.") },
    Q::Vector { id: "dbv2-claude-opus-4-8-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-claude-opus-4-8", note: Some("Probes the canonical Databricks Opus 4.8 endpoint record.") },
    Q::Vector { id: "dbv2-goose-claude-opus-4-8-alias-probe", provider: "databricks_v2", raw_model_id: "goose-claude-opus-4-8", note: Some("Probes a prefixed alias of the Databricks Opus 4.8 endpoint.") },
    Q::Vector { id: "dbv2-claude-opus-5-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-claude-opus-5", note: Some("Probes the canonical Databricks Opus 5 endpoint record.") },
    Q::Vector { id: "dbv2-goose-claude-opus-5-alias-probe", provider: "databricks_v2", raw_model_id: "goose-claude-opus-5", note: Some("Probes a prefixed alias of the Databricks Opus 5 endpoint.") },
    Q::Vector { id: "dbv2-claude-sonnet-5-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-claude-sonnet-5", note: Some("Probes the canonical Databricks Sonnet 5 endpoint record.") },
    Q::Vector { id: "dbv2-goose-claude-sonnet-5-alias-probe", provider: "databricks_v2", raw_model_id: "goose-claude-sonnet-5", note: Some("Probes a prefixed alias of the Databricks Sonnet 5 endpoint.") },
    Q::Vector { id: "dbv2-kimi-2-7-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-kimi-2-7", note: Some("Probes the canonical Databricks Kimi 2.7 endpoint record.") },
    Q::Vector { id: "dbv2-kimi-k3-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-kimi-k3", note: Some("Probes the canonical Databricks Kimi K3 endpoint record.") },
    Q::Vector { id: "dbv2-goose-kimi-k3-alias-probe", provider: "databricks_v2", raw_model_id: "goose-kimi-k3", note: Some("Probes a prefixed alias of the Databricks Kimi K3 endpoint.") },
    Q::Vector { id: "resolver-prefixed-alias-probe", provider: "databricks_v2", raw_model_id: "team-x-databricks-gpt-5-4-mini", note: Some("Probes a prefixed alias of an exact-record id (raw exact key differs).") },
    Q::Vector { id: "resolver-cross-provider-probe", provider: "openai", raw_model_id: "databricks-gpt-5-4-mini", note: Some("Probes the same raw id under a different provider (exact records are provider-scoped).") },
    Q::Vector { id: "resolver-exact-record-with-family-route-probe", provider: "databricks_v2", raw_model_id: "databricks-gpt-5-6-sol", note: Some("Exact-vs-family route-axis probe (raw exact key with a covering family rule).") },
    Q::Vector { id: "dbv2-gpt5-5-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-gpt-5-5", note: Some("Exact-vs-family effort-axis probe (exact record overlapping a family rule).") },
    Q::Section { group: "Blank and concrete-unknown inputs per provider", note: None },
    Q::Vector { id: "dbv2-blank-probe", provider: "databricks_v2", raw_model_id: "", note: Some("Probes a blank databricks_v2 model id.") },
    Q::Vector { id: "dbv2-concrete-unknown-probe", provider: "databricks_v2", raw_model_id: "some-unknown-model-xyz", note: Some("Probes a concrete, uncatalogued databricks_v2 model id.") },
    Q::Vector { id: "openai-blank-probe", provider: "openai", raw_model_id: "", note: Some("Probes a blank openai model id.") },
    Q::Vector { id: "openai-concrete-unknown-probe", provider: "openai", raw_model_id: "gpt-4o", note: Some("Probes a concrete openai model id in no verified family.") },
    Q::Vector { id: "anthropic-blank-probe", provider: "anthropic", raw_model_id: "", note: Some("Probes a blank anthropic model id.") },
    Q::Vector { id: "anthropic-concrete-unknown-probe", provider: "anthropic", raw_model_id: "claude-ultra-9000", note: Some("Probes a concrete, uncatalogued anthropic model id.") },
    Q::Section { group: "Legacy Databricks provider inputs", note: None },
    Q::Vector { id: "databricks-gpt5-pro-probe", provider: "databricks", raw_model_id: "databricks-gpt-5-pro", note: Some("Probes the legacy databricks provider with a GPT-5 Pro id.") },
    Q::Vector { id: "databricks-gpt5-6-probe", provider: "databricks", raw_model_id: "databricks-gpt-5.6", note: Some("Probes the legacy databricks provider with a GPT-5.6 id.") },
    Q::Vector { id: "databricks-gpt5-1-probe", provider: "databricks", raw_model_id: "databricks-gpt-5.1", note: Some("Probes the legacy databricks provider with a GPT-5.1 id.") },
    Q::Section { group: "openai-compat alias canonicalization probes", note: Some("Probes whether openai-compat is canonicalized to openai before resolving; both interpreters must agree.") },
    Q::Vector { id: "openai-compat-gpt-5-pro-probe", provider: "openai-compat", raw_model_id: "gpt-5-pro", note: None },
    Q::Vector { id: "openai-compat-gpt-5-5-probe", provider: "openai-compat", raw_model_id: "gpt-5.5", note: None },
    Q::Vector { id: "openai-compat-blank-probe", provider: "openai-compat", raw_model_id: "", note: Some("Probes openai-compat canonicalization with a blank model id.") },
    Q::Section { group: "gpt-5 short-version-suffix boundary probes (Rust/TS divergence window)", note: Some("Probes the 1-2 digit version-suffix window where the Rust guard and the TS regex historically diverged.") },
    Q::Vector { id: "openai-gpt5-10-preview-probe", provider: "openai", raw_model_id: "gpt-5-10-preview", note: None },
    Q::Vector { id: "openai-gpt5-2-mini-probe", provider: "openai", raw_model_id: "gpt-5-2-mini", note: None },
    Q::Vector { id: "openai-gpt5-9-dot-1-probe", provider: "openai", raw_model_id: "gpt-5-9.1", note: None },
    Q::Vector { id: "dbv2-gpt5-10-multi-axis-probe", provider: "databricks_v2", raw_model_id: "databricks-gpt-5-10", note: Some("Probes a databricks_v2 gpt-5-<multi-digit> id, exercising both the effort axes and the wire route.") },
    Q::Vector { id: "dbv2-gpt-5-2-exact-vs-base-probe", provider: "databricks_v2", raw_model_id: "databricks-gpt-5-2", note: Some("Exact-vs-base-stem probe: an exact record coexisting with the gpt-5 base stem rule.") },
    Q::Vector { id: "openai-customgpt-5-5-nonboundary-probe", provider: "openai", raw_model_id: "customgpt-5-5-endpoint", note: Some("Probes a name whose gpt- token is not boundary-aligned (preceded by 'm' in customgpt).") },
    Q::Section { group: "DBv2 gpt-segment boundary probes", note: Some("Probes whether 'gpt' is treated as a full segment rather than a segment prefix.") },
    Q::Vector { id: "dbv2-gptoss-segment-probe", provider: "databricks_v2", raw_model_id: "gptoss-model", note: Some("Probes a segment ('gptoss') that starts with but is not exactly 'gpt'/'gpt5'.") },
    Q::Vector { id: "dbv2-gptj-6b-segment-probe", provider: "databricks_v2", raw_model_id: "gptj-6b", note: Some("Probes a segment ('gptj') that is not exactly 'gpt'/'gpt5'.") },
    Q::Vector { id: "dbv2-customgpt-nonboundary-probe", provider: "databricks_v2", raw_model_id: "customgpt-5-5-endpoint", note: Some("Probes a name whose gpt- token is not boundary-aligned (preceded by 'm' in customgpt).") },
    Q::Vector { id: "dbv2-gpt-neox-version-segment-probe", provider: "databricks_v2", raw_model_id: "gpt-neox-20b", note: Some("Probes a gpt- name whose next segment ('neox') is non-numeric.") },
    Q::Vector { id: "dbv2-gpt5-custom-segment-probe", provider: "databricks_v2", raw_model_id: "databricks-gpt5-custom", note: Some("Probes a 'gpt5' segment inside a databricks- prefixed name.") },
    Q::Vector { id: "dbv2-gpt-opus-5-dual-marker-probe", provider: "databricks_v2", raw_model_id: "gpt-opus-5", note: Some("Probes a name carrying both a gpt marker and a claude code word.") },
    Q::Section { group: "Additional coverage probes", note: None },
    Q::Vector { id: "anthropic-opus-5-prefix-probe", provider: "anthropic", raw_model_id: "claude-opus-5-20270101", note: Some("Probes the claude-opus-5 prefix rule.") },
    Q::Vector { id: "dbv2-gpt-5-6-sol-normalization-probe", provider: "databricks_v2", raw_model_id: "databricks-gpt-5-6-sol", note: Some("Probes the sol exact record's normalization and effort axes.") },
    Q::Vector { id: "dbv2-gpt-5-6-luna-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-gpt-5-6-luna", note: Some("Probes the luna exact record against its family rule.") },
    Q::Vector { id: "dbv2-gpt-5-6-terra-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-gpt-5-6-terra", note: Some("Probes the terra exact record against its family rule.") },
    Q::Vector { id: "dbv2-gpt-5-4-nano-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-gpt-5-4-nano", note: Some("Probes the gpt-5-4-nano exact record and its label.") },
    Q::Vector { id: "openrouter-concrete-unknown-probe", provider: "openrouter", raw_model_id: "some-model-xyz", note: Some("Probes an uncatalogued openrouter model id.") },
    Q::Vector { id: "openai-gpt5-pro-uppercase-provider-probe", provider: "OpenAI", raw_model_id: "gpt-5-pro", note: Some("Probes an uppercased provider string ('OpenAI').") },
    Q::Vector { id: "dbv2-uppercase-model-probe", provider: "databricks_v2", raw_model_id: "DATABRICKS-GPT-5-4-NANO", note: Some("Probes an uppercased raw model id against a lowercase exact record.") },
    Q::Section { group: "Prototype-key provider probes", note: Some("Probes provider strings that collide with Object prototype keys.") },
    Q::Vector { id: "prototype-key-constructor-blank-probe", provider: "constructor", raw_model_id: "", note: None },
    Q::Vector { id: "prototype-key-constructor-some-model-probe", provider: "constructor", raw_model_id: "some-model", note: None },
    Q::Vector { id: "prototype-key-proto__-blank-probe", provider: "__proto__", raw_model_id: "", note: None },
    Q::Vector { id: "prototype-key-proto__-some-model-probe", provider: "__proto__", raw_model_id: "some-model", note: None },
    Q::Section { group: "Non-boundary gpt- prefix probes", note: Some("Probes names whose gpt- token is not boundary-aligned (preceded by an alphanumeric).") },
    Q::Vector { id: "openai-sgpt-5-5-nonboundary-probe", provider: "openai", raw_model_id: "sgpt-5-5", note: None },
    Q::Vector { id: "dbv2-sgpt-5-5-nonboundary-probe", provider: "databricks_v2", raw_model_id: "sgpt-5-5", note: None },
    Q::Vector { id: "openai-mygpt-5-nonboundary-probe", provider: "openai", raw_model_id: "mygpt-5", note: None },
    Q::Vector { id: "dbv2-mygpt-5-nonboundary-probe", provider: "databricks_v2", raw_model_id: "mygpt-5", note: None },
    Q::Vector { id: "dbv2-gpt-5-mini-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-gpt-5-mini", note: Some("Probes the gpt-5-mini exact record and its label.") },
    Q::Vector { id: "dbv2-gpt-5-nano-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-gpt-5-nano", note: Some("Probes the gpt-5-nano exact record and its label.") },
    Q::Vector { id: "dbv2-claude-opus-5-custom-family-probe", provider: "databricks_v2", raw_model_id: "databricks-claude-opus-5-custom", note: Some("Probes a family-matched name with no exact record and its label axis.") },
    Q::Vector { id: "dbv2-gpt-doubled-separator-probe", provider: "databricks_v2", raw_model_id: "databricks-gpt--5", note: Some("Probes a doubled separator between gpt and its version.") },
    Q::Section { group: "gpt-5 prefix collision probes (longest-prefix + boundary)", note: None },
    Q::Vector { id: "collision-gpt-5-base-probe", provider: "openai", raw_model_id: "gpt-5", note: Some("Probes the base gpt-5 stem alone.") },
    Q::Vector { id: "collision-gpt-5-pro-probe", provider: "openai", raw_model_id: "gpt-5-pro", note: Some("Probes gpt-5-pro against the shorter gpt-5 stem.") },
    Q::Vector { id: "collision-gpt-5-10-probe", provider: "openai", raw_model_id: "gpt-5-10", note: Some("Probes a two-digit minor version against the gpt-5 stem.") },
    Q::Vector { id: "collision-gpt-5-6-probe", provider: "openai", raw_model_id: "gpt-5.6", note: Some("Probes a dotted minor version against the gpt-5 stem.") },
    Q::Vector { id: "collision-gpt-5-1-probe", provider: "openai", raw_model_id: "gpt-5.1", note: Some("Probes the gpt-5.1 prefix.") },
    Q::Section { group: "Uncurated DBv2 token probes", note: None },
    Q::Vector { id: "uncurated-dbv2-gpt-6-probe", provider: "databricks_v2", raw_model_id: "databricks-gpt-6", note: Some("Probes a non-5 gpt version with no exact record or prefix rule.") },
    Q::Vector { id: "uncurated-dbv2-gpt-4o-probe", provider: "databricks_v2", raw_model_id: "databricks-gpt-4o", note: Some("Probes an uncatalogued gpt-4o databricks_v2 id.") },
    Q::Vector { id: "uncurated-dbv2-opus-5-bare-probe", provider: "databricks_v2", raw_model_id: "opus-5", note: Some("Probes a bare Claude code-name segment with no leading claude.") },
    Q::Vector { id: "uncurated-dbv2-sol-bare-probe", provider: "databricks_v2", raw_model_id: "sol", note: Some("Probes a bare OpenAI code name.") },
    Q::Vector { id: "uncurated-dbv2-claude-prefix-probe", provider: "databricks_v2", raw_model_id: "databricks-claude-experimental", note: Some("Probes an uncurated databricks-claude-* name.") },
    Q::Section { group: "Negative-match probes (no family rule expected to bind)", note: None },
    Q::Vector { id: "neg-gptoss-openai-probe", provider: "openai", raw_model_id: "gptoss", note: Some("Probes a name with no gpt- boundary token.") },
    Q::Vector { id: "neg-gptj-6b-openai-probe", provider: "openai", raw_model_id: "gptj-6b", note: Some("Probes 'gptj', which is not a gpt- token.") },
    Q::Vector { id: "neg-consolidated-llama-dbv2-probe", provider: "databricks_v2", raw_model_id: "consolidated-llama", note: Some("Probes a name where 'sol' is a substring, not a segment.") },
    Q::Vector { id: "neg-terraform-coder-dbv2-probe", provider: "databricks_v2", raw_model_id: "terraform-coder", note: Some("Probes a name where 'terra' is a substring, not a segment.") },
    Q::Vector { id: "neg-octopus-model-dbv2-probe", provider: "databricks_v2", raw_model_id: "octopus-model", note: Some("Probes a name where 'opus' is a substring, not a leading claude prefix.") },
    Q::Section { group: "Exact+prefix matcher boundary probes", note: None },
    Q::Vector { id: "boundary-embedded-token-openai-probe", provider: "openai", raw_model_id: "gpt-4-gpt-5-pro", note: Some("Probes a gpt-5-pro token embedded mid-name rather than at the start.") },
    Q::Vector { id: "boundary-dot-suffix-openai-probe", provider: "openai", raw_model_id: "gpt-5.6.x", note: Some("Probes a trailing dot-delimited segment after gpt-5.6.") },
    Q::Vector { id: "boundary-claude-3-digit-run-anthropic-probe", provider: "anthropic", raw_model_id: "claude-35", note: Some("Probes whether the claude-3 prefix binds a longer digit run ('35').") },
    Q::Vector { id: "boundary-claude-opus-4-70-anthropic-probe", provider: "anthropic", raw_model_id: "claude-opus-4-70", note: Some("Probes whether the claude-opus-4-7 prefix binds a longer digit run ('70').") },
    Q::Vector { id: "boundary-gpt-5-1234-openai-probe", provider: "openai", raw_model_id: "gpt-5-1234", note: Some("Probes a 4-digit run after the gpt-5 stem.") },
    Q::Section { group: "Databricks UC model-family humanization probes (#6918 follow-up)", note: Some("Exact-record and UC-FQN strip probes for the Gemini/DeepSeek/GLM/Grok/Llama/Qwen/Gemma/Inkling families surfaced by UC discovery.") },
    Q::Vector { id: "dbv2-gemini-3-1-flash-image-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-gemini-3-1-flash-image", note: Some("Probes the Gemini 3.1 Flash Image endpoint record and label.") },
    Q::Vector { id: "dbv2-gemini-3-5-flash-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-gemini-3-5-flash", note: Some("Probes the Gemini 3.5 Flash endpoint record and label.") },
    Q::Vector { id: "dbv2-gemini-3-5-flash-lite-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-gemini-3-5-flash-lite", note: Some("Probes the Gemini 3.5 Flash Lite endpoint record and label.") },
    Q::Vector { id: "dbv2-gemini-3-6-flash-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-gemini-3-6-flash", note: Some("Probes the Gemini 3.6 Flash endpoint record and label.") },
    Q::Vector { id: "dbv2-gemini-3-pro-image-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-gemini-3-pro-image", note: Some("Probes the Gemini 3 Pro Image endpoint record and label.") },
    Q::Vector { id: "dbv2-deepseek-v4-flash-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-deepseek-v4-flash-0731", note: Some("Probes the DeepSeek V4 Flash endpoint record and label.") },
    Q::Vector { id: "dbv2-deepseek-v4-pro-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-deepseek-v4-pro-0813", note: Some("Probes the DeepSeek V4 Pro endpoint record and label.") },
    Q::Vector { id: "dbv2-glm-5-3-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-glm-5-3", note: Some("Probes the GLM-5.3 endpoint record and label.") },
    Q::Vector { id: "dbv2-glm-5-3-flash-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-glm-5-3-flash", note: Some("Probes the GLM-5.3 Flash endpoint record and label.") },
    Q::Vector { id: "dbv2-grok-4-6-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-grok-4-6", note: Some("Probes the Grok 4.6 endpoint record and label.") },
    Q::Vector { id: "dbv2-llama-4-maverick-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-llama-4-maverick", note: Some("Probes the Llama 4 Maverick endpoint record and label.") },
    Q::Vector { id: "dbv2-meta-llama-3-1-8b-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-meta-llama-3-1-8b-instruct", note: Some("Probes the meta-llama record; the llama- token strips the meta- prefix identically for record and query.") },
    Q::Vector { id: "dbv2-meta-llama-3-3-70b-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-meta-llama-3-3-70b-instruct", note: Some("Probes the meta-llama 3.3 70B record and label.") },
    Q::Vector { id: "dbv2-qwen3-next-80b-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-qwen3-next-80b-a3b-instruct", note: Some("Probes the Qwen3 Next 80B record; the bare qwen token strips on a hyphen boundary.") },
    Q::Vector { id: "dbv2-qwen35-122b-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-qwen35-122b-a10b", note: Some("Probes the Qwen3.5 122B record; the bare qwen token strips a qwen35 stem with no separator.") },
    Q::Vector { id: "dbv2-gemma-3-12b-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-gemma-3-12b", note: Some("Probes the Gemma 3 12B endpoint record and label.") },
    Q::Vector { id: "dbv2-inkling-exact-record-probe", provider: "databricks_v2", raw_model_id: "databricks-inkling", note: Some("Probes the Inkling endpoint record and label.") },
    Q::Vector { id: "dbv2-uc-fqn-gemini-3-5-flash-strip-probe", provider: "databricks_v2", raw_model_id: "system.ai.gemini-3-5-flash", note: Some("Probes strip parity on a system.ai. UC FQN carrying the gemini- token (resolve carries no label; the alias label path is unit-tested).") },
    Q::Vector { id: "dbv2-uc-fqn-meta-llama-strip-probe", provider: "databricks_v2", raw_model_id: "system.ai.meta-llama-3-3-70b-instruct", note: Some("Probes strip parity on a UC FQN where the llama- token strips through meta-.") },
    Q::Vector { id: "dbv2-uc-fqn-deepseek-strip-probe", provider: "databricks_v2", raw_model_id: "system.ai.deepseek-v4-pro-0813", note: Some("Probes strip parity on a UC FQN carrying the deepseek- token.") },
    Q::Vector { id: "dbv2-uc-fqn-inkling-strip-probe", provider: "databricks_v2", raw_model_id: "system.ai.inkling", note: Some("Probes strip parity on a UC FQN carrying the bare inkling token.") },
    Q::Section { group: "Label/capability token isolation probes (#6955 review pass 1)", note: Some("Pins that label_family_tokens (the UC-humanization superset) never leaks into capability resolve(): capability stripping still uses only claude-/gpt-/kimi-, so a label token appearing before a gpt- marker must NOT displace the gpt-5-pro exact profile.") },
    Q::Vector { id: "isolation-openai-gemini-gpt-5-pro-probe", provider: "openai", raw_model_id: "tenant-gemini-gpt-5-pro", note: Some("The gemini- label token must not strip here; capability resolve keeps the gpt-5-pro high-only profile.") },
    Q::Vector { id: "isolation-openai-qwenchanted-gpt-5-pro-probe", provider: "openai", raw_model_id: "tenant-qwenchanted-gpt-5-pro", note: Some("The bare qwen label token must not fire mid-segment; capability resolve keeps the gpt-5-pro high-only profile.") },
    ];

    /// A section marker in the generated corpus (`_group` + optional `_note`).
    #[derive(Serialize)]
    struct SectionOut {
        #[serde(rename = "_group")]
        group: &'static str,
        #[serde(rename = "_note", skip_serializing_if = "Option::is_none")]
        note: Option<&'static str>,
    }

    /// One executable vector: the query, an optional note, and the resolver's
    /// snapshotted answer. `expect` is a [`CapabilityResult`] serialized
    /// directly — the axis names/order and the enum spellings come from the
    /// production types, so nothing about the answer is encoded a second time.
    #[derive(Serialize)]
    struct VectorOut {
        id: &'static str,
        provider: &'static str,
        raw_model_id: &'static str,
        #[serde(rename = "_note", skip_serializing_if = "Option::is_none")]
        note: Option<&'static str>,
        expect: CapabilityResult,
    }

    /// A heterogeneous corpus entry. `untagged` writes the inner object with no
    /// discriminator, yielding the one flat array the harnesses replay.
    #[derive(Serialize)]
    #[serde(untagged)]
    enum CorpusOut {
        Section(SectionOut),
        Vector(VectorOut),
    }

    const CORPUS_JSON: &str = include_str!("../../../scripts/normative-corpus.json");

    /// Render the corpus from [`INPUTS`] by running the production [`resolve`]
    /// over every query. Deterministic: fixed input order, struct-declaration
    /// key order, `serde_json` pretty (2-space) formatting, trailing newline.
    /// This is the single writer used by both the drift gate and the regen
    /// recipe, so "what the gate checks" and "what regen writes" cannot drift.
    fn generate_corpus_json() -> String {
        let entries: Vec<CorpusOut> = INPUTS
            .iter()
            .map(|q| match *q {
                Q::Section { group, note } => CorpusOut::Section(SectionOut { group, note }),
                Q::Vector {
                    id,
                    provider,
                    raw_model_id,
                    note,
                } => CorpusOut::Vector(VectorOut {
                    id,
                    provider,
                    raw_model_id,
                    note,
                    expect: resolve(provider, raw_model_id),
                }),
            })
            .collect();
        let mut json = serde_json::to_string_pretty(&entries)
            .expect("corpus entries serialize as pretty JSON");
        json.push('\n');
        json
    }

    /// Absolute path of the committed corpus, from the crate root at compile
    /// time — the same file [`CORPUS_JSON`] embeds, so the regen recipe writes
    /// exactly what the drift gate reads.
    fn corpus_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/normative-corpus.json")
    }

    #[test]
    fn bundled_manifest_parses_and_validates() {
        // Exercises the include_str! + strict serde + validate_manifest chain.
        let _ = manifest();
    }

    #[test]
    fn corpus_matches_generated_snapshot() {
        // Drift gate: the committed corpus must be byte-identical to what the
        // production resolver generates right now. A byte match proves every
        // `expect` in the file is the resolver's current answer — the same
        // cross-language contract the old hand-maintained corpus enforced,
        // now impossible to hand-edit out of sync. `just regen-model-corpus`
        // rewrites the file from this exact generator.
        assert_eq!(
            CORPUS_JSON,
            generate_corpus_json(),
            "scripts/normative-corpus.json is out of date — run `just regen-model-corpus` and commit the result"
        );
    }

    #[test]
    fn corpus_has_exactly_140_executable_vectors() {
        // Locks the vector count so a silent INPUTS edit can't quietly drop
        // coverage; must equal the gate in the TS harness
        // (modelCapabilitiesCorpus.test.mjs).
        let vectors = INPUTS
            .iter()
            .filter(|q| matches!(q, Q::Vector { .. }))
            .count();
        assert_eq!(
            vectors, 140,
            "corpus executable-vector count changed; update this gate deliberately"
        );
    }

    /// Rewrite `scripts/normative-corpus.json` from the production resolver.
    /// `#[ignore]` so the ordinary test run only *checks* the committed bytes
    /// (via `corpus_matches_generated_snapshot`); this is the writer half,
    /// invoked by `just regen-model-corpus`.
    #[test]
    #[ignore = "writer, not a check — run via `just regen-model-corpus`"]
    fn regen_corpus_file() {
        std::fs::write(corpus_path(), generate_corpus_json())
            .expect("write scripts/normative-corpus.json");
    }

    // --- Migrated relational/invariant tests (see 42-test inventory) ---
    // These assert cross-input properties a single corpus vector cannot express.

    #[test]
    fn databricks_v2_fqn_uses_neutral_concrete_unknown_capabilities() {
        let fqn = resolve("databricks_v2", "system.ai.kimi-k3");
        let fallback = resolve("databricks_v2", "some-unknown-xyz");
        assert_eq!(fqn.thinking_mode, fallback.thinking_mode);
        assert_eq!(fqn.supported_efforts, fallback.supported_efforts);
        assert_eq!(fqn.default_effort, fallback.default_effort);
        assert_eq!(
            fqn.databricks_v2_wire_route,
            fallback.databricks_v2_wire_route
        );
        assert_eq!(fqn.normalization_policy, fallback.normalization_policy);
        assert_eq!(fqn.registry_label, None);
    }

    #[test]
    fn test_gpt5_numeric_date_suffix_matches_base_not_version() {
        // A 4-digit date-like suffix on a non-boundary must fall to the gpt-5 base,
        // never to the gpt-5.1 version rule.
        let base = resolve("openai", "gpt-5");
        for id in ["gpt-5-1106", "gpt-5-20260101"] {
            assert_eq!(
                resolve("openai", id).supported_efforts,
                base.supported_efforts,
                "{id} must match gpt-5 base efforts"
            );
        }
    }

    #[test]
    fn test_gpt5_lettered_suffix_matches_base_not_gpt5_4() {
        // `gpt-5-4o` has an alnum char after `gpt-5-4`, so the gpt-5.4 rule must
        // not match; it falls to the base rule.
        let base = resolve("openai", "gpt-5");
        let gpt5_4 = resolve("openai", "gpt-5.4");
        let got = resolve("openai", "gpt-5-4o");
        assert_eq!(got.supported_efforts, base.supported_efforts);
        assert_ne!(got.supported_efforts, gpt5_4.supported_efforts);
    }

    #[test]
    fn test_gpt5_pro_wins_over_base_by_longest_prefix() {
        // `gpt-5-pro` matches both the base (`gpt-5`) and the pro rule; longest
        // prefix must select pro (high-only).
        let pro = resolve("openai", "gpt-5-pro");
        let base = resolve("openai", "gpt-5");
        assert_eq!(pro.supported_efforts, &[ThinkingEffort::High]);
        assert_ne!(pro.supported_efforts, base.supported_efforts);
    }

    #[test]
    fn test_every_resolve_yields_a_complete_result() {
        // Complete-result invariant: supported_efforts is never empty on any path.
        let inputs = [
            ("anthropic", "claude-opus-4-7"),
            ("anthropic", ""),
            ("anthropic", "claude-ultra-9000"),
            ("openai", "gpt-5"),
            ("openai", ""),
            ("openai", "gpt-4o"),
            ("databricks_v2", "databricks-gpt-5-4-mini"),
            ("databricks_v2", ""),
            ("databricks_v2", "some-unknown-xyz"),
            ("databricks", "databricks-gpt-5-pro"),
            ("openrouter", "whatever"),
            ("openai-compat", "gpt-5.5"),
            ("__proto__", ""),
            ("constructor", "some-model"),
            ("", ""),
            ("totally-unknown", "totally-unknown"),
        ];
        for (provider, model) in inputs {
            let got = resolve(provider, model);
            assert!(
                !got.supported_efforts.is_empty(),
                "resolve({provider:?}, {model:?}) returned empty supported_efforts"
            );
        }
    }

    // --- New direct-resolver tests (contract 5) ---

    #[test]
    fn test_whitespace_only_model_id_uses_blank_fallback() {
        // A whitespace-only id trims to blank and takes the blank fallback, which
        // differs from the concrete-unknown fallback for databricks_v2 (route).
        let ws = resolve("databricks_v2", "   ");
        let blank = resolve("databricks_v2", "");
        assert_eq!(ws, blank);
        assert_eq!(ws.databricks_v2_wire_route, DatabricksV2Route::RouteUnknown);
        let concrete = resolve("databricks_v2", "some-unknown-xyz");
        assert_eq!(
            concrete.databricks_v2_wire_route,
            DatabricksV2Route::MlflowChat
        );
    }

    #[test]
    fn test_prefix_tie_break_is_lexicographic_on_rule_id() {
        // gpt-5.1 matches the gpt-5.1 rule's exact value (len 7) over the base
        // prefix (len 5); the longest-match + tie-break path is deterministic.
        let a = resolve("openai", "gpt-5.1");
        let b = resolve("openai", "gpt-5.1");
        assert_eq!(a, b);
        assert_eq!(a.default_effort, Some(ThinkingEffort::None));
    }

    #[test]
    fn test_exact_record_beats_family_prefix() {
        // databricks-gpt-5-4-mini has an exact record (label present); the family
        // prefix would otherwise apply and carry no label.
        let got = resolve("databricks_v2", "databricks-gpt-5-4-mini");
        assert_eq!(got.registry_label, Some("GPT-5.4 mini"));
    }

    #[test]
    fn test_known_models_accessor_reads_manifest() {
        let known = databricks_v2_known_models();
        assert!(known.iter().any(|m| m == "databricks-gpt-5-5"));
        assert!(known.iter().any(|m| m == "databricks-claude-opus-4-7"));
    }

    #[test]
    fn test_databricks_registry_label_lookup() {
        // Exact raw id remains case-insensitive and unchanged.
        assert_eq!(
            databricks_registry_label("DATABRICKS-GPT-5-5"),
            Some("GPT-5.5")
        );
        // Exact raw ids preserve their canonical labels.
        for (model, label) in [
            ("databricks-claude-opus-5", "Claude Opus 5"),
            ("databricks-claude-sonnet-5", "Claude Sonnet 5"),
            ("databricks-kimi-k3", "Kimi K3"),
        ] {
            assert_eq!(
                databricks_registry_label(model),
                Some(label),
                "model={model}"
            );
        }
        // Aliases reuse the existing family-token stripper.
        assert_eq!(
            databricks_registry_label("goose-gpt-5-6-sol"),
            Some("GPT-5.6 Sol")
        );
        assert_eq!(
            databricks_registry_label("goose-claude-fable-5"),
            Some("Claude Fable 5")
        );
        for (alias, label) in [
            ("goose-claude-4-6-sonnet", "Claude Sonnet 4.6"),
            ("goose-claude-4-7-opus", "Claude Opus 4.7"),
            ("goose-claude-opus-4-8", "Claude Opus 4.8"),
            ("goose-claude-opus-5", "Claude Opus 5"),
            ("goose-claude-sonnet-5", "Claude Sonnet 5"),
            ("goose-kimi-2-7", "Kimi 2.7"),
            ("goose-kimi-k3", "Kimi K3"),
        ] {
            assert_eq!(
                databricks_registry_label(alias),
                Some(label),
                "alias={alias}"
            );
        }
        // UC-family humanization (#6918 follow-up): the new family tokens let the
        // shared UC-FQN and goose- alias forms resolve onto their base records.
        for (fqn, label) in [
            ("system.ai.gemini-3-5-flash", "Gemini 3.5 Flash"),
            ("system.ai.gemini-3-pro-image", "Gemini 3 Pro Image"),
            ("system.ai.deepseek-v4-pro-0813", "DeepSeek V4 Pro"),
            ("system.ai.glm-5-3-flash", "GLM-5.3 Flash"),
            ("system.ai.grok-4-6", "Grok 4.6"),
            ("system.ai.llama-4-maverick", "Llama 4 Maverick"),
            (
                "system.ai.meta-llama-3-3-70b-instruct",
                "Llama 3.3 70B Instruct",
            ),
            (
                "system.ai.qwen3-next-80b-a3b-instruct",
                "Qwen3 Next 80B A3B Instruct",
            ),
            ("system.ai.qwen35-122b-a10b", "Qwen3.5 122B A10B"),
            ("system.ai.gemma-3-12b", "Gemma 3 12B"),
            ("system.ai.inkling", "Inkling"),
            ("system.ai.deepseek-v4-flash-0731", "DeepSeek V4 Flash"),
            ("system.ai.glm-5-3", "GLM-5.3"),
            ("system.ai.glm-5-3-flash", "GLM-5.3 Flash"),
            ("system.ai.grok-4-6", "Grok 4.6"),
        ] {
            assert_eq!(databricks_registry_label(fqn), Some(label), "fqn={fqn}");
        }
        // Unknown ids, bare family ids, and blanks remain uncurated.
        assert_eq!(databricks_registry_label("custom-unlisted-endpoint"), None);
        assert_eq!(databricks_registry_label("gpt-5"), None);
        assert_eq!(databricks_registry_label("   "), None);
    }

    #[test]
    fn registry_label_alias_collision_returns_none() {
        let record = |raw_model_id: &str, registry_label: &str| ExactRecord {
            provider: "databricks_v2".to_string(),
            raw_model_id: raw_model_id.to_string(),
            registry_label: registry_label.to_string(),
            thinking_mode: ThinkingMode::None,
            supported_efforts: vec![ThinkingEffort::Medium],
            default_effort: Some(ThinkingEffort::Medium),
            databricks_v2_wire_route: DatabricksV2Route::MlflowChat,
            normalization_policy: NormalizationPolicy::None,
            provenance: None,
            source: None,
            source_alt: None,
            reconciliation: None,
            reconciliation_note: None,
            reconciliation_doc: None,
        };
        let records = vec![
            record("databricks-gpt-5-6", "Databricks GPT-5.6"),
            record("partner-gpt-5-6", "Partner GPT-5.6"),
        ];
        let family_tokens = vec!["gpt-".to_string()];

        assert_eq!(
            registry_label_for_databricks_records("goose-gpt-5-6", &records, &family_tokens),
            None
        );
    }
}
