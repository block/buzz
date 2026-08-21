//! Default `RUST_LOG` for the spawned harness, and the rule for merging a
//! configured one into it.
//!
//! `buzz-acp` emits almost everything on its own target families — `acp::*`,
//! `pool::*`, `canvas::*`, `engram::*`, `observer` — rather than under the
//! crate path, and `EnvFilter` matches directives by target prefix. A bare
//! `buzz_acp=info` therefore matches none of them, so an agent can log an
//! error that its owner never sees.
//!
//! The families are enabled at `warn`, deliberately, and not at `info`: their
//! info-level events carry conversation content (`acp::stream` logs the
//! model's reply verbatim) and this log is a plaintext file on disk that a
//! long-lived process appends to. Warn and error carry timeouts, ids and
//! failures. Content stays behind an explicit opt-in.
//!
//! Keep in sync with the crate-side fallback in `crates/buzz-acp/src/lib.rs`.
pub(super) const LOG_FILTER: &str =
    "buzz_acp=info,acp=warn,pool=warn,canvas=warn,engram=warn,observer=warn";

/// Whether a single directive is just a level, i.e. `EnvFilter`'s global
/// default. Both spellings count: the names, and the numeric forms `0`..`5`
/// the parser also accepts.
///
/// A target-specific directive outranks the global one, so merging our
/// families into such a filter would *narrow* what the user asked for. `0`
/// matters most: it means "log nothing", and widening that would be the
/// opposite of the request.
fn is_global_level(directive: &str) -> bool {
    const NAMES: [&str; 6] = ["off", "error", "warn", "info", "debug", "trace"];
    let directive = directive.trim();
    NAMES.iter().any(|name| directive.eq_ignore_ascii_case(name))
        || matches!(directive, "0" | "1" | "2" | "3" | "4" | "5")
}

/// Merge a configured `RUST_LOG` with the harness defaults.
///
/// `configured` is the value that survives the desktop's own env layering, so
/// this must be called after that layering rather than before it.
pub(super) fn merge(configured: Option<&str>) -> String {
    let Some(configured) = configured.map(str::trim).filter(|value| !value.is_empty()) else {
        return LOG_FILTER.to_string();
    };
    if configured.split(',').any(is_global_level) {
        return configured.to_string();
    }
    // Defaults first: a later directive for the same target overwrites an
    // earlier one, so an explicit `acp=debug` still wins over ours while the
    // families the user did not name keep their diagnostics.
    format!("{LOG_FILTER},{configured}")
}

/// Write the harness `RUST_LOG` onto `command`.
///
/// Takes the already-layered agent env so the value the user actually saved is
/// the one merged; calling this before that layering would let the layering
/// overwrite the result.
pub(super) fn apply(
    command: &mut std::process::Command,
    env: &std::collections::BTreeMap<String, String>,
) {
    let ambient = std::env::var("RUST_LOG").ok();
    let configured = env
        .get("RUST_LOG")
        .map(String::as_str)
        .or(ambient.as_deref());
    command.env("RUST_LOG", merge(configured));
}

#[cfg(test)]
mod tests {
    use super::{is_global_level, merge, LOG_FILTER};

    #[test]
    fn default_covers_every_harness_target_family() {
        for family in ["buzz_acp", "acp", "pool", "canvas", "engram", "observer"] {
            assert!(
                LOG_FILTER.contains(&format!("{family}=")),
                "default drops the `{family}` target family: {LOG_FILTER}"
            );
        }
    }

    #[test]
    fn default_enables_no_content_bearing_target() {
        // acp::stream logs assistant text verbatim at info, pool::prompt logs
        // command arguments. Enabling a family at info would persist both to a
        // plaintext file, so only warn and error may be on by default.
        for family in ["acp", "pool", "canvas", "engram", "observer"] {
            assert!(
                LOG_FILTER.contains(&format!("{family}=warn")),
                "`{family}` must default to warn, not info: {LOG_FILTER}"
            );
        }
    }

    #[test]
    fn unset_or_blank_takes_the_default() {
        assert_eq!(merge(None), LOG_FILTER);
        assert_eq!(merge(Some("   ")), LOG_FILTER);
    }

    #[test]
    fn explicit_target_directive_keeps_the_other_families() {
        let filter = merge(Some("buzz_acp=debug"));
        assert!(filter.starts_with(LOG_FILTER), "{filter}");
        assert!(filter.ends_with(",buzz_acp=debug"), "{filter}");
    }

    #[test]
    fn target_merely_containing_the_crate_name_does_not_bypass_defaults() {
        assert!(merge(Some("my_buzz_acp=debug")).starts_with(LOG_FILTER));
    }

    #[test]
    fn named_global_level_is_preserved_exactly() {
        for value in ["debug", "TRACE", "off"] {
            assert_eq!(merge(Some(value)), value, "global level must survive");
        }
    }

    #[test]
    fn numeric_global_levels_are_preserved_exactly() {
        // EnvFilter accepts 0..5 as global levels. Treating them as targets
        // would widen `0` (log nothing) and narrow `5` (trace).
        for value in ["0", "1", "2", "3", "4", "5"] {
            assert_eq!(merge(Some(value)), value, "numeric level must survive");
            assert!(is_global_level(value));
        }
    }

    #[test]
    fn unrelated_directive_is_appended_not_replaced() {
        let filter = merge(Some("hyper=warn"));
        assert!(filter.starts_with(LOG_FILTER), "{filter}");
        assert!(filter.ends_with(",hyper=warn"), "{filter}");
    }
}
