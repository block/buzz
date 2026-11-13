//! Git permission types — ref patterns, protection rules, and policy evaluation inputs.
//!
//! This module defines the core data types for the Buzz git permission system.
//! The legacy permission model maps channel role to repository role, while
//! `buzz-protect` tags on kind:30617 add constraints that apply to everyone
//! (including the owner). Repositories may opt in to explicit, expiring actor
//! grants; those grants are checked before the existing protection rules.
//!
//! # Architecture
//!
//! ```text
//! kind:30617 tags → parse → Vec<ProtectionRule>
//!                                    ↓
//! push arrives → classify refs → match patterns → union rules → enforce
//! ```

use crate::channel::MemberRole;
use std::fmt;

/// Machine-readable token prefixing the push-policy denial for a kind:30617
/// announcement with no `buzz-channel` binding.
///
/// This is a **declared cross-component contract**, not a log string. Known
/// consumers switch on it:
/// - relay `api/git/policy.rs` — produces [`GIT_NO_CHANNEL_BINDING_BODY`]
/// - desktop `src-tauri/commands/project_git_workflow.rs` — merge-failure
///   classifier maps it to a structured `no_channel_binding` error code
/// - desktop `src/features/projects/lib/projectBranchErrors.ts` — dialog
///   copy matcher (TS re-types the literal; its test pins the value)
pub const GIT_NO_CHANNEL_BINDING_TOKEN: &str = "no_channel_binding";

/// Full push-policy denial body for an unbound repository.
///
/// Format: `<token>: <legacy phrase>`. The trailing prose deliberately
/// repeats the token's meaning because desktops already in the field match
/// the exact phrase `no channel binding` (spaces, not underscores — the
/// token alone would NOT satisfy that matcher). Do not "fix" the redundancy:
/// removing the phrase silently breaks every shipped desktop, and removing
/// the token breaks the structured consumers above. A relay-side test pins
/// both matchers.
pub const GIT_NO_CHANNEL_BINDING_BODY: &str =
    "no_channel_binding: repository has no channel binding";

/// Maximum number of `buzz-protect` tags per repo.
pub const MAX_PROTECTION_RULES: usize = 50;
/// Maximum character length of a ref pattern.
pub const MAX_PATTERN_LENGTH: usize = 256;
/// Maximum number of wildcard segments per pattern.
pub const MAX_WILDCARDS_PER_PATTERN: usize = 3;
/// Maximum number of explicit Git push grants on one repository announcement.
pub const MAX_GIT_PUSH_GRANTS: usize = 50;
/// Maximum lifetime of an explicit Git push grant: 24 hours.
pub const MAX_GIT_PUSH_GRANT_TTL_SECS: u64 = 24 * 60 * 60;

/// A validated ref pattern for matching git refs.
///
/// Grammar: `segment ("/" segment)*` where segment is either a literal
/// `[a-zA-Z0-9._-]+` or `*` (matches exactly one path segment).
///
/// Patterns MUST start with `refs/`. No `**`, `?`, `[...]`, or partial globs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefPattern {
    /// The original pattern string (e.g., "refs/heads/*").
    raw: String,
    /// Pre-split segments for matching.
    segments: Vec<PatternSegment>,
}

/// A single segment in a ref pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PatternSegment {
    /// Matches exactly this literal string.
    Literal(String),
    /// Matches any single path segment.
    Wildcard,
    /// Matches one or more path segments (recursive). Must be the last segment.
    RecursiveWildcard,
}

/// Errors from parsing a ref pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternError {
    /// Pattern is empty.
    Empty,
    /// Pattern exceeds maximum length.
    TooLong,
    /// Pattern doesn't start with `refs/`.
    MissingRefsPrefix,
    /// A segment contains invalid characters or is a partial glob.
    InvalidSegment(String),
    /// Too many wildcard segments.
    TooManyWildcards,
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "pattern is empty"),
            Self::TooLong => write!(f, "pattern exceeds {MAX_PATTERN_LENGTH} chars"),
            Self::MissingRefsPrefix => write!(f, "pattern must start with 'refs/'"),
            Self::InvalidSegment(s) => write!(f, "invalid segment: {s:?}"),
            Self::TooManyWildcards => {
                write!(f, "pattern exceeds {MAX_WILDCARDS_PER_PATTERN} wildcards")
            }
        }
    }
}

impl std::error::Error for PatternError {}

impl RefPattern {
    /// Parse and validate a ref pattern string.
    pub fn parse(pattern: &str) -> Result<Self, PatternError> {
        if pattern.is_empty() {
            return Err(PatternError::Empty);
        }
        if pattern.len() > MAX_PATTERN_LENGTH {
            return Err(PatternError::TooLong);
        }
        if !pattern.starts_with("refs/") {
            return Err(PatternError::MissingRefsPrefix);
        }

        let mut segments = Vec::new();
        let mut wildcard_count = 0;

        let parts: Vec<&str> = pattern.split('/').collect();
        for (i, part) in parts.iter().enumerate() {
            if *part == "**" {
                // `**` must be the last segment (recursive match).
                if i != parts.len() - 1 {
                    return Err(PatternError::InvalidSegment(
                        "** must be the last segment".to_string(),
                    ));
                }
                wildcard_count += 1;
                if wildcard_count > MAX_WILDCARDS_PER_PATTERN {
                    return Err(PatternError::TooManyWildcards);
                }
                segments.push(PatternSegment::RecursiveWildcard);
            } else if *part == "*" {
                wildcard_count += 1;
                if wildcard_count > MAX_WILDCARDS_PER_PATTERN {
                    return Err(PatternError::TooManyWildcards);
                }
                segments.push(PatternSegment::Wildcard);
            } else if part.is_empty() {
                return Err(PatternError::InvalidSegment(String::new()));
            } else if part.contains('*')
                || part.contains('?')
                || part.contains('[')
                || part.contains(']')
            {
                // Partial globs (e.g., "v*") are not allowed.
                return Err(PatternError::InvalidSegment(part.to_string()));
            } else if !part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
            {
                return Err(PatternError::InvalidSegment(part.to_string()));
            } else {
                segments.push(PatternSegment::Literal(part.to_string()));
            }
        }

        Ok(Self {
            raw: pattern.to_string(),
            segments,
        })
    }

    /// Test whether this pattern matches a given ref name.
    ///
    /// Matching is segment-by-segment:
    /// - `*` matches exactly one path segment
    /// - `**` (must be last) matches one or more remaining segments
    pub fn matches(&self, ref_name: &str) -> bool {
        let ref_segments: Vec<&str> = ref_name.split('/').collect();

        // Check for recursive wildcard (must be last segment).
        if let Some(PatternSegment::RecursiveWildcard) = self.segments.last() {
            let prefix_len = self.segments.len() - 1;
            // Ref must have at least as many segments as the prefix (+ 1 for the **)
            if ref_segments.len() <= prefix_len {
                return false;
            }
            // All prefix segments must match.
            return self.segments[..prefix_len]
                .iter()
                .zip(ref_segments[..prefix_len].iter())
                .all(|(pat, seg)| match pat {
                    PatternSegment::Wildcard => true,
                    PatternSegment::Literal(lit) => lit == *seg,
                    PatternSegment::RecursiveWildcard => unreachable!(),
                });
        }

        // Non-recursive: exact segment count match required.
        if ref_segments.len() != self.segments.len() {
            return false;
        }
        self.segments
            .iter()
            .zip(ref_segments.iter())
            .all(|(pat, seg)| match pat {
                PatternSegment::Wildcard => true,
                PatternSegment::Literal(lit) => lit == *seg,
                PatternSegment::RecursiveWildcard => unreachable!(),
            })
    }

    /// The raw pattern string.
    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

impl fmt::Display for RefPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

/// The type of ref update in a push.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateKind {
    /// New ref (old_oid is zero).
    Create,
    /// Existing ref updated, new commit is a descendant of old (fast-forward).
    FastForward,
    /// Existing ref updated, new commit is NOT a descendant of old.
    NonFastForward,
    /// Ref deleted (new_oid is zero).
    Delete,
}

impl UpdateKind {
    /// Classify a ref update from old/new OIDs.
    ///
    /// `is_ancestor` should be the result of `git merge-base --is-ancestor old new`.
    /// For creates/deletes, the value is ignored.
    pub fn classify(old_oid: &str, new_oid: &str, is_ancestor: bool) -> Self {
        const ZERO_OID: &str = "0000000000000000000000000000000000000000";
        if old_oid == ZERO_OID {
            Self::Create
        } else if new_oid == ZERO_OID {
            Self::Delete
        } else if is_ancestor {
            Self::FastForward
        } else {
            Self::NonFastForward
        }
    }
}

/// A single ref update within a push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefUpdate {
    /// The ref being updated (e.g., "refs/heads/main").
    pub ref_name: String,
    /// The type of update.
    pub kind: UpdateKind,
    /// Old OID (hex, 40 chars). Zero OID for creates.
    pub old_oid: String,
    /// New OID (hex, 40 chars). Zero OID for deletes.
    pub new_oid: String,
}

/// A single protection rule parsed from a `buzz-protect` tag on kind:30617.
///
/// Format: `["buzz-protect", "<ref-pattern>", "<rule>", ...]`
/// Multiple rules per tag are allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionRule {
    /// The ref pattern this rule applies to.
    pub pattern: RefPattern,
    /// Minimum role required to push (if specified).
    pub push_role: Option<MemberRole>,
    /// Whether non-fast-forward updates are forbidden.
    pub no_force_push: bool,
    /// Whether ref deletion is forbidden.
    pub no_delete: bool,
    /// Whether direct push is denied (must use NIP-34 patch).
    ///
    /// NOTE: This blocks ALL ref update kinds (create, FF, NFF, delete) — not just
    /// fast-forward pushes. If set on a ref pattern, that ref can only be modified
    /// via the NIP-34 patch workflow. This is intentional: the ref is fully governed
    /// by the patch review process.
    pub require_patch: bool,
}

/// Errors from parsing a `buzz-protect` tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleParseError {
    /// Tag has fewer than 2 values (need at least pattern + one rule).
    TooFewValues,
    /// Too many protection rules on this repo.
    TooManyRules,
    /// Invalid ref pattern.
    InvalidPattern(PatternError),
    /// Unknown rule string.
    UnknownRule(String),
    /// Invalid role in `push:<role>`.
    InvalidRole(String),
}

impl fmt::Display for RuleParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooFewValues => write!(f, "buzz-protect tag needs pattern + at least one rule"),
            Self::TooManyRules => write!(f, "exceeds max {MAX_PROTECTION_RULES} rules per repo"),
            Self::InvalidPattern(e) => write!(f, "invalid pattern: {e}"),
            Self::UnknownRule(r) => write!(f, "unknown rule: {r:?}"),
            Self::InvalidRole(r) => write!(f, "invalid role in push rule: {r:?}"),
        }
    }
}

impl std::error::Error for RuleParseError {}

/// Parse a single `buzz-protect` tag into a `ProtectionRule`.
///
/// Tag format: `["buzz-protect", "<pattern>", "<rule1>", "<rule2>", ...]`
/// The first element ("buzz-protect") should already be stripped — pass
/// the remaining values starting with the pattern.
/// Parse a single `buzz-protect` tag (simple API, discards unknown rules).
pub fn parse_protection_tag(values: &[&str]) -> Result<ProtectionRule, RuleParseError> {
    let (rule, _unknowns) = parse_protection_tag_with_warnings(values)?;
    Ok(rule)
}

/// Parse a single `buzz-protect` tag, returning unknown rules for logging.
pub fn parse_protection_tag_with_warnings(
    values: &[&str],
) -> Result<(ProtectionRule, Vec<String>), RuleParseError> {
    if values.len() < 2 {
        return Err(RuleParseError::TooFewValues);
    }

    let pattern = RefPattern::parse(values[0]).map_err(RuleParseError::InvalidPattern)?;

    let mut push_role: Option<MemberRole> = None;
    let mut no_force_push = false;
    let mut no_delete = false;
    let mut require_patch = false;
    let mut unknown_rules = Vec::new();

    for &rule_str in &values[1..] {
        if let Some(role_str) = rule_str.strip_prefix("push:") {
            let role: MemberRole = role_str
                .parse()
                .map_err(|_| RuleParseError::InvalidRole(role_str.to_string()))?;
            // Reject push:bot and push:guest — nonsensical rules.
            // Bot is promoted to Member at the policy layer; push:bot is meaningless.
            // Guest cannot push regardless; push:guest would be confusing.
            if matches!(role, MemberRole::Bot | MemberRole::Guest) {
                return Err(RuleParseError::InvalidRole(role_str.to_string()));
            }
            // Take the strictest (highest permission level).
            push_role = Some(match push_role {
                None => role,
                Some(existing) => {
                    if role.permission_level() > existing.permission_level() {
                        role
                    } else {
                        existing
                    }
                }
            });
        } else {
            match rule_str {
                "no-force-push" => no_force_push = true,
                "no-delete" => no_delete = true,
                "require-patch" => require_patch = true,
                // Forward-compatibility: unknown rules are skipped but reported.
                other => unknown_rules.push(other.to_string()),
            }
        }
    }

    Ok((
        ProtectionRule {
            pattern,
            push_role,
            no_force_push,
            no_delete,
            require_patch,
        },
        unknown_rules,
    ))
}

/// Result of parsing protection tags — includes rules and any warnings.
#[derive(Debug, Clone)]
pub struct ParsedProtection {
    /// Successfully parsed protection rules.
    pub rules: Vec<ProtectionRule>,
    /// Unknown rule strings that were skipped (potential typos or future rules).
    /// Callers should log these as warnings.
    pub unknown_rules: Vec<String>,
}

/// Parse all `buzz-protect` tags from a kind:30617 event's tag list.
///
/// Returns an error if any `buzz-protect` tag is structurally malformed.
/// Unknown rule strings are skipped but reported in `ParsedProtection::unknown_rules`
/// so callers can log warnings (helps catch typos while maintaining forward-compat).
/// Enforces the per-repo rule count limit.
pub fn parse_protection_tags(tags: &[Vec<String>]) -> Result<ParsedProtection, RuleParseError> {
    let mut rules = Vec::new();
    let mut unknown_rules = Vec::new();

    for tag in tags {
        if tag.first().map(|s| s.as_str()) != Some("buzz-protect") {
            continue;
        }
        if rules.len() >= MAX_PROTECTION_RULES {
            return Err(RuleParseError::TooManyRules);
        }
        let values: Vec<&str> = tag[1..].iter().map(|s| s.as_str()).collect();
        let (rule, unknowns) = parse_protection_tag_with_warnings(&values)?;
        rules.push(rule);
        unknown_rules.extend(unknowns);
    }

    Ok(ParsedProtection {
        rules,
        unknown_rules,
    })
}

/// A single explicit push grant from a signed kind:30617 announcement.
///
/// The containing announcement supplies the repository scope. The grant binds
/// one lowercase Nostr public key to one validated ref pattern until the
/// absolute Unix timestamp in [`Self::expires_at`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitPushGrant {
    /// Lowercase hex-encoded 32-byte Nostr public key.
    pub actor_pubkey: String,
    /// Git refs this actor may update.
    pub pattern: RefPattern,
    /// Exclusive Unix timestamp after which the grant is invalid.
    pub expires_at: u64,
}

/// Repository-level Git capability mode parsed from kind:30617 tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitCapabilityPolicy {
    /// No capability opt-in; preserve the existing channel-role behavior.
    Legacy,
    /// Every pusher and every updated ref requires an explicit current grant.
    ExplicitGrantsV1 {
        /// Grants signed into the repository announcement.
        grants: Vec<GitPushGrant>,
    },
}

/// Result of evaluating the capability layer for one atomic push.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitCapabilityDecision {
    /// Continue with the pusher's existing channel-derived role.
    Legacy,
    /// Every ref is covered by an explicit grant; role-only protection may be
    /// evaluated as Owner while non-role protection remains in force.
    ExplicitGrant,
}

/// Errors from parsing versioned Git capability tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitCapabilityParseError {
    /// A grant was present without an explicit policy version.
    GrantWithoutPolicy,
    /// More than one policy selector was present.
    DuplicatePolicy,
    /// The policy tag did not have exactly a name and version.
    MalformedPolicy,
    /// The policy version is not supported by this relay.
    UnsupportedPolicy(String),
    /// Explicit mode omitted the legacy-safe owner-only protection baseline.
    MissingLegacyProtection,
    /// The repository contains more grants than the hard limit.
    TooManyGrants,
    /// A grant did not have exactly actor, pattern, capability, and expiry.
    MalformedGrant,
    /// The actor was not a lowercase hex-encoded 32-byte public key.
    InvalidActorPubkey,
    /// The grant ref pattern was invalid.
    InvalidGrantPattern(PatternError),
    /// The grant requested a capability other than `push`.
    UnsupportedCapability(String),
    /// The expiry was not an unsigned Unix timestamp.
    InvalidExpiry(String),
}

impl fmt::Display for GitCapabilityParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GrantWithoutPolicy => write!(f, "buzz-git-grant requires buzz-git-policy"),
            Self::DuplicatePolicy => write!(f, "multiple buzz-git-policy tags"),
            Self::MalformedPolicy => write!(f, "malformed buzz-git-policy tag"),
            Self::UnsupportedPolicy(version) => {
                write!(f, "unsupported buzz-git-policy version: {version:?}")
            }
            Self::MissingLegacyProtection => {
                write!(f, "explicit grants require buzz-protect refs/** push:owner")
            }
            Self::TooManyGrants => {
                write!(f, "exceeds max {MAX_GIT_PUSH_GRANTS} Git push grants")
            }
            Self::MalformedGrant => write!(f, "malformed buzz-git-grant tag"),
            Self::InvalidActorPubkey => write!(f, "invalid Git grant actor pubkey"),
            Self::InvalidGrantPattern(error) => {
                write!(f, "invalid Git grant pattern: {error}")
            }
            Self::UnsupportedCapability(capability) => {
                write!(f, "unsupported Git grant capability: {capability:?}")
            }
            Self::InvalidExpiry(expiry) => {
                write!(f, "invalid Git grant expiry: {expiry:?}")
            }
        }
    }
}

impl std::error::Error for GitCapabilityParseError {}

/// Parse the versioned, explicit Git capability policy from kind:30617 tags.
///
/// Tag grammar:
///
/// ```text
/// ["buzz-git-policy", "explicit-grants-v1"]
/// ["buzz-git-grant", "<pubkey-hex>", "<ref-pattern>", "push", "<expires-at>"]
/// ```
///
/// Explicit mode also requires a `buzz-protect` tag whose pattern is exactly
/// `refs/**` and whose rules contain `push:owner`. Older relays ignore the new
/// tags but understand that baseline, so ordinary channel members fail closed.
pub fn parse_git_capability_policy(
    tags: &[Vec<String>],
) -> Result<GitCapabilityPolicy, GitCapabilityParseError> {
    let policy_tags: Vec<&Vec<String>> = tags
        .iter()
        .filter(|tag| tag.first().map(String::as_str) == Some("buzz-git-policy"))
        .collect();
    let grant_tags: Vec<&Vec<String>> = tags
        .iter()
        .filter(|tag| tag.first().map(String::as_str) == Some("buzz-git-grant"))
        .collect();

    if policy_tags.is_empty() {
        return if grant_tags.is_empty() {
            Ok(GitCapabilityPolicy::Legacy)
        } else {
            Err(GitCapabilityParseError::GrantWithoutPolicy)
        };
    }
    if policy_tags.len() > 1 {
        return Err(GitCapabilityParseError::DuplicatePolicy);
    }

    let policy_tag = policy_tags[0];
    if policy_tag.len() != 2 {
        return Err(GitCapabilityParseError::MalformedPolicy);
    }
    if policy_tag[1] != "explicit-grants-v1" {
        return Err(GitCapabilityParseError::UnsupportedPolicy(
            policy_tag[1].clone(),
        ));
    }

    let has_legacy_protection = tags.iter().any(|tag| {
        tag.first().map(String::as_str) == Some("buzz-protect")
            && tag.get(1).map(String::as_str) == Some("refs/**")
            && tag[2..].iter().any(|rule| rule == "push:owner")
    });
    if !has_legacy_protection {
        return Err(GitCapabilityParseError::MissingLegacyProtection);
    }
    if grant_tags.len() > MAX_GIT_PUSH_GRANTS {
        return Err(GitCapabilityParseError::TooManyGrants);
    }

    let mut grants = Vec::with_capacity(grant_tags.len());
    for tag in grant_tags {
        if tag.len() != 5 {
            return Err(GitCapabilityParseError::MalformedGrant);
        }
        let actor_pubkey = &tag[1];
        if actor_pubkey.len() != 64
            || !actor_pubkey
                .chars()
                .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
        {
            return Err(GitCapabilityParseError::InvalidActorPubkey);
        }
        let pattern =
            RefPattern::parse(&tag[2]).map_err(GitCapabilityParseError::InvalidGrantPattern)?;
        if tag[3] != "push" {
            return Err(GitCapabilityParseError::UnsupportedCapability(
                tag[3].clone(),
            ));
        }
        let expires_at = tag[4]
            .parse::<u64>()
            .map_err(|_| GitCapabilityParseError::InvalidExpiry(tag[4].clone()))?;
        grants.push(GitPushGrant {
            actor_pubkey: actor_pubkey.clone(),
            pattern,
            expires_at,
        });
    }

    Ok(GitCapabilityPolicy::ExplicitGrantsV1 { grants })
}

/// Evaluate one atomic push against a parsed Git capability policy.
///
/// `issued_at` is the signed repository announcement timestamp and `now` is
/// the relay hook time. In explicit mode every grant must have a positive
/// lifetime of at most 24 hours, and every ref update must match a current
/// grant for the authenticated actor. One uncovered ref denies the whole push.
pub fn evaluate_git_push_capabilities(
    policy: &GitCapabilityPolicy,
    actor_pubkey: &str,
    updates: &[RefUpdate],
    issued_at: u64,
    now: u64,
) -> Result<GitCapabilityDecision, Vec<Denial>> {
    let GitCapabilityPolicy::ExplicitGrantsV1 { grants } = policy else {
        return Ok(GitCapabilityDecision::Legacy);
    };

    let deny_every_ref = |reason: &str| {
        updates
            .iter()
            .map(|update| Denial {
                ref_name: update.ref_name.clone(),
                reason: reason.to_string(),
            })
            .collect::<Vec<_>>()
    };

    if issued_at > now {
        return Err(deny_every_ref(
            "Git capability policy timestamp is in the future",
        ));
    }
    if grants.iter().any(|grant| {
        grant.expires_at <= issued_at
            || grant.expires_at.saturating_sub(issued_at) > MAX_GIT_PUSH_GRANT_TTL_SECS
    }) {
        return Err(deny_every_ref("invalid Git capability grant lifetime"));
    }

    let denials: Vec<Denial> = updates
        .iter()
        .filter(|update| {
            !grants.iter().any(|grant| {
                grant.actor_pubkey == actor_pubkey
                    && grant.expires_at > now
                    && grant.pattern.matches(&update.ref_name)
            })
        })
        .map(|update| Denial {
            ref_name: update.ref_name.clone(),
            reason: "no current Git capability grant for actor and ref".to_string(),
        })
        .collect();

    if denials.is_empty() {
        Ok(GitCapabilityDecision::ExplicitGrant)
    } else {
        Err(denials)
    }
}

/// Built-in default minimum role for an operation when no `buzz-protect` tag matches.
pub fn default_min_role(ref_name: &str, kind: UpdateKind) -> MemberRole {
    let is_branch = ref_name.starts_with("refs/heads/");
    let is_tag = ref_name.starts_with("refs/tags/");

    match kind {
        UpdateKind::Create => {
            if is_branch || is_tag {
                MemberRole::Member
            } else {
                MemberRole::Admin
            }
        }
        UpdateKind::FastForward => {
            if is_branch {
                MemberRole::Member
            } else if is_tag {
                // Tag "move" (overwrite) = Admin.
                MemberRole::Admin
            } else {
                MemberRole::Admin
            }
        }
        UpdateKind::NonFastForward => MemberRole::Admin,
        UpdateKind::Delete => MemberRole::Admin,
    }
}

/// The effective constraints for a ref after unioning all matching rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveRules {
    /// Strictest `push:<role>` from all matching patterns (if any).
    pub push_role: Option<MemberRole>,
    /// Whether non-fast-forward is forbidden (any match sets this).
    pub no_force_push: bool,
    /// Whether deletion is forbidden (any match sets this).
    pub no_delete: bool,
    /// Whether direct push is denied (any match sets this).
    pub require_patch: bool,
    /// Whether any explicit rule matched (vs. using defaults).
    pub has_explicit_match: bool,
}

impl EffectiveRules {
    /// Compute effective rules by unioning all protection rules that match a ref.
    pub fn for_ref(ref_name: &str, rules: &[ProtectionRule]) -> Self {
        let mut push_role: Option<MemberRole> = None;
        let mut no_force_push = false;
        let mut no_delete = false;
        let mut require_patch = false;
        let mut has_explicit_match = false;

        for rule in rules {
            if !rule.pattern.matches(ref_name) {
                continue;
            }
            has_explicit_match = true;

            // Union: take strictest push role.
            if let Some(role) = rule.push_role {
                push_role = Some(match push_role {
                    None => role,
                    Some(existing) => {
                        if role.permission_level() > existing.permission_level() {
                            role
                        } else {
                            existing
                        }
                    }
                });
            }

            // Union: any match sets these flags.
            no_force_push = no_force_push || rule.no_force_push;
            no_delete = no_delete || rule.no_delete;
            require_patch = require_patch || rule.require_patch;
        }

        Self {
            push_role,
            no_force_push,
            no_delete,
            require_patch,
            has_explicit_match,
        }
    }
}

/// A single denial reason from the policy engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denial {
    /// The ref that was denied.
    pub ref_name: String,
    /// Human-readable reason.
    pub reason: String,
}

impl fmt::Display for Denial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.ref_name, self.reason)
    }
}

/// Evaluate a single ref update against effective rules and the pusher's role.
///
/// Returns `Ok(())` if allowed, `Err(Denial)` if denied.
pub fn evaluate_ref_update(
    update: &RefUpdate,
    role: MemberRole,
    rules: &[ProtectionRule],
) -> Result<(), Denial> {
    let effective = EffectiveRules::for_ref(&update.ref_name, rules);

    // If no explicit rules match, use built-in defaults.
    if !effective.has_explicit_match {
        let min_role = default_min_role(&update.ref_name, update.kind);
        if !role.has_at_least(min_role) {
            return Err(Denial {
                ref_name: update.ref_name.clone(),
                reason: format!(
                    "requires {} role (you have {}), using built-in defaults",
                    min_role, role
                ),
            });
        }
        return Ok(());
    }

    // Check require-patch (blocks all direct pushes).
    if effective.require_patch {
        return Err(Denial {
            ref_name: update.ref_name.clone(),
            reason: "direct push denied: require-patch is set, submit a NIP-34 patch".to_string(),
        });
    }

    // Check push role.
    // Explicit push:role can NEVER weaken the built-in default. Always take the
    // HIGHER of (explicit, default). This prevents `push:member` from accidentally
    // allowing Members to force-push, delete, or overwrite tags.
    let default_role = default_min_role(&update.ref_name, update.kind);
    let min_role = match effective.push_role {
        Some(explicit) => {
            // Take the stricter (higher permission level) of explicit vs default.
            if explicit.permission_level() >= default_role.permission_level() {
                explicit
            } else {
                default_role
            }
        }
        None => default_role,
    };
    if !role.has_at_least(min_role) {
        return Err(Denial {
            ref_name: update.ref_name.clone(),
            reason: format!("requires {} role (you have {})", min_role, role),
        });
    }

    // Check no-force-push.
    if effective.no_force_push && update.kind == UpdateKind::NonFastForward {
        return Err(Denial {
            ref_name: update.ref_name.clone(),
            reason: "non-fast-forward update denied: no-force-push is set".to_string(),
        });
    }

    // Check no-delete.
    if effective.no_delete && update.kind == UpdateKind::Delete {
        return Err(Denial {
            ref_name: update.ref_name.clone(),
            reason: "ref deletion denied: no-delete is set".to_string(),
        });
    }

    Ok(())
}

/// Evaluate an entire push (multiple ref updates) against protection rules.
///
/// Returns `Ok(())` if ALL refs are allowed, `Err(Vec<Denial>)` if any are denied.
/// A push is atomic — if any ref fails, the entire push is rejected.
pub fn evaluate_push(
    updates: &[RefUpdate],
    role: MemberRole,
    rules: &[ProtectionRule],
) -> Result<(), Vec<Denial>> {
    let denials: Vec<Denial> = updates
        .iter()
        .filter_map(|update| evaluate_ref_update(update, role, rules).err())
        .collect();

    if denials.is_empty() {
        Ok(())
    } else {
        Err(denials)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_parse_valid() {
        let p = RefPattern::parse("refs/heads/main").unwrap();
        assert_eq!(p.segments.len(), 3);
        assert!(p.matches("refs/heads/main"));
        assert!(!p.matches("refs/heads/develop"));
    }

    #[test]
    fn pattern_wildcard_matches_one_segment() {
        let p = RefPattern::parse("refs/heads/*").unwrap();
        assert!(p.matches("refs/heads/main"));
        assert!(p.matches("refs/heads/feature"));
        assert!(!p.matches("refs/heads/feature/sub"));
        assert!(!p.matches("refs/tags/v1"));
    }

    #[test]
    fn pattern_multi_wildcard() {
        let p = RefPattern::parse("refs/*/release/*").unwrap();
        assert!(p.matches("refs/heads/release/v1"));
        assert!(!p.matches("refs/heads/release/v1/hotfix"));
    }

    #[test]
    fn pattern_rejects_partial_glob() {
        assert!(matches!(
            RefPattern::parse("refs/tags/v*"),
            Err(PatternError::InvalidSegment(_))
        ));
    }

    #[test]
    fn pattern_rejects_missing_refs_prefix() {
        assert!(matches!(
            RefPattern::parse("heads/main"),
            Err(PatternError::MissingRefsPrefix)
        ));
    }

    #[test]
    fn pattern_rejects_empty() {
        assert!(matches!(RefPattern::parse(""), Err(PatternError::Empty)));
    }

    #[test]
    fn pattern_rejects_too_many_wildcards() {
        assert!(matches!(
            RefPattern::parse("refs/*/*/*/*"),
            Err(PatternError::TooManyWildcards)
        ));
    }

    #[test]
    fn pattern_recursive_wildcard_matches_nested() {
        let p = RefPattern::parse("refs/heads/**").unwrap();
        assert!(p.matches("refs/heads/main"));
        assert!(p.matches("refs/heads/feature/foo"));
        assert!(p.matches("refs/heads/feature/foo/bar"));
        assert!(!p.matches("refs/tags/v1"));
    }

    #[test]
    fn pattern_recursive_wildcard_must_be_last() {
        assert!(matches!(
            RefPattern::parse("refs/**/heads"),
            Err(PatternError::InvalidSegment(_))
        ));
    }

    #[test]
    fn pattern_recursive_requires_at_least_one_segment() {
        let p = RefPattern::parse("refs/heads/**").unwrap();
        // Must match at least one segment after prefix
        assert!(!p.matches("refs/heads"));
    }

    #[test]
    fn classify_create() {
        let zero = "0000000000000000000000000000000000000000";
        assert_eq!(
            UpdateKind::classify(zero, "abc123abc123abc123abc123abc123abc123abcd", false),
            UpdateKind::Create
        );
    }

    #[test]
    fn classify_delete() {
        let zero = "0000000000000000000000000000000000000000";
        assert_eq!(
            UpdateKind::classify("abc123abc123abc123abc123abc123abc123abcd", zero, false),
            UpdateKind::Delete
        );
    }

    #[test]
    fn classify_fast_forward() {
        assert_eq!(
            UpdateKind::classify("aaa", "bbb", true),
            UpdateKind::FastForward
        );
    }

    #[test]
    fn classify_non_fast_forward() {
        assert_eq!(
            UpdateKind::classify("aaa", "bbb", false),
            UpdateKind::NonFastForward
        );
    }

    #[test]
    fn parse_protection_tag_basic() {
        let rule =
            parse_protection_tag(&["refs/heads/main", "push:admin", "no-force-push"]).unwrap();
        assert_eq!(rule.push_role, Some(MemberRole::Admin));
        assert!(rule.no_force_push);
        assert!(!rule.no_delete);
        assert!(!rule.require_patch);
    }

    #[test]
    fn parse_protection_tag_all_rules() {
        let rule = parse_protection_tag(&[
            "refs/heads/main",
            "push:owner",
            "no-force-push",
            "no-delete",
            "require-patch",
        ])
        .unwrap();
        assert_eq!(rule.push_role, Some(MemberRole::Owner));
        assert!(rule.no_force_push);
        assert!(rule.no_delete);
        assert!(rule.require_patch);
    }

    #[test]
    fn parse_protection_tag_unknown_rule_skipped() {
        // Forward-compatibility: unknown rules are silently skipped.
        let rule = parse_protection_tag(&["refs/heads/main", "yolo", "no-force-push"]).unwrap();
        // "yolo" was skipped, but "no-force-push" was still applied.
        assert!(rule.no_force_push);
        assert!(rule.push_role.is_none());
    }

    #[test]
    fn parse_protection_tag_invalid_role() {
        assert!(matches!(
            parse_protection_tag(&["refs/heads/main", "push:superadmin"]),
            Err(RuleParseError::InvalidRole(_))
        ));
    }

    #[test]
    fn parse_protection_tag_rejects_push_bot_and_guest() {
        // push:bot and push:guest are rejected — they're almost certainly user errors.
        assert!(matches!(
            parse_protection_tag(&["refs/heads/main", "push:bot"]),
            Err(RuleParseError::InvalidRole(_))
        ));
        assert!(matches!(
            parse_protection_tag(&["refs/heads/main", "push:guest"]),
            Err(RuleParseError::InvalidRole(_))
        ));
    }

    #[test]
    fn effective_rules_union_strictest_role() {
        let rules = vec![
            parse_protection_tag(&["refs/heads/*", "push:member", "no-force-push"]).unwrap(),
            parse_protection_tag(&["refs/heads/main", "push:admin"]).unwrap(),
        ];
        let eff = EffectiveRules::for_ref("refs/heads/main", &rules);
        assert_eq!(eff.push_role, Some(MemberRole::Admin)); // strictest
        assert!(eff.no_force_push); // from the wildcard rule
        assert!(eff.has_explicit_match);
    }

    #[test]
    fn effective_rules_no_match_uses_defaults() {
        let rules = vec![parse_protection_tag(&["refs/heads/main", "push:admin"]).unwrap()];
        let eff = EffectiveRules::for_ref("refs/heads/develop", &rules);
        assert!(!eff.has_explicit_match);
    }

    #[test]
    fn evaluate_owner_passes_push_role() {
        let rules = vec![parse_protection_tag(&["refs/heads/main", "push:admin"]).unwrap()];
        let update = RefUpdate {
            ref_name: "refs/heads/main".to_string(),
            kind: UpdateKind::FastForward,
            old_oid: "a".repeat(40),
            new_oid: "b".repeat(40),
        };
        assert!(evaluate_ref_update(&update, MemberRole::Owner, &rules).is_ok());
    }

    #[test]
    fn evaluate_member_denied_push_admin() {
        let rules = vec![parse_protection_tag(&["refs/heads/main", "push:admin"]).unwrap()];
        let update = RefUpdate {
            ref_name: "refs/heads/main".to_string(),
            kind: UpdateKind::FastForward,
            old_oid: "a".repeat(40),
            new_oid: "b".repeat(40),
        };
        assert!(evaluate_ref_update(&update, MemberRole::Member, &rules).is_err());
    }

    #[test]
    fn evaluate_no_force_push_blocks_owner() {
        let rules =
            vec![
                parse_protection_tag(&["refs/heads/main", "push:member", "no-force-push"]).unwrap(),
            ];
        let update = RefUpdate {
            ref_name: "refs/heads/main".to_string(),
            kind: UpdateKind::NonFastForward,
            old_oid: "a".repeat(40),
            new_oid: "b".repeat(40),
        };
        // Owner is blocked by no-force-push!
        assert!(evaluate_ref_update(&update, MemberRole::Owner, &rules).is_err());
    }

    #[test]
    fn evaluate_no_force_push_allows_fast_forward() {
        let rules =
            vec![
                parse_protection_tag(&["refs/heads/main", "push:member", "no-force-push"]).unwrap(),
            ];
        let update = RefUpdate {
            ref_name: "refs/heads/main".to_string(),
            kind: UpdateKind::FastForward,
            old_oid: "a".repeat(40),
            new_oid: "b".repeat(40),
        };
        // no-force-push should NOT block fast-forward pushes.
        assert!(evaluate_ref_update(&update, MemberRole::Member, &rules).is_ok());
    }

    #[test]
    fn evaluate_no_delete_blocks_admin() {
        let rules =
            vec![parse_protection_tag(&["refs/heads/main", "push:member", "no-delete"]).unwrap()];
        let update = RefUpdate {
            ref_name: "refs/heads/main".to_string(),
            kind: UpdateKind::Delete,
            old_oid: "a".repeat(40),
            new_oid: "0".repeat(40),
        };
        assert!(evaluate_ref_update(&update, MemberRole::Admin, &rules).is_err());
    }

    #[test]
    fn evaluate_require_patch_blocks_all() {
        let rules = vec![parse_protection_tag(&["refs/heads/main", "require-patch"]).unwrap()];
        let update = RefUpdate {
            ref_name: "refs/heads/main".to_string(),
            kind: UpdateKind::FastForward,
            old_oid: "a".repeat(40),
            new_oid: "b".repeat(40),
        };
        assert!(evaluate_ref_update(&update, MemberRole::Owner, &rules).is_err());
    }

    #[test]
    fn evaluate_defaults_member_can_ff_branch() {
        let rules = vec![]; // No explicit rules
        let update = RefUpdate {
            ref_name: "refs/heads/feature".to_string(),
            kind: UpdateKind::FastForward,
            old_oid: "a".repeat(40),
            new_oid: "b".repeat(40),
        };
        assert!(evaluate_ref_update(&update, MemberRole::Member, &rules).is_ok());
    }

    #[test]
    fn evaluate_defaults_member_cannot_force_push() {
        let rules = vec![]; // No explicit rules — defaults apply
        let update = RefUpdate {
            ref_name: "refs/heads/feature".to_string(),
            kind: UpdateKind::NonFastForward,
            old_oid: "a".repeat(40),
            new_oid: "b".repeat(40),
        };
        // Default: non-fast-forward requires Admin
        assert!(evaluate_ref_update(&update, MemberRole::Member, &rules).is_err());
    }

    #[test]
    fn evaluate_defaults_guest_cannot_push() {
        let rules = vec![];
        let update = RefUpdate {
            ref_name: "refs/heads/feature".to_string(),
            kind: UpdateKind::FastForward,
            old_oid: "a".repeat(40),
            new_oid: "b".repeat(40),
        };
        assert!(evaluate_ref_update(&update, MemberRole::Guest, &rules).is_err());
    }

    #[test]
    fn evaluate_bot_cannot_push_without_explicit_grant() {
        let rules = vec![];
        let update = RefUpdate {
            ref_name: "refs/heads/feature".to_string(),
            kind: UpdateKind::FastForward,
            old_oid: "a".repeat(40),
            new_oid: "b".repeat(40),
        };
        // Bot has permission_level 0 at the core evaluator level.
        // NOTE: The policy layer (policy.rs) promotes Bot → Member before calling
        // evaluate_push, so bots in a channel CAN push in practice. This test
        // verifies the raw evaluator behavior; the promotion is tested in policy.
        assert!(evaluate_ref_update(&update, MemberRole::Bot, &rules).is_err());
    }

    #[test]
    fn evaluate_guest_denied_even_with_only_no_force_push_rule() {
        // Regression test: a rule that only sets no-force-push (no push:role)
        // should NOT let a Guest bypass the built-in default (Member required).
        let rules = vec![parse_protection_tag(&["refs/heads/main", "no-force-push"]).unwrap()];
        let update = RefUpdate {
            ref_name: "refs/heads/main".to_string(),
            kind: UpdateKind::FastForward,
            old_oid: "a".repeat(40),
            new_oid: "b".repeat(40),
        };
        // Guest should be denied — built-in default requires Member for FF push.
        assert!(evaluate_ref_update(&update, MemberRole::Guest, &rules).is_err());
        // Member should be allowed (meets default requirement).
        assert!(evaluate_ref_update(&update, MemberRole::Member, &rules).is_ok());
    }

    #[test]
    fn evaluate_push_member_cannot_weaken_destructive_defaults() {
        // push:member should NOT allow Members to force-push or delete.
        // The built-in default for NFF/Delete is Admin — explicit push:member
        // can't weaken that for destructive operations.
        let rules = vec![parse_protection_tag(&["refs/heads/main", "push:member"]).unwrap()];
        // Member can FF push (non-destructive, explicit overrides default).
        let ff_update = RefUpdate {
            ref_name: "refs/heads/main".to_string(),
            kind: UpdateKind::FastForward,
            old_oid: "a".repeat(40),
            new_oid: "b".repeat(40),
        };
        assert!(evaluate_ref_update(&ff_update, MemberRole::Member, &rules).is_ok());

        // Member CANNOT force-push (destructive, default Admin still enforced).
        let nff_update = RefUpdate {
            ref_name: "refs/heads/main".to_string(),
            kind: UpdateKind::NonFastForward,
            old_oid: "a".repeat(40),
            new_oid: "b".repeat(40),
        };
        assert!(evaluate_ref_update(&nff_update, MemberRole::Member, &rules).is_err());

        // Admin CAN force-push (meets the default Admin requirement).
        assert!(evaluate_ref_update(&nff_update, MemberRole::Admin, &rules).is_ok());

        // Member CANNOT delete (destructive, default Admin still enforced).
        let del_update = RefUpdate {
            ref_name: "refs/heads/main".to_string(),
            kind: UpdateKind::Delete,
            old_oid: "a".repeat(40),
            new_oid: "0".repeat(40),
        };
        assert!(evaluate_ref_update(&del_update, MemberRole::Member, &rules).is_err());
    }

    #[test]
    fn evaluate_push_multiple_refs_partial_deny() {
        let rules = vec![parse_protection_tag(&["refs/heads/main", "push:admin"]).unwrap()];
        let updates = vec![
            RefUpdate {
                ref_name: "refs/heads/feature".to_string(),
                kind: UpdateKind::FastForward,
                old_oid: "a".repeat(40),
                new_oid: "b".repeat(40),
            },
            RefUpdate {
                ref_name: "refs/heads/main".to_string(),
                kind: UpdateKind::FastForward,
                old_oid: "c".repeat(40),
                new_oid: "d".repeat(40),
            },
        ];
        // Member can push to feature but not main
        let result = evaluate_push(&updates, MemberRole::Member, &rules);
        assert!(result.is_err());
        let denials = result.unwrap_err();
        assert_eq!(denials.len(), 1);
        assert_eq!(denials[0].ref_name, "refs/heads/main");
    }

    fn capability_tags(actor_a: &str, actor_b: &str, expires_at: u64) -> Vec<Vec<String>> {
        vec![
            vec!["buzz-protect".into(), "refs/**".into(), "push:owner".into()],
            vec!["buzz-git-policy".into(), "explicit-grants-v1".into()],
            vec![
                "buzz-git-grant".into(),
                actor_a.into(),
                "refs/heads/agent/a/**".into(),
                "push".into(),
                expires_at.to_string(),
            ],
            vec![
                "buzz-git-grant".into(),
                actor_b.into(),
                "refs/heads/agent/b/**".into(),
                "push".into(),
                expires_at.to_string(),
            ],
        ]
    }

    fn create_update(ref_name: &str) -> RefUpdate {
        RefUpdate {
            ref_name: ref_name.to_string(),
            kind: UpdateKind::Create,
            old_oid: "0".repeat(40),
            new_oid: "a".repeat(40),
        }
    }

    #[test]
    fn git_capability_policy_preserves_legacy_without_opt_in() {
        assert_eq!(
            parse_git_capability_policy(&[]).unwrap(),
            GitCapabilityPolicy::Legacy
        );
    }

    #[test]
    fn git_capability_policy_rejects_orphan_duplicate_and_unknown_policy() {
        let actor = "a".repeat(64);
        let orphan = vec![vec![
            "buzz-git-grant".into(),
            actor,
            "refs/heads/agent/a/**".into(),
            "push".into(),
            "100".into(),
        ]];
        assert_eq!(
            parse_git_capability_policy(&orphan),
            Err(GitCapabilityParseError::GrantWithoutPolicy)
        );

        let duplicate = vec![
            vec!["buzz-git-policy".into(), "explicit-grants-v1".into()],
            vec!["buzz-git-policy".into(), "explicit-grants-v1".into()],
        ];
        assert_eq!(
            parse_git_capability_policy(&duplicate),
            Err(GitCapabilityParseError::DuplicatePolicy)
        );

        let unknown = vec![vec!["buzz-git-policy".into(), "future-v2".into()]];
        assert_eq!(
            parse_git_capability_policy(&unknown),
            Err(GitCapabilityParseError::UnsupportedPolicy(
                "future-v2".into()
            ))
        );
    }

    #[test]
    fn git_capability_policy_requires_legacy_owner_protection() {
        let tags = vec![vec!["buzz-git-policy".into(), "explicit-grants-v1".into()]];
        assert_eq!(
            parse_git_capability_policy(&tags),
            Err(GitCapabilityParseError::MissingLegacyProtection)
        );
    }

    #[test]
    fn git_capability_policy_parses_two_actor_grants() {
        let actor_a = "a".repeat(64);
        let actor_b = "b".repeat(64);
        let policy = parse_git_capability_policy(&capability_tags(&actor_a, &actor_b, 86_500))
            .expect("valid capability policy");
        let GitCapabilityPolicy::ExplicitGrantsV1 { grants } = policy else {
            panic!("expected explicit grants");
        };
        assert_eq!(grants.len(), 2);
        assert_eq!(grants[0].actor_pubkey, actor_a);
        assert_eq!(grants[0].pattern.as_str(), "refs/heads/agent/a/**");
        assert_eq!(grants[0].expires_at, 86_500);
    }

    #[test]
    fn git_capability_policy_rejects_malformed_grants() {
        let base = vec![
            vec!["buzz-protect".into(), "refs/**".into(), "push:owner".into()],
            vec!["buzz-git-policy".into(), "explicit-grants-v1".into()],
        ];
        let lowercase_actor = "a".repeat(64);
        let uppercase_actor = "A".repeat(64);
        let cases = [
            vec![
                "buzz-git-grant".into(),
                "abc".into(),
                "refs/heads/a/**".into(),
                "push".into(),
                "10".into(),
            ],
            vec![
                "buzz-git-grant".into(),
                uppercase_actor,
                "refs/heads/a/**".into(),
                "push".into(),
                "10".into(),
            ],
            vec![
                "buzz-git-grant".into(),
                lowercase_actor.clone(),
                "heads/a/**".into(),
                "push".into(),
                "10".into(),
            ],
            vec![
                "buzz-git-grant".into(),
                lowercase_actor.clone(),
                "refs/heads/a/**".into(),
                "merge".into(),
                "10".into(),
            ],
            vec![
                "buzz-git-grant".into(),
                lowercase_actor,
                "refs/heads/a/**".into(),
                "push".into(),
                "later".into(),
            ],
        ];
        for case in cases {
            let mut tags = base.clone();
            tags.push(case);
            assert!(
                parse_git_capability_policy(&tags).is_err(),
                "case: {tags:?}"
            );
        }
    }

    #[test]
    fn git_capability_policy_enforces_grant_limit() {
        let mut tags = vec![
            vec!["buzz-protect".into(), "refs/**".into(), "push:owner".into()],
            vec!["buzz-git-policy".into(), "explicit-grants-v1".into()],
        ];
        for index in 0..=MAX_GIT_PUSH_GRANTS {
            tags.push(vec![
                "buzz-git-grant".into(),
                format!("{index:064x}"),
                format!("refs/heads/agent/{index}/**"),
                "push".into(),
                "100".into(),
            ]);
        }
        assert_eq!(
            parse_git_capability_policy(&tags),
            Err(GitCapabilityParseError::TooManyGrants)
        );
    }

    #[test]
    fn git_capability_policy_isolates_actor_prefixes_and_main() {
        let actor_a = "a".repeat(64);
        let actor_b = "b".repeat(64);
        let policy =
            parse_git_capability_policy(&capability_tags(&actor_a, &actor_b, 86_500)).unwrap();

        assert_eq!(
            evaluate_git_push_capabilities(
                &policy,
                &actor_a,
                &[create_update("refs/heads/agent/a/task-1")],
                100,
                200,
            ),
            Ok(GitCapabilityDecision::ExplicitGrant)
        );
        assert!(evaluate_git_push_capabilities(
            &policy,
            &actor_a,
            &[create_update("refs/heads/main")],
            100,
            200,
        )
        .is_err());
        assert!(evaluate_git_push_capabilities(
            &policy,
            &actor_a,
            &[create_update("refs/heads/agent/b/task-1")],
            100,
            200,
        )
        .is_err());
        assert_eq!(
            evaluate_git_push_capabilities(
                &policy,
                &actor_b,
                &[create_update("refs/heads/agent/b/task-1")],
                100,
                200,
            ),
            Ok(GitCapabilityDecision::ExplicitGrant)
        );
    }

    #[test]
    fn git_capability_policy_denies_expired_overlong_and_future_issued_grants() {
        let actor = "a".repeat(64);
        let update = create_update("refs/heads/agent/a/task-1");

        let expired =
            parse_git_capability_policy(&capability_tags(&actor, &"b".repeat(64), 200)).unwrap();
        assert!(evaluate_git_push_capabilities(
            &expired,
            &actor,
            std::slice::from_ref(&update),
            100,
            200
        )
        .is_err());

        let overlong = parse_git_capability_policy(&capability_tags(
            &actor,
            &"b".repeat(64),
            100 + MAX_GIT_PUSH_GRANT_TTL_SECS + 1,
        ))
        .unwrap();
        assert!(evaluate_git_push_capabilities(
            &overlong,
            &actor,
            std::slice::from_ref(&update),
            100,
            200,
        )
        .is_err());

        let current =
            parse_git_capability_policy(&capability_tags(&actor, &"b".repeat(64), 500)).unwrap();
        assert!(evaluate_git_push_capabilities(&current, &actor, &[update], 300, 200).is_err());
    }

    #[test]
    fn git_capability_policy_denies_atomic_push_with_uncovered_ref() {
        let actor = "a".repeat(64);
        let policy =
            parse_git_capability_policy(&capability_tags(&actor, &"b".repeat(64), 500)).unwrap();
        let result = evaluate_git_push_capabilities(
            &policy,
            &actor,
            &[
                create_update("refs/heads/agent/a/task-1"),
                create_update("refs/heads/main"),
            ],
            100,
            200,
        );
        let denials = result.expect_err("one uncovered ref denies the push");
        assert_eq!(denials.len(), 1);
        assert_eq!(denials[0].ref_name, "refs/heads/main");
    }

    #[test]
    fn explicit_grant_does_not_bypass_non_role_protection() {
        let actor = "a".repeat(64);
        let policy =
            parse_git_capability_policy(&capability_tags(&actor, &"b".repeat(64), 500)).unwrap();
        let update = RefUpdate {
            ref_name: "refs/heads/agent/a/task-1".into(),
            kind: UpdateKind::NonFastForward,
            old_oid: "a".repeat(40),
            new_oid: "b".repeat(40),
        };
        assert_eq!(
            evaluate_git_push_capabilities(
                &policy,
                &actor,
                std::slice::from_ref(&update),
                100,
                200,
            ),
            Ok(GitCapabilityDecision::ExplicitGrant)
        );
        let rules =
            vec![
                parse_protection_tag(&["refs/heads/agent/a/**", "push:owner", "no-force-push"])
                    .unwrap(),
            ];
        assert!(evaluate_push(&[update], MemberRole::Owner, &rules).is_err());
    }
}
