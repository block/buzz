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

#[test]
fn p0_pool_acquisitions_use_typed_operation_pairs_without_other() {
    let observability = include_str!("../src/runtime/observability.rs");
    assert!(observability.contains("enum PoolOperation"));
    assert!(observability.contains("pub(crate) enum WriterOperation"));
    assert!(observability.contains("pub(crate) enum ReaderOperation"));
    assert!(observability.contains("Self::WriterAuthentication"));
    assert!(observability.contains("Self::ReaderSubscriptionHistory"));
    assert!(observability.contains("pub(crate) async fn acquire_writer("));
    assert!(observability.contains("pub(super) async fn acquire_reader_with_legacy_metrics("));
    assert!(observability.contains("static POOL_WAITERS: [Mutex<u64>"));
    assert!(!observability.contains("AtomicU64"));
    assert!(!observability.contains("DbOperation::Other"));
    assert!(!observability.contains("\"other\""));
    assert!(!observability.contains("buzz_db_pool_acquire_timeouts_total"));
    assert!(!observability.contains("\"result\" =>"));
    let legacy_transaction = observability
        .split_once("pub(crate) async fn begin_transaction(")
        .expect("observability must expose attributed transaction acquisition")
        .1
        .split_once("pub(crate) async fn observe_advisory_lock")
        .expect("transaction acquisition must precede advisory-lock observation")
        .0;
    assert!(legacy_transaction.contains("acquire_writer_with_legacy_metrics("));

    let runtime = include_str!("../src/runtime/mod.rs");
    assert!(runtime.contains("observability::acquire_writer_until("));
    assert!(runtime.contains("WriterOperation::Readiness"));
    assert!(runtime.contains("WriterOperation::EventWrite"));
    assert!(runtime.contains("ReaderOperation::Bootstrap"));
    assert!(runtime.contains("pub async fn begin_event_write_transaction"));
    let reader_boot = runtime
        .split_once("async fn read_pool_boot_ping_once(")
        .expect("runtime must expose the reader boot probe")
        .1
        .split_once("#[cfg(test)]")
        .expect("reader boot probe must precede its test seam")
        .0;
    assert!(reader_boot.contains("acquire_reader_with_legacy_metrics("));
    let routed_reader = runtime
        .split_once("async fn proved_reader(")
        .expect("runtime must expose the routed-reader checkout")
        .1
        .split_once("async fn reader_aurora_capability_on(")
        .expect("routed-reader checkout must precede capability probing")
        .0;
    assert!(routed_reader.contains("acquire_reader_with_legacy_metrics(read_pool, operation)"));
    let event_write_transaction = runtime
        .split_once("pub async fn begin_event_write_transaction(")
        .expect("runtime must expose the legacy event-write transaction seam")
        .1
        .split_once("pub async fn insert_event_with_serving_write_guard(")
        .expect("legacy event-write transaction must precede guarded writes")
        .0;
    assert!(event_write_transaction.contains("acquire_writer_with_legacy_metrics("));

    let migration = include_str!("../src/runtime/migration.rs");
    let migration_lock = migration
        .split_once("pub(crate) async fn with_exclusive_schema_destruction_lock")
        .expect("migration must expose the schema-safety acquisition seam")
        .1
        .split_once("async fn reject_legacy_nip_rs_cardinality_ambiguity")
        .expect("schema-safety acquisition must precede migration validation")
        .0;
    assert!(migration_lock.contains("acquire_writer_with_legacy_metrics("));

    let allowlist = include_str!("../src/store/allowlist.rs");
    assert!(allowlist.contains("WriterOperation::Authentication"));
    assert!(allowlist.contains("WriterOperation::Authorization"));
    assert!(!allowlist.contains("fetch_one(&self.pool)"));

    let event = include_str!("../src/store/event.rs");
    assert!(event.contains("query_events_with_operation"));
    assert!(event.contains("WriterOperation::Authorization"));
    assert!(event.contains("WriterOperation::SubscriptionHistory"));
    assert!(event.contains("ReaderOperation::SubscriptionHistory"));
    let backfill_d_tags = event
        .split_once("pub async fn backfill_d_tags")
        .expect("event store must expose the startup d-tag backfill")
        .1
        .split_once("/// Soft-delete NIP-29 discovery events")
        .expect("d-tag backfill must precede discovery deletion")
        .0;
    assert!(backfill_d_tags.contains("WriterOperation::Bootstrap"));
    assert!(backfill_d_tags.contains("execute(&mut *connection)"));
    let soft_delete_discovery = event
        .split_once("pub async fn soft_delete_discovery_events")
        .expect("event store must expose discovery-event deletion")
        .1
        .split_once("\n}\n\n#[cfg(test)]")
        .expect("discovery deletion must end the production Db implementation")
        .0;
    assert!(soft_delete_discovery.contains("WriterOperation::EventWrite"));
    assert!(soft_delete_discovery.contains("begin_community_event_write_transaction("));
    assert!(soft_delete_discovery.contains("execute(&mut *tx)"));

    let side_effects = include_str!("../../buzz-relay/src/handlers/side_effects.rs");
    assert!(side_effects.contains("query_events_for_event_write"));
    assert!(side_effects.contains("query_events_for_bootstrap"));
    assert!(side_effects.contains(".list_channels_for_bootstrap("));

    let deletion = include_str!("../src/store/deletion.rs");
    let public_serving_catalog = deletion
        .split_once("pub async fn validate_serving_catalog(&self)")
        .expect("deletion store must preserve its public serving-catalog API")
        .1
        .split_once("async fn validate_serving_catalog_on")
        .expect("public serving-catalog validation must delegate to its connection helper")
        .0;
    assert!(public_serving_catalog.contains("WriterOperation::Bootstrap"));
    assert!(public_serving_catalog.contains("observability::acquire_writer("));
    assert!(public_serving_catalog.contains("validate_serving_catalog_on"));
    assert!(!public_serving_catalog.contains("self.pool.acquire()"));

    let thread = include_str!("../src/store/thread.rs");
    let thread_metadata = thread
        .split_once("pub async fn get_thread_metadata_by_event(")
        .expect("thread store must expose metadata lookup")
        .1
        .split_once("// -- Db API")
        .expect("metadata lookup must precede the Db wrapper section")
        .0;
    assert!(thread_metadata.contains("WriterOperation::EventWrite"));
    assert!(thread_metadata.contains("fetch_optional(&mut *connection)"));
    assert!(!thread_metadata.contains("fetch_optional(pool)"));

    let channel = include_str!("../src/store/channel.rs");
    assert!(channel.contains("async fn begin_event_write_transaction("));
    assert!(channel.contains("async fn acquire_event_write_connection("));
    for (start, end, expected) in [
        (
            "pub async fn create_channel(\n",
            "/// Creates a channel with a client-supplied UUID",
            "begin_event_write_transaction(pool)",
        ),
        (
            "pub async fn create_channel_with_id(\n",
            "/// Fetches a channel record by `(community_id, id)`",
            "begin_event_write_transaction(pool)",
        ),
        (
            "pub async fn update_channel(\n",
            "/// Sets the topic for a channel",
            "begin_event_write_transaction(pool)",
        ),
        (
            "pub async fn set_topic(\n",
            "/// Sets the purpose for a channel",
            "acquire_event_write_connection(pool)",
        ),
        (
            "pub async fn set_purpose(\n",
            "/// Archives a channel",
            "acquire_event_write_connection(pool)",
        ),
        (
            "pub async fn archive_channel(\n",
            "/// Unarchives a channel",
            "acquire_event_write_connection(pool)",
        ),
        (
            "pub async fn unarchive_channel(\n",
            "/// Soft-delete a channel",
            "acquire_event_write_connection(pool)",
        ),
        (
            "pub async fn soft_delete_channel(\n",
            "/// Archive ephemeral channels",
            "acquire_event_write_connection(pool)",
        ),
    ] {
        let function = channel
            .split_once(start)
            .unwrap_or_else(|| panic!("missing channel seam {start}"))
            .1
            .split_once(end)
            .unwrap_or_else(|| panic!("channel seam {start} must precede {end}"))
            .0;
        assert!(
            function.contains(expected),
            "channel seam {start} must use {expected}"
        );
        assert!(!function.contains("pool.begin().await"));
        assert!(!function.contains(".execute(pool)"));
        assert!(!function.contains(".fetch_optional(pool)"));
    }
    let get_channel = channel
        .split_once("async fn get_channel_with_operation(")
        .expect("channel store must route shared lookups through caller-owned intent")
        .1
        .split_once("/// Returns the canvas content")
        .expect("channel lookup helper must precede canvas reads")
        .0;
    assert!(get_channel.contains("acquire_writer(pool, operation)"));
    assert!(get_channel.contains("fetch_optional(&mut *connection)"));
    assert!(!get_channel.contains("fetch_optional(pool)"));
    assert!(channel.contains("pub async fn get_channel_for_event_write("));
    let list_channels = channel
        .split_once("async fn list_channels_with_operation(")
        .expect("channel listing must accept caller-owned intent")
        .1
        .split_once("/// A channel archived by the ephemeral-channel reaper")
        .expect("channel listing must precede ephemeral-channel types")
        .0;
    assert!(list_channels.contains("acquire_writer(pool, operation)"));
    assert!(list_channels.contains("fetch_all(&mut *connection)"));
    assert!(!list_channels.contains("fetch_all(pool)"));
    assert!(channel.contains("pub async fn list_channels_for_bootstrap("));

    let channel_members = include_str!("../src/store/channel_members.rs");
    assert!(channel_members.contains("async fn get_members_with_operation("));
    assert!(channel_members.contains("pub async fn get_members_for_event_write("));
    assert!(channel_members.contains("async fn get_users_bulk_with_operation("));
    assert!(channel_members.contains("pub async fn get_users_bulk_for_event_write("));

    let huddle_link = event
        .split_once("async fn huddle_started_link_exists_with_operation(")
        .expect("huddle link lookup must accept caller-owned intent")
        .1
        .split_once("/// Insert a Nostr event")
        .expect("huddle link lookup must precede event insertion")
        .0;
    assert!(huddle_link.contains("acquire_writer(pool, operation)"));
    assert!(event.contains("pub async fn huddle_started_link_exists_for_event_write("));
    let ingest = include_str!("../../buzz-relay/src/handlers/ingest.rs");
    assert!(ingest.contains(".huddle_started_link_exists_for_event_write("));
    let audio = include_str!("../../buzz-relay/src/audio/handler.rs");
    assert!(audio.contains(".huddle_started_link_exists("));

    let workflow_sink = include_str!("../../buzz-relay/src/workflow_sink.rs");
    assert!(workflow_sink.contains(".get_members_for_event_write("));
    assert!(workflow_sink.contains(".get_users_bulk_for_event_write("));

    for write_caller in [
        include_str!("../../buzz-relay/src/handlers/side_effects.rs"),
        include_str!("../../buzz-relay/src/handlers/ingest.rs"),
        include_str!("../../buzz-relay/src/handlers/command_executor.rs"),
        workflow_sink,
    ] {
        assert!(!write_caller.contains(".get_channel("));
        assert!(write_caller.contains(".get_channel_for_event_write("));
    }

    let user = include_str!("../src/store/user.rs");
    let agent_channel_policy = user
        .split_once("pub async fn get_agent_channel_policy(")
        .expect("user store must expose get_agent_channel_policy")
        .1
        .split_once("/// Check whether `actor_pubkey`")
        .expect("agent policy lookup must precede owner lookup")
        .0;
    assert!(agent_channel_policy.contains("WriterOperation::Authorization"));
    assert!(agent_channel_policy.contains("fetch_optional(&mut *connection)"));
    assert!(!agent_channel_policy.contains("fetch_optional(pool)"));
    let is_agent_owner = user
        .split_once("pub async fn is_agent_owner(")
        .expect("user store must expose is_agent_owner")
        .1
        .split_once("/// Set the channel_add_policy")
        .expect("is_agent_owner must precede set_agent_channel_policy")
        .0;
    assert!(is_agent_owner.contains("WriterOperation::Authorization"));
    assert!(is_agent_owner.contains("acquire_writer("));
    assert!(is_agent_owner.contains("fetch_optional(&mut *connection)"));
    assert!(!is_agent_owner.contains("fetch_optional(pool)"));

    let moderation = include_str!("../src/store/moderation.rs");
    let restriction_state = moderation
        .split_once("pub async fn restriction_state(")
        .expect("moderation store must expose restriction_state")
        .1
        .split_once("/// Fetch the full ban/timeout row")
        .expect("restriction state must precede full ban reads")
        .0;
    assert!(restriction_state.contains("WriterOperation::Authorization"));
    assert!(restriction_state.contains("fetch_optional(&mut *connection)"));
    assert!(!restriction_state.contains("fetch_optional(pool)"));

    let community_store = include_str!("../src/store/community.rs");
    let ensure_community = community_store
        .split_once("pub async fn ensure_configured_community(")
        .expect("community store must expose ensure_configured_community")
        .1
        .split_once("/// Atomically creates a community")
        .expect("configured-community helpers must precede community creation")
        .0;
    assert!(ensure_community.contains("WriterOperation::Authorization"));
    assert!(ensure_community.contains("WriterOperation::Bootstrap"));
    assert!(ensure_community.contains("ensure_configured_community_with_operation"));
    assert!(ensure_community.contains("acquire_writer(&self.pool, operation)"));
    assert!(ensure_community.contains("fetch_optional(&mut *connection)"));
    let management_lookup = community_store
        .split_once("pub async fn lookup_community_by_host_for_management(")
        .expect("community store must expose management host lookup")
        .1
        .split_once("/// Lists communities where")
        .expect("management lookup must precede owner listing")
        .0;
    assert!(management_lookup.contains("WriterOperation::Authorization"));
    assert!(management_lookup.contains("fetch_optional(&mut *connection)"));
    assert!(!management_lookup.contains("fetch_optional(&self.pool)"));
    let community_production = community_store
        .split("\n#[cfg(test)]")
        .next()
        .expect("community production source");
    for required in [
        "WriterOperation::TenantResolution",
        "WriterOperation::Authorization",
        "WriterOperation::SubscriptionHistory",
        "WriterOperation::EventWrite",
    ] {
        assert!(
            community_production.contains(required),
            "community P0 paths must include {required} attribution"
        );
    }
    assert!(!community_production.contains("self.pool.begin().await"));
    assert!(!community_production.contains(".fetch_one(&self.pool)"));
    assert!(!community_production.contains(".fetch_all(&self.pool)"));
    assert!(!community_production.contains(".execute(&self.pool)"));
    assert_eq!(
        community_production
            .matches(".fetch_optional(&self.pool)")
            .count(),
        1,
        "only the out-of-scope NIP-11 metadata read may retain a raw pool checkout"
    );

    let thread_summary = thread
        .split_once("pub async fn get_thread_summary(")
        .expect("thread store must expose get_thread_summary")
        .1
        .split_once("/// Fetch one channel window")
        .expect("thread summary must precede channel-window reads")
        .0;
    assert!(thread_summary.contains("WriterOperation::EventWrite"));
    assert!(thread_summary.contains("fetch_optional(&mut *connection)"));
    assert!(thread_summary.contains("fetch_all(&mut *connection)"));
    assert!(!thread_summary.contains("fetch_optional(pool)"));
    assert!(!thread_summary.contains("fetch_all(pool)"));

    let archived_identities = include_str!("../src/store/archived_identities.rs");
    let archived_identity_production = archived_identities
        .split("\n#[cfg(test)]")
        .next()
        .expect("archived identity production source");
    assert_eq!(
        archived_identity_production
            .matches("WriterOperation::EventWrite")
            .count(),
        4,
        "all four archived identity operations must be attributed to event writes"
    );
    assert!(!archived_identity_production.contains("fetch_optional(pool)"));
    assert!(!archived_identity_production.contains("fetch_all(pool)"));
    assert!(!archived_identity_production.contains("execute(pool)"));

    let relay_main = include_str!("../../buzz-relay/src/main.rs");
    assert!(relay_main.contains("pool_state.db.refresh_pool_waiter_metrics();"));
    assert!(relay_main.contains(".ensure_configured_community_for_bootstrap("));

    let runtime = include_str!("../src/runtime/mod.rs");
    assert!(runtime.contains("observability::refresh_pool_waiters(self.read_pool.is_some())"));
    assert!(runtime.contains("self.verify_replica_fence_at_boot().await?"));
    let fence_boot = runtime
        .split_once("pub(crate) async fn verify_replica_fence_at_boot")
        .expect("runtime must expose attributed boot fence verification")
        .1
        .split_once("/// The pool for lag-tolerant reads")
        .expect("boot fence verification must precede routed-read plumbing")
        .0;
    assert!(fence_boot.contains("WriterOperation::Bootstrap"));

    let replica_fence = include_str!("../src/runtime/replica_fence.rs");
    let replica_fence_production = replica_fence
        .split("\n#[cfg(test)]")
        .next()
        .expect("replica-fence production source");
    assert!(replica_fence_production.contains("WriterOperation::Bootstrap"));
    assert!(replica_fence_production.contains("WriterOperation::Maintenance"));
    assert!(!replica_fence_production.contains("pool.begin().await"));
    assert!(!replica_fence_production.contains("writer.acquire().await"));
    assert!(!replica_fence_production.contains("fetch_optional(writer)"));

    let usage = include_str!("../src/store/usage.rs");
    let usage_production = usage
        .split("\n#[cfg(test)]")
        .next()
        .expect("usage production source");
    let usage_leader_lock = usage_production
        .split_once("pub async fn try_lock_usage_metrics(")
        .expect("usage store must expose the legacy leader-lock acquisition")
        .1
        .split_once("pub async fn usage_community_count(")
        .expect("usage leader lock must precede counter reads")
        .0;
    assert!(usage_leader_lock.contains("acquire_writer_with_legacy_metrics("));
    assert!(
        usage_production
            .matches("WriterOperation::Maintenance")
            .count()
            >= 11,
        "every periodic usage checkout must be maintenance-attributed"
    );
    for bypass in [
        ".fetch_one(pool)",
        ".fetch_all(pool)",
        ".fetch_optional(pool)",
        ".execute(pool)",
    ] {
        assert!(
            !usage_production.contains(bypass),
            "usage production path bypasses operation attribution with {bypass}"
        );
    }

    let channel_reaper = channel
        .split_once("pub async fn reap_expired_ephemeral_channels(pool:")
        .expect("channel store must expose ephemeral reaper")
        .1
        .split_once("\nimpl Db {")
        .expect("ephemeral reaper must precede Db wrappers")
        .0;
    assert!(channel_reaper.contains("WriterOperation::Maintenance"));
    assert!(channel_reaper.contains("fetch_all(&mut *connection)"));

    let deletion = include_str!("../src/store/deletion.rs");
    let lease_reaper = deletion
        .split_once("pub async fn reap_expired_serving_write_leases")
        .expect("deletion store must expose serving-lease reaper")
        .1
        .split_once("/// Return serving-lease counts")
        .expect("serving-lease reaper must precede stats")
        .0;
    assert!(lease_reaper.contains("WriterOperation::Maintenance"));
    assert!(lease_reaper.contains("execute(&mut *connection)"));
    let lease_stats = deletion
        .split_once("pub async fn serving_lease_stats")
        .expect("deletion store must expose serving-lease stats")
        .1
        .split_once("/// Whether a community remains active")
        .expect("serving-lease stats must precede serving-state reads")
        .0;
    assert!(lease_stats.contains("WriterOperation::Maintenance"));
    assert!(lease_stats.contains("fetch_one(&mut *connection)"));
    for (start, end) in [
        (
            "pub async fn acquire_serving_write_lease",
            "/// Renew an already-admitted external side-effect lease",
        ),
        (
            "pub async fn renew_serving_write_lease",
            "/// Release a serving side-effect lease",
        ),
        (
            "pub async fn release_serving_write_lease",
            "/// Check that an external side-effect lease remains current",
        ),
        (
            "pub async fn verify_serving_write_lease",
            "/// Delete expired serving leases",
        ),
        (
            "pub async fn is_serving_active",
            "async fn advance_with_checkpoint",
        ),
    ] {
        let function = deletion
            .split_once(start)
            .unwrap_or_else(|| panic!("missing serving-write seam {start}"))
            .1
            .split_once(end)
            .unwrap_or_else(|| panic!("serving-write seam {start} must precede {end}"))
            .0;
        assert!(
            function.contains("WriterOperation::EventWrite"),
            "serving-write seam {start} must be event-write attributed"
        );
        assert!(!function.contains("self.pool.begin().await"));
        assert!(!function.contains(".execute(&self.pool)"));
        assert!(!function.contains(".fetch_one(&self.pool)"));
    }

    let ensure_authorization = user
        .split_once("pub async fn ensure_user_for_authorization(")
        .expect("user store must expose NIP-OA authorization ensure")
        .1
        .split_once("/// Get a single user record")
        .expect("authorization ensure must precede generic user reads")
        .0;
    assert!(ensure_authorization.contains("WriterOperation::Authorization"));
    let set_owner_authorization = user
        .split_once("pub async fn set_agent_owner_for_authorization(")
        .expect("user store must expose NIP-OA authorization owner write")
        .1
        .split_once("/// Get the channel_add_policy")
        .expect("authorization owner write must precede policy reads")
        .0;
    assert!(set_owner_authorization.contains("WriterOperation::Authorization"));
    let relay_api = include_str!("../../buzz-relay/src/api/mod.rs");
    assert!(relay_api.contains(".ensure_user_for_authorization("));
    assert!(relay_api.contains(".set_agent_owner_for_authorization("));

    for (domain, source) in [
        (
            "channel_members",
            include_str!("../src/store/channel_members.rs"),
        ),
        ("archived_identities", archived_identities),
        ("event", event),
        ("git_repo", include_str!("../src/store/git_repo.rs")),
        ("push", include_str!("../src/store/push.rs")),
        ("replica_fence", replica_fence),
        ("reaction", include_str!("../src/store/reaction.rs")),
        ("relay_invite", include_str!("../src/store/relay_invite.rs")),
        (
            "relay_members",
            include_str!("../src/store/relay_members.rs"),
        ),
        ("thread", thread),
        (
            "relay_operators",
            include_str!("../src/store/relay_operators.rs"),
        ),
        ("usage", usage),
    ] {
        let production = source.split("\n#[cfg(test)]").next().unwrap_or(source);
        for bypass in [
            "pool.begin().await",
            "self.pool.begin().await",
            ".fetch_one(pool)",
            ".fetch_one(&self.pool)",
            ".fetch_all(pool)",
            ".fetch_all(&self.pool)",
            ".fetch_optional(pool)",
            ".fetch_optional(&self.pool)",
            ".execute(pool)",
            ".execute(&self.pool)",
        ] {
            assert!(
                !production.contains(bypass),
                "{domain} production path bypasses operation attribution with {bypass}"
            );
        }
    }
}

#[test]
fn insert_mentions_docs_describe_syntactic_scope_and_db_backstop() {
    let runtime = include_str!("../src/runtime/mod.rs");
    assert!(
        runtime.contains("does not provide application-admission provenance"),
        "runtime::insert_mentions docs must describe syntactic-only routing scope"
    );
    assert!(
        runtime.contains("community-write fence remains the authoritative safety"),
        "runtime::insert_mentions docs must name database fencing as the safety backstop"
    );
}

#[test]
fn event_write_paths_include_tenant_local_chokepoint_calls() {
    fn has_any_tenant_local_chokepoint(source: &str) -> bool {
        source.contains(COMMUNITY_CHOKEPOINT_MARKER)
            || source.contains(COMMUNITY_CHOKEPOINT_LEGACY_MARKER)
    }

    let event = include_str!("../src/store/event.rs");
    let insert_event = event
        .split_once("pub async fn insert_event(\n")
        .expect("event store must expose pool-level insert_event")
        .1
        .split_once("/// Insert a Nostr event in a caller-owned PostgreSQL transaction.")
        .expect("pool insert must precede transaction-seam insert")
        .0;
    assert!(
        has_any_tenant_local_chokepoint(insert_event),
        "pool-level event inserts must include a tenant-local event-write chokepoint call"
    );
    let insert_with_thread_meta = event
        .split_once("pub async fn insert_event_with_thread_metadata(\n")
        .expect("event store must expose pool-level thread-metadata insert")
        .1
        .split_once("impl Db {")
        .expect("pool thread-metadata insert must precede Db wrappers")
        .0;
    assert!(
        has_any_tenant_local_chokepoint(insert_with_thread_meta),
        "thread-metadata event inserts must include a tenant-local chokepoint call"
    );

    let replaceable = include_str!("../src/store/replaceable.rs");
    let replace_addressable = replaceable
        .split_once("pub async fn replace_addressable_event(\n")
        .expect("replaceable store must expose replace_addressable_event")
        .1
        .split_once("/// Replace a NIP-33 event inside a caller-owned transaction.")
        .expect("addressable replacement must precede parameterized transaction seam")
        .0;
    assert!(
        has_any_tenant_local_chokepoint(replace_addressable),
        "addressable replacements must include a tenant-local chokepoint call"
    );
    let replace_parameterized = replaceable
        .split_once("pub async fn replace_parameterized_event(\n")
        .expect("replaceable store must expose replace_parameterized_event")
        .1
        .split_once("}\n\n#[cfg(test)]")
        .expect("parameterized replacement must precede tests")
        .0;
    assert!(
        has_any_tenant_local_chokepoint(replace_parameterized),
        "parameterized replacements must include a tenant-local chokepoint call"
    );

    let channel_members = include_str!("../src/store/channel_members.rs");
    let snapshot_lock = channel_members
        .split_once("pub async fn lock_member_snapshot(\n")
        .expect("channel_members must expose lock_member_snapshot")
        .1
        .split_once("/// Add a member to a channel.")
        .expect("snapshot lock path must precede member add path")
        .0;
    assert!(
        has_any_tenant_local_chokepoint(snapshot_lock),
        "snapshot publication locks must include a tenant-local chokepoint call"
    );

    let relay_members = include_str!("../src/store/relay_members.rs");
    let publish_snapshot = relay_members
        .split_once("pub async fn publish_nip43_membership_locked(\n")
        .expect("relay_members must expose publish_nip43_membership_locked")
        .1
        .split_once("}\n\n#[cfg(test)]")
        .expect("membership publish path must precede tests")
        .0;
    assert!(
        has_any_tenant_local_chokepoint(publish_snapshot),
        "NIP-43 membership publication must include a tenant-local chokepoint call"
    );

    let push = include_str!("../src/store/push.rs");
    let accept_lease = push
        .split_once("pub async fn accept_lease_event(\n")
        .expect("push store must expose accept_lease_event")
        .1
        .split_once("fn constraint_acceptance_outcome")
        .expect("accept_lease_event must precede constraint outcome mapping")
        .0;
    assert!(
        has_any_tenant_local_chokepoint(accept_lease),
        "push lease source-event writes must include a tenant-local chokepoint call"
    );

    let reaction = include_str!("../src/store/reaction.rs");
    let insert_reaction = reaction
        .split_once("pub async fn insert_reaction_event_with_thread_metadata(\n")
        .expect("reaction store must expose insert_reaction_event_with_thread_metadata")
        .1
        .split_once("/// Soft-delete a reaction by setting")
        .expect("reaction insert must precede reaction soft-delete")
        .0;
    assert!(
        has_any_tenant_local_chokepoint(insert_reaction),
        "live kind:7 reaction inserts must include a tenant-local chokepoint call"
    );
}

#[test]
fn legacy_compatibility_metrics_remain_pinned_to_the_preexisting_event_write_entrypoints() {
    let runtime = include_str!("../src/runtime/mod.rs");
    let typed_helper = runtime
        .split_once("pub(crate) async fn begin_community_event_write_transaction(\n")
        .expect("runtime must expose the typed tenant-local chokepoint")
        .1
        .split_once(
            "pub(crate) async fn begin_community_event_write_transaction_with_legacy_metrics(\n",
        )
        .expect("typed chokepoint must precede the legacy compatibility wrapper")
        .0;
    assert!(
        typed_helper.contains("CommunityEventWriteMetricPopulation::TypedOnly"),
        "the default tenant-local chokepoint must stay typed-only"
    );
    assert!(
        !typed_helper.contains("CommunityEventWriteMetricPopulation::LegacyCompatibility"),
        "the default tenant-local chokepoint must not emit legacy compatibility metrics"
    );

    let legacy_db_wrapper = runtime
        .split_once("pub async fn begin_community_write_transaction(\n")
        .expect("Db must expose the legacy community write entrypoint")
        .1
        .split_once("/// Begin an event-write transaction that takes the shared replica-floor")
        .expect("legacy Db wrapper must precede the replica-floor entrypoint")
        .0;
    assert!(
        legacy_db_wrapper.contains(COMMUNITY_CHOKEPOINT_LEGACY_MARKER),
        "Db::begin_community_write_transaction must preserve the legacy compatibility population"
    );

    let replaceable = include_str!("../src/store/replaceable.rs");
    for (label, start, end) in [
        (
            "replace_addressable_event",
            "pub async fn replace_addressable_event(\n",
            "/// Atomically replace a NIP-33 parameterized replaceable event.",
        ),
        (
            "replace_parameterized_event",
            "pub async fn replace_parameterized_event(\n",
            "}\n\n#[cfg(test)]",
        ),
    ] {
        let seam = replaceable
            .split_once(start)
            .unwrap_or_else(|| panic!("replaceable store must expose {label}"))
            .1
            .split_once(end)
            .unwrap_or_else(|| panic!("{label} must precede its next production seam"))
            .0;
        assert!(
            seam.contains(COMMUNITY_CHOKEPOINT_LEGACY_MARKER),
            "{label} must preserve the legacy compatibility population"
        );
    }

    let relay_members = include_str!("../src/store/relay_members.rs");
    let publish_snapshot = relay_members
        .split_once("pub async fn publish_nip43_membership_locked(\n")
        .expect("relay_members must expose publish_nip43_membership_locked")
        .1
        .split_once("}\n\n#[cfg(test)]")
        .expect("membership publish path must precede tests")
        .0;
    assert!(
        publish_snapshot.contains(COMMUNITY_CHOKEPOINT_LEGACY_MARKER),
        "NIP-43 membership publication must preserve the legacy compatibility population"
    );

    let push = include_str!("../src/store/push.rs");
    let accept_lease = push
        .split_once("pub async fn accept_lease_event(\n")
        .expect("push store must expose accept_lease_event")
        .1
        .split_once("fn constraint_acceptance_outcome")
        .expect("accept_lease_event must precede constraint outcome mapping")
        .0;
    assert!(
        accept_lease.contains(COMMUNITY_CHOKEPOINT_LEGACY_MARKER),
        "push lease acceptance must preserve the legacy compatibility population"
    );

    let event = include_str!("../src/store/event.rs");
    let insert_event = event
        .split_once("pub async fn insert_event(\n")
        .expect("event store must expose pool-level insert_event")
        .1
        .split_once("/// Insert a Nostr event in a caller-owned PostgreSQL transaction.")
        .expect("pool insert must precede transaction-seam insert")
        .0;
    assert!(
        insert_event.contains(COMMUNITY_CHOKEPOINT_MARKER),
        "typed-only event inserts must stay on the typed tenant-local chokepoint"
    );
    assert!(
        !insert_event.contains(COMMUNITY_CHOKEPOINT_LEGACY_MARKER),
        "typed-only event inserts must not emit legacy compatibility metrics"
    );

    let reaction = include_str!("../src/store/reaction.rs");
    let insert_reaction = reaction
        .split_once("pub async fn insert_reaction_event_with_thread_metadata(\n")
        .expect("reaction store must expose insert_reaction_event_with_thread_metadata")
        .1
        .split_once("/// Soft-delete a reaction by setting")
        .expect("reaction insert must precede reaction soft-delete")
        .0;
    assert!(
        insert_reaction.contains(COMMUNITY_CHOKEPOINT_MARKER),
        "typed-only reaction inserts must stay on the typed tenant-local chokepoint"
    );
    assert!(
        !insert_reaction.contains(COMMUNITY_CHOKEPOINT_LEGACY_MARKER),
        "typed-only reaction inserts must not emit legacy compatibility metrics"
    );
}

/// Function-level syntactic routing backstop.
///
/// A file can contain both a legitimate chokepoint writer and a bypass writer.
/// This check is intentionally source-shape only: every writing function must
/// expose a syntactic route marker by either calling the tenant-local
/// community chokepoint, accepting a caller-owned guarded
/// transaction/connection, or using reviewed adapter-owned transaction state
/// whose constructor is pinned to the same chokepoint.
///
/// It does not prove transaction/connection provenance or relay-side admission;
/// commit-time database fences remain the authoritative safety backstop.
const GUARDED_TABLE_WRITE_MARKERS: [&str; 9] = [
    "INSERT INTO events",
    "UPDATE events",
    "DELETE FROM events",
    "INSERT INTO reactions",
    "UPDATE reactions",
    "DELETE FROM reactions",
    "INSERT INTO event_mentions",
    "UPDATE event_mentions",
    "DELETE FROM event_mentions",
];

const COMMUNITY_CHOKEPOINT_MARKER: &str = "begin_community_event_write_transaction(";
const COMMUNITY_CHOKEPOINT_LEGACY_MARKER: &str =
    "begin_community_event_write_transaction_with_legacy_metrics(";

fn has_any_tenant_local_chokepoint(source: &str) -> bool {
    source.contains(COMMUNITY_CHOKEPOINT_MARKER)
        || source.contains(COMMUNITY_CHOKEPOINT_LEGACY_MARKER)
}

const GUARDED_TX_SIGNATURE_MARKERS: [&str; 4] = [
    "&mut sqlx::Transaction<",
    "&mut Transaction<",
    "&mut PgConnection",
    "&mut sqlx::PgConnection",
];

// Narrow reviewed exceptions for non-serving verification probes only.
const GUARDED_WRITE_FUNCTION_EXCEPTIONS: [&str; 3] = [
    "pub async fn verify_floor_guard_behavior(",
    "pub async fn verify_channel_roster_fence_behavior(",
    "pub async fn backfill_d_tags(&self) -> Result<u64> {",
];

const GUARDED_TX_ADAPTER_METHOD_PINS: [(&str, &str); 1] = [(
    "pub async fn replace_member_event(",
    "pub async fn lock_member_snapshot(",
)];

fn production_contains_guarded_write(production_source: &str) -> bool {
    GUARDED_TABLE_WRITE_MARKERS
        .iter()
        .any(|marker| production_source.contains(marker))
}

fn fn_signature_starts_here(trimmed_line: &str) -> bool {
    [
        "fn ",
        "async fn ",
        "pub fn ",
        "pub async fn ",
        "pub(crate) fn ",
        "pub(crate) async fn ",
        "pub(super) fn ",
        "pub(super) async fn ",
    ]
    .iter()
    .any(|prefix| trimmed_line.starts_with(prefix))
}

fn function_slices(production_source: &str) -> Vec<&str> {
    let mut starts = Vec::new();
    let mut offset = 0usize;
    for line in production_source.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if fn_signature_starts_here(trimmed) {
            starts.push(offset + (line.len() - trimmed.len()));
        }
        offset += line.len();
    }

    let mut functions = Vec::new();
    for (index, start) in starts.iter().enumerate() {
        let end = starts
            .get(index + 1)
            .copied()
            .unwrap_or(production_source.len());
        functions.push(&production_source[*start..end]);
    }
    functions
}

fn function_header(function_source: &str) -> &str {
    function_source
        .lines()
        .next()
        .unwrap_or("<unknown function>")
        .trim()
}

fn function_is_guarded_write_exception(function_header: &str) -> bool {
    GUARDED_WRITE_FUNCTION_EXCEPTIONS
        .iter()
        .any(|exception| function_header.starts_with(exception))
}

fn function_accepts_guarded_transaction_or_connection(function_source: &str) -> bool {
    let signature = function_source.split('{').next().unwrap_or(function_source);
    GUARDED_TX_SIGNATURE_MARKERS
        .iter()
        .any(|marker| signature.contains(marker))
}

fn function_uses_guarded_tx_adapter_state(function_source: &str) -> bool {
    function_source.contains("&mut *self.tx") || function_source.contains("&mut self.tx")
}

fn adapter_constructor_is_chokepoint_pinned(production_source: &str, method_header: &str) -> bool {
    for (adapter_method, adapter_constructor) in GUARDED_TX_ADAPTER_METHOD_PINS {
        if method_header.starts_with(adapter_method) {
            return function_slices(production_source)
                .into_iter()
                .find(|function_source| {
                    function_header(function_source).starts_with(adapter_constructor)
                })
                .is_some_and(has_any_tenant_local_chokepoint);
        }
    }

    false
}

fn function_has_syntactic_guarded_write_route(
    function_source: &str,
    production_source: &str,
) -> bool {
    let header = function_header(function_source);

    has_any_tenant_local_chokepoint(function_source)
        || function_accepts_guarded_transaction_or_connection(function_source)
        || (function_uses_guarded_tx_adapter_state(function_source)
            && adapter_constructor_is_chokepoint_pinned(production_source, header))
}

fn syntactic_guarded_write_route_violations(production_source: &str) -> Vec<String> {
    function_slices(production_source)
        .into_iter()
        .filter(|function_source| {
            let header = function_header(function_source);
            production_contains_guarded_write(function_source)
                && !function_is_guarded_write_exception(header)
                && !function_has_syntactic_guarded_write_route(function_source, production_source)
        })
        .map(|function_source| function_header(function_source).to_owned())
        .collect()
}

#[test]
fn serving_table_policy_rejects_same_file_mixed_guarded_writers() {
    let mixed_source = r#"
pub async fn guarded_writer(pool: &sqlx::PgPool) {
    let mut tx = begin_community_event_write_transaction(pool, community, WriterOperation::EventWrite)
        .await
        .expect("tx");
    sqlx::query("INSERT INTO events (community_id, id) VALUES ($1, $2)")
        .execute(&mut *tx)
        .await
        .expect("write");
}
pub async fn unguarded_writer(pool: &sqlx::PgPool) {
    sqlx::query("INSERT INTO events (community_id, id) VALUES ($1, $2)")
        .execute(pool)
        .await
        .expect("unguarded");
}
"#;

    let violations = syntactic_guarded_write_route_violations(mixed_source);
    assert!(
        violations
            .iter()
            .any(|name| name.starts_with("pub async fn unguarded_writer(")),
        "whole-file co-occurrence policy is too weak: one guarded writer in a file must not bless \
         a sibling unguarded writer; violations: {violations:?}"
    );
    assert!(
        !violations
            .iter()
            .any(|name| name.starts_with("pub async fn guarded_writer(")),
        "guarded writer must remain allowed; violations: {violations:?}"
    );
}

#[test]
fn serving_table_policy_rejects_writer_connection_only_bypasses() {
    let acquire_writer_bypass = r#"
pub async fn bypass_with_writer_connection(pool: &sqlx::PgPool) {
    let mut connection = crate::observability::acquire_writer(pool, WriterOperation::EventWrite)
        .await
        .expect("connection");
    sqlx::query("INSERT INTO events (community_id, id) VALUES ($1, $2)")
        .execute(&mut *connection)
        .await
        .expect("write");
}
"#;

    let pool_begin_bypass = r#"
pub async fn bypass_with_pool_begin(pool: &sqlx::PgPool) {
    let mut tx = pool.begin().await.expect("tx");
    sqlx::query("INSERT INTO events (community_id, id) VALUES ($1, $2)")
        .execute(&mut *tx)
        .await
        .expect("write");
}
"#;

    let acquire_writer_violations = syntactic_guarded_write_route_violations(acquire_writer_bypass);
    assert!(
        acquire_writer_violations
            .iter()
            .any(|name| name.starts_with("pub async fn bypass_with_writer_connection(")),
        "acquire_writer + raw guarded-table write must be rejected; violations: \
         {acquire_writer_violations:?}"
    );

    let pool_begin_violations = syntactic_guarded_write_route_violations(pool_begin_bypass);
    assert!(
        pool_begin_violations
            .iter()
            .any(|name| name.starts_with("pub async fn bypass_with_pool_begin(")),
        "pool.begin + raw guarded-table write must be rejected; violations: \
         {pool_begin_violations:?}"
    );
}

fn should_scan_guarded_write_source_file(path: &std::path::Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    path.extension().is_some_and(|ext| ext == "rs")
        // Whole files loaded only via `#[cfg(test)] #[path = "..."] mod ...;` are
        // entirely test code but carry no internal `#[cfg(test)]` marker of their
        // own to slice against. Standalone `*_tests.rs` modules share that shape.
        && file_name != "tests.rs"
        && !file_name.ends_with("_tests.rs")
}

#[test]
fn serving_table_policy_skips_standalone_test_modules() {
    use std::path::Path;

    assert!(
        !should_scan_guarded_write_source_file(Path::new("src/runtime/tests.rs")),
        "`tests.rs` is a standalone test-only module"
    );
    assert!(
        !should_scan_guarded_write_source_file(Path::new(
            "src/store/thread_window/postgres_tests.rs",
        )),
        "`*_tests.rs` modules are standalone test-only sources, not production seams"
    );
    assert!(
        !should_scan_guarded_write_source_file(Path::new("src/store/foo_tests.rs")),
        "the suffix-based rule must cover other standalone test-only modules"
    );
    assert!(
        should_scan_guarded_write_source_file(Path::new("src/store/event.rs")),
        "production source must remain in scope"
    );
}

#[test]
fn serving_table_writes_expose_syntactic_chokepoint_or_guarded_tx_routes() {
    use std::path::{Path, PathBuf};

    fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read source directory") {
            let entry = entry.expect("read directory entry");
            let path = entry.path();
            if path.is_dir() {
                collect_rs_files(&path, out);
            } else if should_scan_guarded_write_source_file(&path) {
                out.push(path);
            }
        }
    }

    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&src_root, &mut files);
    assert!(
        !files.is_empty(),
        "guarded-table scan must see production source files"
    );

    let mut checked_guarded_files = 0usize;
    for path in files {
        let relative = path
            .strip_prefix(&src_root)
            .expect("file is under src root")
            .to_string_lossy()
            .replace('\\', "/");
        let source = std::fs::read_to_string(&path).expect("read source file");
        let production = source.split("\n#[cfg(test)]").next().unwrap_or(&source);
        if production_contains_guarded_write(production) {
            checked_guarded_files += 1;
            let violations = syntactic_guarded_write_route_violations(production);
            assert!(
                violations.is_empty(),
                "{relative} has guarded-table INSERT/UPDATE/DELETE seams without a syntactic \
                 route marker (tenant-local chokepoint call or caller-owned guarded \
                 transaction/connection): {violations:?}; this source policy does not prove \
                 provenance, so database fences remain the authoritative backstop"
            );
        }
    }
    assert!(
        checked_guarded_files > 0,
        "guarded-table scan must exercise at least one file that writes a fenced table"
    );
}
