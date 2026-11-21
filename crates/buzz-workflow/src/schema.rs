//! YAML/JSON workflow definition types.
//!
//! Workflow definitions are authored in YAML and stored as canonical JSON.
//! All types must round-trip through both formats without loss.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::error::WorkflowError;

/// Top-level workflow definition, authored in YAML and stored as canonical JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDef {
    /// Human-readable workflow name (must be non-empty).
    pub name: String,
    /// Optional description shown in the UI.
    #[serde(default)]
    pub description: Option<String>,
    /// The event trigger that starts this workflow.
    pub trigger: TriggerDef,
    /// Ordered list of steps to execute when triggered.
    pub steps: Vec<Step>,
    /// Whether this workflow is active. Defaults to `true`.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Trigger definition. The `on` field is the tag.
///
/// Serde internally-tagged: `on: message_posted`, `on: reaction_added`, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "on", rename_all = "snake_case")]
pub enum TriggerDef {
    /// Fires when any message is posted in the workflow's channel.
    MessagePosted {
        /// Optional evalexpr filter (flat var names, e.g. `trigger_text`).
        #[serde(default)]
        filter: Option<String>,
    },
    /// Fires when an emoji reaction is added to a message.
    ReactionAdded {
        /// Optional: only fire for this specific emoji.
        #[serde(default)]
        emoji: Option<String>,
        /// Optional evalexpr filter over the reaction context.
        #[serde(default)]
        filter: Option<String>,
    },
    /// Fires when a diff message (kind:40008) is posted in the workflow's channel.
    DiffPosted {
        /// Optional evalexpr filter expression (same variables as MessagePosted).
        #[serde(default)]
        filter: Option<String>,
    },
    /// Fires on a cron schedule.
    Schedule {
        /// Cron expression (UTC). Mutually exclusive with `interval`.
        #[serde(default)]
        cron: Option<String>,
        /// Simple interval string (e.g. "1h", "30m"). Mutually exclusive with `cron`.
        #[serde(default)]
        interval: Option<String>,
    },
    /// Fires when HTTP POST arrives at `/hooks/{id}`.
    Webhook,
}

/// A single step in a workflow definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    /// Unique step identifier within this workflow.
    pub id: String,
    /// Optional human-readable step name.
    #[serde(default)]
    pub name: Option<String>,
    /// evalexpr condition. Step is skipped (not failed) if false.
    #[serde(rename = "if", default)]
    pub if_expr: Option<String>,
    /// Maximum seconds this step may run before timing out.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// The action to perform when this step executes.
    #[serde(flatten)]
    pub action: ActionDef,
}

/// Action definition. The `action` field is the tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ActionDef {
    /// Post a message to the workflow's channel (or an override channel).
    SendMessage {
        /// Message text (supports template variables).
        text: String,
        /// Optional channel UUID override. Must be a valid UUID string.
        #[serde(default)]
        channel: Option<String>,
        /// Reply to the triggering message in its thread instead of posting a
        /// new top-level message. Only valid for message-based triggers, which
        /// carry a triggering event to reply to.
        #[serde(default)]
        reply_in_thread: bool,
    },
    /// Send a direct message to a user.
    SendDm {
        /// Recipient — pubkey hex or `{{trigger.author}}`.
        to: String,
        /// Message text (supports template variables).
        text: String,
    },
    /// Update the channel topic.
    SetChannelTopic {
        /// New topic string.
        topic: String,
    },
    /// Add an emoji reaction to the triggering message.
    AddReaction {
        /// Emoji name (e.g. `"thumbsup"`).
        emoji: String,
    },
    /// HTTP POST to an external URL.
    CallWebhook {
        /// Target URL (must be a public HTTPS endpoint).
        url: String,
        /// HTTP method override (default: `"POST"`).
        #[serde(default)]
        method: Option<String>,
        /// Additional request headers.
        #[serde(default)]
        headers: Option<HashMap<String, String>>,
        /// Request body template.
        #[serde(default)]
        body: Option<String>,
    },
    /// Suspend execution and request approval.
    RequestApproval {
        /// User mention or role (e.g. `"@release-manager"`).
        from: String,
        /// Message shown to the approver.
        message: String,
        /// Duration string (e.g. `"24h"`). Defaults to 24h.
        #[serde(default)]
        timeout: Option<String>,
    },
    /// Pause execution for a duration (e.g. `"5m"`, `"1h"`).
    Delay {
        /// Duration string (e.g. `"5m"`, `"1h"`).
        duration: String,
    },
}

impl TriggerDef {
    /// The evalexpr `filter` of a message-based trigger, if it has one.
    pub fn filter(&self) -> Option<&str> {
        match self {
            TriggerDef::MessagePosted { filter }
            | TriggerDef::ReactionAdded { filter, .. }
            | TriggerDef::DiffPosted { filter } => filter.as_deref(),
            TriggerDef::Schedule { .. } | TriggerDef::Webhook => None,
        }
    }
}

impl ActionDef {
    /// Every field whose `{{...}}` placeholders
    /// [`resolve_step_templates`](crate::executor::resolve_step_templates)
    /// substitutes. Patterns name every field so a new one must be sorted here.
    fn template_fields(&self) -> Vec<&str> {
        match self {
            ActionDef::SendMessage {
                text,
                channel,
                reply_in_thread: _,
            } => [Some(text.as_str()), channel.as_deref()]
                .into_iter()
                .flatten()
                .collect(),
            ActionDef::SendDm { to, text } => vec![to, text],
            ActionDef::SetChannelTopic { topic } => vec![topic],
            ActionDef::AddReaction { emoji } => vec![emoji],
            ActionDef::CallWebhook {
                url,
                method: _,
                headers,
                body,
            } => std::iter::once(url.as_str())
                .chain(headers.iter().flat_map(|h| h.values().map(String::as_str)))
                .chain(body.as_deref())
                .collect(),
            ActionDef::RequestApproval {
                from,
                message,
                timeout: _,
            } => vec![from, message],
            ActionDef::Delay { duration: _ } => Vec::new(),
        }
    }
}

impl WorkflowDef {
    /// True when any step performs an action that can exfiltrate channel data
    /// outside the workflow channel (`call_webhook` or `send_dm`).
    ///
    /// Definitions with such steps require elevated (owner/admin) channel
    /// authority both to save and to run — plain membership is not enough,
    /// because a workflow forwards channel content with the *owner's* standing
    /// authority long after the save (SEC-006).
    pub fn requires_elevated_authority(&self) -> bool {
        self.steps.iter().any(|s| {
            matches!(
                s.action,
                ActionDef::CallWebhook { .. } | ActionDef::SendDm { .. }
            )
        })
    }

    /// Validate the workflow definition. Returns `Err` with a descriptive message
    /// if any invariant is violated.
    pub fn validate(&self) -> Result<(), WorkflowError> {
        if self.name.trim().is_empty() {
            return Err(WorkflowError::InvalidDefinition(
                "name is required and must not be empty".into(),
            ));
        }

        if self.steps.is_empty() {
            return Err(WorkflowError::InvalidDefinition(
                "at least one step is required".into(),
            ));
        }

        // Validate step IDs are safe for use in evalexpr variable names.
        // Step IDs become variable names like `steps_{id}_output_{field}`,
        // so they must only contain alphanumeric chars and underscores.
        let valid_step_id = |id: &str| -> bool {
            !id.is_empty()
                && id.len() <= 64
                && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        };

        let mut seen_ids: HashSet<&str> = HashSet::new();
        for step in &self.steps {
            if step.id.trim().is_empty() {
                return Err(WorkflowError::InvalidDefinition(
                    "step id must not be empty".into(),
                ));
            }
            if !valid_step_id(&step.id) {
                return Err(WorkflowError::InvalidDefinition(format!(
                    "step id '{}' is invalid: must contain only alphanumeric characters and underscores",
                    step.id
                )));
            }
            if let ActionDef::SendDm { to, .. } = &step.action {
                validate_dm_recipient(to)?;
            }
            if !seen_ids.insert(step.id.as_str()) {
                return Err(WorkflowError::InvalidDefinition(format!(
                    "duplicate step id: {}",
                    step.id
                )));
            }
        }

        // `reply_in_thread` requires a triggering message to reply to. Schedule
        // and webhook triggers have none, so reject the combination at
        // definition time rather than failing silently at run time.
        let trigger_has_message = matches!(
            self.trigger,
            TriggerDef::MessagePosted { .. }
                | TriggerDef::ReactionAdded { .. }
                | TriggerDef::DiffPosted { .. }
        );
        if !trigger_has_message {
            for step in &self.steps {
                if matches!(
                    step.action,
                    ActionDef::SendMessage {
                        reply_in_thread: true,
                        ..
                    }
                ) {
                    return Err(WorkflowError::InvalidDefinition(format!(
                        "step '{}': reply_in_thread requires a message-based trigger \
                         (message_posted, reaction_added, or diff_posted); \
                         schedule and webhook triggers have no message to reply to",
                        step.id
                    )));
                }
            }
        }

        // A broken filter or `if` saves fine but then fails on every
        // evaluation: the workflow is skipped on every event, or the run fails
        // at that step. Reject what can never evaluate now instead.
        if let Some(filter) = self.trigger.filter() {
            // Filters run against a trigger built from the event alone.
            let event_ctx = crate::executor::build_eval_context(
                &crate::executor::TriggerContext::default(),
                &HashMap::new(),
            )?;
            validate_condition(filter, "trigger filter", |name| {
                evalexpr::Context::get_value(&event_ctx, name).is_some()
            })?;
        }
        for step in &self.steps {
            if let Some(expr) = &step.if_expr {
                validate_condition(
                    expr,
                    &format!("step '{}' if", step.id),
                    crate::executor::may_be_run_variable,
                )?;
            }
            validate_step_output_references(step)?;
        }

        if let TriggerDef::Schedule { cron, interval } = &self.trigger {
            if cron.is_none() && interval.is_none() {
                return Err(WorkflowError::InvalidDefinition(
                    "schedule trigger requires either 'cron' or 'interval'".into(),
                ));
            }

            if cron.is_some() && interval.is_some() {
                return Err(WorkflowError::InvalidDefinition(
                    "schedule trigger cannot specify both 'cron' and 'interval'; use one or the other".into(),
                ));
            }

            if let Some(expr) = cron {
                validate_cron(expr)?;
            }

            if let Some(dur) = interval {
                let secs = crate::executor::parse_duration_secs(dur).map_err(|_| {
                    WorkflowError::InvalidDefinition(format!(
                        "invalid interval '{dur}': expected a duration like '30m', '1h', or '60s'"
                    ))
                })?;
                // Fix 4: the cron loop ticks every 60s, so sub-minute intervals
                // can never fire correctly. Reject them at definition time.
                if secs < 60 {
                    return Err(WorkflowError::InvalidDefinition(
                        "interval must be at least 60s (cron loop ticks every 60 seconds)".into(),
                    ));
                }
            }
        }

        Ok(())
    }
}

/// Reject an evalexpr condition that fails on every evaluation.
///
/// Conditions are evaluated with `eval_boolean_with_context` on an immutable
/// context, and evalexpr 11 evaluates every operand (no short-circuiting), so
/// a syntax error, an assignment, an over-long expression, or a read of a
/// variable the context never defines fails every time. Anything else may
/// work depending on run-time values and is accepted.
fn validate_condition(
    expr: &str,
    location: &str,
    is_defined: impl Fn(&str) -> bool,
) -> Result<(), WorkflowError> {
    let invalid =
        |reason: String| WorkflowError::InvalidDefinition(format!("{location}: {reason}"));
    if expr.trim().is_empty() {
        return Err(invalid("expression is empty".into()));
    }
    let limit = crate::executor::MAX_CONDITION_LEN;
    if expr.len() > limit {
        return Err(invalid(format!(
            "expression is {} bytes, over the {limit}-byte limit",
            expr.len()
        )));
    }
    let tree = evalexpr::build_operator_tree(expr)
        .map_err(|e| invalid(format!("invalid expression '{expr}': {e}")))?;
    // The parser accepts a dangling operator (`a &&`), but evaluation
    // requires exactly this many operands.
    let missing_operand = tree.iter().any(|node| {
        use evalexpr::Operator as Op;
        let operands = match node.operator() {
            Op::Add
            | Op::Sub
            | Op::Mul
            | Op::Div
            | Op::Mod
            | Op::Exp
            | Op::Eq
            | Op::Neq
            | Op::Gt
            | Op::Lt
            | Op::Geq
            | Op::Leq
            | Op::And
            | Op::Or => 2,
            Op::Neg | Op::Not => 1,
            _ => return false,
        };
        node.children().len() != operands
    });
    if missing_operand {
        return Err(invalid(format!(
            "invalid expression '{expr}': an operator is missing an operand"
        )));
    }
    if let Some(name) = tree.iter_write_variable_identifiers().next() {
        return Err(invalid(format!(
            "'{expr}' assigns to '{name}'; compare with '==' instead"
        )));
    }
    if let Some(name) = tree
        .iter_read_variable_identifiers()
        .find(|n| !is_defined(n))
    {
        return Err(invalid(format!(
            "'{expr}' reads unknown variable '{name}'; string values need double quotes"
        )));
    }
    Ok(())
}

/// Reject a `{{steps...}}` placeholder that is not shaped
/// `steps.<id>.output.<field>`: the resolver leaves it as literal text on
/// every run. Step ids are not checked, because a run resumed after an edit
/// keeps the outputs of steps the new revision may have renamed.
fn validate_step_output_references(step: &Step) -> Result<(), WorkflowError> {
    for template in step.action.template_fields() {
        for path in crate::executor::template_variable_paths(template) {
            let Some(rest) = path.strip_prefix("steps.") else {
                continue;
            };
            if crate::executor::parse_step_output_path(rest).is_none() {
                return Err(WorkflowError::InvalidDefinition(format!(
                    "step '{}': '{{{{{path}}}}}' never resolves; \
                     reference step outputs as {{{{steps.<id>.output.<field>}}}}",
                    step.id
                )));
            }
        }
    }
    Ok(())
}

/// Enforce the recipient forms before trigger-controlled template resolution.
pub(crate) fn validate_dm_recipient(to: &str) -> Result<(), WorkflowError> {
    if to == "{{trigger.author}}" || nostr::PublicKey::from_hex(to).is_ok() {
        Ok(())
    } else {
        Err(WorkflowError::InvalidDefinition(
            "send_dm recipient must be a hex pubkey or {{trigger.author}}".into(),
        ))
    }
}

/// Validate a cron expression using the `cron` crate.
///
/// The `cron` crate requires 7 fields: `sec min hour dom month dow year`.
/// Standard 5-field cron (`min hour dom month dow`) is normalized by prepending
/// `0` (seconds) and appending `*` (any year).
fn validate_cron(expr: &str) -> Result<(), WorkflowError> {
    let normalized = normalize_cron(expr);
    normalized.parse::<cron::Schedule>().map_err(|e| {
        WorkflowError::InvalidDefinition(format!("invalid cron expression '{expr}': {e}"))
    })?;
    Ok(())
}

/// Normalize a cron expression to the 7-field format required by the `cron` crate.
///
/// - 5 fields (`min hour dom month dow`) → prepend `0` (sec), append `*` (year)
/// - 6 fields → append `*` (year)
/// - 7 fields → unchanged
pub(crate) fn normalize_cron(expr: &str) -> String {
    let field_count = expr.split_whitespace().count();
    match field_count {
        5 => format!("0 {expr} *"),
        6 => format!("{expr} *"),
        _ => expr.to_owned(),
    }
}

/// Parse a YAML workflow definition, validate it, and return the canonical JSON.
///
/// Returns `(WorkflowDef, canonical_json)` on success.
pub fn parse_yaml(yaml: &str) -> Result<(WorkflowDef, String), WorkflowError> {
    let def: WorkflowDef = serde_yaml::from_str(yaml)?;
    def.validate()?;
    let json =
        serde_json::to_string(&def).map_err(|e| WorkflowError::InvalidDefinition(e.to_string()))?;
    Ok((def, json))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_message_posted_workflow() {
        // Use single-quoted YAML strings to avoid raw-string delimiter conflicts.
        let yaml = "name: 'Incident Alert'\ndescription: 'Alert on P1 messages'\ntrigger:\n  on: message_posted\n  filter: 'str_contains(trigger_text, \"P1\")'\nsteps:\n  - id: notify\n    action: send_message\n    text: 'P1 alert'\n";
        let (def, json) = parse_yaml(yaml).expect("parse failed");
        assert_eq!(def.name, "Incident Alert");
        assert!(def.enabled); // default true
        assert_eq!(def.steps.len(), 1);
        assert_eq!(def.steps[0].id, "notify");

        let reparsed: WorkflowDef = serde_json::from_str(&json).expect("json round-trip");
        assert_eq!(reparsed.name, def.name);
    }

    #[test]
    fn parse_reaction_added_trigger() {
        let yaml = "name: Triage\ntrigger:\n  on: reaction_added\n  emoji: clipboard\n  filter: 'trigger_message_id == \"abc123\"'\nsteps:\n  - id: ack\n    action: add_reaction\n    emoji: eyes\n";
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        match &def.trigger {
            TriggerDef::ReactionAdded { emoji, filter } => {
                assert_eq!(emoji.as_deref(), Some("clipboard"));
                assert_eq!(filter.as_deref(), Some("trigger_message_id == \"abc123\""));
            }
            other => panic!("unexpected trigger: {other:?}"),
        }
    }

    #[test]
    fn parse_schedule_trigger() {
        let yaml = "name: Daily Standup\ntrigger:\n  on: schedule\n  cron: '0 9 * * 1-5'\nsteps:\n  - id: prompt\n    action: send_message\n    text: Standup time\n";
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        match &def.trigger {
            TriggerDef::Schedule { cron, .. } => {
                assert_eq!(cron.as_deref(), Some("0 9 * * 1-5"));
            }
            other => panic!("unexpected trigger: {other:?}"),
        }
    }

    #[test]
    fn parse_workflow_with_conditions() {
        // Use single-quoted YAML strings; evalexpr expressions use double quotes inside.
        let yaml = concat!(
            "name: Conditional Workflow\n",
            "trigger:\n  on: message_posted\n",
            "steps:\n",
            "  - id: escalate\n",
            "    if: 'str_contains(trigger_text, \"P1\") || str_contains(trigger_text, \"SEV1\")'\n",
            "    action: send_message\n",
            "    text: P1 escalation\n",
            "  - id: normal\n",
            "    if: '!str_contains(trigger_text, \"P1\")'\n",
            "    action: send_message\n",
            "    text: Normal message\n",
        );
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        assert_eq!(def.steps.len(), 2);
        assert!(def.steps[0].if_expr.is_some());
        assert!(def.steps[1].if_expr.is_some());
    }

    #[test]
    fn parse_all_action_types() {
        // Avoid "# in YAML values (would close r# raw strings).
        // Use unquoted or single-quoted YAML values throughout.
        let yaml = concat!(
            "name: All Actions\n",
            "trigger:\n  on: webhook\n",
            "steps:\n",
            "  - id: msg\n    action: send_message\n    text: Hello\n    channel: general\n",
            "  - id: dm\n    action: send_dm\n    to: '{{trigger.author}}'\n    text: You triggered this\n",
            "  - id: topic\n    action: set_channel_topic\n    topic: Status active\n",
            "  - id: react\n    action: add_reaction\n    emoji: white_check_mark\n",
            "  - id: hook\n    action: call_webhook\n    url: https://hooks.example.com/notify\n    method: POST\n",
            "  - id: approve\n    action: request_approval\n    from: '@manager'\n    message: Approve?\n    timeout: 4h\n",
            "  - id: wait\n    action: delay\n    duration: 5m\n",
        );
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        assert_eq!(def.steps.len(), 7);

        assert!(matches!(
            &def.steps[0].action,
            ActionDef::SendMessage { .. }
        ));
        assert!(matches!(&def.steps[1].action, ActionDef::SendDm { .. }));
        assert!(matches!(
            &def.steps[2].action,
            ActionDef::SetChannelTopic { .. }
        ));
        assert!(matches!(
            &def.steps[3].action,
            ActionDef::AddReaction { .. }
        ));
        assert!(matches!(
            &def.steps[4].action,
            ActionDef::CallWebhook { .. }
        ));
        assert!(matches!(
            &def.steps[5].action,
            ActionDef::RequestApproval { .. }
        ));
        assert!(matches!(&def.steps[6].action, ActionDef::Delay { .. }));
    }

    #[test]
    fn parse_approval_gate_example() {
        let yaml = concat!(
            "name: Deploy Approval\n",
            "trigger:\n  on: webhook\n",
            "steps:\n",
            "  - id: request\n    action: request_approval\n    from: '@engineering-lead'\n",
            "    message: Approve deploy?\n    timeout: 4h\n",
            "  - id: notify_approved\n    if: 'steps_request_output_approved == true'\n",
            "    action: send_message\n    text: Deploy approved\n",
            "  - id: notify_denied\n    if: 'steps_request_output_approved == false'\n",
            "    action: send_message\n    text: Deploy denied\n",
        );
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        assert_eq!(def.steps.len(), 3);
    }

    #[test]
    fn validate_rejects_empty_name() {
        let yaml =
            "name: ''\ntrigger:\n  on: message_posted\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n";
        let err = parse_yaml(yaml).unwrap_err();
        assert!(
            matches!(err, WorkflowError::InvalidDefinition(_)),
            "expected InvalidDefinition, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_empty_steps() {
        let yaml = "name: No Steps\ntrigger:\n  on: message_posted\nsteps: []\n";
        let err = parse_yaml(yaml).unwrap_err();
        assert!(matches!(err, WorkflowError::InvalidDefinition(_)));
    }

    #[test]
    fn validate_rejects_duplicate_step_ids() {
        let yaml = concat!(
            "name: Duplicate IDs\n",
            "trigger:\n  on: message_posted\n",
            "steps:\n",
            "  - id: step1\n    action: send_message\n    text: first\n",
            "  - id: step1\n    action: send_message\n    text: second\n",
        );
        let err = parse_yaml(yaml).unwrap_err();
        match &err {
            WorkflowError::InvalidDefinition(msg) => {
                assert!(msg.contains("duplicate"), "expected 'duplicate' in: {msg}");
            }
            other => panic!("expected InvalidDefinition, got: {other}"),
        }
    }

    #[test]
    fn validate_rejects_invalid_cron() {
        let yaml = "name: Bad Cron\ntrigger:\n  on: schedule\n  cron: not-a-cron\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n";
        let err = parse_yaml(yaml).unwrap_err();
        assert!(matches!(err, WorkflowError::InvalidDefinition(_)));
    }

    #[test]
    fn validate_rejects_schedule_without_cron_or_interval() {
        let yaml = "name: Empty Schedule\ntrigger:\n  on: schedule\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n";
        let err = parse_yaml(yaml).unwrap_err();
        assert!(matches!(err, WorkflowError::InvalidDefinition(_)));
    }

    #[test]
    fn reply_in_thread_defaults_false_and_round_trips() {
        // Absent field defaults to false.
        let yaml = "name: Auto Reply\ntrigger:\n  on: message_posted\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n";
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        match &def.steps[0].action {
            ActionDef::SendMessage {
                reply_in_thread, ..
            } => assert!(!reply_in_thread, "should default to false"),
            other => panic!("unexpected action: {other:?}"),
        }

        // Explicit true parses, and survives a JSON round-trip.
        let yaml = "name: Auto Reply\ntrigger:\n  on: message_posted\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n    reply_in_thread: true\n";
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        match &def.steps[0].action {
            ActionDef::SendMessage {
                reply_in_thread, ..
            } => assert!(reply_in_thread),
            other => panic!("unexpected action: {other:?}"),
        }
        let json = serde_json::to_string(&def).expect("serialize");
        let reparsed: WorkflowDef = serde_json::from_str(&json).expect("json round-trip");
        assert!(matches!(
            &reparsed.steps[0].action,
            ActionDef::SendMessage {
                reply_in_thread: true,
                ..
            }
        ));
    }

    #[test]
    fn validate_accepts_reply_in_thread_on_message_triggers() {
        for on in ["message_posted", "reaction_added", "diff_posted"] {
            let yaml = format!(
                "name: Auto Reply\ntrigger:\n  on: {on}\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n    reply_in_thread: true\n"
            );
            parse_yaml(&yaml)
                .unwrap_or_else(|e| panic!("reply_in_thread should be valid on {on}: {e}"));
        }
    }

    #[test]
    fn validate_rejects_reply_in_thread_on_schedule_trigger() {
        let yaml = "name: Bad\ntrigger:\n  on: schedule\n  cron: '0 9 * * 1-5'\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n    reply_in_thread: true\n";
        let err = parse_yaml(yaml).unwrap_err();
        match &err {
            WorkflowError::InvalidDefinition(msg) => {
                assert!(
                    msg.contains("reply_in_thread"),
                    "expected reply_in_thread in: {msg}"
                );
            }
            other => panic!("expected InvalidDefinition, got: {other}"),
        }
    }

    #[test]
    fn validate_rejects_reply_in_thread_on_webhook_trigger() {
        let yaml = "name: Bad\ntrigger:\n  on: webhook\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n    channel: 00000000-0000-0000-0000-000000000000\n    reply_in_thread: true\n";
        let err = parse_yaml(yaml).unwrap_err();
        assert!(matches!(err, WorkflowError::InvalidDefinition(_)));
    }

    #[test]
    fn validate_allows_reply_in_thread_false_on_schedule() {
        // Explicit `false` on a schedule trigger is fine — no message needed.
        let yaml = "name: OK\ntrigger:\n  on: schedule\n  cron: '0 9 * * 1-5'\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n    reply_in_thread: false\n";
        parse_yaml(yaml).expect("reply_in_thread: false on schedule should be valid");
    }

    #[test]
    fn enabled_defaults_to_true() {
        let yaml = "name: Test\ntrigger:\n  on: webhook\nsteps:\n  - id: s1\n    action: delay\n    duration: 1m\n";
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        assert!(def.enabled);
    }

    #[test]
    fn enabled_can_be_set_false() {
        let yaml = "name: Disabled\nenabled: false\ntrigger:\n  on: webhook\nsteps:\n  - id: s1\n    action: delay\n    duration: 1m\n";
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        assert!(!def.enabled);
    }

    #[test]
    fn parse_missing_optional_description_defaults_to_none() {
        let yaml = "name: No Desc\ntrigger:\n  on: webhook\nsteps:\n  - id: s1\n    action: delay\n    duration: 1m\n";
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        assert!(def.description.is_none());
    }

    #[test]
    fn parse_explicit_description_is_present() {
        let yaml = "name: With Desc\ndescription: 'A helpful description'\ntrigger:\n  on: webhook\nsteps:\n  - id: s1\n    action: delay\n    duration: 1m\n";
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        assert_eq!(def.description.as_deref(), Some("A helpful description"));
    }

    #[test]
    fn parse_reaction_added_without_emoji_defaults_to_none() {
        // emoji is optional on ReactionAdded — omitting it means match any emoji.
        let yaml = "name: Any Reaction\ntrigger:\n  on: reaction_added\nsteps:\n  - id: s1\n    action: add_reaction\n    emoji: eyes\n";
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        match &def.trigger {
            TriggerDef::ReactionAdded { emoji, filter } => {
                assert!(emoji.is_none(), "emoji should default to None");
                assert!(filter.is_none(), "filter should default to None");
            }
            other => panic!("unexpected trigger: {other:?}"),
        }
    }

    #[test]
    fn parse_message_posted_without_filter_defaults_to_none() {
        let yaml = "name: All Messages\ntrigger:\n  on: message_posted\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n";
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        match &def.trigger {
            TriggerDef::MessagePosted { filter } => {
                assert!(filter.is_none(), "filter should default to None");
            }
            other => panic!("unexpected trigger: {other:?}"),
        }
    }

    #[test]
    fn parse_schedule_with_interval_instead_of_cron() {
        let yaml = "name: Interval Schedule\ntrigger:\n  on: schedule\n  interval: 30m\nsteps:\n  - id: s1\n    action: send_message\n    text: tick\n";
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        match &def.trigger {
            TriggerDef::Schedule { cron, interval } => {
                assert!(cron.is_none());
                assert_eq!(interval.as_deref(), Some("30m"));
            }
            other => panic!("unexpected trigger: {other:?}"),
        }
    }

    #[test]
    fn parse_step_without_optional_name_defaults_to_none() {
        let yaml = "name: Test\ntrigger:\n  on: webhook\nsteps:\n  - id: s1\n    action: delay\n    duration: 5s\n";
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        assert!(def.steps[0].name.is_none());
    }

    #[test]
    fn parse_step_with_optional_name() {
        let yaml = concat!(
            "name: Test\ntrigger:\n  on: webhook\n",
            "steps:\n  - id: s1\n    name: 'Wait a bit'\n    action: delay\n    duration: 5s\n"
        );
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        assert_eq!(def.steps[0].name.as_deref(), Some("Wait a bit"));
    }

    #[test]
    fn parse_step_without_if_expr_defaults_to_none() {
        let yaml = "name: Test\ntrigger:\n  on: webhook\nsteps:\n  - id: s1\n    action: delay\n    duration: 5s\n";
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        assert!(def.steps[0].if_expr.is_none());
    }

    #[test]
    fn parse_step_without_timeout_defaults_to_none() {
        let yaml = "name: Test\ntrigger:\n  on: webhook\nsteps:\n  - id: s1\n    action: delay\n    duration: 5s\n";
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        assert!(def.steps[0].timeout_secs.is_none());
    }

    #[test]
    fn parse_step_with_timeout_secs() {
        let yaml = concat!(
            "name: Test\ntrigger:\n  on: webhook\n",
            "steps:\n  - id: s1\n    timeout_secs: 120\n    action: delay\n    duration: 5s\n"
        );
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        assert_eq!(def.steps[0].timeout_secs, Some(120));
    }

    #[test]
    fn parse_call_webhook_with_all_optional_fields() {
        let yaml = concat!(
            "name: Full Webhook\ntrigger:\n  on: webhook\n",
            "steps:\n",
            "  - id: call\n    action: call_webhook\n",
            "    url: https://example.com/hook\n",
            "    method: PUT\n",
            "    headers:\n      Authorization: 'Bearer token123'\n      Content-Type: application/json\n",
            "    body: '{\"key\": \"value\"}'\n",
        );
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        match &def.steps[0].action {
            ActionDef::CallWebhook {
                url,
                method,
                headers,
                body,
            } => {
                assert_eq!(url, "https://example.com/hook");
                assert_eq!(method.as_deref(), Some("PUT"));
                let hdrs = headers.as_ref().expect("headers should be present");
                assert_eq!(
                    hdrs.get("Authorization").map(|s| s.as_str()),
                    Some("Bearer token123")
                );
                assert!(body.is_some());
            }
            other => panic!("unexpected action: {other:?}"),
        }
    }

    #[test]
    fn parse_call_webhook_minimal_only_url() {
        let yaml = concat!(
            "name: Min Webhook\ntrigger:\n  on: webhook\n",
            "steps:\n  - id: call\n    action: call_webhook\n    url: https://example.com/hook\n",
        );
        let (def, _) = parse_yaml(yaml).expect("parse failed");
        match &def.steps[0].action {
            ActionDef::CallWebhook {
                url,
                method,
                headers,
                body,
            } => {
                assert_eq!(url, "https://example.com/hook");
                assert!(method.is_none());
                assert!(headers.is_none());
                assert!(body.is_none());
            }
            other => panic!("unexpected action: {other:?}"),
        }
    }

    #[test]
    fn parse_invalid_yaml_returns_error() {
        let yaml = "name: [unclosed bracket\ntrigger:\n  on: message_posted\n";
        let err = parse_yaml(yaml).unwrap_err();
        assert!(
            matches!(err, WorkflowError::InvalidYaml(_)),
            "expected InvalidYaml, got: {err}"
        );
    }

    #[test]
    fn parse_yaml_with_unknown_trigger_type_returns_error() {
        // Unknown trigger `on:` value should fail deserialization.
        let yaml = "name: Bad Trigger\ntrigger:\n  on: unknown_trigger_type\nsteps:\n  - id: s1\n    action: delay\n    duration: 1m\n";
        let err = parse_yaml(yaml).unwrap_err();
        // serde_yaml will return an InvalidYaml error for unknown enum variant.
        assert!(
            matches!(
                err,
                WorkflowError::InvalidYaml(_) | WorkflowError::InvalidDefinition(_)
            ),
            "expected parse error, got: {err}"
        );
    }

    #[test]
    fn parse_yaml_with_unknown_action_type_returns_error() {
        let yaml = concat!(
            "name: Bad Action\ntrigger:\n  on: webhook\n",
            "steps:\n  - id: s1\n    action: fly_to_moon\n    destination: moon\n",
        );
        let err = parse_yaml(yaml).unwrap_err();
        assert!(
            matches!(
                err,
                WorkflowError::InvalidYaml(_) | WorkflowError::InvalidDefinition(_)
            ),
            "expected parse error, got: {err}"
        );
    }

    #[test]
    fn canonical_json_round_trips_all_fields() {
        let yaml = concat!(
            "name: 'Full Round Trip'\n",
            "description: 'Tests all fields'\n",
            "enabled: true\n",
            "trigger:\n  on: message_posted\n  filter: 'str_contains(trigger_text, \"alert\")'\n",
            "steps:\n",
            "  - id: notify\n    name: 'Send Alert'\n    timeout_secs: 60\n",
            "    if: 'str_len(trigger_text) > 5'\n",
            "    action: send_message\n    text: 'Alert: {{trigger.text}}'\n",
        );
        let (def, json) = parse_yaml(yaml).expect("parse failed");

        let reparsed: WorkflowDef = serde_json::from_str(&json).expect("json round-trip");

        assert_eq!(reparsed.name, def.name);
        assert_eq!(reparsed.description, def.description);
        assert_eq!(reparsed.enabled, def.enabled);
        assert_eq!(reparsed.steps.len(), def.steps.len());
        assert_eq!(reparsed.steps[0].id, def.steps[0].id);
        assert_eq!(reparsed.steps[0].name, def.steps[0].name);
        assert_eq!(reparsed.steps[0].timeout_secs, def.steps[0].timeout_secs);
        assert_eq!(reparsed.steps[0].if_expr, def.steps[0].if_expr);
    }

    #[test]
    fn validate_rejects_whitespace_only_name() {
        let yaml =
            "name: '   '\ntrigger:\n  on: message_posted\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n";
        let err = parse_yaml(yaml).unwrap_err();
        assert!(
            matches!(err, WorkflowError::InvalidDefinition(_)),
            "expected InvalidDefinition for whitespace-only name, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_empty_step_id() {
        let yaml = concat!(
            "name: Empty Step ID\ntrigger:\n  on: message_posted\n",
            "steps:\n  - id: ''\n    action: send_message\n    text: hi\n",
        );
        let err = parse_yaml(yaml).unwrap_err();
        assert!(matches!(err, WorkflowError::InvalidDefinition(_)));
    }

    #[test]
    fn validate_rejects_whitespace_only_step_id() {
        let yaml = concat!(
            "name: Whitespace Step ID\ntrigger:\n  on: message_posted\n",
            "steps:\n  - id: '  '\n    action: send_message\n    text: hi\n",
        );
        let err = parse_yaml(yaml).unwrap_err();
        assert!(matches!(err, WorkflowError::InvalidDefinition(_)));
    }

    #[test]
    fn validate_accepts_valid_5_field_cron() {
        // Standard 5-field cron: min hour dom month dow
        let yaml = "name: Cron5\ntrigger:\n  on: schedule\n  cron: '0 9 * * 1-5'\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n";
        assert!(parse_yaml(yaml).is_ok(), "5-field cron should be valid");
    }

    #[test]
    fn validate_accepts_valid_6_field_cron() {
        // 6-field cron: sec min hour dom month dow
        let yaml = "name: Cron6\ntrigger:\n  on: schedule\n  cron: '0 0 9 * * 1-5'\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n";
        assert!(parse_yaml(yaml).is_ok(), "6-field cron should be valid");
    }

    #[test]
    fn validate_accepts_valid_7_field_cron() {
        // 7-field cron: sec min hour dom month dow year
        let yaml = "name: Cron7\ntrigger:\n  on: schedule\n  cron: '0 0 9 * * 1-5 *'\nsteps:\n  - id: s1\n    action: send_message\n    text: hi\n";
        assert!(parse_yaml(yaml).is_ok(), "7-field cron should be valid");
    }

    #[test]
    fn validate_rejects_three_duplicate_step_ids() {
        let yaml = concat!(
            "name: Triple Duplicate\ntrigger:\n  on: message_posted\n",
            "steps:\n",
            "  - id: step1\n    action: send_message\n    text: first\n",
            "  - id: step1\n    action: send_message\n    text: second\n",
            "  - id: step1\n    action: send_message\n    text: third\n",
        );
        let err = parse_yaml(yaml).unwrap_err();
        match &err {
            WorkflowError::InvalidDefinition(msg) => {
                assert!(msg.contains("duplicate"), "expected 'duplicate' in: {msg}");
            }
            other => panic!("expected InvalidDefinition, got: {other}"),
        }
    }

    #[test]
    fn validate_accepts_multiple_steps_with_unique_ids() {
        let yaml = concat!(
            "name: Multi Step\ntrigger:\n  on: message_posted\n",
            "steps:\n",
            "  - id: step1\n    action: send_message\n    text: first\n",
            "  - id: step2\n    action: send_message\n    text: second\n",
            "  - id: step3\n    action: send_message\n    text: third\n",
        );
        let (def, _) = parse_yaml(yaml).expect("unique step IDs should be valid");
        assert_eq!(def.steps.len(), 3);
    }

    #[test]
    fn step_id_validation_rejects_dashes() {
        // Step ID with dash would cause evalexpr to interpret as subtraction:
        // `steps_my-step_output_field` → `steps_my` minus `step_output_field`
        let yaml = concat!(
            "name: Dash Step\ntrigger:\n  on: webhook\n",
            "steps:\n  - id: my-step\n    action: send_message\n    text: hi\n",
        );
        let err = parse_yaml(yaml).unwrap_err();
        match &err {
            WorkflowError::InvalidDefinition(msg) => {
                assert!(
                    msg.contains("my-step"),
                    "error message should mention the invalid id, got: {msg}"
                );
            }
            other => panic!("expected InvalidDefinition, got: {other}"),
        }
    }

    #[test]
    fn step_id_validation_accepts_underscores() {
        // Underscores are safe in evalexpr variable names.
        let yaml = concat!(
            "name: Underscore Step\ntrigger:\n  on: webhook\n",
            "steps:\n  - id: my_step\n    action: send_message\n    text: hi\n",
        );
        let (def, _) = parse_yaml(yaml).expect("underscore step id should be valid");
        assert_eq!(def.steps[0].id, "my_step");
    }

    #[test]
    fn step_id_validation_rejects_special_chars() {
        // Special characters (semicolons, spaces, etc.) must be rejected.
        let yaml = concat!(
            "name: Special Chars\ntrigger:\n  on: webhook\n",
            "steps:\n  - id: 'step;drop table'\n    action: send_message\n    text: hi\n",
        );
        let err = parse_yaml(yaml).unwrap_err();
        assert!(
            matches!(err, WorkflowError::InvalidDefinition(_)),
            "expected InvalidDefinition for step id with special chars, got: {err}"
        );
    }

    #[test]
    fn normalize_cron_5_fields_prepends_sec_appends_year() {
        let result = normalize_cron("0 9 * * 1-5");
        assert_eq!(result, "0 0 9 * * 1-5 *");
    }

    #[test]
    fn normalize_cron_6_fields_appends_year() {
        let result = normalize_cron("0 0 9 * * 1-5");
        assert_eq!(result, "0 0 9 * * 1-5 *");
    }

    #[test]
    fn normalize_cron_7_fields_unchanged() {
        let result = normalize_cron("0 0 9 * * 1-5 *");
        assert_eq!(result, "0 0 9 * * 1-5 *");
    }

    #[test]
    fn normalize_cron_every_minute_5_fields() {
        let result = normalize_cron("* * * * *");
        assert_eq!(result, "0 * * * * * *");
    }

    #[test]
    fn validate_rejects_sub_minute_interval() {
        let yaml = "name: Too Fast\ntrigger:\n  on: schedule\n  interval: 30s\nsteps:\n  - id: s1\n    action: send_message\n    text: tick\n";
        let err = parse_yaml(yaml).unwrap_err();
        match &err {
            WorkflowError::InvalidDefinition(msg) => {
                assert!(
                    msg.contains("60s") || msg.contains("60 seconds"),
                    "error should mention 60s minimum, got: {msg}"
                );
            }
            other => panic!("expected InvalidDefinition, got: {other}"),
        }
    }

    #[test]
    fn validate_rejects_sub_minute_interval_59s() {
        let yaml = "name: Too Fast\ntrigger:\n  on: schedule\n  interval: 59s\nsteps:\n  - id: s1\n    action: send_message\n    text: tick\n";
        let err = parse_yaml(yaml).unwrap_err();
        assert!(matches!(err, WorkflowError::InvalidDefinition(_)));
    }

    #[test]
    fn validate_accepts_exactly_60s_interval() {
        let yaml = "name: Exactly 60s\ntrigger:\n  on: schedule\n  interval: 60s\nsteps:\n  - id: s1\n    action: send_message\n    text: tick\n";
        assert!(parse_yaml(yaml).is_ok(), "60s interval should be valid");
    }

    #[test]
    fn validate_accepts_interval_above_60s() {
        // 30m = 1800s, well above the 60s minimum.
        let yaml = "name: Interval Schedule\ntrigger:\n  on: schedule\n  interval: 30m\nsteps:\n  - id: s1\n    action: send_message\n    text: tick\n";
        assert!(parse_yaml(yaml).is_ok(), "30m interval should be valid");
    }

    #[test]
    fn diff_posted_trigger_roundtrips_yaml() {
        let yaml = "on: diff_posted\n";
        let trigger: TriggerDef = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(trigger, TriggerDef::DiffPosted { filter: None }));
        let back = serde_yaml::to_string(&trigger).unwrap();
        assert!(back.contains("diff_posted"));
    }

    #[test]
    fn diff_posted_trigger_with_filter_roundtrips_yaml() {
        let yaml = "on: diff_posted\nfilter: 'str_contains(trigger_text, \"src/\")'\n";
        let trigger: TriggerDef = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(
            trigger,
            TriggerDef::DiffPosted { filter: Some(_) }
        ));
    }

    // ── Expressions and templates that can never work at run time ──────────

    use crate::executor::{evaluate_condition, resolve_template, TriggerContext};
    use serde_json::{json, Value as JsonValue};

    const MESSAGE_TRIGGERS: [&str; 3] = ["message_posted", "reaction_added", "diff_posted"];

    /// YAML is a superset of JSON, so build definitions as JSON to avoid
    /// hand-escaping quotes inside expressions.
    fn with_filter(on: &str, filter: &str) -> String {
        json!({
            "name": "Filtered",
            "trigger": { "on": on, "filter": filter },
            "steps": [{ "id": "s1", "action": "send_message", "text": "hi" }],
        })
        .to_string()
    }

    fn with_steps(trigger: JsonValue, after: JsonValue) -> String {
        json!({
            "name": "Two steps",
            "trigger": trigger,
            "steps": [
                { "id": "hit", "action": "call_webhook", "url": "https://hooks.example.com/x" },
                after,
            ],
        })
        .to_string()
    }

    fn with_step_if(trigger: JsonValue, if_expr: &str) -> String {
        with_steps(
            trigger,
            json!({ "id": "after", "if": if_expr, "action": "send_message", "text": "hi" }),
        )
    }

    fn rejection(yaml: &str) -> String {
        match parse_yaml(yaml) {
            Err(WorkflowError::InvalidDefinition(msg)) => msg,
            other => panic!("expected InvalidDefinition, got: {other:?}"),
        }
    }

    /// A fully populated run: every trigger field is set, a webhook field
    /// `ref` is present, and the earlier `hit` step has produced output.
    fn runtime_context() -> (TriggerContext, HashMap<String, JsonValue>) {
        let trigger = TriggerContext {
            text: "P1 incident!".into(),
            author: "a".repeat(64),
            channel_id: "channel".into(),
            timestamp: "1700000000".into(),
            emoji: "fire".into(),
            message_id: "b".repeat(64),
            is_reply: false,
            webhook_fields: HashMap::from([("ref".to_owned(), "main".to_owned())]),
        };
        let outputs = HashMap::from([("hit".to_owned(), json!({ "status": 200, "body": "ok" }))]);
        (trigger, outputs)
    }

    #[tokio::test]
    async fn validate_rejects_trigger_filters_that_can_never_evaluate() {
        let cases = [
            ("str_contains(trigger_text, \"P1\"", "invalid expression"),
            ("trigger_text == \"a\" &&", "invalid expression"),
            ("trigger_text ==", "invalid expression"),
            ("!", "invalid expression"),
            // An unquoted value is a variable read, which no event defines.
            ("trigger_author != npub1example", "'npub1example'"),
            // evalexpr string literals use double quotes only.
            ("str_contains(trigger_text, 'P1')", "'P1'"),
            ("trigger_text == \"a\" || text == \"a\"", "'text'"),
            ("trigger_text = \"deploy\"", "'=='"),
            ("", "empty"),
            ("   ", "empty"),
        ];
        for on in MESSAGE_TRIGGERS {
            for (filter, needle) in cases {
                let msg = rejection(&with_filter(on, filter));
                assert!(
                    msg.contains("trigger filter") && msg.contains(needle),
                    "{on} {filter:?}: {msg}"
                );
            }
        }
        // Each rejected filter also fails on every real event.
        let (trigger, _) = runtime_context();
        let trigger = TriggerContext {
            webhook_fields: HashMap::new(),
            ..trigger
        };
        for (filter, _) in cases {
            assert!(
                evaluate_condition(filter, &trigger, &HashMap::new())
                    .await
                    .is_err(),
                "{filter:?} evaluated at run time, so it must not be rejected"
            );
        }
    }

    #[tokio::test]
    async fn validate_accepts_trigger_filters_over_event_variables() {
        let filters = [
            "str_contains(trigger_text, \"P1\")",
            "trigger_is_reply == false",
            "trigger_author != \"x\" && trigger_channel_id != \"\" && trigger_timestamp != \"\" \
             && trigger_emoji != \"\" && trigger_message_id != \"\"",
            "str_starts_with(trigger_text, \"P\") || str_ends_with(trigger_text, \"!\") \
             || str_len(trigger_text) > 3",
            // evalexpr built-in functions are not in the context but still resolve.
            "str::to_lowercase(trigger_text) == \"p1 incident!\"",
        ];
        for on in MESSAGE_TRIGGERS {
            for filter in filters {
                parse_yaml(&with_filter(on, filter))
                    .unwrap_or_else(|e| panic!("{on} {filter:?} rejected: {e}"));
            }
        }
        let (trigger, _) = runtime_context();
        for filter in filters {
            evaluate_condition(filter, &trigger, &HashMap::new())
                .await
                .unwrap_or_else(|e| panic!("{filter:?} failed at run time: {e}"));
        }
    }

    #[tokio::test]
    async fn validate_rejects_step_conditions_that_can_never_evaluate() {
        let cases = [
            ("steps_hit_output_status ==", "invalid expression"),
            ("text == \"P1\"", "'text'"),
            // Step outputs are `steps_<id>_output_<field>`.
            ("steps_hit_status == 200", "'steps_hit_status'"),
            ("steps_hit_output_status = 200", "'=='"),
            ("", "empty"),
        ];
        let triggers = [
            json!({ "on": "message_posted" }),
            json!({ "on": "webhook" }),
            json!({ "on": "schedule", "cron": "0 9 * * *" }),
        ];
        for trigger in triggers {
            for (if_expr, needle) in cases {
                let msg = rejection(&with_step_if(trigger.clone(), if_expr));
                assert!(
                    msg.contains("step 'after'") && msg.contains(needle),
                    "{trigger} {if_expr:?}: {msg}"
                );
            }
        }
        let (trigger, outputs) = runtime_context();
        for (if_expr, _) in cases {
            assert!(
                evaluate_condition(if_expr, &trigger, &outputs)
                    .await
                    .is_err(),
                "{if_expr:?} evaluated at run time, so it must not be rejected"
            );
        }
    }

    #[tokio::test]
    async fn validate_accepts_step_conditions_over_run_variables() {
        let conditions = [
            "steps_hit_output_status == 200",
            "trigger_is_reply == false && str_contains(trigger_text, \"P1\")",
            // Webhook body fields arrive as `trigger_<field>`.
            "trigger_ref == \"main\"",
        ];
        for on in ["message_posted", "webhook"] {
            for if_expr in conditions {
                parse_yaml(&with_step_if(json!({ "on": on }), if_expr))
                    .unwrap_or_else(|e| panic!("{on} {if_expr:?} rejected: {e}"));
            }
        }
        let (trigger, outputs) = runtime_context();
        for if_expr in conditions {
            evaluate_condition(if_expr, &trigger, &outputs)
                .await
                .unwrap_or_else(|e| panic!("{if_expr:?} failed at run time: {e}"));
        }
    }

    #[test]
    fn validate_rejects_conditions_over_the_runtime_length_limit() {
        let long = format!("{}true", "true || ".repeat(625));
        assert!(long.len() > crate::executor::MAX_CONDITION_LEN);
        assert!(rejection(&with_filter("message_posted", &long)).contains("limit"));
        assert!(
            rejection(&with_step_if(json!({ "on": "message_posted" }), &long)).contains("limit")
        );
    }

    #[test]
    fn validate_rejects_step_references_that_can_never_resolve() {
        let refs = [
            "{{steps.hit.status}}",
            "{{ steps.hit.status | truncate(3) }}",
            "{{steps.hit.output}}",
            "{{steps.hit}}",
        ];
        for r in refs {
            let after_steps = [
                json!({ "id": "after", "action": "send_message", "text": format!("status {r}") }),
                json!({ "id": "after", "action": "send_message", "text": "hi", "channel": r }),
                json!({ "id": "after", "action": "send_dm", "to": "{{trigger.author}}", "text": r }),
                json!({ "id": "after", "action": "set_channel_topic", "topic": r }),
                json!({ "id": "after", "action": "add_reaction", "emoji": r }),
                json!({ "id": "after", "action": "call_webhook", "url": r }),
                json!({ "id": "after", "action": "call_webhook", "url": "https://hooks.example.com/x", "body": r }),
                json!({ "id": "after", "action": "call_webhook", "url": "https://hooks.example.com/x", "headers": { "X-Status": r } }),
                json!({ "id": "after", "action": "request_approval", "from": r, "message": "ok?" }),
                json!({ "id": "after", "action": "request_approval", "from": "@lead", "message": r }),
            ];
            for after in after_steps {
                let msg = rejection(&with_steps(json!({ "on": "webhook" }), after.clone()));
                assert!(
                    msg.contains("step 'after'") && msg.contains("steps.<id>.output.<field>"),
                    "{after}: {msg}"
                );
            }
        }
        // The shape reported in block/buzz#8016.
        let reported = with_steps(
            json!({ "on": "message_posted" }),
            json!({ "id": "after", "action": "send_message", "text": "status {{steps.hit.status}}" }),
        );
        assert_eq!(
            rejection(&reported),
            "step 'after': '{{steps.hit.status}}' never resolves; \
             reference step outputs as {{steps.<id>.output.<field>}}"
        );
        // The resolver leaves each one literal even when `hit` has output.
        let (trigger, outputs) = runtime_context();
        for r in refs {
            let out = resolve_template(r, &trigger, &outputs).unwrap();
            assert!(out.contains("steps.hit"), "{r:?} resolved to {out:?}");
        }
    }

    #[test]
    fn validate_accepts_template_references_that_can_resolve() {
        let texts = [
            "{{steps.hit.output.status}}",
            "{{ steps.hit.output.status | truncate(2) }}",
            "{{trigger.text}} {{trigger.ref}}",
            // Not checked: a resumed run can carry outputs saved under an
            // earlier revision of the definition.
            "{{steps.renamed.output.status}}",
            // Not references: the resolver emits these as written.
            "{{steps.hit",
            "{{ anything else }}",
        ];
        for text in texts {
            let yaml = with_steps(
                json!({ "on": "webhook" }),
                json!({ "id": "after", "action": "send_message", "text": text }),
            );
            parse_yaml(&yaml).unwrap_or_else(|e| panic!("{text:?} rejected: {e}"));
        }
        let (trigger, outputs) = runtime_context();
        assert_eq!(
            resolve_template(texts[0], &trigger, &outputs).unwrap(),
            "200"
        );
    }
}
