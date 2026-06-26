//! Content filtering and subscription rule matching.
//!
//! Responsibilities:
//! - Building an evalexpr context from a Nostr event
//! - Evaluating boolean filter expressions with a hard timeout
//! - Matching events against ordered subscription rules (first match wins)

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tracing::{error, warn};

/// Errors that can occur during filter expression evaluation.
#[derive(Debug, thiserror::Error)]
pub enum FilterError {
    #[error("expression too long ({len} bytes, max {max})")]
    ExpressionTooLong { len: usize, max: usize },
    #[error("evaluation timed out")]
    Timeout,
    #[error("evaluation error: {0}")]
    EvalError(String),
}

/// Variables extracted from a Nostr event for use in filter expressions.
#[derive(Debug, Clone)]
pub struct FilterContext {
    /// Event content (message body).
    pub content: String,
    /// Event author pubkey as hex string.
    pub author: String,
    /// Nostr event kind number.
    pub kind: u32,
    /// Channel UUID as string.
    pub channel_id: String,
    /// Event `created_at` unix timestamp.
    pub timestamp: u64,
}

impl FilterContext {
    /// Build a `FilterContext` from a Nostr event and its channel UUID.
    pub fn from_event(event: &nostr::Event, channel_id: uuid::Uuid) -> Self {
        Self {
            content: event.content.clone(),
            author: event.pubkey.to_hex(),
            kind: event.kind.as_u16() as u32,
            channel_id: channel_id.to_string(),
            timestamp: event.created_at.as_secs(),
        }
    }
}

/// Scope of channels a subscription rule applies to.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
pub enum ChannelScope {
    /// The literal string `"all"` — matches every channel.
    All(String),
    /// An explicit list of channel UUID strings.
    List(Vec<String>),
}

impl ChannelScope {
    /// Returns `true` if this scope covers the given channel UUID.
    ///
    /// `ChannelScope::All` only matches when the inner string is exactly `"all"`.
    pub fn matches(&self, channel_id: &uuid::Uuid) -> bool {
        match self {
            ChannelScope::All(s) => s == "all",
            ChannelScope::List(ids) => ids.iter().any(|id| id == &channel_id.to_string()),
        }
    }
}

/// A single subscription rule from the agent config.
///
/// # Thread safety
///
/// `consecutive_timeouts` is an `AtomicU32` so `match_event` can update it
/// without requiring `&mut self` — rules are shared via `Arc<[SubscriptionRule]>`
/// across the event-dispatch loop.
#[derive(Debug, serde::Deserialize)]
pub struct SubscriptionRule {
    /// Human-readable rule name; used as fallback `prompt_tag`.
    pub name: String,
    /// Which channels this rule applies to.
    pub channels: ChannelScope,
    /// Nostr event kinds to match. Empty = wildcard (all kinds).
    #[serde(default)]
    pub kinds: Vec<u32>,
    /// If `true`, the event must contain a `p` tag referencing the agent pubkey.
    #[serde(default)]
    pub require_mention: bool,
    /// Optional evalexpr boolean expression for fine-grained filtering.
    #[serde(default)]
    pub filter: Option<String>,
    /// Tag passed to the prompt template. Falls back to `name` if absent.
    #[serde(default)]
    pub prompt_tag: Option<String>,
    /// Pre-compiled evalexpr AST for the `filter` expression (finding #34).
    ///
    /// Populated by `load_rules()` at startup so `match_event` never re-parses
    /// the expression string on the hot path. `None` when `filter` is `None`
    /// or the rule was constructed without calling `load_rules()` (e.g. tests).
    #[serde(skip)]
    pub compiled_filter: Option<Arc<evalexpr::Node>>,
    /// Consecutive filter-evaluation timeout counter (finding #25).
    ///
    /// Incremented on each timeout; reset on any successful evaluation.
    /// When this reaches `MAX_CONSECUTIVE_TIMEOUTS`, the rule is treated as
    /// disabled and `match_event` returns `None` (fail-closed).
    #[serde(skip)]
    pub consecutive_timeouts: Arc<AtomicU32>,
}

impl Default for SubscriptionRule {
    fn default() -> Self {
        Self {
            name: String::new(),
            channels: ChannelScope::All("all".into()),
            kinds: Vec::new(),
            require_mention: false,
            filter: None,
            prompt_tag: None,
            compiled_filter: None,
            consecutive_timeouts: Arc::new(AtomicU32::new(0)),
        }
    }
}

impl Clone for SubscriptionRule {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            channels: self.channels.clone(),
            kinds: self.kinds.clone(),
            require_mention: self.require_mention,
            filter: self.filter.clone(),
            prompt_tag: self.prompt_tag.clone(),
            compiled_filter: self.compiled_filter.clone(),
            // Share the same counter across clones so all copies of a rule
            // agree on the timeout state.
            consecutive_timeouts: self.consecutive_timeouts.clone(),
        }
    }
}

/// The result of a successful rule match.
#[derive(Debug, Clone)]
pub struct MatchedRule {
    /// Zero-based index of the matching rule in the rules slice.
    #[cfg_attr(not(test), allow(dead_code))]
    pub rule_index: usize,
    /// Prompt tag to use (rule's `prompt_tag` or its `name`).
    pub prompt_tag: String,
}

/// Maximum expression length accepted by `evaluate_filter`.
///
/// Bounds worst-case O(2^n) evaluation paths. The spawn_blocking thread cannot
/// be cancelled after a timeout fires, so we cap length before dispatching.
const MAX_EXPR_LEN: usize = 4096;

/// Maximum wall-clock time allowed for a single evalexpr evaluation.
const EVAL_TIMEOUT: Duration = Duration::from_millis(100);

/// Maximum concurrent blocking filter evaluations (finding #13 / Issue 3).
///
/// The semaphore permit is moved into each `spawn_blocking` closure so it is
/// held until the blocking thread finishes — not just until the caller's timeout
/// fires. This truly bounds the number of live blocking evals even under repeated
/// slow expressions.
const MAX_CONCURRENT_FILTER_EVALS: usize = 4;

/// Semaphore that bounds concurrent `spawn_blocking` filter evaluations.
///
/// Wrapped in `Arc` so `acquire_owned()` can be used, which returns an
/// `OwnedSemaphorePermit` that can be moved into the `spawn_blocking` closure.
/// This ensures the permit is held until the blocking task actually finishes —
/// not just until the caller's timeout fires — so the semaphore truly bounds
/// the number of live blocking threads (finding #13 / Issue 3).
static FILTER_EVAL_SEMAPHORE: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_FILTER_EVALS)));

/// Evaluate a boolean filter expression against a `FilterContext`.
///
/// - Caps expression length at [`MAX_EXPR_LEN`] bytes.
/// - Acquires an owned permit from [`FILTER_EVAL_SEMAPHORE`] and moves it into
///   the blocking closure so it is held until the task finishes, not just until
///   the caller's timeout fires (finding #13 / Issue 3).
/// - Runs evaluation on a blocking thread with a [`EVAL_TIMEOUT`] hard timeout.
/// - When a pre-compiled `node` is provided (via `Arc`), uses
///   `node.eval_boolean_with_context()` instead of re-parsing the expression
///   string on every call (finding #34).
/// - Registers custom string helpers: `str_contains`, `str_starts_with`,
///   `str_ends_with`, `str_len` (duplicated intentionally from buzz-workflow).
pub async fn evaluate_filter(
    expr: &str,
    ctx: &FilterContext,
    node: Option<Arc<evalexpr::Node>>,
) -> Result<bool, FilterError> {
    if expr.len() > MAX_EXPR_LEN {
        return Err(FilterError::ExpressionTooLong {
            len: expr.len(),
            max: MAX_EXPR_LEN,
        });
    }

    let eval_ctx = build_eval_context(ctx).map_err(FilterError::EvalError)?;
    let expr_owned = expr.to_owned();

    // Acquire an *owned* permit so it can be moved into the spawn_blocking closure.
    // The permit is held until the blocking task actually completes — not just until
    // the caller's timeout fires — so the semaphore truly bounds the number of live
    // blocking threads even when callers time out (Issue 3 / finding #13).
    //
    // The acquire itself is bounded by EVAL_TIMEOUT: if all permits are held by
    // wedged blocking tasks, we time out instead of blocking the main event loop.
    let permit = tokio::time::timeout(
        EVAL_TIMEOUT,
        Arc::clone(&*FILTER_EVAL_SEMAPHORE).acquire_owned(),
    )
    .await
    .map_err(|_| FilterError::Timeout)?
    .map_err(|e| FilterError::EvalError(format!("semaphore closed: {e}")))?;

    let result = tokio::time::timeout(
        EVAL_TIMEOUT,
        tokio::task::spawn_blocking(move || {
            // Hold the permit for the lifetime of this closure: released only
            // when the blocking thread returns, not when the caller times out.
            let _permit = permit;
            // Use the pre-compiled AST when available; fall back to string parsing.
            if let Some(node) = node {
                node.eval_boolean_with_context(&eval_ctx)
            } else {
                evalexpr::eval_boolean_with_context(&expr_owned, &eval_ctx)
            }
        }),
    )
    .await
    .map_err(|_| FilterError::Timeout)?
    .map_err(|e| FilterError::EvalError(format!("eval task panicked: {e}")))?
    .map_err(|e| FilterError::EvalError(e.to_string()))?;

    Ok(result)
}

/// Build an `evalexpr::HashMapContext` from a `FilterContext`.
///
/// Variables exposed to expressions:
///
/// | Name         | Type   | Source                    |
/// |--------------|--------|---------------------------|
/// | `content`    | string | `event.content`           |
/// | `author`     | string | `event.pubkey` (hex)      |
/// | `kind`       | int    | `event.kind`              |
/// | `channel_id` | string | channel UUID              |
/// | `timestamp`  | int    | `event.created_at`        |
///
/// Also registers `str_contains`, `str_starts_with`, `str_ends_with`,
/// `str_len` — duplicated from buzz-workflow intentionally so this crate
/// has no runtime dependency on buzz-workflow.
fn build_eval_context(ctx: &FilterContext) -> Result<evalexpr::HashMapContext, String> {
    use evalexpr::*;

    let mut eval_ctx = HashMapContext::new();

    // evalexpr v11 does not ship these helpers; register them manually.

    eval_ctx
        .set_function(
            "str_contains".into(),
            Function::new(|args| {
                let args = args.as_fixed_len_tuple(2)?;
                let haystack = args[0].as_string()?;
                let needle = args[1].as_string()?;
                Ok(Value::Boolean(haystack.contains(needle.as_str())))
            }),
        )
        .map_err(|e| e.to_string())?;

    eval_ctx
        .set_function(
            "str_starts_with".into(),
            Function::new(|args| {
                let args = args.as_fixed_len_tuple(2)?;
                let s = args[0].as_string()?;
                let prefix = args[1].as_string()?;
                Ok(Value::Boolean(s.starts_with(prefix.as_str())))
            }),
        )
        .map_err(|e| e.to_string())?;

    eval_ctx
        .set_function(
            "str_ends_with".into(),
            Function::new(|args| {
                let args = args.as_fixed_len_tuple(2)?;
                let s = args[0].as_string()?;
                let suffix = args[1].as_string()?;
                Ok(Value::Boolean(s.ends_with(suffix.as_str())))
            }),
        )
        .map_err(|e| e.to_string())?;

    eval_ctx
        .set_function(
            "str_len".into(),
            Function::new(|arg| {
                let s = arg.as_string()?;
                Ok(Value::Int(s.len() as i64))
            }),
        )
        .map_err(|e| e.to_string())?;

    eval_ctx
        .set_value("content".into(), Value::String(ctx.content.clone()))
        .map_err(|e| e.to_string())?;
    eval_ctx
        .set_value("author".into(), Value::String(ctx.author.clone()))
        .map_err(|e| e.to_string())?;
    eval_ctx
        .set_value("kind".into(), Value::Int(ctx.kind as i64))
        .map_err(|e| e.to_string())?;
    eval_ctx
        .set_value("channel_id".into(), Value::String(ctx.channel_id.clone()))
        .map_err(|e| e.to_string())?;
    eval_ctx
        .set_value("timestamp".into(), Value::Int(ctx.timestamp as i64))
        .map_err(|e| e.to_string())?;

    Ok(eval_ctx)
}

/// Consecutive timeout threshold before a rule is treated as disabled.
///
/// After this many back-to-back timeouts on a single rule, the rule is logged
/// at ERROR level and `match_event` returns `None` (fail-closed). This prevents
/// a pathological expression from silently widening the subscription.
const MAX_CONSECUTIVE_TIMEOUTS: u32 = 5;

/// Match a Nostr event against an ordered list of subscription rules.
///
/// Rules are evaluated in order; the first rule whose conditions all pass
/// wins. Returns `None` if no rule matches.
///
/// # Matching logic (per rule)
///
/// 1. **channels** — if not `"all"`, the event's channel UUID must be in the list.
/// 2. **kinds** — if non-empty, the event kind must be in the list.
/// 3. **require_mention** — if `true`, a `p` tag matching `agent_pubkey_hex` must
///    exist. Tag kind is checked via `tag.as_slice()` for stable, library-independent
///    access (finding #45).
/// 4. **filter** — if `Some`, the evalexpr expression must evaluate to `true`.
///
/// # Fail-closed filter error handling (finding #25)
///
/// Any filter evaluation error — including timeout — causes the **entire
/// `match_event` call** to return `None` (no match for any rule). We never
/// fall through to the next rule on error because that would silently widen
/// the subscription: a broken/slow rule would let events through that were
/// meant to be gated.
///
/// After [`MAX_CONSECUTIVE_TIMEOUTS`] consecutive timeouts on a single rule,
/// that rule is logged at ERROR and the call returns `None` immediately to
/// avoid blocking the event loop indefinitely.
pub async fn match_event(
    event: &nostr::Event,
    channel_id: uuid::Uuid,
    rules: &[SubscriptionRule],
    agent_pubkey_hex: &str,
) -> Option<MatchedRule> {
    let filter_ctx = FilterContext::from_event(event, channel_id);

    for (index, rule) in rules.iter().enumerate() {
        // 1. Channel scope check.
        if !rule.channels.matches(&channel_id) {
            continue;
        }

        // 2. Kind filter (empty = wildcard).
        if !rule.kinds.is_empty() && !rule.kinds.contains(&(event.kind.as_u16() as u32)) {
            continue;
        }

        // 3. Mention check — look for a `p` tag whose first element equals
        //    agent_pubkey_hex. Uses tag.as_slice() for stable, library-independent
        //    access (finding #45) — avoids relying on the Display impl of tag kind.
        if rule.require_mention {
            let mentioned = event.tags.iter().any(|tag| {
                let s = tag.as_slice();
                s.first().map(|k| k.as_str()) == Some("p")
                    && s.get(1).map(|v| v.as_str()) == Some(agent_pubkey_hex)
            });
            if !mentioned {
                continue;
            }
        }

        // 4. Optional evalexpr filter expression.
        if let Some(expr) = &rule.filter {
            // Skip rules that have timed out too many times — treat as disabled.
            let prior_timeouts = rule.consecutive_timeouts.load(Ordering::Relaxed);
            if prior_timeouts >= MAX_CONSECUTIVE_TIMEOUTS {
                error!(
                    rule = %rule.name,
                    rule_index = index,
                    timeouts = prior_timeouts,
                    "filter rule disabled after too many consecutive timeouts; \
                     failing closed (no match for any rule)"
                );
                // Fail-closed: disabled rule → no match for this event.
                return None;
            }

            match evaluate_filter(expr, &filter_ctx, rule.compiled_filter.clone()).await {
                Ok(true) => {
                    // Successful match — reset timeout counter.
                    rule.consecutive_timeouts.store(0, Ordering::Relaxed);
                }
                Ok(false) => {
                    rule.consecutive_timeouts.store(0, Ordering::Relaxed);
                    continue;
                }
                Err(FilterError::Timeout) => {
                    let n = rule.consecutive_timeouts.fetch_add(1, Ordering::Relaxed) + 1;
                    warn!(
                        rule = %rule.name,
                        rule_index = index,
                        consecutive_timeouts = n,
                        "filter expression timed out; failing closed (no match for any rule)"
                    );
                    // Fail-closed: timeout → no match, not next rule.
                    return None;
                }
                Err(e) => {
                    warn!(
                        rule = %rule.name,
                        rule_index = index,
                        error = %e,
                        "filter expression error; failing closed (no match for any rule)"
                    );
                    // Fail-closed: any error → no match, not next rule.
                    return None;
                }
            }
        }

        // All checks passed — this rule wins.
        let prompt_tag = rule.prompt_tag.clone().unwrap_or_else(|| rule.name.clone());

        return Some(MatchedRule {
            rule_index: index,
            prompt_tag,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use uuid::Uuid;

    /// Build a minimal test event with the given kind and content.
    fn make_event(kind: u32, content: &str) -> nostr::Event {
        let keys = Keys::generate();
        EventBuilder::new(Kind::Custom(kind as u16), content)
            .tags([])
            .sign_with_keys(&keys)
            .unwrap()
    }

    /// Build a test event with an explicit `p` tag.
    fn make_event_with_p_tag(kind: u32, content: &str, p_hex: &str) -> nostr::Event {
        let keys = Keys::generate();
        let p_tag = Tag::parse(["p", p_hex]).expect("tag parse");
        EventBuilder::new(Kind::Custom(kind as u16), content)
            .tags([p_tag])
            .sign_with_keys(&keys)
            .unwrap()
    }

    fn any_channel() -> Uuid {
        Uuid::new_v4()
    }

    fn make_rule(
        name: &str,
        channels: ChannelScope,
        kinds: Vec<u32>,
        mention: bool,
        filter: Option<&str>,
        prompt_tag: Option<&str>,
    ) -> SubscriptionRule {
        SubscriptionRule {
            name: name.into(),
            channels,
            kinds,
            require_mention: mention,
            filter: filter.map(|s| s.into()),
            prompt_tag: prompt_tag.map(|s| s.into()),
            compiled_filter: None,
            consecutive_timeouts: Arc::new(AtomicU32::new(0)),
        }
    }

    #[test]
    fn test_filter_context_from_event() {
        let event = make_event(9, "hello world");
        let channel_id = any_channel();
        let ctx = FilterContext::from_event(&event, channel_id);

        assert_eq!(ctx.content, "hello world");
        assert_eq!(ctx.author, event.pubkey.to_hex());
        assert_eq!(ctx.kind, 9);
        assert_eq!(ctx.channel_id, channel_id.to_string());
        assert_eq!(ctx.timestamp, event.created_at.as_secs());
    }

    #[tokio::test]
    async fn test_evaluate_filter_str_contains() {
        let event = make_event(9, "P1 incident in production");
        let ctx = FilterContext::from_event(&event, any_channel());

        let result = evaluate_filter(r#"str_contains(content, "P1")"#, &ctx, None)
            .await
            .unwrap();
        assert!(result);

        let result = evaluate_filter(r#"str_contains(content, "P2")"#, &ctx, None)
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_evaluate_filter_kind_check() {
        let event = make_event(9, "some content");
        let ctx = FilterContext::from_event(&event, any_channel());

        let result = evaluate_filter("kind == 9", &ctx, None).await.unwrap();
        assert!(result);

        let result = evaluate_filter("kind == 1", &ctx, None).await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_evaluate_filter_too_long() {
        let event = make_event(9, "content");
        let ctx = FilterContext::from_event(&event, any_channel());

        let long_expr = "a".repeat(MAX_EXPR_LEN + 1);
        let err = evaluate_filter(&long_expr, &ctx, None).await.unwrap_err();

        assert!(matches!(
            err,
            FilterError::ExpressionTooLong { len, max }
            if len == MAX_EXPR_LEN + 1 && max == MAX_EXPR_LEN
        ));
    }

    #[tokio::test]
    async fn test_evaluate_filter_precompiled_node() {
        let event = make_event(9, "hello world");
        let ctx = FilterContext::from_event(&event, any_channel());

        let node =
            Arc::new(evalexpr::build_operator_tree(r#"str_contains(content, "hello")"#).unwrap());
        let result = evaluate_filter(r#"str_contains(content, "hello")"#, &ctx, Some(node))
            .await
            .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn test_match_event_first_match_wins() {
        let event = make_event(9, "hello");
        let channel_id = any_channel();

        let rules = vec![
            make_rule(
                "first",
                ChannelScope::All("all".into()),
                vec![],
                false,
                None,
                Some("tag-first"),
            ),
            make_rule(
                "second",
                ChannelScope::All("all".into()),
                vec![],
                false,
                None,
                Some("tag-second"),
            ),
        ];

        let matched = match_event(&event, channel_id, &rules, "").await.unwrap();
        assert_eq!(matched.rule_index, 0);
        assert_eq!(matched.prompt_tag, "tag-first");
    }

    #[tokio::test]
    async fn test_match_event_kind_filter() {
        let event = make_event(9, "hello");
        let channel_id = any_channel();

        let rules = vec![
            make_rule(
                "wrong-kind",
                ChannelScope::All("all".into()),
                vec![1],
                false,
                None,
                None,
            ),
            make_rule(
                "right-kind",
                ChannelScope::All("all".into()),
                vec![9],
                false,
                None,
                Some("matched"),
            ),
        ];

        let matched = match_event(&event, channel_id, &rules, "").await.unwrap();
        assert_eq!(matched.rule_index, 1);
        assert_eq!(matched.prompt_tag, "matched");
    }

    #[tokio::test]
    async fn test_match_event_require_mention() {
        let agent_pubkey = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

        let event_no_mention = make_event(9, "hello");
        let event_with_mention = make_event_with_p_tag(9, "hello", agent_pubkey);
        let channel_id = any_channel();

        let rules = vec![make_rule(
            "mention-only",
            ChannelScope::All("all".into()),
            vec![],
            true,
            None,
            Some("mentioned"),
        )];

        // Without mention — no match.
        let result = match_event(&event_no_mention, channel_id, &rules, agent_pubkey).await;
        assert!(result.is_none());

        // With mention — matches.
        let matched = match_event(&event_with_mention, channel_id, &rules, agent_pubkey)
            .await
            .unwrap();
        assert_eq!(matched.prompt_tag, "mentioned");
    }

    #[tokio::test]
    async fn test_match_event_no_match() {
        let event = make_event(1, "hello");
        let channel_id = any_channel();

        let rules = vec![make_rule(
            "kind-9-only",
            ChannelScope::All("all".into()),
            vec![9],
            false,
            None,
            None,
        )];

        let result = match_event(&event, channel_id, &rules, "").await;
        assert!(result.is_none());
    }

    #[test]
    fn test_channel_scope_all() {
        let scope = ChannelScope::All("all".into());
        assert!(scope.matches(&Uuid::new_v4()));
        assert!(scope.matches(&Uuid::new_v4()));
    }

    #[test]
    fn test_channel_scope_all_invalid_string() {
        // Only the literal "all" should match; other strings must not.
        let scope = ChannelScope::All("ALL".into());
        assert!(!scope.matches(&Uuid::new_v4()));

        let scope = ChannelScope::All("".into());
        assert!(!scope.matches(&Uuid::new_v4()));
    }

    #[test]
    fn test_channel_scope_list() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let id_c = Uuid::new_v4();

        let scope = ChannelScope::List(vec![id_a.to_string(), id_b.to_string()]);

        assert!(scope.matches(&id_a));
        assert!(scope.matches(&id_b));
        assert!(!scope.matches(&id_c));
    }

    #[tokio::test]
    async fn test_prompt_tag_falls_back_to_name() {
        let event = make_event(9, "hello");
        let channel_id = any_channel();

        let rules = vec![make_rule(
            "my-rule",
            ChannelScope::All("all".into()),
            vec![],
            false,
            None,
            None, // no explicit tag
        )];

        let matched = match_event(&event, channel_id, &rules, "").await.unwrap();
        assert_eq!(matched.prompt_tag, "my-rule");
    }

    #[tokio::test]
    async fn test_filter_error_fails_closed_no_fallthrough() {
        // A broken filter on rule[0] must NOT fall through to rule[1].
        let event = make_event(9, "hello");
        let channel_id = any_channel();

        let rules = vec![
            make_rule(
                "broken-filter",
                ChannelScope::All("all".into()),
                vec![],
                false,
                Some("this is not valid evalexpr syntax !!!"),
                Some("should-not-match"),
            ),
            make_rule(
                "catch-all",
                ChannelScope::All("all".into()),
                vec![],
                false,
                None,
                Some("catch-all"),
            ),
        ];

        // Must return None — not "catch-all".
        let result = match_event(&event, channel_id, &rules, "").await;
        assert!(
            result.is_none(),
            "filter error must fail closed, not fall through to next rule"
        );
    }

    #[tokio::test]
    async fn test_consecutive_timeouts_disables_rule() {
        // After MAX_CONSECUTIVE_TIMEOUTS, the rule is skipped and None returned.
        let event = make_event(9, "hello");
        let channel_id = any_channel();

        let rule = make_rule(
            "timed-out-rule",
            ChannelScope::All("all".into()),
            vec![],
            false,
            Some("kind == 9"),
            Some("should-not-match"),
        );
        // Pre-seed the counter at the threshold.
        rule.consecutive_timeouts
            .store(MAX_CONSECUTIVE_TIMEOUTS, Ordering::Relaxed);

        let rules = vec![rule];
        let result = match_event(&event, channel_id, &rules, "").await;
        assert!(result.is_none(), "disabled rule must return None");
    }
}
