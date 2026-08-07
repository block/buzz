//! Per-turn model routing.
//!
//! Picks the model for an inbound turn instead of always using the agent's
//! configured default. The decision is applied through the EXISTING
//! [`OwnedAgent::desired_model`](crate::pool::OwnedAgent) mechanism that
//! `switch_model` already uses, so nothing new touches the ACP wire, the relay,
//! or the trust boundary — in particular this needs no owner-signed kind:24200
//! control frame, because the decision is made in-process by the harness that is
//! already trusted to run the turn.
//!
//! # Opt-in, and fails open
//!
//! Routing is off unless `BUZZ_ROUTING_POLICY` names a readable policy file. A
//! missing file, unparseable JSON, `enabled: false`, no matching rule, or a
//! classifier that errors or times out all resolve to "no opinion" — the turn
//! proceeds on the agent's configured model exactly as before. Routing must never
//! be able to fail a turn; a router that can block work is worse than no router.
//!
//! # Two stages, cheap first
//!
//! 1. `rules` — deterministic substring/regex-free matchers over the prompt text.
//!    No network, no latency. Most routing intent is expressible here.
//! 2. `classifier` — an optional local Ollama call, used only when no rule
//!    matched. Local by design: this code sees raw channel content, so shipping
//!    every turn's text to a hosted classifier to decide where to send it would
//!    leak exactly what a routing decision is supposed to protect.
//!
//! # What this does NOT do
//!
//! It selects a MODEL, not a harness. One buzz-acp process serves one agent, so
//! routing a turn to opencode-vs-codex-vs-claude means choosing a different agent
//! — that is a dispatcher concern (there is none today: selection is a `p`-tag
//! mention with relay fan-out) and is deliberately out of scope here.

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

/// Env var naming the policy file. Absent => routing disabled.
pub const POLICY_ENV: &str = "BUZZ_ROUTING_POLICY";

/// How a rule matches the prompt text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    /// Any of `any` appears in the prompt, case-insensitively.
    Contains,
    /// Every one of `any` appears in the prompt, case-insensitively.
    ContainsAll,
}

impl Default for MatchKind {
    fn default() -> Self {
        Self::Contains
    }
}

/// One deterministic routing rule.
#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    /// Human label, surfaced in the log line explaining a routing decision.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub match_kind: MatchKind,
    /// Needles to look for. An empty list never matches — a rule that matches
    /// everything must be written as the policy's `default_model` instead, so
    /// "always this model" cannot be created by accident.
    #[serde(default)]
    pub any: Vec<String>,
    /// Model id to use when this rule matches. Must be a model the agent's
    /// provider actually advertises, or the existing apply step logs a miss and
    /// falls back to the agent default.
    pub model: String,
}

/// Shared needle check for both the model [`Rule`] and the [`HarnessRule`], so
/// the two matchers cannot drift. An empty `any` never matches — a rule that
/// matches everything must be expressed as a `default_*`, not by omission.
fn any_contains(kind: MatchKind, any: &[String], haystack_lower: &str) -> bool {
    if any.is_empty() {
        return false;
    }
    let hit = |needle: &String| {
        let n = needle.trim().to_lowercase();
        !n.is_empty() && haystack_lower.contains(&n)
    };
    match kind {
        MatchKind::Contains => any.iter().any(hit),
        MatchKind::ContainsAll => any.iter().all(hit),
    }
}

impl Rule {
    fn matches(&self, haystack_lower: &str) -> bool {
        any_contains(self.match_kind, &self.any, haystack_lower)
    }
}

/// Optional local classifier, consulted only when no rule matched.
#[derive(Debug, Clone, Deserialize)]
pub struct Classifier {
    /// Ollama base url, e.g. `http://localhost:11434`.
    pub url: String,
    /// Ollama model id doing the classifying, e.g. `gemma3:27b`.
    pub model: String,
    /// Map from a classifier label to the model to run the turn on.
    #[serde(default)]
    pub labels: Vec<LabelTarget>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    20_000
}

#[derive(Debug, Clone, Deserialize)]
pub struct LabelTarget {
    pub label: String,
    pub model: String,
}

/// Harness-class routing — an optional sibling of the model-routing fields.
///
/// Where the model router picks a *model* for one agent's process, this picks a
/// *harness class* (claude / opencode / codex) that should own a turn. It is
/// consumed by an ingress "decline gate": each harness-agent runs the same
/// deterministic decision and simply skips a turn another class owns, since the
/// relay already delivered that turn to every subscribed process. It therefore
/// mutates nothing and emits no wire frame — strictly less privileged than the
/// model router.
///
/// Deterministic rules ONLY, by design: the decision is distributed across
/// independent processes, so it must be reproducible in each one. A non-
/// deterministic classifier could make two processes disagree and yield zero
/// handlers (a dropped turn), which violates the fail-open contract. Semantic
/// harness routing needs a single decision authority and is out of scope here —
/// hence there is deliberately no `classifier` field.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessRouting {
    /// This process's own harness class. `None` => the caller supplies the
    /// class derived from the agent's command, so one shared harness block is
    /// portable across every agent in a group.
    #[serde(default)]
    pub self_class: Option<String>,
    /// Class that owns any turn no rule matched. `None` => never decline on a
    /// no-match (the process handles it — fail-open).
    #[serde(default)]
    pub default_class: Option<String>,
    /// Deterministic prompt -> class rules, reusing [`MatchKind`].
    #[serde(default)]
    pub rules: Vec<HarnessRule>,
}

/// A [`Rule`] whose target is a harness `class` rather than a `model`.
#[derive(Debug, Clone, Deserialize)]
pub struct HarnessRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub match_kind: MatchKind,
    #[serde(default)]
    pub any: Vec<String>,
    pub class: String,
}

impl HarnessRule {
    fn matches(&self, haystack_lower: &str) -> bool {
        any_contains(self.match_kind, &self.any, haystack_lower)
    }
}

/// Why a harness class was chosen — the analogue of [`Reason`], carried into
/// the log so a decline is never silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessReason {
    Rule(String),
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessDecision {
    pub class: String,
    pub reason: HarnessReason,
}

/// A routing policy, loaded from `$BUZZ_ROUTING_POLICY`.
#[derive(Debug, Clone, Deserialize)]
pub struct Policy {
    /// Off by default so dropping a file in place cannot silently start routing.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub classifier: Option<Classifier>,
    /// Model used when nothing matched. `None` => leave the agent's default alone.
    #[serde(default)]
    pub default_model: Option<String>,
    /// Optional harness-class routing. Absent => the harness decline gate is off
    /// (fail-open), identical to today's behavior.
    #[serde(default)]
    pub harness: Option<HarnessRouting>,
}

/// Why a model was chosen — carried into the log so a routing decision is never
/// silent. An operator debugging "why did this turn use that model" needs the
/// reason, not just the outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    Rule(String),
    Classifier { label: String },
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub model: String,
    pub reason: Reason,
}

impl Policy {
    /// Load from `$BUZZ_ROUTING_POLICY`. Returns `None` when the var is unset,
    /// the file is unreadable, or the JSON does not parse — routing is a
    /// convenience, so a broken policy degrades to "no routing" rather than
    /// preventing the harness from starting.
    pub fn from_env() -> Option<Self> {
        let path = PathBuf::from(std::env::var_os(POLICY_ENV)?);
        match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<Self>(&raw) {
                Ok(policy) => Some(policy),
                Err(e) => {
                    tracing::warn!(
                        target: "acp::routing",
                        path = %path.display(),
                        error = %e,
                        "routing policy failed to parse — routing disabled for this process"
                    );
                    None
                }
            },
            Err(e) => {
                tracing::warn!(
                    target: "acp::routing",
                    path = %path.display(),
                    error = %e,
                    "routing policy unreadable — routing disabled for this process"
                );
                None
            }
        }
    }

    /// Deterministic stage. Pure: no IO, so it is fully testable and adds no
    /// latency to a turn.
    pub fn decide_static(&self, prompt: &str) -> Option<Decision> {
        if !self.enabled {
            return None;
        }
        let lower = prompt.to_lowercase();
        for (i, rule) in self.rules.iter().enumerate() {
            if rule.matches(&lower) {
                return Some(Decision {
                    model: rule.model.clone(),
                    reason: Reason::Rule(
                        rule.name.clone().unwrap_or_else(|| format!("rules[{i}]")),
                    ),
                });
            }
        }
        None
    }

    /// The fallback applied when neither a rule nor the classifier decided.
    pub fn fallback(&self) -> Option<Decision> {
        if !self.enabled {
            return None;
        }
        self.default_model.as_ref().map(|m| Decision {
            model: m.clone(),
            reason: Reason::Default,
        })
    }

    /// Which harness class should own this turn. `None` => no opinion
    /// (fail-open). Deterministic and IO-free, mirroring [`decide_static`], so
    /// every process reaches the same answer and exactly one handles the turn.
    ///
    /// [`decide_static`]: Policy::decide_static
    pub fn decide_harness(&self, prompt: &str) -> Option<HarnessDecision> {
        if !self.enabled {
            return None;
        }
        let h = self.harness.as_ref()?;
        let lower = prompt.to_lowercase();
        for (i, rule) in h.rules.iter().enumerate() {
            if rule.matches(&lower) {
                return Some(HarnessDecision {
                    class: rule.class.clone(),
                    reason: HarnessReason::Rule(
                        rule.name
                            .clone()
                            .unwrap_or_else(|| format!("harness.rules[{i}]")),
                    ),
                });
            }
        }
        h.default_class.clone().map(|class| HarnessDecision {
            class,
            reason: HarnessReason::Default,
        })
    }

    /// Should the process whose class is `self_class` DECLINE this turn?
    ///
    /// Returns `Some(target)` only when a *different* class owns the turn — the
    /// caller then skips its local enqueue and a matching-class agent takes it.
    /// Returns `None` in every fail-open case: routing disabled, no harness
    /// block, no rule matched with no `default_class`, or the target equals this
    /// process's class. The harness block's `self_class` overrides the passed
    /// `self_class`, so one shared block is portable across a group.
    pub fn harness_decline(&self, prompt: &str, self_class: &str) -> Option<HarnessDecision> {
        let self_class = self
            .harness
            .as_ref()
            .and_then(|h| h.self_class.as_deref())
            .unwrap_or(self_class);
        match self.decide_harness(prompt) {
            Some(d) if !d.class.eq_ignore_ascii_case(self_class) => Some(d),
            _ => None,
        }
    }

    /// Full decision for a turn: rules, then classifier, then default.
    ///
    /// Never returns an error. Any classifier failure is logged and treated as
    /// "no opinion", so the turn falls through to `default_model` or the agent's
    /// own configured model.
    pub async fn decide(&self, prompt: &str) -> Option<Decision> {
        if !self.enabled || prompt.trim().is_empty() {
            return None;
        }
        if let Some(d) = self.decide_static(prompt) {
            return Some(d);
        }
        if let Some(classifier) = self.classifier.as_ref() {
            match classify_ollama(classifier, prompt).await {
                Ok(Some(label)) => {
                    if let Some(target) = classifier
                        .labels
                        .iter()
                        .find(|l| l.label.eq_ignore_ascii_case(label.trim()))
                    {
                        return Some(Decision {
                            model: target.model.clone(),
                            reason: Reason::Classifier {
                                label: target.label.clone(),
                            },
                        });
                    }
                    tracing::debug!(
                        target: "acp::routing",
                        label = %label,
                        "classifier returned a label with no configured target — using default"
                    );
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(
                    target: "acp::routing",
                    error = %e,
                    "classifier call failed — using default"
                ),
            }
        }
        self.fallback()
    }
}

/// Ask a local Ollama model to pick one label. Returns `Ok(None)` when the reply
/// is unusable — an unparseable classification is not an error worth failing a
/// turn over.
async fn classify_ollama(cfg: &Classifier, prompt: &str) -> Result<Option<String>, String> {
    if cfg.labels.is_empty() {
        return Ok(None);
    }
    let labels: Vec<&str> = cfg.labels.iter().map(|l| l.label.as_str()).collect();
    // Ask for a bare label rather than JSON: there is exactly one field wanted,
    // and a one-word reply cannot be half-parsed the way a JSON object can.
    let instruction = format!(
        "Classify the task below into exactly one of these categories: {}.\n\
         Reply with ONLY the category word. No punctuation, no explanation.\n\n\
         TASK:\n{}",
        labels.join(", "),
        prompt
    );
    let body = serde_json::json!({
        "model": cfg.model,
        "stream": false,
        "options": { "temperature": 0 },
        "messages": [{ "role": "user", "content": instruction }],
    });
    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{}/api/chat", cfg.url.trim_end_matches('/')))
        .timeout(Duration::from_millis(cfg.timeout_ms))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("ollama request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("ollama HTTP {}", resp.status()));
    }
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("ollama response parse failed: {e}"))?;
    let text = v
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if text.is_empty() {
        return Ok(None);
    }
    // Small models sometimes answer in a sentence. Accept the first configured
    // label that appears anywhere in the reply rather than discarding it.
    let lower = text.to_lowercase();
    for l in &labels {
        if lower.contains(&l.to_lowercase()) {
            return Ok(Some((*l).to_string()));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(json: serde_json::Value) -> Policy {
        serde_json::from_value(json).expect("policy should parse")
    }

    #[test]
    fn disabled_policy_never_decides() {
        let p = policy(serde_json::json!({
            "enabled": false,
            "rules": [{ "any": ["migration"], "model": "m1" }],
            "default_model": "fallback"
        }));
        assert_eq!(p.decide_static("a migration"), None);
        assert_eq!(p.fallback(), None);
    }

    #[test]
    fn first_matching_rule_wins_and_carries_its_name() {
        let p = policy(serde_json::json!({
            "enabled": true,
            "rules": [
                { "name": "db", "any": ["migration", "schema"], "model": "codex-model" },
                { "name": "ui", "any": ["button"], "model": "ui-model" }
            ]
        }));
        let d = p.decide_static("Add a Postgres MIGRATION for members").unwrap();
        assert_eq!(d.model, "codex-model");
        assert_eq!(d.reason, Reason::Rule("db".into()));
        // Case-insensitive, and the later rule still reachable.
        assert_eq!(p.decide_static("fix the Button").unwrap().model, "ui-model");
        // No match => no opinion, NOT the first rule.
        assert_eq!(p.decide_static("unrelated text"), None);
    }

    #[test]
    fn contains_all_requires_every_needle() {
        let p = policy(serde_json::json!({
            "enabled": true,
            "rules": [{
                "name": "both", "match_kind": "contains_all",
                "any": ["relay", "membership"], "model": "m"
            }]
        }));
        assert!(p.decide_static("relay membership check").is_some());
        assert!(p.decide_static("relay only").is_none());
    }

    #[test]
    fn an_empty_needle_list_never_matches() {
        // Guards against "always route here" being created by omission — that
        // intent must be expressed as default_model.
        let p = policy(serde_json::json!({
            "enabled": true,
            "rules": [{ "any": [], "model": "everything" }],
            "default_model": "fallback"
        }));
        assert_eq!(p.decide_static("literally anything"), None);
        assert_eq!(p.fallback().unwrap().model, "fallback");
    }

    #[test]
    fn absent_default_model_leaves_the_agent_alone() {
        let p = policy(serde_json::json!({ "enabled": true, "rules": [] }));
        assert_eq!(p.fallback(), None);
    }

    #[test]
    fn harness_rules_pick_a_class_and_an_absent_block_is_fail_open() {
        let p = policy(serde_json::json!({
            "enabled": true,
            "rules": [],
            "harness": {
                "default_class": "claude",
                "rules": [
                    { "name": "db", "match_kind": "contains_all", "any": ["migration"], "class": "codex" },
                    { "name": "ui", "any": ["button"], "class": "opencode" }
                ]
            }
        }));

        // A matched rule names the owning class, case-insensitively.
        let d = p.decide_harness("write the MIGRATION").unwrap();
        assert_eq!(d.class, "codex");
        assert_eq!(d.reason, HarnessReason::Rule("db".into()));

        // Same turn: the codex process owns it => no decline; the claude process
        // declines toward codex.
        assert!(p.harness_decline("write the migration", "codex").is_none());
        assert_eq!(
            p.harness_decline("write the migration", "claude").unwrap().class,
            "codex"
        );

        // An unmatched turn falls to default_class; a codex process declines
        // toward claude, a claude process handles it.
        assert_eq!(
            p.harness_decline("just chatting", "codex").unwrap().class,
            "claude"
        );
        assert!(p.harness_decline("just chatting", "claude").is_none());

        // self_class in the block overrides the passed identity.
        let owned = policy(serde_json::json!({
            "enabled": true, "rules": [],
            "harness": { "self_class": "codex", "default_class": "claude" }
        }));
        assert_eq!(
            owned.harness_decline("anything", "opencode").unwrap().class,
            "claude"
        );

        // No harness block => never declines (unchanged behavior).
        let bare = policy(serde_json::json!({ "enabled": true, "rules": [] }));
        assert!(bare.harness_decline("anything", "codex").is_none());
        assert_eq!(bare.decide_harness("anything"), None);

        // Disabled policy => no opinion even with a harness block.
        let off = policy(serde_json::json!({
            "enabled": false,
            "harness": { "default_class": "codex" }
        }));
        assert!(off.harness_decline("x", "claude").is_none());
    }

    #[test]
    fn a_harness_block_with_a_classifier_field_is_rejected() {
        // Guards R2: a distributed decline cannot use a non-deterministic
        // classifier, so the field must not exist. deny_unknown_fields makes the
        // mistake loud rather than silently ignoring it.
        let err = serde_json::from_value::<Policy>(serde_json::json!({
            "enabled": true,
            "harness": {
                "default_class": "codex",
                "classifier": { "url": "http://x", "model": "m" }
            }
        }));
        assert!(err.is_err(), "a classifier inside harness must not parse");
    }

    #[tokio::test]
    async fn empty_prompt_and_unreachable_classifier_both_degrade_to_default() {
        let p = policy(serde_json::json!({
            "enabled": true,
            "rules": [],
            // Port 1 is reserved and never listening, so this exercises the
            // failure path without depending on a live Ollama.
            "classifier": {
                "url": "http://127.0.0.1:1", "model": "gemma3:27b", "timeout_ms": 500,
                "labels": [{ "label": "code", "model": "code-model" }]
            },
            "default_model": "fallback"
        }));
        assert_eq!(p.decide("").await, None, "empty prompt must not route");
        let d = p.decide("some real task text").await.unwrap();
        assert_eq!(d.model, "fallback");
        assert_eq!(d.reason, Reason::Default);
    }

    /// Live classifier check against a real Ollama. Skips unless
    /// `BUZZ_ROUTING_LIVE_OLLAMA` names a base url, matching the existing
    /// env-gated pattern in `crates/buzz-test-client/tests/e2e_mesh_llm.rs` —
    /// the unit tests above cover the logic, but only a live model proves the
    /// prompt actually elicits a usable one-word label.
    ///
    ///   BUZZ_ROUTING_LIVE_OLLAMA=http://localhost:11434 \
    ///     cargo test -p buzz-acp --lib -- routing::tests::live_ollama --nocapture
    #[tokio::test]
    async fn live_ollama_classifier_returns_a_configured_label() {
        let Ok(url) = std::env::var("BUZZ_ROUTING_LIVE_OLLAMA") else {
            eprintln!("SKIP: BUZZ_ROUTING_LIVE_OLLAMA not set — needs a live Ollama endpoint");
            return;
        };
        let model =
            std::env::var("BUZZ_ROUTING_LIVE_MODEL").unwrap_or_else(|_| "gemma3:27b".to_string());
        let p = policy(serde_json::json!({
            "enabled": true,
            "rules": [],
            "classifier": {
                "url": url, "model": model, "timeout_ms": 120000,
                "labels": [
                    { "label": "database", "model": "db-model" },
                    { "label": "frontend", "model": "ui-model" }
                ]
            },
            "default_model": "fallback"
        }));

        let d = p
            .decide("Write the Postgres migration adding a unique index on relay_members.")
            .await
            .expect("a decision");
        eprintln!("live classifier -> {:?}", d);
        assert_eq!(
            d.model, "db-model",
            "a migration task should classify as database, got {:?}",
            d.reason
        );
        assert_eq!(
            d.reason,
            Reason::Classifier {
                label: "database".into()
            }
        );
    }

    /// The other half of the desktop contract. Buzz Desktop writes this file
    /// (`commands/agent_routing_policy.rs`) and points `BUZZ_ROUTING_POLICY` at
    /// it; `from_env` swallows a parse failure by design, so a field rename
    /// would disable routing silently. This document is byte-for-byte what
    /// `policy_shape_matches_the_harness_contract` asserts the desktop emits —
    /// if one side is renamed, one of the two tests fails.
    #[test]
    fn a_desktop_written_policy_parses() {
        let raw = r#"{
          "enabled": true,
          "rules": [
            {
              "name": "db",
              "match_kind": "contains_all",
              "any": ["migration"],
              "model": "codex-model"
            }
          ],
          "classifier": {
            "url": "http://localhost:11434",
            "model": "gemma3:27b",
            "labels": [{ "label": "database", "model": "db-model" }],
            "timeout_ms": 20000
          },
          "default_model": "fallback"
        }"#;

        let p: Policy = serde_json::from_str(raw).expect("desktop-written policy must parse");
        assert!(p.enabled);
        assert_eq!(p.rules[0].match_kind, MatchKind::ContainsAll);
        assert_eq!(p.rules[0].model, "codex-model");
        assert_eq!(p.default_model.as_deref(), Some("fallback"));
        let classifier = p.classifier.as_ref().expect("classifier");
        assert_eq!(classifier.timeout_ms, 20_000);
        assert_eq!(classifier.labels[0].model, "db-model");

        // A rule the desktop saved must actually route.
        let decision = p.decide_static("write the migration").expect("a decision");
        assert_eq!(decision.model, "codex-model");
    }

    /// The desktop omits `classifier` and `default_model` when unset
    /// (`skip_serializing_if`). That minimal document must still load.
    #[test]
    fn a_minimal_desktop_policy_parses() {
        let p: Policy = serde_json::from_str(r#"{"enabled": false, "rules": []}"#).expect("parse");
        assert!(!p.enabled);
        assert!(p.classifier.is_none());
        assert!(p.default_model.is_none());
    }

    #[test]
    fn a_broken_policy_file_disables_routing_rather_than_erroring() {
        assert!(serde_json::from_str::<Policy>("{ not json").is_err());
        // from_env's contract: unparseable => None (verified by the type above;
        // from_env itself is exercised by the runtime probe, not here, because it
        // reads process env).
    }
}
