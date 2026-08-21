//! Log filter for spawned agent processes.
//!
//! Extracted from `runtime.rs` to keep that file under the desktop file-size
//! ratchet; it is a self-contained decision with its own tests.

/// Log filter for the spawned harness and, through it, the agent.
///
/// `buzz-acp` inherits its stderr to the agent subprocess (`acp.rs:463`), so
/// whatever this filter admits is what lands in the per-agent log file. That
/// makes `buzz_agent` as load-bearing as `buzz_acp`: without it the agent's own
/// turn diagnostics — compaction, the `_Stop` veto, max-rounds — are filtered
/// out before they reach disk, and an agent misbehaving in the desktop leaves
/// no trace to debug it with.
pub(super) fn child_rust_log_filter() -> String {
    const DEFAULTS: &str = "buzz_acp=info,buzz_agent=info";
    match std::env::var("RUST_LOG") {
        // An operator filter that already names both crates is taken verbatim.
        Ok(existing) if existing.contains("buzz_acp") && existing.contains("buzz_agent") => {
            existing
        }
        // Otherwise append only the directive that is missing, so an explicit
        // `buzz_agent=debug` is not overridden by our default.
        Ok(existing) if !existing.trim().is_empty() => {
            let mut filter = existing;
            if !filter.contains("buzz_acp") {
                filter.push_str(",buzz_acp=info");
            }
            if !filter.contains("buzz_agent") {
                filter.push_str(",buzz_agent=info");
            }
            filter
        }
        _ => DEFAULTS.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::child_rust_log_filter;

    /// Serialize: `RUST_LOG` is process-global and these tests mutate it.
    fn with_rust_log<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("RUST_LOG").ok();
        match value {
            Some(v) => std::env::set_var("RUST_LOG", v),
            None => std::env::remove_var("RUST_LOG"),
        }
        let result = body();
        match previous {
            Some(v) => std::env::set_var("RUST_LOG", v),
            None => std::env::remove_var("RUST_LOG"),
        }
        result
    }

    #[test]
    fn default_admits_both_the_harness_and_the_agent() {
        // buzz_agent was missing here, which silently discarded every agent
        // turn diagnostic from the desktop's per-agent log files.
        let filter = with_rust_log(None, child_rust_log_filter);
        assert!(filter.contains("buzz_acp=info"));
        assert!(filter.contains("buzz_agent=info"));
    }

    #[test]
    fn an_unrelated_operator_filter_is_extended_not_replaced() {
        let filter = with_rust_log(Some("warn"), child_rust_log_filter);
        assert!(filter.starts_with("warn"), "operator filter must survive");
        assert!(filter.contains("buzz_acp=info"));
        assert!(filter.contains("buzz_agent=info"));
    }

    #[test]
    fn an_explicit_agent_directive_is_not_downgraded() {
        let filter = with_rust_log(Some("buzz_agent=debug"), child_rust_log_filter);
        assert!(filter.contains("buzz_agent=debug"));
        assert!(!filter.contains("buzz_agent=info"), "must not override");
        assert!(filter.contains("buzz_acp=info"), "still needs the harness");
    }

    #[test]
    fn a_fully_specified_filter_is_taken_verbatim() {
        let filter = with_rust_log(
            Some("buzz_acp=warn,buzz_agent=trace"),
            child_rust_log_filter,
        );
        assert_eq!(filter, "buzz_acp=warn,buzz_agent=trace");
    }
}
