#[test]
fn database_metrics_and_slow_logs_exclude_sensitive_or_unbounded_fields() {
    let implementation = include_str!("../src/runtime/observability.rs");
    let datastore_macro = include_str!("../../buzz-datastore-tracing/src/lib.rs");
    let instrumentation = format!("{implementation}\n{datastore_macro}");

    for forbidden in [
        "\"community\" =>",
        "\"event_id\" =>",
        "\"event_kind\" =>",
        "\"kind\" =>",
        "\"sql\" =>",
        "\"query\" =>",
        "\"query_id\" =>",
        "\"d_tag\" =>",
        "\"coordinate\" =>",
        "community =",
        "event_id =",
        "event_kind =",
        "sql =",
        "query_id =",
        "d_tag =",
        "coordinate =",
    ] {
        assert!(
            !instrumentation.contains(forbidden),
            "database instrumentation must not expose {forbidden}"
        );
    }

    assert!(datastore_macro.contains("name: LitStr"));
    assert!(datastore_macro.contains("\"operation\" => #name"));
    assert!(datastore_macro.contains("elapsed_ms ="));
    assert!(
        datastore_macro.contains("parent: None"),
        "slow warnings must not inherit dynamic datastore span fields"
    );
    // The runtime tracing-layer assertion covers field names because a source
    // search would also match ordinary local variables such as `record_error`.
}

#[test]
fn relay_admin_db_wrappers_have_exactly_one_datastore_span() {
    for (domain, source) in [
        (
            "relay_admin_actions",
            include_str!("../src/store/relay_admin_actions.rs"),
        ),
        (
            "relay_operators",
            include_str!("../src/store/relay_operators.rs"),
        ),
    ] {
        let db_impl = source
            .split_once("impl crate::Db {")
            .unwrap_or_else(|| panic!("{domain} must own its Db wrappers"))
            .1
            .split_once("\n#[cfg(test)]")
            .unwrap_or_else(|| panic!("{domain} Db wrappers must precede focused tests"))
            .0;
        let mut pending_spans = 0;
        let mut methods = 0;

        for line in db_impl.lines() {
            if line.contains("#[datastore_span(") {
                pending_spans += 1;
            }
            if line.trim_start().starts_with("pub async fn ") {
                assert_eq!(pending_spans, 1, "{domain} wrapper `{line}` span count");
                pending_spans = 0;
                methods += 1;
            }
        }

        assert!(methods > 0, "{domain} must own public Db wrappers");
        assert_eq!(
            pending_spans, 0,
            "{domain} has an unattached datastore span"
        );
    }
}
