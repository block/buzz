use super::*;

#[test]
fn unavailable_memory_degrades_only_memory_dependent_specialists() {
    let backend = FakeBackend::with_state(|state| {
        state.memory_results =
            VecDeque::from([Err(SourceReadError::new("local_memory_unavailable"))]);
    });
    let context = collector(backend, "Brief")
        .freeze()
        .expect("RAG and Apple remain usable");

    assert_eq!(
        context.degraded_sections(),
        &[
            BriefSection::Operations,
            BriefSection::Navigation,
            BriefSection::DailyRoutine,
            BriefSection::Reports,
            BriefSection::Planning306090,
        ]
    );
    assert!(!source_kinds(&context).contains(&SourceKind::Memory));
    assert!(source_kinds(&context).contains(&SourceKind::Rag));
    assert!(source_kinds(&context).contains(&SourceKind::Calendar));
}

#[test]
fn conflicted_memory_result_is_rejected_and_memory_sections_degrade() {
    let backend = FakeBackend::with_state(|state| {
        let mut wrapper = phase3_memory_wrapper();
        wrapper["results"][0]["conflicted_fields"] = json!(["content"]);
        state.memory_results = VecDeque::from([Ok(wrapper)]);
    });
    let context = collector(backend, "Brief")
        .freeze()
        .expect("invalid Memory evidence remains fail soft");

    assert!(!source_kinds(&context).contains(&SourceKind::Memory));
    assert!(context
        .limitations()
        .iter()
        .any(|item| item.contains("revision") && item.contains("citation")));
    assert!(context
        .degraded_sections()
        .contains(&BriefSection::Operations));
}

#[test]
fn unresolved_memory_heads_are_visible_when_conflict_safe_context_omits_them() {
    let backend = FakeBackend::with_state(|state| {
        state.memory_results = VecDeque::from([Ok(empty_memory_wrapper())]);
        state.memory_conflict_count = 2;
    });
    let context = collector(backend, "Brief")
        .freeze()
        .expect("conflict-safe omission remains fail soft");

    assert!(context
        .limitations()
        .iter()
        .any(|item| item.contains("2 unresolved Memory conflicts")));
    assert!(context
        .degraded_sections()
        .contains(&BriefSection::ConflictsAndGaps));
}

#[test]
fn missing_or_mixed_rag_evidence_degrades_rag_sections_without_losing_local_inputs() {
    for rag_result in [
        Err(SourceReadError::new("rag_unavailable")),
        Ok(rag_evidence(SNAPSHOT_B, "rag:mixed", "Wrong snapshot.")),
    ] {
        let backend = FakeBackend::with_state(|state| {
            state.rag_results = VecDeque::from([rag_result]);
        });
        let context = collector(backend, "Brief")
            .freeze()
            .expect("fail soft retrieval");
        assert_eq!(
            context.degraded_sections(),
            &[
                BriefSection::Operations,
                BriefSection::Navigation,
                BriefSection::DailyRoutine,
                BriefSection::Reports,
                BriefSection::Planning306090,
            ]
        );
        let rag_sources = context
            .ledger()
            .iter()
            .filter(|source| source.source_kind() == SourceKind::Rag)
            .collect::<Vec<_>>();
        assert_eq!(rag_sources.len(), 1);
        assert!(rag_sources[0].source_id().starts_with("rag:snapshot:"));
        assert!(source_kinds(&context).contains(&SourceKind::Memory));
        assert!(source_kinds(&context).contains(&SourceKind::Calendar));
    }
}

#[test]
fn stale_or_invalid_active_rag_snapshot_fails_before_any_source_read() {
    for error in [
        SourceCollectionError::RagStale,
        SourceCollectionError::RagInvalid,
        SourceCollectionError::RagUnavailable,
    ] {
        let backend = FakeBackend {
            initial_snapshot: Err(error.clone()),
            state: Mutex::new(FakeState::default()),
        };
        let collector = collector(backend, "Brief");
        assert!(matches!(collector.freeze(), Err(observed) if observed == error));
        assert!(collector.backend().requests().is_empty());
    }
}

#[test]
fn snapshot_change_returns_restart_signal_and_recheck_contract_detects_later_change() {
    let changed_during = FakeBackend::with_state(|state| {
        state.recheck_snapshot = Some(SNAPSHOT_B.to_string());
    });
    assert!(matches!(
        collector(changed_during, "Brief").freeze(),
        Err(SourceCollectionError::SnapshotChanged)
    ));

    let stable = collector(FakeBackend::fresh(), "Brief");
    let frozen = stable.freeze().expect("stable freeze");
    stable
        .recheck_snapshot(&frozen)
        .expect("before consolidation");
    stable
        .backend()
        .state
        .lock()
        .expect("fake state")
        .recheck_snapshot = Some(SNAPSHOT_B.to_string());
    assert_eq!(
        stable.recheck_snapshot(&frozen),
        Err(SourceCollectionError::SnapshotChanged)
    );
}

#[test]
fn exact_duplicate_sources_are_deduplicated_but_conflicting_ids_fail_closed() {
    let duplicate = rag_evidence(SNAPSHOT_A, "rag:duplicate", "Same quote.");
    let backend = FakeBackend::with_state(|state| {
        state.rag_results = VecDeque::from([Ok(json!({
            "schema": duplicate["schema"],
            "tool_policy": duplicate["tool_policy"],
            "query": duplicate["query"],
            "snapshot": duplicate["snapshot"],
            "retrieved_at": duplicate["retrieved_at"],
            "total": 2,
            "results": [duplicate["results"][0], duplicate["results"][0]]
        }))]);
    });
    let context = collector(backend, "Brief")
        .freeze()
        .expect("exact duplicate is canonicalised");
    assert_eq!(
        context
            .ledger()
            .iter()
            .filter(|source| source.source_id() == "rag:duplicate")
            .count(),
        1
    );

    let backend = FakeBackend::with_state(|state| {
        let first = rag_evidence(SNAPSHOT_A, "rag:duplicate", "First quote.");
        let mut second = first.clone();
        second["results"][0]["quoted_text"] = json!("Second quote.");
        state.rag_results = VecDeque::from([Ok(json!({
            "schema": first["schema"],
            "tool_policy": first["tool_policy"],
            "query": first["query"],
            "snapshot": first["snapshot"],
            "retrieved_at": first["retrieved_at"],
            "total": 2,
            "results": [first["results"][0], second["results"][0]]
        }))]);
    });
    assert!(matches!(
        collector(backend, "Brief").freeze(),
        Err(SourceCollectionError::ConflictingSourceIdentity)
    ));
}

#[test]
fn multiline_rag_and_memory_quotes_are_canonical_control_safe_strings() {
    let multiline = "Line one.\n\tLine two.";
    let backend = FakeBackend::with_state(|state| {
        state.rag_results =
            VecDeque::from([Ok(rag_evidence(SNAPSHOT_A, "rag:multiline", multiline))]);
        state.memory_results = VecDeque::from([Ok(phase3_memory_wrapper_with_quote(multiline))]);
    });
    let context = collector(backend, "Brief")
        .freeze()
        .expect("multiline producer evidence is normalised");

    for (kind, source_id) in [
        (SourceKind::Rag, Some("rag:multiline")),
        (SourceKind::Memory, None),
    ] {
        let source = context
            .ledger()
            .iter()
            .find(|source| {
                source.source_kind() == kind
                    && source_id.is_none_or(|source_id| source.source_id() == source_id)
            })
            .expect("normalised source");
        assert!(!source.quote().chars().any(char::is_control));
        assert!(source.quote().contains(r"\n"));
        assert!(source.quote().contains(r"\t"));
        assert_eq!(
            serde_json::from_str::<String>(source.quote()).expect("reversible JSON string"),
            multiline,
        );
    }
}

#[test]
fn unnormalisable_quote_degrades_only_its_source_path() {
    let backend = FakeBackend::with_state(|state| {
        state.rag_results = VecDeque::from([Ok(rag_evidence(SNAPSHOT_A, "rag:blank", " \n\t "))]);
    });
    let context = collector(backend, "Brief")
        .freeze()
        .expect("bad RAG quote remains fail soft");

    assert!(!context
        .ledger()
        .iter()
        .any(|source| source.source_id() == "rag:blank"));
    assert!(source_kinds(&context).contains(&SourceKind::Memory));
    assert!(source_kinds(&context).contains(&SourceKind::Calendar));
    assert!(context
        .degraded_sections()
        .contains(&BriefSection::Navigation));
    assert!(!context
        .degraded_sections()
        .contains(&BriefSection::Decisions));
    assert!(context
        .limitations()
        .iter()
        .any(|item| item.contains("RAG") && item.contains("malformed")));
}

#[test]
fn oversized_source_text_is_utf8_safely_truncated_and_reported() {
    let quote = "⚓".repeat(MAX_TEXT_BYTES);
    let backend = FakeBackend::with_state(|state| {
        state.rag_results = VecDeque::from([Ok(rag_evidence(SNAPSHOT_A, "rag:large", &quote))]);
    });
    let context = collector(backend, "Brief")
        .freeze()
        .expect("bounded source");
    let source = context
        .ledger()
        .iter()
        .find(|source| source.source_id() == "rag:large")
        .expect("large source retained");
    assert!(source.quote().len() <= 1_024);
    assert!(source.quote().is_char_boundary(source.quote().len()));
    assert!(context
        .limitations()
        .iter()
        .any(|item| item.contains("rag:large") && item.contains("truncated")));
}

#[test]
fn frozen_limitations_are_deterministically_bounded_with_visible_omission() {
    let backend = FakeBackend::with_state(|state| {
        state.rag_results =
            VecDeque::from([Ok(rag_evidence_many(SNAPSHOT_A, 70, MAX_TEXT_BYTES + 1))]);
        state.apple_results = VecDeque::from([
            apple_response(
                "calendar",
                "authorized",
                vec![calendar_record("event-1", false, false, false)],
                true,
                None,
            ),
            apple_response(
                "reminders",
                "authorized",
                vec![reminder_record("reminder-1", false, false)],
                true,
                None,
            ),
            apple_response("notes", "authorized", vec![note_record()], true, None),
            apple_response("files", "authorized", vec![file_record()], true, None),
        ]);
    });

    let context = collector(backend, "Brief")
        .freeze()
        .expect("bounded context");
    assert_eq!(context.limitations().len(), MAX_ARRAY_ITEMS);
    assert!(context
        .limitations()
        .last()
        .is_some_and(|item| item.contains("additional source limitations omitted")));
}

#[test]
fn ledger_truncation_tracks_omitted_kinds_and_degrades_affected_sections() {
    let backend = FakeBackend::with_state(|state| {
        state.rag_results = (0..5)
            .map(|batch| Ok(rag_evidence_batch(SNAPSHOT_A, batch, 200)))
            .collect();
    });
    let context = collector(backend, "Brief")
        .freeze()
        .expect("bounded ledger");

    assert_eq!(context.ledger().len(), 72);
    for kind in [
        SourceKind::Calendar,
        SourceKind::Reminders,
        SourceKind::Notes,
        SourceKind::File,
    ] {
        assert!(
            context
                .ledger()
                .iter()
                .any(|source| source.source_kind() == kind),
            "{kind:?} must survive bounded ledger retention"
        );
    }
    assert!(context
        .degraded_sections()
        .contains(&BriefSection::Navigation));
    assert!(context
        .limitations()
        .iter()
        .any(|item| item.contains("RAG") && item.contains("omitted")));
}

#[test]
fn apple_selection_rejects_unknown_keys_duplicates_relative_files_and_bad_bounds() {
    let valid = json!({
        "schema_version": 1,
        "calendar_ids": ["calendar-command"],
        "reminder_list_ids": ["reminders-command"],
        "note_folder_ids": ["Notes"],
        "file_paths": ["/Users/command/brief.txt"],
        "maximum_records_per_source": 25
    });
    for mutation in [
        ("unknown", json!(true)),
        ("maximum_records_per_source", json!(0)),
        ("file_paths", json!(["relative.txt"])),
        ("calendar_ids", json!(["duplicate", "duplicate"])),
    ] {
        let mut value = valid.clone();
        value[mutation.0] = mutation.1;
        assert!(AppleBriefSelection::for_test(value).is_err());
    }
}

#[test]
fn production_apple_selection_api_has_no_unprotected_constructor() {
    let source = include_str!("../../command_services/apple_inputs.rs");
    assert!(!source.contains("pub(crate) fn parse("));
    assert!(source.contains("pub(crate) fn for_test("));
    assert!(source.contains("pub(crate) fn load_protected("));
}

#[cfg(unix)]
#[test]
fn apple_selection_loads_only_from_a_protected_native_config_file() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let working_directory = std::env::current_dir().expect("current directory");
    let directory = tempfile::Builder::new()
        .prefix(".apple-brief-config-test-")
        .tempdir_in(working_directory)
        .expect("temporary config directory");
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
        .expect("protect directory");
    let path = directory.path().join("command-apple-inputs.json");
    fs::write(
        &path,
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "calendar_ids": ["calendar-command"],
            "reminder_list_ids": ["reminders-command"],
            "note_folder_ids": ["Notes"],
            "file_paths": ["/Users/command/brief.txt"],
            "maximum_records_per_source": 25
        }))
        .expect("encode config"),
    )
    .expect("write config");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("protect config");

    let loaded = AppleBriefSelection::load_protected(&path).expect("protected config accepted");
    assert_eq!(
        serde_json::to_value(
            loaded
                .brief_requests(OBSERVED_AT)
                .expect("trusted requests")
                .first()
                .expect("calendar request")
        )
        .expect("serialize request")["arguments"]["calendar_ids"],
        json!(["calendar-command"])
    );

    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("weaken config");
    assert!(AppleBriefSelection::load_protected(&path).is_err());
}

#[test]
fn rag_evidence_requires_exact_native_query_and_verified_catalogue_membership() {
    for mut wrapper in [
        rag_evidence(SNAPSHOT_A, "rag:wrong-query", "Wrong query."),
        {
            let mut value = rag_evidence(SNAPSHOT_A, "rag:outside", "Outside catalogue.");
            value["results"][0]["source"]["collection"] = json!("unverified");
            value
        },
        {
            let mut value = rag_evidence_many(SNAPSHOT_A, 2, 16);
            value["results"][1]["source"]["collection"] = json!("unverified");
            value
        },
    ] {
        let mismatched_query = wrapper["results"][0]["source"]["source_id"] == "rag:wrong-query";
        if mismatched_query {
            wrapper["query"] = json!("renderer supplied query");
        }
        let backend = FakeBackend::with_state(|state| {
            state.bind_rag_query = false;
            state.rag_results = VecDeque::from([Ok(wrapper)]);
        });
        let context = collector(backend, "Brief")
            .freeze()
            .expect("invalid retrieval evidence degrades fail soft");

        assert!(context
            .ledger()
            .iter()
            .filter(|source| source.source_kind() == SourceKind::Rag)
            .all(|source| source.collection() == "verified_catalogue"));
        assert!(context
            .degraded_sections()
            .contains(&BriefSection::Navigation));
    }

    let valid = collector(FakeBackend::fresh(), "Brief")
        .freeze()
        .expect("verified catalogue result");
    assert!(valid
        .ledger()
        .iter()
        .any(|source| source.source_kind() == SourceKind::Rag
            && source.collection() == "navy-publications"));
}

#[test]
fn canonical_ledger_ids_are_stable_across_backend_order() {
    let context_a = collector(FakeBackend::fresh(), "Brief")
        .freeze()
        .expect("first");
    let backend_b = FakeBackend::with_state(|state| {
        state.apple_results.make_contiguous().reverse();
    });
    let context_b = collector(backend_b, "Brief").freeze().expect("second");
    let ids_a = context_a
        .ledger()
        .iter()
        .map(|source| (source.source_id(), source.ledger_id()))
        .collect::<BTreeMap<_, _>>();
    let ids_b = context_b
        .ledger()
        .iter()
        .map(|source| (source.source_id(), source.ledger_id()))
        .collect::<BTreeMap<_, _>>();
    for (source_id, ledger_id) in ids_a {
        if let Some(other) = ids_b.get(source_id) {
            assert_eq!(ledger_id, *other);
        }
    }
}

#[test]
fn canonical_ledger_ids_are_run_bound_but_stable_under_reorder_within_one_run() {
    let first = collector_for_run(FakeBackend::fresh(), RUN_A, "Brief")
        .freeze()
        .expect("first run view");
    let reordered_backend = FakeBackend::with_state(|state| {
        state.apple_results.make_contiguous().reverse();
    });
    let reordered = collector_for_run(reordered_backend, RUN_A, "Brief")
        .freeze()
        .expect("reordered same run");
    let second_run = collector_for_run(FakeBackend::fresh(), RUN_B, "Brief")
        .freeze()
        .expect("second run");
    assert_eq!(first.run_id(), RUN_A);
    assert_eq!(reordered.run_id(), RUN_A);
    assert_eq!(second_run.run_id(), RUN_B);

    let ids = |context: &FrozenSourceContext| {
        context
            .ledger()
            .iter()
            .map(|source| {
                (
                    source.source_id().to_string(),
                    source.ledger_id().to_string(),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let first_ids = ids(&first);
    let reordered_ids = ids(&reordered);
    let second_ids = ids(&second_run);
    for (source_id, ledger_id) in &first_ids {
        if let Some(reordered_id) = reordered_ids.get(source_id) {
            assert_eq!(ledger_id, reordered_id);
        }
        assert_ne!(
            Some(ledger_id),
            second_ids.get(source_id),
            "{source_id} must be bound to the trusted run"
        );
    }
}

fn authenticated_service(kind: KnowledgeServiceKind) -> AuthenticatedSourceService {
    let (name, endpoint, identity, tools) = match kind {
        KnowledgeServiceKind::Memory => (
            "memory",
            "http://127.0.0.1:18006/mcp",
            "node:command",
            MEMORY_CATALOG_TOOLS,
        ),
        KnowledgeServiceKind::Rag => (
            "rag",
            "http://127.0.0.1:18005/mcp/",
            SNAPSHOT_A,
            RAG_CATALOG_TOOLS,
        ),
    };
    AuthenticatedSourceService::new(
        VerifiedService {
            kind,
            server_identity: name.to_string(),
            endpoint: endpoint.to_string(),
            bearer_token: "fixture-source-token-123456789".to_string(),
            active_identity: identity.to_string(),
            advertised_tools: tools.iter().map(|tool| (*tool).to_string()).collect(),
            verified_at: OBSERVED_AT.to_string(),
        },
        "fixture-source-attestation-secret-123456789",
    )
    .expect("authenticated source")
}

struct FakeSourceToolCaller {
    calls: Mutex<Vec<Value>>,
    observed_snapshot: String,
}

impl SourceToolCaller for FakeSourceToolCaller {
    fn call(
        &self,
        _service: &AuthenticatedSourceService,
        tool_name: &str,
        arguments: Value,
        _cancellation: &CancellationToken,
    ) -> Result<Value, AdmissionError> {
        self.calls
            .lock()
            .expect("source caller")
            .push(json!({"tool":tool_name,"arguments":arguments}));
        match tool_name {
            "search_knowledge_base" => Ok(rag_evidence(
                SNAPSHOT_A,
                "rag:production",
                "Production evidence.",
            )),
            "command_memory_context" => Ok(empty_memory_wrapper()),
            "get_snapshot_status" => Ok(json!({
                "active_snapshot_id": self.observed_snapshot
            })),
            _ => Err(AdmissionError::UnexpectedToolCatalog),
        }
    }
}

#[test]
fn production_backend_uses_fixed_tool_arguments_and_rejects_snapshot_mismatch() {
    let caller = Arc::new(FakeSourceToolCaller {
        calls: Mutex::new(Vec::new()),
        observed_snapshot: SNAPSHOT_B.to_string(),
    });
    let snapshot = VerifiedRagSnapshot::for_test(SNAPSHOT_A, OBSERVED_AT, OBSERVED_AT);
    let backend = ProductionSourceBackend::from_bindings_for_test(
        snapshot.clone(),
        authenticated_service(KnowledgeServiceKind::Rag),
        authenticated_service(KnowledgeServiceKind::Memory),
        2,
        caller.clone(),
    );
    let intent = collector(FakeBackend::fresh(), "Brief")
        .freeze()
        .expect("fixture context")
        .retrieval_intents()[0]
        .clone();

    let cancellation = CancellationToken::new();
    assert!(backend
        .collect_rag(
            &snapshot,
            &intent,
            intent.context_query(),
            snapshot.logical_collections(),
            &cancellation,
        )
        .is_ok());
    assert!(backend.collect_memory(&intent, &cancellation).is_ok());
    assert_eq!(backend.memory_conflict_count(), 2);
    assert_eq!(
        backend.recheck_rag_snapshot(&snapshot, &cancellation),
        Err(SourceCollectionError::SnapshotChanged)
    );
    let calls = caller.calls.lock().expect("calls").clone();
    assert_eq!(calls[0]["tool"], "search_knowledge_base");
    assert_eq!(calls[0]["arguments"]["query"], intent.query());
    assert_eq!(
        calls[0]["arguments"]["collections"],
        json!(["navy-publications"])
    );
    assert_eq!(calls[0]["arguments"]["top_k"], 3);
    assert_eq!(calls[1]["tool"], "command_memory_context");
    assert_eq!(calls[1]["arguments"]["query"], intent.query());
    assert_eq!(calls[1]["arguments"]["limit"], 3);
    assert_eq!(calls[2]["tool"], "get_snapshot_status");
    assert_eq!(calls[2]["arguments"], json!({}));
}
