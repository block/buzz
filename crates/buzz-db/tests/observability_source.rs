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
        .expect("runtime must expose the public event-write transaction seam")
        .1
        .split_once("pub async fn insert_event_with_serving_write_guard(")
        .expect("public event-write transaction must precede guarded writes")
        .0;
    assert!(event_write_transaction.contains(COMMUNITY_CHOKEPOINT_LEGACY_MARKER));

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
    assert!(soft_delete_discovery.contains("execute(tx.conn())"));

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
    // The single aggregate row is read on the attributed writer connection,
    // never directly on the pool.
    assert!(restriction_state.contains("fetch_one(&mut *connection)"));
    assert!(!restriction_state.contains("(pool)"));

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
    // Lease seams that delegate to the bounded helper inherit its attribution.
    let bounded_lease_sql = deletion
        .split_once("async fn bounded_serving_lease_sql<T>(")
        .expect("deletion store must expose the bounded serving-lease helper")
        .1
        .split_once("/// Acquire a durable, expiring lease")
        .expect("bounded serving-lease helper must precede lease acquisition")
        .0;
    assert!(bounded_lease_sql.contains("WriterOperation::EventWrite"));
    assert!(!bounded_lease_sql.contains("self.pool.begin().await"));
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
            function.contains("WriterOperation::EventWrite")
                || function.contains("self.bounded_serving_lease_sql("),
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
fn pool_level_insert_mentions_opens_the_tenant_local_chokepoint() {
    let runtime = include_str!("../src/runtime/mod.rs");
    let insert_mentions = runtime
        .split_once("pub async fn insert_mentions(\n")
        .expect("runtime must expose pool-level insert_mentions")
        .1
        .split_once("pub(crate) async fn insert_mentions_in_transaction(")
        .expect("pool-level insert_mentions must precede its transaction seam")
        .0;
    assert!(
        insert_mentions.contains(COMMUNITY_CHOKEPOINT_MARKER),
        "post-commit mention indexing must be admitted like any other event-table write"
    );
}

#[test]
fn event_write_paths_include_tenant_local_chokepoint_calls() {
    let event = include_str!("../src/store/event.rs");
    let insert_event = event
        .split_once("pub async fn insert_event(\n")
        .expect("event store must expose pool-level insert_event")
        .1
        .split_once("/// Insert a Nostr event in a caller-owned admitted transaction")
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
        .split_once("pub async fn begin_event_write_transaction(\n")
        .expect("Db must expose the public event-write entrypoint")
        .1
        .split_once("/// Insert an event while holding and validating an admitted serving-write")
        .expect("public Db entrypoint must precede the serving-lease writer")
        .0;
    assert!(
        legacy_db_wrapper.contains(COMMUNITY_CHOKEPOINT_LEGACY_MARKER),
        "Db::begin_event_write_transaction must preserve the legacy compatibility population"
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
        .split_once("/// Insert a Nostr event in a caller-owned admitted transaction")
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

/// Function-level routing check for guarded-table writers.
///
/// A file can contain both a legitimate chokepoint writer and a bypass writer.
/// Every writing function must either open the tenant-local community
/// chokepoint itself, take a caller-owned `&mut AdmittedTx` (which only the
/// chokepoint can construct, so the compiler proves provenance), use reviewed
/// adapter-owned transaction state whose constructor is pinned to the
/// chokepoint, or appear in a reviewed exception list below.
///
/// A raw `&mut Transaction` or `&mut PgConnection` parameter is not a route:
/// nothing about its type says the transaction was admitted.
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

const ADMITTED_TX_SIGNATURE_MARKERS: [&str; 2] = ["&mut AdmittedTx", "&mut crate::AdmittedTx"];

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

/// Split production source into one slice per function. A slice runs from its
/// signature to the doc comment or attributes of the next function, so a
/// following function's docs (which may quote SQL) are never attributed to the
/// function before it.
fn function_slices(production_source: &str) -> Vec<&str> {
    // (signature offset, offset where the next function's lead-in begins)
    let mut starts = Vec::new();
    let mut lead_ins = Vec::new();
    let mut lead_in: Option<usize> = None;
    let mut offset = 0usize;
    for line in production_source.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if fn_signature_starts_here(trimmed) {
            starts.push(offset + indent);
            lead_ins.push(lead_in.take().unwrap_or(offset + indent));
        } else if trimmed.starts_with("///") || trimmed.starts_with("#[") {
            lead_in.get_or_insert(offset);
        } else {
            lead_in = None;
        }
        offset += line.len();
    }

    let mut functions = Vec::new();
    for (index, start) in starts.iter().enumerate() {
        let end = lead_ins
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

fn function_accepts_admitted_transaction(function_source: &str) -> bool {
    let signature = function_source.split('{').next().unwrap_or(function_source);
    ADMITTED_TX_SIGNATURE_MARKERS
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
        || function_accepts_admitted_transaction(function_source)
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

#[test]
fn serving_table_policy_requires_admitted_tx_not_raw_transactions() {
    let source = r#"
pub(crate) async fn raw_transaction_writer(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) {
    sqlx::query("INSERT INTO events (community_id, id) VALUES ($1, $2)")
        .execute(&mut **tx)
        .await
        .expect("write");
}
pub(crate) async fn raw_connection_writer(conn: &mut sqlx::PgConnection) {
    sqlx::query("INSERT INTO event_mentions (community_id, event_id) VALUES ($1, $2)")
        .execute(conn)
        .await
        .expect("write");
}
pub(crate) async fn admitted_writer(tx: &mut AdmittedTx) {
    sqlx::query("INSERT INTO events (community_id, id) VALUES ($1, $2)")
        .bind(tx.community().as_uuid())
        .execute(&mut **tx)
        .await
        .expect("write");
}
"#;

    let violations = syntactic_guarded_write_route_violations(source);
    assert_eq!(
        violations,
        [
            "pub(crate) async fn raw_transaction_writer(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) {",
            "pub(crate) async fn raw_connection_writer(conn: &mut sqlx::PgConnection) {",
        ],
        "a raw transaction or connection parameter carries no admission proof; only \
         `&mut AdmittedTx` does"
    );
}

#[test]
fn admitted_tx_is_constructed_only_by_admitting_constructors() {
    // The fields are private to `runtime/admitted_tx.rs`, so only that file can
    // build the value. Pin that each function there that builds it also admits
    // the transaction it wraps, so a new unguarded constructor cannot slip in.
    let source = include_str!("../src/runtime/admitted_tx.rs");
    let production: String = strip_cfg_test_items(source)
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        // `impl … for AdmittedTx {` headers open a block, not a value.
        .filter(|line| !line.trim_start().starts_with("impl"))
        .map(|line| format!("{line}\n"))
        .collect();
    let constructors: Vec<&str> = function_slices(&production)
        .into_iter()
        .filter(|function| function.contains("Self {") || function.contains("AdmittedTx {"))
        .collect();
    assert_eq!(
        constructors.len(),
        2,
        "AdmittedTx must have exactly the two admitting constructors"
    );
    for constructor in constructors {
        assert!(
            constructor.contains(".guard_transaction(&mut tx, community)")
                || constructor.contains(".guard_transaction_with_serving_lease(&mut tx, lease)"),
            "AdmittedTx constructor must admit the transaction it wraps: {constructor}"
        );
    }
}

/// Remove each top-level `#[cfg(test)]` item and keep the production code
/// around it. Truncating at the first `#[cfg(test)]` would hide production
/// functions that follow a test-only item. Tracks brace depth: the item ends
/// on the first line that leaves depth at or below zero and ends with `;` or
/// `}`, ignoring a trailing `//` comment. A column-0 `}` line (rustfmt's
/// top-level close) always ends it. Braces inside string or char literals are
/// still counted, so a test-only item with unbalanced literal braces (for
/// example `"{"`) runs on to the next column-0 `}`; no such item exists today.
fn strip_cfg_test_items(source: &str) -> String {
    let mut kept = String::with_capacity(source.len());
    let mut lines = source.lines();
    while let Some(line) = lines.next() {
        if line != "#[cfg(test)]" {
            kept.push_str(line);
            kept.push('\n');
            continue;
        }
        let mut depth = 0_isize;
        for line in lines.by_ref().skip_while(|line| line.starts_with("#[")) {
            depth += line.matches('{').count() as isize - line.matches('}').count() as isize;
            // Any `//` may start the trailing comment (an earlier one can sit
            // inside a string such as `"https://…"`), so try every prefix. A
            // false match inside a string only ends the item early, which
            // scans more lines and can never hide production code.
            let item_end = depth <= 0
                && line
                    .match_indices("//")
                    .map(|(at, _)| &line[..at])
                    .chain([line])
                    .any(|text| {
                        let text = text.trim_end();
                        text.ends_with(';') || text.ends_with('}')
                    });
            if item_end || line == "}" || line == "};" {
                break;
            }
        }
    }
    kept
}

#[test]
fn cfg_test_items_are_skipped_without_hiding_later_production_code() {
    let source = "pub fn before() {}\n\
#[cfg(test)]\n\
struct Marker;\n\
pub fn after_struct() {}\n\
#[cfg(test)]\n\
static LOCK: std::sync::Mutex<()> =\n\
    std::sync::Mutex::new(());\n\
pub fn after_static() {}\n\
#[cfg(test)]\n\
static S: [u8; 1] =\n\
    [const { 0 }; 1];\n\
pub fn after_const_block() {}\n\
#[cfg(test)]\n\
#[derive(Debug)]\n\
struct Fields {\n\
    value: u8,\n\
}\n\
pub fn after_fields() {}\n\
#[cfg(test)]\n\
mod tests {\n\
    fn hidden() {\n\
    }\n\
}\n\
pub fn after_module() {}\n";
    let production = strip_cfg_test_items(source);
    for name in [
        "before",
        "after_struct",
        "after_static",
        "after_const_block",
        "after_fields",
        "after_module",
    ] {
        assert!(
            production.contains(&format!("pub fn {name}()")),
            "{name} is production code and must stay visible: {production}"
        );
    }
    for hidden in ["Marker", "LOCK", "static S", "value: u8", "fn hidden"] {
        assert!(
            !production.contains(hidden),
            "{hidden} is test-only and must be skipped: {production}"
        );
    }

    let raw_writer = "pub(crate) async fn raw_writer(conn: &mut sqlx::PgConnection) {\n\
    sqlx::query(\"INSERT INTO events (community_id, id) VALUES ($1, $2)\")\n\
        .execute(conn)\n\
        .await\n\
        .expect(\"write\");\n\
}\n";
    for (test_item, hidden) in [
        ("struct X;", "struct X"),
        ("fn helper() {} // test helper", "fn helper"),
        ("const BRACES: &str = \"{}\"; // fixture", "BRACES"),
        (
            "const URL: &str = \"https://relay.test\"; // fixture",
            "relay.test",
        ),
        ("fn u() -> &'static str { \"ws://x\" } // c", "ws://x"),
    ] {
        let production = strip_cfg_test_items(&format!("#[cfg(test)]\n{test_item}\n{raw_writer}"));
        assert!(
            !production.contains(hidden),
            "`{test_item}` is test-only and must be skipped: {production}"
        );
        assert!(
            production_contains_guarded_write(&production),
            "a guarded write after `{test_item}` must stay visible: {production}"
        );
        assert_eq!(
            syntactic_guarded_write_route_violations(&production),
            ["pub(crate) async fn raw_writer(conn: &mut sqlx::PgConnection) {"],
            "a raw writer after `{test_item}` must still be scanned"
        );
    }
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

    // `buzz-relay` is scanned too: relay code reaches fenced tables only through
    // `buzz-db` helpers, and a direct relay write must meet the same rule.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut checked_guarded_files = 0usize;
    for (crate_name, src_root) in [
        ("buzz-db", manifest.join("src")),
        ("buzz-relay", manifest.join("../buzz-relay/src")),
    ] {
        let mut files = Vec::new();
        collect_rs_files(&src_root, &mut files);
        assert!(
            !files.is_empty(),
            "guarded-table scan must see {crate_name} production source files"
        );
        for path in files {
            let relative = path
                .strip_prefix(&src_root)
                .expect("file is under src root")
                .to_string_lossy()
                .replace('\\', "/");
            let source = std::fs::read_to_string(&path).expect("read source file");
            let production = strip_cfg_test_items(&source);
            if production_contains_guarded_write(&production) {
                checked_guarded_files += 1;
                let violations = syntactic_guarded_write_route_violations(&production);
                assert!(
                    violations.is_empty(),
                    "{crate_name}/src/{relative} has guarded-table INSERT/UPDATE/DELETE seams \
                     that neither open the tenant-local chokepoint nor take `&mut AdmittedTx`: \
                     {violations:?}"
                );
            }
        }
    }
    assert!(
        checked_guarded_files > 0,
        "guarded-table scan must exercise at least one file that writes a fenced table"
    );
}

// Rolled-back verification probes write `events` rows that never commit, so
// they owe no push job or TTL refresh.
const EVENT_INSERT_FOLLOW_UP_EXCEPTIONS: [&str; 2] = [
    "pub async fn verify_floor_guard_behavior(",
    "pub async fn verify_channel_roster_fence_behavior(",
];

/// `source` with every `#[cfg(test)]` item removed, wherever it sits in the
/// file. Each item runs to the `;` that ends it or to the brace that closes
/// its body. Braces inside string, raw string, and char literals and in line
/// comments are not counted. Unlike cutting the file at the first
/// `#[cfg(test)]`, this keeps production code that follows a test-only item.
/// The marker counts as an attribute only when it is the first non-whitespace
/// text on its line, so a mention in a comment or string strips nothing.
fn strip_cfg_test_items(source: &str) -> String {
    const MARKER: &str = "#[cfg(test)]";
    let mut kept = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(at) = rest.find(MARKER) {
        let line_start = rest[..at].rfind('\n').map_or(0, |newline| newline + 1);
        if !rest[line_start..at].trim().is_empty() {
            kept.push_str(&rest[..at + MARKER.len()]);
            rest = &rest[at + MARKER.len()..];
            continue;
        }
        kept.push_str(&rest[..at]);
        let item = &rest[at + MARKER.len()..];
        rest = &item[cfg_test_item_len(item)..];
    }
    kept.push_str(rest);
    kept
}

fn cfg_test_item_len(item: &str) -> usize {
    let bytes = item.as_bytes();
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                continue;
            }
            b'r' if matches!(bytes.get(index + 1), Some(b'"' | b'#'))
                && !bytes
                    .get(index.wrapping_sub(1))
                    .is_some_and(|prev| prev.is_ascii_alphanumeric() || *prev == b'_') =>
            {
                let hashes = bytes[index + 1..]
                    .iter()
                    .take_while(|byte| **byte == b'#')
                    .count();
                if bytes.get(index + 1 + hashes) == Some(&b'"') {
                    let close = format!("\"{}", "#".repeat(hashes));
                    let body = index + 2 + hashes;
                    index = item[body..]
                        .find(&close)
                        .map_or(bytes.len(), |offset| body + offset + close.len());
                    continue;
                }
            }
            b'"' => {
                index += 1;
                while index < bytes.len() && bytes[index] != b'"' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
            }
            // A char literal such as '{', '"', or '\''. A lifetime has no
            // closing quote and falls through.
            b'\''
                if bytes.get(index + 2) == Some(&b'\'') && bytes.get(index + 1) != Some(&b'\\') =>
            {
                index += 3;
                continue;
            }
            b'\''
                if bytes.get(index + 1) == Some(&b'\\') && bytes.get(index + 3) == Some(&b'\'') =>
            {
                index += 4;
                continue;
            }
            b';' if depth == 0 => return index + 1,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return index + 1;
                }
            }
            _ => {}
        }
        index += 1;
    }
    item.len()
}

/// Whether `source` inserts into `events` itself (not `event_mentions` or any
/// other `events_*` table), in any letter case, across line breaks and string
/// continuations, `\n`/`\t`/`\r` escapes, and with or without a `public.`
/// schema prefix.
fn inserts_events_row(source: &str) -> bool {
    let mut normalized = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_whitespace() || ch == '\\' {
            if ch == '\\' && matches!(chars.peek(), Some('n' | 't' | 'r')) {
                chars.next();
            }
            if !normalized.ends_with(' ') {
                normalized.push(' ');
            }
        } else {
            normalized.push(ch.to_ascii_lowercase());
        }
    }
    ["insert into events", "insert into public.events"]
        .iter()
        .any(|marker| {
            normalized.match_indices(marker).any(|(index, marker)| {
                normalized[index + marker.len()..]
                    .chars()
                    .next()
                    .is_none_or(|next| !(next.is_ascii_alphanumeric() || next == '_'))
            })
        })
}

/// An events insert must run both follow-ups: the push enqueue and the
/// pre-commit TTL refresh. `after_admitted_insert` does both; a writer that
/// inserts inside a savepoint calls the two halves separately.
fn runs_event_follow_ups(function_source: &str) -> bool {
    function_source.contains("event_follow_up::after_admitted_insert(")
        || (function_source.contains("event_follow_up::enqueue_push_match(")
            && function_source.contains(".record_channel_event("))
}

fn event_insert_follow_up_violations(production_source: &str) -> Vec<String> {
    function_slices(production_source)
        .into_iter()
        .filter(|function_source| {
            let header = function_header(function_source);
            inserts_events_row(function_source)
                && !EVENT_INSERT_FOLLOW_UP_EXCEPTIONS
                    .iter()
                    .any(|exception| header.starts_with(exception))
                && !runs_event_follow_ups(function_source)
        })
        .map(|function_source| function_header(function_source).to_owned())
        .collect()
}

#[test]
fn event_insert_follow_up_policy_rejects_writers_without_the_hook() {
    let source = r##"
pub(crate) async fn unhooked_writer(tx: &mut AdmittedTx) {
    sqlx::query("INSERT INTO events (community_id, id) VALUES ($1, $2)")
        .execute(&mut **tx)
        .await
        .expect("write");
}
pub(crate) async fn push_only_writer(tx: &mut AdmittedTx) {
    sqlx::query("INSERT INTO events \
                 (community_id, id) VALUES ($1, $2)")
        .execute(&mut **tx)
        .await
        .expect("write");
    crate::store::event_follow_up::enqueue_push_match(&mut **tx, community, id, kind)
        .await
        .expect("enqueue");
}
pub(crate) async fn hooked_writer(tx: &mut AdmittedTx) {
    sqlx::query("INSERT INTO events (community_id, id) VALUES ($1, $2)")
        .execute(&mut **tx)
        .await
        .expect("write");
    crate::store::event_follow_up::after_admitted_insert(tx, id, kind, channel)
        .await
        .expect("follow up");
}
pub(crate) async fn savepoint_writer(tx: &mut AdmittedTx) {
    sqlx::query("INSERT INTO events (community_id, id) VALUES ($1, $2)")
        .execute(&mut **tx)
        .await
        .expect("write");
    crate::store::event_follow_up::enqueue_push_match(&mut **tx, community, id, kind)
        .await
        .expect("enqueue");
    tx.record_channel_event(channel, kind);
}
pub(crate) async fn mention_writer(tx: &mut AdmittedTx) {
    sqlx::query("INSERT INTO event_mentions (community_id, event_id) VALUES ($1, $2)")
        .execute(&mut **tx)
        .await
        .expect("write");
}
pub async fn verify_floor_guard_behavior(pool: &PgPool) {
    sqlx::query("INSERT INTO events (community_id, id) VALUES ($1, $2)")
        .execute(pool)
        .await
        .expect("probe");
}
pub(crate) async fn lowercase_writer(tx: &mut AdmittedTx) {
    sqlx::query("insert into
        public.events (community_id, id) VALUES ($1, $2)")
        .execute(&mut **tx)
        .await
        .expect("write");
}
pub(crate) async fn continued_writer(tx: &mut AdmittedTx) {
    sqlx::query("INSERT INTO \
                 events (community_id, id) VALUES ($1, $2)")
        .execute(&mut **tx)
        .await
        .expect("write");
}
/// Kept at module scope, not `#[cfg(test)]`, because production calls it.
pub(crate) async fn writer_under_doc_mention(tx: &mut AdmittedTx) {
    sqlx::query("INSERT INTO events (community_id, id) VALUES ($1, $2)")
        .execute(&mut **tx)
        .await
        .expect("write");
}
// see `#[cfg(test)] mod x {`
pub(crate) async fn writer_under_brace_mention(tx: &mut AdmittedTx) {
    sqlx::query("INSERT INTO events (community_id, id) VALUES ($1, $2)")
        .execute(&mut **tx)
        .await
        .expect("write");
}
pub(crate) async fn writer_with_escaped_newline(tx: &mut AdmittedTx) {
    sqlx::query("INSERT INTO\nevents (community_id, id) VALUES ($1, $2)")
        .execute(&mut **tx)
        .await
        .expect("write");
}
#[cfg(test)]
fn test_only_helper() -> (&'static str, char, &'static str, char) {
    // an unbalanced { in a comment
    ("{", '{', r#"}"} {"#, '"')
}
pub(crate) async fn writer_after_test_helper(tx: &mut AdmittedTx) {
    sqlx::query("INSERT INTO events (community_id, id) VALUES ($1, $2)")
        .execute(&mut **tx)
        .await
        .expect("write");
}
#[cfg(test)]
mod tests {
    async fn test_writer(pool: &PgPool) {
        sqlx::query("INSERT INTO events (community_id, id) VALUES ($1, $2)")
            .execute(pool)
            .await
            .expect("fixture");
    }
}
"##;

    assert_eq!(
        event_insert_follow_up_violations(&strip_cfg_test_items(source)),
        [
            "pub(crate) async fn unhooked_writer(tx: &mut AdmittedTx) {",
            "pub(crate) async fn push_only_writer(tx: &mut AdmittedTx) {",
            "pub(crate) async fn lowercase_writer(tx: &mut AdmittedTx) {",
            "pub(crate) async fn continued_writer(tx: &mut AdmittedTx) {",
            "pub(crate) async fn writer_under_doc_mention(tx: &mut AdmittedTx) {",
            "pub(crate) async fn writer_under_brace_mention(tx: &mut AdmittedTx) {",
            "pub(crate) async fn writer_with_escaped_newline(tx: &mut AdmittedTx) {",
            "pub(crate) async fn writer_after_test_helper(tx: &mut AdmittedTx) {",
        ],
        "an events insert must run the push enqueue and record the TTL refresh"
    );
}

#[test]
fn every_production_event_insert_runs_the_follow_up_hook() {
    use std::path::{Path, PathBuf};

    fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read source directory") {
            let path = entry.expect("read directory entry").path();
            if path.is_dir() {
                collect_rs_files(&path, out);
            } else if should_scan_guarded_write_source_file(&path) {
                out.push(path);
            }
        }
    }

    let crates_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate lives under crates/");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(crates_root).expect("read crates directory") {
        let src = entry.expect("read crates entry").path().join("src");
        if src.is_dir() {
            collect_rs_files(&src, &mut files);
        }
    }

    let mut hooked_writers = 0usize;
    let mut violations = Vec::new();
    for path in files {
        let source = std::fs::read_to_string(&path).expect("read source file");
        let production = strip_cfg_test_items(&source);
        if !inserts_events_row(&production) {
            continue;
        }
        for function_source in function_slices(&production) {
            if inserts_events_row(function_source) && runs_event_follow_ups(function_source) {
                hooked_writers += 1;
            }
        }
        violations.extend(
            event_insert_follow_up_violations(&production)
                .into_iter()
                .map(|header| format!("{}: {header}", path.display())),
        );
    }

    assert!(
        violations.is_empty(),
        "production `INSERT INTO events` writers must call `event_follow_up` so the push \
         enqueue and channel TTL refresh survive the trigger retirement: {violations:?}"
    );
    // The seven writers at the time of BUZZ-176. A lower count means the scan
    // stopped seeing a writer, not that one was removed safely.
    assert!(
        hooked_writers >= 7,
        "expected at least seven hooked event writers, found {hooked_writers}"
    );
}
