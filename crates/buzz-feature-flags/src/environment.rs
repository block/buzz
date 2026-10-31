//! Process-wide environment-backed feature-flag evaluator.
//!
//! Environment variables use the fixed `BUZZ_FEATURE_FLAG_` namespace and a
//! normalized form of each flag key.
//!
//! Normalization algorithm:
//! - Uppercase ASCII letters.
//! - Replace each run of non-ASCII-alphanumeric characters with `_`.
//! - Trim leading and trailing `_`.
//!
//! If normalization yields an empty key segment, evaluation falls back to the
//! flag-declared default.
//!
//! Different flag keys can normalize to the same environment variable name
//! (for example `a-b` and `a_b`), so callers must keep normalized names unique.

use std::{
    collections::{BTreeSet, HashMap},
    ffi::OsString,
    fmt::{Debug, Formatter},
    sync::{Arc, Mutex},
};

use crate::{BooleanFlag, EvaluationContext, FlagEvaluator, IntegerFlag};

const ENV_NAMESPACE: &str = "BUZZ_FEATURE_FLAG_";

#[derive(Clone)]
enum SnapshotValue {
    Unicode(String),
    NonUnicode,
}

/// Sanitized diagnostic produced for a malformed configured environment value.
///
/// Diagnostics identify only the provider-neutral flag key and expected type.
/// They never contain raw values or environment-variable names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EnvironmentDiagnostic {
    /// A present value could not be evaluated as a boolean.
    InvalidBooleanValue {
        /// Stable provider-neutral flag key.
        flag_key: &'static str,
    },
    /// A present value could not be evaluated as a signed integer.
    InvalidIntegerValue {
        /// Stable provider-neutral flag key.
        flag_key: &'static str,
    },
}

#[derive(Default)]
struct DiagnosticState {
    seen: BTreeSet<EnvironmentDiagnostic>,
    pending: BTreeSet<EnvironmentDiagnostic>,
}

/// Process-wide feature-flag evaluator backed by an immutable environment
/// snapshot.
///
/// This evaluator captures key/value pairs once at construction and never
/// rereads or mutates process environment state afterward. Evaluation ignores
/// [`EvaluationContext`] because all values are process-wide.
#[derive(Clone)]
pub struct EnvironmentEvaluator {
    snapshot: HashMap<String, SnapshotValue>,
    diagnostics: Arc<Mutex<DiagnosticState>>,
}

impl Default for EnvironmentEvaluator {
    fn default() -> Self {
        Self::from_process_environment()
    }
}

impl EnvironmentEvaluator {
    /// Capture a one-time snapshot of the current process environment.
    ///
    /// Only namespaced entries are retained. Entries with non-Unicode names are
    /// ignored; non-Unicode values are retained only as invalid-value markers.
    pub fn from_process_environment() -> Self {
        Self::from_pairs(std::env::vars_os())
    }

    /// Build an evaluator from a deterministic key/value snapshot.
    ///
    /// Only entries whose names start with `BUZZ_FEATURE_FLAG_` are retained.
    /// Entries with non-Unicode names are ignored; non-Unicode values are
    /// retained only as invalid-value markers.
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        let mut snapshot = HashMap::new();
        for (name, value) in pairs {
            let name = name.into();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !name.starts_with(ENV_NAMESPACE) {
                continue;
            }

            let value = match value.into().into_string() {
                Ok(value) => SnapshotValue::Unicode(value),
                Err(_) => SnapshotValue::NonUnicode,
            };
            snapshot.insert(name.to_owned(), value);
        }

        Self {
            snapshot,
            diagnostics: Arc::new(Mutex::new(DiagnosticState::default())),
        }
    }

    /// Drain sanitized diagnostics that have not previously been reported.
    ///
    /// Each malformed value is returned at most once per flag key and expected
    /// type, including across cloned evaluators. Missing values do not produce
    /// diagnostics.
    pub fn take_diagnostics(&self) -> Vec<EnvironmentDiagnostic> {
        let mut state = match self.diagnostics.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        std::mem::take(&mut state.pending).into_iter().collect()
    }

    fn lookup(&self, flag_key: &str) -> Option<&SnapshotValue> {
        let env_name = normalize_flag_env_name(flag_key)?;
        self.snapshot.get(&env_name)
    }

    fn record_diagnostic(&self, diagnostic: EnvironmentDiagnostic) {
        let mut state = match self.diagnostics.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        if state.seen.insert(diagnostic) {
            state.pending.insert(diagnostic);
        }
    }
}

impl Debug for EnvironmentEvaluator {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentEvaluator")
            .field("namespace", &ENV_NAMESPACE)
            .field("entry_count", &self.snapshot.len())
            .field("entries", &"[REDACTED]")
            .finish()
    }
}

impl FlagEvaluator for EnvironmentEvaluator {
    fn evaluate_bool(&self, flag: BooleanFlag, _context: &EvaluationContext) -> bool {
        let Some(value) = self.lookup(flag.key()) else {
            return flag.default();
        };
        if let SnapshotValue::Unicode(value) = value {
            if let Some(value) = parse_bool(value) {
                return value;
            }
        }

        self.record_diagnostic(EnvironmentDiagnostic::InvalidBooleanValue {
            flag_key: flag.key(),
        });
        flag.default()
    }

    fn evaluate_int(&self, flag: IntegerFlag, _context: &EvaluationContext) -> i64 {
        let Some(value) = self.lookup(flag.key()) else {
            return flag.default();
        };
        if let SnapshotValue::Unicode(value) = value {
            if let Some(value) = parse_i64(value) {
                return value;
            }
        }

        self.record_diagnostic(EnvironmentDiagnostic::InvalidIntegerValue {
            flag_key: flag.key(),
        });
        flag.default()
    }
}

fn parse_bool(raw: &str) -> Option<bool> {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("true") {
        Some(true)
    } else if trimmed.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

fn parse_i64(raw: &str) -> Option<i64> {
    raw.trim().parse::<i64>().ok()
}

fn normalize_flag_env_name(flag_key: &str) -> Option<String> {
    let mut normalized = String::with_capacity(flag_key.len());
    let mut in_separator = false;

    for character in flag_key.chars() {
        if character.is_ascii_alphanumeric() {
            normalized.push(character.to_ascii_uppercase());
            in_separator = false;
        } else if !in_separator {
            normalized.push('_');
            in_separator = true;
        }
    }

    let trimmed = normalized.trim_matches('_');
    if trimmed.is_empty() {
        return None;
    }

    Some(format!("{ENV_NAMESPACE}{trimmed}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{BooleanFlag, EvaluationContext};
    use buzz_core::CommunityId;

    fn community(id: &str) -> CommunityId {
        CommunityId::from_uuid(id.parse().expect("valid community UUID"))
    }

    #[test]
    fn normalization_uses_fixed_namespace_and_collapses_separators() {
        assert_eq!(
            normalize_flag_env_name("relay.feature.query-v2"),
            Some("BUZZ_FEATURE_FLAG_RELAY_FEATURE_QUERY_V2".to_owned())
        );
        assert_eq!(
            normalize_flag_env_name("___relay..foo---bar***baz??9___"),
            Some("BUZZ_FEATURE_FLAG_RELAY_FOO_BAR_BAZ_9".to_owned())
        );
    }

    #[test]
    fn normalization_rejects_empty_segment() {
        assert_eq!(normalize_flag_env_name(""), None);
        assert_eq!(normalize_flag_env_name("---***___"), None);
    }

    #[test]
    fn snapshot_retains_only_namespaced_entries() {
        let evaluator = EnvironmentEvaluator::from_pairs([
            ("BUZZ_FEATURE_FLAG_RELAY_ENABLED", "true"),
            ("DATABASE_URL", "sensitive"),
        ]);

        assert_eq!(evaluator.snapshot.len(), 1);
        assert!(evaluator
            .snapshot
            .contains_key("BUZZ_FEATURE_FLAG_RELAY_ENABLED"));
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_names_are_ignored_and_values_become_invalid_markers() {
        use std::os::unix::ffi::OsStringExt;

        let evaluator = EnvironmentEvaluator::from_pairs([
            (
                OsString::from("BUZZ_FEATURE_FLAG_RELAY_NON_UNICODE_NAME"),
                OsString::from_vec(vec![0x80]),
            ),
            (OsString::from_vec(vec![0x81]), OsString::from("true")),
        ]);
        let context =
            EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));

        assert!(
            evaluator.evaluate_bool(BooleanFlag::new("relay.non-unicode-name", true), &context,)
        );
        assert_eq!(
            evaluator.take_diagnostics(),
            vec![EnvironmentDiagnostic::InvalidBooleanValue {
                flag_key: "relay.non-unicode-name",
            }]
        );
    }

    #[test]
    fn poisoned_diagnostic_mutex_recovers_without_panicking() {
        let evaluator = EnvironmentEvaluator::from_pairs([(
            "BUZZ_FEATURE_FLAG_RELAY_POISONED",
            "not-a-boolean",
        )]);
        let diagnostics = Arc::clone(&evaluator.diagnostics);
        let poison_result = std::thread::spawn(move || {
            let _guard = diagnostics.lock().expect("diagnostic mutex lock");
            panic!("poison diagnostic mutex");
        })
        .join();
        assert!(poison_result.is_err());

        let context =
            EvaluationContext::for_community(community("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));
        assert!(evaluator.evaluate_bool(BooleanFlag::new("relay.poisoned", true), &context));
        assert_eq!(
            evaluator.take_diagnostics(),
            vec![EnvironmentDiagnostic::InvalidBooleanValue {
                flag_key: "relay.poisoned",
            }]
        );
    }
}
