--
-- pgschema database dump
--

-- Dumped from database version PostgreSQL 17.10
-- Dumped by pgschema version 1.7.4


--
-- Name: approval_status; Type: TYPE; Schema: -; Owner: -
--

CREATE TYPE approval_status AS ENUM (
    'pending',
    'granted',
    'denied',
    'expired'
);

--
-- Name: channel_add_policy; Type: TYPE; Schema: -; Owner: -
--

CREATE TYPE channel_add_policy AS ENUM (
    'anyone',
    'owner_only',
    'nobody'
);

--
-- Name: channel_type; Type: TYPE; Schema: -; Owner: -
--

CREATE TYPE channel_type AS ENUM (
    'stream',
    'forum',
    'dm',
    'workflow'
);

--
-- Name: channel_visibility; Type: TYPE; Schema: -; Owner: -
--

CREATE TYPE channel_visibility AS ENUM (
    'open',
    'private'
);

--
-- Name: delivery_method; Type: TYPE; Schema: -; Owner: -
--

CREATE TYPE delivery_method AS ENUM (
    'webhook',
    'websocket'
);

--
-- Name: member_role; Type: TYPE; Schema: -; Owner: -
--

CREATE TYPE member_role AS ENUM (
    'owner',
    'admin',
    'member',
    'guest',
    'bot'
);

--
-- Name: pause_reason; Type: TYPE; Schema: -; Owner: -
--

CREATE TYPE pause_reason AS ENUM (
    'user',
    'system',
    'rate_limit'
);

--
-- Name: run_status; Type: TYPE; Schema: -; Owner: -
--

CREATE TYPE run_status AS ENUM (
    'pending',
    'running',
    'waiting_approval',
    'completed',
    'failed',
    'cancelled'
);

--
-- Name: subscription_status; Type: TYPE; Schema: -; Owner: -
--

CREATE TYPE subscription_status AS ENUM (
    'active',
    'paused',
    'deleted'
);

--
-- Name: workflow_status; Type: TYPE; Schema: -; Owner: -
--

CREATE TYPE workflow_status AS ENUM (
    'active',
    'disabled',
    'archived'
);

--
-- Name: _operator_global_tables; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS _operator_global_tables (
    table_name text,
    reason text NOT NULL,
    CONSTRAINT _operator_global_tables_pkey PRIMARY KEY (table_name)
);

--
-- Name: communities; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS communities (
    id uuid DEFAULT gen_random_uuid(),
    host varchar(255) NOT NULL,
    signing_key bytea,
    created_at timestamptz DEFAULT now() NOT NULL,
    icon text,
    archived_at timestamptz,
    CONSTRAINT communities_pkey PRIMARY KEY (id),
    CONSTRAINT chk_communities_id_not_nil CHECK (id <> '00000000-0000-0000-0000-000000000000'::uuid)
);

--
-- Name: idx_communities_host; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS idx_communities_host ON communities (lower(host::text));

--
-- Name: archived_identities; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS archived_identities (
    community_id uuid,
    pubkey text,
    consent_path text NOT NULL,
    actor text NOT NULL,
    reason text,
    replaced_by text,
    request_event_id text NOT NULL,
    archived_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT archived_identities_pkey PRIMARY KEY (community_id, pubkey),
    CONSTRAINT archived_identities_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT archived_identities_consent_path_check CHECK (consent_path IN ('self'::text, 'owner'::text, 'admin'::text))
);

--
-- Name: audit_log; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS audit_log (
    community_id uuid,
    seq bigint,
    hash bytea NOT NULL,
    prev_hash bytea,
    action varchar(64) NOT NULL,
    actor_pubkey bytea,
    object_id text,
    detail jsonb,
    created_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT audit_log_pkey PRIMARY KEY (community_id, seq),
    CONSTRAINT audit_log_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: idx_audit_log_hash; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS idx_audit_log_hash ON audit_log (community_id, hash);

--
-- Name: authorization_event_capacity; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS authorization_event_capacity (
    community_id uuid,
    max_events_per_domain bigint NOT NULL,
    max_bytes_per_domain bigint NOT NULL,
    max_envelope_bytes integer NOT NULL,
    retained_event_count bigint DEFAULT 0 NOT NULL,
    retained_envelope_bytes bigint DEFAULT 0 NOT NULL,
    health_state smallint DEFAULT 1 NOT NULL,
    failure_code smallint,
    failure_observed_at timestamptz,
    configured_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    updated_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    restrictive_reserve_events bigint NOT NULL,
    restrictive_reserve_bytes bigint NOT NULL,
    failure_generation bigint DEFAULT 0 NOT NULL,
    recovery_generation bigint DEFAULT 0 NOT NULL,
    recovered_at timestamptz,
    CONSTRAINT authorization_event_capacity_pkey PRIMARY KEY (community_id),
    CONSTRAINT authorization_event_capacity_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT authorization_event_capacity_check CHECK (max_envelope_bytes <= max_bytes_per_domain),
    CONSTRAINT authorization_event_capacity_check1 CHECK (retained_event_count <= max_events_per_domain),
    CONSTRAINT authorization_event_capacity_check2 CHECK (retained_envelope_bytes <= max_bytes_per_domain),
    CONSTRAINT authorization_event_capacity_failure_code_check CHECK (failure_code IS NULL OR (failure_code IN (1, 2, 3))),
    CONSTRAINT authorization_event_capacity_generations CHECK (recovery_generation <= failure_generation),
    CONSTRAINT authorization_event_capacity_health_state_check CHECK (health_state IN (1, 2)),
    CONSTRAINT authorization_event_capacity_max_bytes CHECK (max_bytes_per_domain >= 1 AND max_bytes_per_domain <= 16777216),
    CONSTRAINT authorization_event_capacity_max_envelope CHECK (max_envelope_bytes >= 1 AND max_envelope_bytes <= 16384),
    CONSTRAINT authorization_event_capacity_max_events CHECK (max_events_per_domain >= 1 AND max_events_per_domain <= 10000),
    CONSTRAINT authorization_event_capacity_reserve_bytes CHECK (restrictive_reserve_bytes =
CASE
    WHEN max_bytes_per_domain > max_envelope_bytes THEN GREATEST(max_envelope_bytes::bigint, LEAST(262144::bigint, max_bytes_per_domain / 8))
    ELSE 0::bigint
END),
    CONSTRAINT authorization_event_capacity_reserve_events CHECK (restrictive_reserve_events =
CASE
    WHEN max_events_per_domain > 1 THEN GREATEST(1::bigint, LEAST(64::bigint, max_events_per_domain / 8))
    ELSE 0::bigint
END),
    CONSTRAINT authorization_event_capacity_retained_envelope_bytes_check CHECK (retained_envelope_bytes >= 0),
    CONSTRAINT authorization_event_capacity_retained_event_count_check CHECK (retained_event_count >= 0)
);

--
-- Name: authorization_invalidation_domains; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS authorization_invalidation_domains (
    community_id uuid,
    current_generation bigint NOT NULL,
    activated_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    updated_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    CONSTRAINT authorization_invalidation_domains_pkey PRIMARY KEY (community_id),
    CONSTRAINT authorization_invalidation_domains_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT authorization_invalidation_domains_current_generation_check CHECK (current_generation >= 0)
);

--
-- Name: authorization_operation_receipts; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS authorization_operation_receipts (
    community_id uuid,
    operation_id uuid,
    request_fingerprint bytea NOT NULL,
    operation_kind smallint NOT NULL,
    actor_fingerprint bytea NOT NULL,
    outcome_code smallint NOT NULL,
    result_digest bytea NOT NULL,
    recorded_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    CONSTRAINT authorization_operation_receipts_pkey PRIMARY KEY (community_id, operation_id),
    CONSTRAINT authorization_operation_recei_community_id_operation_id_re_key1 UNIQUE (community_id, operation_id, request_fingerprint, operation_kind, outcome_code),
    CONSTRAINT authorization_operation_recei_community_id_operation_id_req_key UNIQUE (community_id, operation_id, request_fingerprint),
    CONSTRAINT authorization_operation_receipts_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT authorization_operation_receipts_actor_fingerprint_check CHECK (octet_length(actor_fingerprint) = 32),
    CONSTRAINT authorization_operation_receipts_operation_id_check CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CONSTRAINT authorization_operation_receipts_operation_kind_check CHECK (operation_kind IN (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12)),
    CONSTRAINT authorization_operation_receipts_outcome_code_check CHECK (outcome_code IN (1, 2, 3)),
    CONSTRAINT authorization_operation_receipts_request_fingerprint_check CHECK (octet_length(request_fingerprint) = 32),
    CONSTRAINT authorization_operation_receipts_result_digest_check CHECK (octet_length(result_digest) = 32)
);

--
-- Name: authorization_admission_results; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS authorization_admission_results (
    community_id uuid,
    operation_id uuid,
    request_fingerprint bytea NOT NULL,
    semantic_fingerprint bytea NOT NULL,
    object_kind smallint NOT NULL,
    object_key bytea NOT NULL,
    application_type bytea,
    application_version smallint,
    application_code smallint,
    application_payload bytea,
    application_intent_digest bytea,
    application_effect_digest bytea,
    application_result_digest bytea,
    recorded_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    CONSTRAINT authorization_admission_results_pkey PRIMARY KEY (community_id, operation_id),
    CONSTRAINT authorization_admission_resul_community_id_operation_id_re_fkey FOREIGN KEY (community_id, operation_id, request_fingerprint) REFERENCES authorization_operation_receipts (community_id, operation_id, request_fingerprint),
    CONSTRAINT authorization_admission_results_application_code_check CHECK (application_code > 0),
    CONSTRAINT authorization_admission_results_application_effect_digest_check CHECK (application_effect_digest IS NULL OR octet_length(application_effect_digest) = 32 AND application_effect_digest <> decode(repeat('00'::text, 32), 'hex'::text)),
    CONSTRAINT authorization_admission_results_application_intent_digest_check CHECK (application_intent_digest IS NULL OR octet_length(application_intent_digest) = 32 AND application_intent_digest <> decode(repeat('00'::text, 32), 'hex'::text)),
    CONSTRAINT authorization_admission_results_application_payload_check CHECK (application_payload IS NULL OR octet_length(application_payload) <= 4096),
    CONSTRAINT authorization_admission_results_application_result_digest_check CHECK (application_result_digest IS NULL OR octet_length(application_result_digest) = 32 AND application_result_digest <> decode(repeat('00'::text, 32), 'hex'::text)),
    CONSTRAINT authorization_admission_results_application_type_check CHECK (application_type IS NULL OR octet_length(application_type) = 32 AND application_type <> decode(repeat('00'::text, 32), 'hex'::text)),
    CONSTRAINT authorization_admission_results_application_version_check CHECK (application_version > 0),
    CONSTRAINT authorization_admission_results_object_key_check CHECK (octet_length(object_key) = 32 AND object_key <> decode(repeat('00'::text, 32), 'hex'::text)),
    CONSTRAINT authorization_admission_results_object_kind_check CHECK (object_kind IN (1, 2, 3, 4, 5, 6, 9)),
    CONSTRAINT authorization_admission_results_request_fingerprint_check CHECK (octet_length(request_fingerprint) = 32),
    CONSTRAINT authorization_admission_results_semantic_fingerprint_check CHECK (octet_length(semantic_fingerprint) = 32 AND semantic_fingerprint <> decode(repeat('00'::text, 32), 'hex'::text))
);

--
-- Name: authorization_authority_epochs; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS authorization_authority_epochs (
    community_id uuid,
    object_kind smallint,
    object_key bytea,
    authority_epoch bigint NOT NULL,
    fence bytea NOT NULL,
    operation_id uuid NOT NULL,
    request_fingerprint bytea NOT NULL,
    updated_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    CONSTRAINT authorization_authority_epochs_pkey PRIMARY KEY (community_id, object_kind, object_key),
    CONSTRAINT authorization_authority_epoch_community_id_object_kind_obje_key UNIQUE (community_id, object_kind, object_key, authority_epoch, fence, operation_id, request_fingerprint),
    CONSTRAINT authorization_authority_epoch_community_id_operation_id_re_fkey FOREIGN KEY (community_id, operation_id, request_fingerprint) REFERENCES authorization_operation_receipts (community_id, operation_id, request_fingerprint) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT authorization_authority_epochs_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT authorization_authority_epochs_authority_epoch_check CHECK (authority_epoch > 0),
    CONSTRAINT authorization_authority_epochs_fence_check CHECK (octet_length(fence) = 32 AND fence <> decode(repeat('00'::text, 32), 'hex'::text)),
    CONSTRAINT authorization_authority_epochs_object_key_check CHECK (octet_length(object_key) = 32),
    CONSTRAINT authorization_authority_epochs_object_kind_check CHECK (object_kind IN (1, 2, 3, 4, 5, 6, 9)),
    CONSTRAINT authorization_authority_epochs_request_fingerprint_check CHECK (octet_length(request_fingerprint) = 32)
);

--
-- Name: authorization_events; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS authorization_events (
    community_id uuid,
    event_id uuid,
    schema_version smallint DEFAULT 1 NOT NULL,
    event_kind smallint NOT NULL,
    outcome_code smallint NOT NULL,
    reason_code smallint NOT NULL,
    actor_kind smallint NOT NULL,
    actor_fingerprint bytea,
    subject_fingerprint bytea,
    operation_id uuid NOT NULL,
    request_fingerprint bytea,
    correlation_id uuid NOT NULL,
    attempt_id uuid NOT NULL,
    occurred_at timestamptz NOT NULL,
    accepted_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    canonical_envelope bytea NOT NULL,
    envelope_digest bytea NOT NULL,
    CONSTRAINT authorization_events_pkey PRIMARY KEY (community_id, event_id),
    CONSTRAINT authorization_events_community_id_event_id_event_kind_opera_key UNIQUE (community_id, event_id, event_kind, operation_id),
    CONSTRAINT authorization_events_community_id_event_id_operation_id_key UNIQUE (community_id, event_id, operation_id),
    CONSTRAINT authorization_events_community_id_operation_id_event_kind_a_key UNIQUE (community_id, operation_id, event_kind, attempt_id),
    CONSTRAINT authorization_events_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT authorization_events_community_id_operation_id_request_fin_fkey FOREIGN KEY (community_id, operation_id, request_fingerprint) REFERENCES authorization_operation_receipts (community_id, operation_id, request_fingerprint) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT authorization_events_actor_fingerprint_check CHECK (actor_fingerprint IS NULL OR octet_length(actor_fingerprint) = 32),
    CONSTRAINT authorization_events_actor_kind_check CHECK (actor_kind IN (1, 2, 3, 4)),
    CONSTRAINT authorization_events_attempt_id_check CHECK (attempt_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CONSTRAINT authorization_events_correlation_id_check CHECK (correlation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CONSTRAINT authorization_events_envelope_digest_check CHECK (octet_length(envelope_digest) = 32),
    CONSTRAINT authorization_events_envelope_size CHECK (octet_length(canonical_envelope) >= 1 AND octet_length(canonical_envelope) <= 16384),
    CONSTRAINT authorization_events_event_id_check CHECK (event_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CONSTRAINT authorization_events_event_kind_check CHECK (event_kind IN (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 14)),
    CONSTRAINT authorization_events_operation_id_check CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CONSTRAINT authorization_events_outcome_code_check CHECK (outcome_code IN (1, 2, 3, 4, 5)),
    CONSTRAINT authorization_events_reason_code_check CHECK (reason_code IN (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17)),
    CONSTRAINT authorization_events_request_fingerprint_check CHECK (request_fingerprint IS NULL OR octet_length(request_fingerprint) = 32),
    CONSTRAINT authorization_events_schema_version_check CHECK (schema_version = 1),
    CONSTRAINT authorization_events_subject_fingerprint_check CHECK (subject_fingerprint IS NULL OR octet_length(subject_fingerprint) = 32)
);

--
-- Name: authorization_authentication_denial_attempts; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS authorization_authentication_denial_attempts (
    community_id uuid,
    operation_id uuid,
    correlation_id uuid,
    semantic_fingerprint bytea,
    denial_reason smallint,
    expected_revision bigint NOT NULL,
    action smallint NOT NULL,
    reason_code smallint NOT NULL,
    audit_event_id uuid NOT NULL,
    audit_event_kind smallint DEFAULT 9 NOT NULL,
    recorded_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    CONSTRAINT authorization_authentication_denial_attempts_pkey PRIMARY KEY (community_id, operation_id, correlation_id, semantic_fingerprint, denial_reason),
    CONSTRAINT authorization_authentication_de_community_id_audit_event_id_key UNIQUE (community_id, audit_event_id),
    CONSTRAINT authorization_authentication__community_id_audit_event_id__fkey FOREIGN KEY (community_id, audit_event_id, audit_event_kind, operation_id) REFERENCES authorization_events (community_id, event_id, event_kind, operation_id) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT authorization_authentication_denial_attempts_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT authorization_authentication_denial__semantic_fingerprint_check CHECK (octet_length(semantic_fingerprint) = 32),
    CONSTRAINT authorization_authentication_denial_att_expected_revision_check CHECK (expected_revision > 0),
    CONSTRAINT authorization_authentication_denial_atte_audit_event_kind_check CHECK (audit_event_kind = 9),
    CONSTRAINT authorization_authentication_denial_attemp_correlation_id_check CHECK (correlation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CONSTRAINT authorization_authentication_denial_attempt_denial_reason_check CHECK (denial_reason IN (1, 2, 3)),
    CONSTRAINT authorization_authentication_denial_attempts_action_check CHECK (action IN (1, 2, 3, 4, 5, 6, 7, 8)),
    CONSTRAINT authorization_authentication_denial_attempts_operation_id_check CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CONSTRAINT authorization_authentication_denial_attempts_reason_code_check CHECK (reason_code IN (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16))
);

--
-- Name: authorization_invalidation_floors; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS authorization_invalidation_floors (
    community_id uuid,
    selector_kind smallint,
    selector_fingerprint bytea,
    floor_generation bigint NOT NULL,
    binding_version_floor bigint,
    relationship_revision_floor bigint,
    operation_id uuid NOT NULL,
    request_fingerprint bytea NOT NULL,
    updated_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    CONSTRAINT authorization_invalidation_floors_pkey PRIMARY KEY (community_id, selector_kind, selector_fingerprint),
    CONSTRAINT authorization_invalidation_fl_community_id_operation_id_re_fkey FOREIGN KEY (community_id, operation_id, request_fingerprint) REFERENCES authorization_operation_receipts (community_id, operation_id, request_fingerprint) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT authorization_invalidation_floors_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT authorization_invalidation_fl_relationship_revision_floor_check CHECK (relationship_revision_floor IS NULL OR relationship_revision_floor > 0),
    CONSTRAINT authorization_invalidation_floors_binding_version_floor_check CHECK (binding_version_floor IS NULL OR binding_version_floor > 0),
    CONSTRAINT authorization_invalidation_floors_floor_generation_check CHECK (floor_generation > 0),
    CONSTRAINT authorization_invalidation_floors_request_fingerprint_check CHECK (octet_length(request_fingerprint) = 32),
    CONSTRAINT authorization_invalidation_floors_selector_fingerprint_check CHECK (octet_length(selector_fingerprint) = 32),
    CONSTRAINT authorization_invalidation_floors_selector_kind_check CHECK (selector_kind IN (1, 2, 3, 4, 5, 6, 7))
);

--
-- Name: authorization_operation_version_delta_manifests; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS authorization_operation_version_delta_manifests (
    community_id uuid,
    operation_id uuid,
    request_fingerprint bytea NOT NULL,
    component_count integer NOT NULL,
    before_digest bytea NOT NULL,
    after_digest bytea NOT NULL,
    manifest_digest bytea NOT NULL,
    recorded_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    CONSTRAINT authorization_operation_version_delta_manifests_pkey PRIMARY KEY (community_id, operation_id),
    CONSTRAINT authorization_operation_versi_community_id_operation_id_req_key UNIQUE (community_id, operation_id, request_fingerprint),
    CONSTRAINT authorization_operation_versi_community_id_operation_id_re_fkey FOREIGN KEY (community_id, operation_id, request_fingerprint) REFERENCES authorization_operation_receipts (community_id, operation_id, request_fingerprint) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT authorization_operation_version_delta_manifes_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT authorization_operation_version_delta_man_component_count_check CHECK (component_count >= 0 AND component_count <= 1024),
    CONSTRAINT authorization_operation_version_delta_man_manifest_digest_check CHECK (octet_length(manifest_digest) = 32),
    CONSTRAINT authorization_operation_version_delta_manif_before_digest_check CHECK (octet_length(before_digest) = 32),
    CONSTRAINT authorization_operation_version_delta_manife_after_digest_check CHECK (octet_length(after_digest) = 32),
    CONSTRAINT authorization_operation_version_delta_request_fingerprint_check CHECK (octet_length(request_fingerprint) = 32)
);

--
-- Name: authorization_operation_version_deltas; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS authorization_operation_version_deltas (
    community_id uuid,
    operation_id uuid,
    component_kind smallint,
    component_key bytea,
    before_version bigint NOT NULL,
    after_version bigint NOT NULL,
    component_digest bytea NOT NULL,
    CONSTRAINT authorization_operation_version_deltas_pkey PRIMARY KEY (community_id, operation_id, component_kind, component_key),
    CONSTRAINT authorization_operation_version__community_id_operation_id_fkey FOREIGN KEY (community_id, operation_id) REFERENCES authorization_operation_version_delta_manifests (community_id, operation_id),
    CONSTRAINT authorization_operation_version_deltas_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT authorization_operation_version_deltas_before_version_check CHECK (before_version >= 0),
    CONSTRAINT authorization_operation_version_deltas_check CHECK (after_version > before_version),
    CONSTRAINT authorization_operation_version_deltas_component_digest_check CHECK (octet_length(component_digest) = 32),
    CONSTRAINT authorization_operation_version_deltas_component_key_check CHECK (octet_length(component_key) = 32),
    CONSTRAINT authorization_operation_version_deltas_component_kind_check CHECK (component_kind IN (1, 2, 3, 4, 6, 7))
);

--
-- Name: authorization_operator_denial_buckets; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS authorization_operator_denial_buckets (
    community_id uuid,
    surface_kind smallint,
    denial_class smallint,
    action_kind smallint,
    slot smallint,
    window_generation bigint NOT NULL,
    window_started_at timestamptz NOT NULL,
    denial_count bigint NOT NULL,
    lifetime_count bigint NOT NULL,
    last_denied_at timestamptz NOT NULL,
    CONSTRAINT authorization_operator_denial_buckets_pkey PRIMARY KEY (community_id, surface_kind, denial_class, action_kind, slot),
    CONSTRAINT authorization_operator_denial_buckets_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT authorization_operator_denial_buckets_action_kind_check CHECK (action_kind >= 1 AND action_kind <= 8),
    CONSTRAINT authorization_operator_denial_buckets_check CHECK (surface_kind = 1 AND action_kind >= 1 AND action_kind <= 8 OR surface_kind = 2 AND action_kind = 1 OR surface_kind = 3 AND action_kind = 2),
    CONSTRAINT authorization_operator_denial_buckets_check1 CHECK (window_started_at <= last_denied_at),
    CONSTRAINT authorization_operator_denial_buckets_denial_class_check CHECK (denial_class >= 1 AND denial_class <= 8),
    CONSTRAINT authorization_operator_denial_buckets_denial_count_check CHECK (denial_count > 0),
    CONSTRAINT authorization_operator_denial_buckets_lifetime_count_check CHECK (lifetime_count > 0),
    CONSTRAINT authorization_operator_denial_buckets_slot_check CHECK (slot >= 0 AND slot <= 11),
    CONSTRAINT authorization_operator_denial_buckets_surface_kind_check CHECK (surface_kind >= 1 AND surface_kind <= 3),
    CONSTRAINT authorization_operator_denial_buckets_window_generation_check CHECK (window_generation >= 0)
);

--
-- Name: authorization_proxy_nonce_claims; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS authorization_proxy_nonce_claims (
    authorization_domain uuid,
    claim_kind smallint,
    claim_key bytea,
    committed_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    retain_until timestamptz NOT NULL,
    CONSTRAINT authorization_proxy_nonce_claims_pkey PRIMARY KEY (authorization_domain, claim_kind, claim_key),
    CONSTRAINT authorization_proxy_nonce_claims_authorization_domain_fkey FOREIGN KEY (authorization_domain) REFERENCES communities (id),
    CONSTRAINT authorization_proxy_nonce_claims_check CHECK (committed_at < retain_until),
    CONSTRAINT authorization_proxy_nonce_claims_claim_key_check CHECK (octet_length(claim_key) = 32 AND claim_key <> decode(repeat('00'::text, 32), 'hex'::text)),
    CONSTRAINT authorization_proxy_nonce_claims_claim_kind_check CHECK (claim_kind = 1)
);

--
-- Name: channels; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS channels (
    id uuid DEFAULT gen_random_uuid(),
    community_id uuid,
    name varchar(255) NOT NULL,
    channel_type channel_type DEFAULT 'stream'::channel_type NOT NULL,
    visibility channel_visibility DEFAULT 'open'::channel_visibility NOT NULL,
    description text,
    canvas text,
    created_by bytea NOT NULL,
    created_at timestamptz DEFAULT now() NOT NULL,
    updated_at timestamptz DEFAULT now() NOT NULL,
    archived_at timestamptz,
    deleted_at timestamptz,
    nip29_group_id varchar(255),
    topic_required boolean DEFAULT false NOT NULL,
    max_members integer,
    topic text,
    topic_set_by bytea,
    topic_set_at timestamptz,
    purpose text,
    purpose_set_by bytea,
    purpose_set_at timestamptz,
    participant_hash bytea,
    ttl_seconds integer,
    ttl_deadline timestamptz,
    CONSTRAINT channels_pkey PRIMARY KEY (community_id, id),
    CONSTRAINT channels_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT chk_channels_id_not_nil CHECK (id <> '00000000-0000-0000-0000-000000000000'::uuid)
);

--
-- Name: idx_channels_community_type; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_channels_community_type ON channels (community_id, channel_type);

--
-- Name: idx_channels_community_visibility; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_channels_community_visibility ON channels (community_id, visibility);

--
-- Name: idx_channels_created_by; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_channels_created_by ON channels (community_id, created_by);

--
-- Name: idx_channels_dm_hash; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS idx_channels_dm_hash ON channels (community_id, participant_hash) WHERE (participant_hash IS NOT NULL);

--
-- Name: idx_channels_id_live; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_channels_id_live ON channels (id, community_id) WHERE (deleted_at IS NULL);

--
-- Name: idx_channels_nip29_group; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS idx_channels_nip29_group ON channels (community_id, nip29_group_id) WHERE (nip29_group_id IS NOT NULL);

--
-- Name: idx_channels_ttl_expiry; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_channels_ttl_expiry ON channels (ttl_deadline) WHERE (ttl_seconds IS NOT NULL) AND (archived_at IS NULL) AND (deleted_at IS NULL);

--
-- Name: channel_members; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS channel_members (
    community_id uuid,
    channel_id uuid,
    pubkey bytea,
    role member_role DEFAULT 'member'::member_role NOT NULL,
    joined_at timestamptz DEFAULT now() NOT NULL,
    invited_by bytea,
    removed_at timestamptz,
    removed_by bytea,
    hidden_at timestamptz,
    CONSTRAINT channel_members_pkey PRIMARY KEY (community_id, channel_id, pubkey),
    CONSTRAINT channel_members_community_id_channel_id_fkey FOREIGN KEY (community_id, channel_id) REFERENCES channels (community_id, id) ON DELETE CASCADE,
    CONSTRAINT channel_members_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: idx_channel_members_pubkey; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_channel_members_pubkey ON channel_members (community_id, pubkey) WHERE (removed_at IS NULL);

--
-- Name: community_bans; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS community_bans (
    community_id uuid,
    pubkey bytea,
    banned boolean DEFAULT false NOT NULL,
    ban_expires_at timestamptz,
    ban_reason text,
    muted_until timestamptz,
    mute_reason text,
    actor_pubkey bytea NOT NULL,
    created_at timestamptz DEFAULT now() NOT NULL,
    updated_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT community_bans_pkey PRIMARY KEY (community_id, pubkey),
    CONSTRAINT community_bans_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT community_bans_actor_pubkey_check CHECK (length(actor_pubkey) = 32),
    CONSTRAINT community_bans_pubkey_check CHECK (length(pubkey) = 32)
);

--
-- Name: delivery_log; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS delivery_log (
    community_id uuid NOT NULL,
    id bigint GENERATED ALWAYS AS IDENTITY,
    subscription_id varchar(255),
    event_id bytea,
    method delivery_method,
    delivered_at timestamptz DEFAULT now(),
    success boolean,
    http_status integer,
    error_message text,
    attempt_number integer DEFAULT 1,
    CONSTRAINT delivery_log_pkey PRIMARY KEY (delivered_at, id),
    CONSTRAINT delivery_log_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
) PARTITION BY RANGE (delivered_at);

--
-- Name: idx_delivery_log_community_sub; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_delivery_log_community_sub ON delivery_log (community_id, subscription_id);

--
-- Name: delivery_log_p2026_03; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS delivery_log_p2026_03 (
    community_id uuid NOT NULL,
    id bigint GENERATED ALWAYS AS IDENTITY,
    subscription_id varchar(255),
    event_id bytea,
    method delivery_method,
    delivered_at timestamptz DEFAULT now(),
    success boolean,
    http_status integer,
    error_message text,
    attempt_number integer DEFAULT 1,
    CONSTRAINT delivery_log_p2026_03_pkey PRIMARY KEY (delivered_at, id),
    CONSTRAINT delivery_log_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: delivery_log_p2026_03_community_id_subscription_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS delivery_log_p2026_03_community_id_subscription_id_idx ON delivery_log_p2026_03 (community_id, subscription_id);

--
-- Name: delivery_log_p2026_04; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS delivery_log_p2026_04 (
    community_id uuid NOT NULL,
    id bigint GENERATED ALWAYS AS IDENTITY,
    subscription_id varchar(255),
    event_id bytea,
    method delivery_method,
    delivered_at timestamptz DEFAULT now(),
    success boolean,
    http_status integer,
    error_message text,
    attempt_number integer DEFAULT 1,
    CONSTRAINT delivery_log_p2026_04_pkey PRIMARY KEY (delivered_at, id),
    CONSTRAINT delivery_log_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: delivery_log_p2026_04_community_id_subscription_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS delivery_log_p2026_04_community_id_subscription_id_idx ON delivery_log_p2026_04 (community_id, subscription_id);

--
-- Name: delivery_log_p2026_05; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS delivery_log_p2026_05 (
    community_id uuid NOT NULL,
    id bigint GENERATED ALWAYS AS IDENTITY,
    subscription_id varchar(255),
    event_id bytea,
    method delivery_method,
    delivered_at timestamptz DEFAULT now(),
    success boolean,
    http_status integer,
    error_message text,
    attempt_number integer DEFAULT 1,
    CONSTRAINT delivery_log_p2026_05_pkey PRIMARY KEY (delivered_at, id),
    CONSTRAINT delivery_log_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: delivery_log_p2026_05_community_id_subscription_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS delivery_log_p2026_05_community_id_subscription_id_idx ON delivery_log_p2026_05 (community_id, subscription_id);

--
-- Name: delivery_log_p2026_06; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS delivery_log_p2026_06 (
    community_id uuid NOT NULL,
    id bigint GENERATED ALWAYS AS IDENTITY,
    subscription_id varchar(255),
    event_id bytea,
    method delivery_method,
    delivered_at timestamptz DEFAULT now(),
    success boolean,
    http_status integer,
    error_message text,
    attempt_number integer DEFAULT 1,
    CONSTRAINT delivery_log_p2026_06_pkey PRIMARY KEY (delivered_at, id),
    CONSTRAINT delivery_log_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: delivery_log_p2026_06_community_id_subscription_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS delivery_log_p2026_06_community_id_subscription_id_idx ON delivery_log_p2026_06 (community_id, subscription_id);

--
-- Name: delivery_log_p_future; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS delivery_log_p_future (
    community_id uuid NOT NULL,
    id bigint GENERATED ALWAYS AS IDENTITY,
    subscription_id varchar(255),
    event_id bytea,
    method delivery_method,
    delivered_at timestamptz DEFAULT now(),
    success boolean,
    http_status integer,
    error_message text,
    attempt_number integer DEFAULT 1,
    CONSTRAINT delivery_log_p_future_pkey PRIMARY KEY (delivered_at, id),
    CONSTRAINT delivery_log_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: delivery_log_p_future_community_id_subscription_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS delivery_log_p_future_community_id_subscription_id_idx ON delivery_log_p_future (community_id, subscription_id);

--
-- Name: delivery_log_p_past; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS delivery_log_p_past (
    community_id uuid NOT NULL,
    id bigint GENERATED ALWAYS AS IDENTITY,
    subscription_id varchar(255),
    event_id bytea,
    method delivery_method,
    delivered_at timestamptz DEFAULT now(),
    success boolean,
    http_status integer,
    error_message text,
    attempt_number integer DEFAULT 1,
    CONSTRAINT delivery_log_p_past_pkey PRIMARY KEY (delivered_at, id),
    CONSTRAINT delivery_log_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: delivery_log_p_past_community_id_subscription_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS delivery_log_p_past_community_id_subscription_id_idx ON delivery_log_p_past (community_id, subscription_id);

--
-- Name: event_mentions; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS event_mentions (
    community_id uuid,
    pubkey_hex varchar(64),
    event_id bytea,
    event_created_at timestamptz NOT NULL,
    channel_id uuid,
    event_kind integer,
    CONSTRAINT event_mentions_pkey PRIMARY KEY (community_id, pubkey_hex, event_id),
    CONSTRAINT event_mentions_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: idx_event_mentions_community_event; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_event_mentions_community_event ON event_mentions (community_id, event_id);

--
-- Name: idx_event_mentions_pubkey_created; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_event_mentions_pubkey_created ON event_mentions (community_id, pubkey_hex, event_created_at DESC);

--
-- Name: idx_event_mentions_pubkey_kind_created; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_event_mentions_pubkey_kind_created ON event_mentions (community_id, pubkey_hex, event_kind, event_created_at DESC);

--
-- Name: events; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS events (
    community_id uuid,
    id bytea,
    pubkey bytea NOT NULL,
    created_at timestamptz,
    kind integer NOT NULL,
    tags jsonb NOT NULL,
    content text NOT NULL,
    sig bytea NOT NULL,
    received_at timestamptz DEFAULT now() NOT NULL,
    channel_id uuid,
    deleted_at timestamptz,
    d_tag text,
    not_before bigint,
    delivered_at bigint,
    search_tsv tsvector GENERATED ALWAYS AS (
CASE
    WHEN (kind = 30350) THEN NULL::tsvector
    ELSE
    CASE
        WHEN (kind = ANY (ARRAY[0, 9, 40002, 45001, 45003])) THEN to_tsvector('simple'::regconfig, content)
        ELSE NULL::tsvector
    END
END) STORED,
    CONSTRAINT events_pkey PRIMARY KEY (created_at, community_id, id),
    CONSTRAINT events_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
) PARTITION BY RANGE (created_at);

--
-- Name: idx_events_addressable; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_events_addressable ON events (community_id, kind, pubkey, channel_id, deleted_at);

--
-- Name: idx_events_community_channel_created; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_events_community_channel_created ON events (community_id, channel_id, created_at DESC, id);

--
-- Name: idx_events_community_deleted; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_events_community_deleted ON events (community_id, deleted_at);

--
-- Name: idx_events_community_id; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_events_community_id ON events (community_id, id, created_at DESC);

--
-- Name: idx_events_community_kind_created; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_events_community_kind_created ON events (community_id, kind, created_at DESC, id);

--
-- Name: idx_events_community_pubkey_kind_created; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_events_community_pubkey_kind_created ON events (community_id, pubkey, kind, created_at DESC, id);

--
-- Name: idx_events_not_before; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_events_not_before ON events (community_id, not_before) WHERE (not_before IS NOT NULL) AND (deleted_at IS NULL) AND (delivered_at IS NULL);

--
-- Name: idx_events_parameterized; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_events_parameterized ON events (community_id, kind, pubkey, d_tag, created_at DESC, id) WHERE (d_tag IS NOT NULL) AND (deleted_at IS NULL);

--
-- Name: idx_events_search_tsv; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_events_search_tsv ON events USING gin (search_tsv);

--
-- Name: idx_events_tags_gin; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_events_tags_gin ON events USING gin (tags jsonb_path_ops);

--
-- Name: events_p2026_01; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS events_p2026_01 (
    community_id uuid,
    id bytea,
    pubkey bytea NOT NULL,
    created_at timestamptz,
    kind integer NOT NULL,
    tags jsonb NOT NULL,
    content text NOT NULL,
    sig bytea NOT NULL,
    received_at timestamptz DEFAULT now() NOT NULL,
    channel_id uuid,
    deleted_at timestamptz,
    d_tag text,
    not_before bigint,
    delivered_at bigint,
    search_tsv tsvector GENERATED ALWAYS AS (
CASE
    WHEN (kind = 30350) THEN NULL::tsvector
    ELSE
    CASE
        WHEN (kind = ANY (ARRAY[0, 9, 40002, 45001, 45003])) THEN to_tsvector('simple'::regconfig, content)
        ELSE NULL::tsvector
    END
END) STORED,
    CONSTRAINT events_p2026_01_pkey PRIMARY KEY (created_at, community_id, id),
    CONSTRAINT events_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: events_p2026_01_community_id_channel_id_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_01_community_id_channel_id_created_at_id_idx ON events_p2026_01 (community_id, channel_id, created_at DESC, id);

--
-- Name: events_p2026_01_community_id_deleted_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_01_community_id_deleted_at_idx ON events_p2026_01 (community_id, deleted_at);

--
-- Name: events_p2026_01_community_id_id_created_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_01_community_id_id_created_at_idx ON events_p2026_01 (community_id, id, created_at DESC);

--
-- Name: events_p2026_01_community_id_kind_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_01_community_id_kind_created_at_id_idx ON events_p2026_01 (community_id, kind, created_at DESC, id);

--
-- Name: events_p2026_01_community_id_kind_pubkey_channel_id_deleted_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_01_community_id_kind_pubkey_channel_id_deleted_idx ON events_p2026_01 (community_id, kind, pubkey, channel_id, deleted_at);

--
-- Name: events_p2026_01_community_id_kind_pubkey_d_tag_created_at_i_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_01_community_id_kind_pubkey_d_tag_created_at_i_idx ON events_p2026_01 (community_id, kind, pubkey, d_tag, created_at DESC, id) WHERE (d_tag IS NOT NULL) AND (deleted_at IS NULL);

--
-- Name: events_p2026_01_community_id_not_before_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_01_community_id_not_before_idx ON events_p2026_01 (community_id, not_before) WHERE (not_before IS NOT NULL) AND (deleted_at IS NULL) AND (delivered_at IS NULL);

--
-- Name: events_p2026_01_community_id_pubkey_kind_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_01_community_id_pubkey_kind_created_at_id_idx ON events_p2026_01 (community_id, pubkey, kind, created_at DESC, id);

--
-- Name: events_p2026_01_search_tsv_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_01_search_tsv_idx ON events_p2026_01 USING gin (search_tsv);

--
-- Name: events_p2026_01_tags_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_01_tags_idx ON events_p2026_01 USING gin (tags jsonb_path_ops);

--
-- Name: events_p2026_02; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS events_p2026_02 (
    community_id uuid,
    id bytea,
    pubkey bytea NOT NULL,
    created_at timestamptz,
    kind integer NOT NULL,
    tags jsonb NOT NULL,
    content text NOT NULL,
    sig bytea NOT NULL,
    received_at timestamptz DEFAULT now() NOT NULL,
    channel_id uuid,
    deleted_at timestamptz,
    d_tag text,
    not_before bigint,
    delivered_at bigint,
    search_tsv tsvector GENERATED ALWAYS AS (
CASE
    WHEN (kind = 30350) THEN NULL::tsvector
    ELSE
    CASE
        WHEN (kind = ANY (ARRAY[0, 9, 40002, 45001, 45003])) THEN to_tsvector('simple'::regconfig, content)
        ELSE NULL::tsvector
    END
END) STORED,
    CONSTRAINT events_p2026_02_pkey PRIMARY KEY (created_at, community_id, id),
    CONSTRAINT events_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: events_p2026_02_community_id_channel_id_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_02_community_id_channel_id_created_at_id_idx ON events_p2026_02 (community_id, channel_id, created_at DESC, id);

--
-- Name: events_p2026_02_community_id_deleted_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_02_community_id_deleted_at_idx ON events_p2026_02 (community_id, deleted_at);

--
-- Name: events_p2026_02_community_id_id_created_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_02_community_id_id_created_at_idx ON events_p2026_02 (community_id, id, created_at DESC);

--
-- Name: events_p2026_02_community_id_kind_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_02_community_id_kind_created_at_id_idx ON events_p2026_02 (community_id, kind, created_at DESC, id);

--
-- Name: events_p2026_02_community_id_kind_pubkey_channel_id_deleted_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_02_community_id_kind_pubkey_channel_id_deleted_idx ON events_p2026_02 (community_id, kind, pubkey, channel_id, deleted_at);

--
-- Name: events_p2026_02_community_id_kind_pubkey_d_tag_created_at_i_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_02_community_id_kind_pubkey_d_tag_created_at_i_idx ON events_p2026_02 (community_id, kind, pubkey, d_tag, created_at DESC, id) WHERE (d_tag IS NOT NULL) AND (deleted_at IS NULL);

--
-- Name: events_p2026_02_community_id_not_before_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_02_community_id_not_before_idx ON events_p2026_02 (community_id, not_before) WHERE (not_before IS NOT NULL) AND (deleted_at IS NULL) AND (delivered_at IS NULL);

--
-- Name: events_p2026_02_community_id_pubkey_kind_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_02_community_id_pubkey_kind_created_at_id_idx ON events_p2026_02 (community_id, pubkey, kind, created_at DESC, id);

--
-- Name: events_p2026_02_search_tsv_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_02_search_tsv_idx ON events_p2026_02 USING gin (search_tsv);

--
-- Name: events_p2026_02_tags_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_02_tags_idx ON events_p2026_02 USING gin (tags jsonb_path_ops);

--
-- Name: events_p2026_03; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS events_p2026_03 (
    community_id uuid,
    id bytea,
    pubkey bytea NOT NULL,
    created_at timestamptz,
    kind integer NOT NULL,
    tags jsonb NOT NULL,
    content text NOT NULL,
    sig bytea NOT NULL,
    received_at timestamptz DEFAULT now() NOT NULL,
    channel_id uuid,
    deleted_at timestamptz,
    d_tag text,
    not_before bigint,
    delivered_at bigint,
    search_tsv tsvector GENERATED ALWAYS AS (
CASE
    WHEN (kind = 30350) THEN NULL::tsvector
    ELSE
    CASE
        WHEN (kind = ANY (ARRAY[0, 9, 40002, 45001, 45003])) THEN to_tsvector('simple'::regconfig, content)
        ELSE NULL::tsvector
    END
END) STORED,
    CONSTRAINT events_p2026_03_pkey PRIMARY KEY (created_at, community_id, id),
    CONSTRAINT events_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: events_p2026_03_community_id_channel_id_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_03_community_id_channel_id_created_at_id_idx ON events_p2026_03 (community_id, channel_id, created_at DESC, id);

--
-- Name: events_p2026_03_community_id_deleted_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_03_community_id_deleted_at_idx ON events_p2026_03 (community_id, deleted_at);

--
-- Name: events_p2026_03_community_id_id_created_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_03_community_id_id_created_at_idx ON events_p2026_03 (community_id, id, created_at DESC);

--
-- Name: events_p2026_03_community_id_kind_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_03_community_id_kind_created_at_id_idx ON events_p2026_03 (community_id, kind, created_at DESC, id);

--
-- Name: events_p2026_03_community_id_kind_pubkey_channel_id_deleted_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_03_community_id_kind_pubkey_channel_id_deleted_idx ON events_p2026_03 (community_id, kind, pubkey, channel_id, deleted_at);

--
-- Name: events_p2026_03_community_id_kind_pubkey_d_tag_created_at_i_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_03_community_id_kind_pubkey_d_tag_created_at_i_idx ON events_p2026_03 (community_id, kind, pubkey, d_tag, created_at DESC, id) WHERE (d_tag IS NOT NULL) AND (deleted_at IS NULL);

--
-- Name: events_p2026_03_community_id_not_before_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_03_community_id_not_before_idx ON events_p2026_03 (community_id, not_before) WHERE (not_before IS NOT NULL) AND (deleted_at IS NULL) AND (delivered_at IS NULL);

--
-- Name: events_p2026_03_community_id_pubkey_kind_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_03_community_id_pubkey_kind_created_at_id_idx ON events_p2026_03 (community_id, pubkey, kind, created_at DESC, id);

--
-- Name: events_p2026_03_search_tsv_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_03_search_tsv_idx ON events_p2026_03 USING gin (search_tsv);

--
-- Name: events_p2026_03_tags_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_03_tags_idx ON events_p2026_03 USING gin (tags jsonb_path_ops);

--
-- Name: events_p2026_04; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS events_p2026_04 (
    community_id uuid,
    id bytea,
    pubkey bytea NOT NULL,
    created_at timestamptz,
    kind integer NOT NULL,
    tags jsonb NOT NULL,
    content text NOT NULL,
    sig bytea NOT NULL,
    received_at timestamptz DEFAULT now() NOT NULL,
    channel_id uuid,
    deleted_at timestamptz,
    d_tag text,
    not_before bigint,
    delivered_at bigint,
    search_tsv tsvector GENERATED ALWAYS AS (
CASE
    WHEN (kind = 30350) THEN NULL::tsvector
    ELSE
    CASE
        WHEN (kind = ANY (ARRAY[0, 9, 40002, 45001, 45003])) THEN to_tsvector('simple'::regconfig, content)
        ELSE NULL::tsvector
    END
END) STORED,
    CONSTRAINT events_p2026_04_pkey PRIMARY KEY (created_at, community_id, id),
    CONSTRAINT events_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: events_p2026_04_community_id_channel_id_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_04_community_id_channel_id_created_at_id_idx ON events_p2026_04 (community_id, channel_id, created_at DESC, id);

--
-- Name: events_p2026_04_community_id_deleted_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_04_community_id_deleted_at_idx ON events_p2026_04 (community_id, deleted_at);

--
-- Name: events_p2026_04_community_id_id_created_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_04_community_id_id_created_at_idx ON events_p2026_04 (community_id, id, created_at DESC);

--
-- Name: events_p2026_04_community_id_kind_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_04_community_id_kind_created_at_id_idx ON events_p2026_04 (community_id, kind, created_at DESC, id);

--
-- Name: events_p2026_04_community_id_kind_pubkey_channel_id_deleted_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_04_community_id_kind_pubkey_channel_id_deleted_idx ON events_p2026_04 (community_id, kind, pubkey, channel_id, deleted_at);

--
-- Name: events_p2026_04_community_id_kind_pubkey_d_tag_created_at_i_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_04_community_id_kind_pubkey_d_tag_created_at_i_idx ON events_p2026_04 (community_id, kind, pubkey, d_tag, created_at DESC, id) WHERE (d_tag IS NOT NULL) AND (deleted_at IS NULL);

--
-- Name: events_p2026_04_community_id_not_before_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_04_community_id_not_before_idx ON events_p2026_04 (community_id, not_before) WHERE (not_before IS NOT NULL) AND (deleted_at IS NULL) AND (delivered_at IS NULL);

--
-- Name: events_p2026_04_community_id_pubkey_kind_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_04_community_id_pubkey_kind_created_at_id_idx ON events_p2026_04 (community_id, pubkey, kind, created_at DESC, id);

--
-- Name: events_p2026_04_search_tsv_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_04_search_tsv_idx ON events_p2026_04 USING gin (search_tsv);

--
-- Name: events_p2026_04_tags_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_04_tags_idx ON events_p2026_04 USING gin (tags jsonb_path_ops);

--
-- Name: events_p2026_05; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS events_p2026_05 (
    community_id uuid,
    id bytea,
    pubkey bytea NOT NULL,
    created_at timestamptz,
    kind integer NOT NULL,
    tags jsonb NOT NULL,
    content text NOT NULL,
    sig bytea NOT NULL,
    received_at timestamptz DEFAULT now() NOT NULL,
    channel_id uuid,
    deleted_at timestamptz,
    d_tag text,
    not_before bigint,
    delivered_at bigint,
    search_tsv tsvector GENERATED ALWAYS AS (
CASE
    WHEN (kind = 30350) THEN NULL::tsvector
    ELSE
    CASE
        WHEN (kind = ANY (ARRAY[0, 9, 40002, 45001, 45003])) THEN to_tsvector('simple'::regconfig, content)
        ELSE NULL::tsvector
    END
END) STORED,
    CONSTRAINT events_p2026_05_pkey PRIMARY KEY (created_at, community_id, id),
    CONSTRAINT events_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: events_p2026_05_community_id_channel_id_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_05_community_id_channel_id_created_at_id_idx ON events_p2026_05 (community_id, channel_id, created_at DESC, id);

--
-- Name: events_p2026_05_community_id_deleted_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_05_community_id_deleted_at_idx ON events_p2026_05 (community_id, deleted_at);

--
-- Name: events_p2026_05_community_id_id_created_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_05_community_id_id_created_at_idx ON events_p2026_05 (community_id, id, created_at DESC);

--
-- Name: events_p2026_05_community_id_kind_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_05_community_id_kind_created_at_id_idx ON events_p2026_05 (community_id, kind, created_at DESC, id);

--
-- Name: events_p2026_05_community_id_kind_pubkey_channel_id_deleted_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_05_community_id_kind_pubkey_channel_id_deleted_idx ON events_p2026_05 (community_id, kind, pubkey, channel_id, deleted_at);

--
-- Name: events_p2026_05_community_id_kind_pubkey_d_tag_created_at_i_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_05_community_id_kind_pubkey_d_tag_created_at_i_idx ON events_p2026_05 (community_id, kind, pubkey, d_tag, created_at DESC, id) WHERE (d_tag IS NOT NULL) AND (deleted_at IS NULL);

--
-- Name: events_p2026_05_community_id_not_before_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_05_community_id_not_before_idx ON events_p2026_05 (community_id, not_before) WHERE (not_before IS NOT NULL) AND (deleted_at IS NULL) AND (delivered_at IS NULL);

--
-- Name: events_p2026_05_community_id_pubkey_kind_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_05_community_id_pubkey_kind_created_at_id_idx ON events_p2026_05 (community_id, pubkey, kind, created_at DESC, id);

--
-- Name: events_p2026_05_search_tsv_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_05_search_tsv_idx ON events_p2026_05 USING gin (search_tsv);

--
-- Name: events_p2026_05_tags_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_05_tags_idx ON events_p2026_05 USING gin (tags jsonb_path_ops);

--
-- Name: events_p2026_06; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS events_p2026_06 (
    community_id uuid,
    id bytea,
    pubkey bytea NOT NULL,
    created_at timestamptz,
    kind integer NOT NULL,
    tags jsonb NOT NULL,
    content text NOT NULL,
    sig bytea NOT NULL,
    received_at timestamptz DEFAULT now() NOT NULL,
    channel_id uuid,
    deleted_at timestamptz,
    d_tag text,
    not_before bigint,
    delivered_at bigint,
    search_tsv tsvector GENERATED ALWAYS AS (
CASE
    WHEN (kind = 30350) THEN NULL::tsvector
    ELSE
    CASE
        WHEN (kind = ANY (ARRAY[0, 9, 40002, 45001, 45003])) THEN to_tsvector('simple'::regconfig, content)
        ELSE NULL::tsvector
    END
END) STORED,
    CONSTRAINT events_p2026_06_pkey PRIMARY KEY (created_at, community_id, id),
    CONSTRAINT events_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: events_p2026_06_community_id_channel_id_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_06_community_id_channel_id_created_at_id_idx ON events_p2026_06 (community_id, channel_id, created_at DESC, id);

--
-- Name: events_p2026_06_community_id_deleted_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_06_community_id_deleted_at_idx ON events_p2026_06 (community_id, deleted_at);

--
-- Name: events_p2026_06_community_id_id_created_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_06_community_id_id_created_at_idx ON events_p2026_06 (community_id, id, created_at DESC);

--
-- Name: events_p2026_06_community_id_kind_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_06_community_id_kind_created_at_id_idx ON events_p2026_06 (community_id, kind, created_at DESC, id);

--
-- Name: events_p2026_06_community_id_kind_pubkey_channel_id_deleted_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_06_community_id_kind_pubkey_channel_id_deleted_idx ON events_p2026_06 (community_id, kind, pubkey, channel_id, deleted_at);

--
-- Name: events_p2026_06_community_id_kind_pubkey_d_tag_created_at_i_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_06_community_id_kind_pubkey_d_tag_created_at_i_idx ON events_p2026_06 (community_id, kind, pubkey, d_tag, created_at DESC, id) WHERE (d_tag IS NOT NULL) AND (deleted_at IS NULL);

--
-- Name: events_p2026_06_community_id_not_before_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_06_community_id_not_before_idx ON events_p2026_06 (community_id, not_before) WHERE (not_before IS NOT NULL) AND (deleted_at IS NULL) AND (delivered_at IS NULL);

--
-- Name: events_p2026_06_community_id_pubkey_kind_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_06_community_id_pubkey_kind_created_at_id_idx ON events_p2026_06 (community_id, pubkey, kind, created_at DESC, id);

--
-- Name: events_p2026_06_search_tsv_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_06_search_tsv_idx ON events_p2026_06 USING gin (search_tsv);

--
-- Name: events_p2026_06_tags_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p2026_06_tags_idx ON events_p2026_06 USING gin (tags jsonb_path_ops);

--
-- Name: events_p_future; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS events_p_future (
    community_id uuid,
    id bytea,
    pubkey bytea NOT NULL,
    created_at timestamptz,
    kind integer NOT NULL,
    tags jsonb NOT NULL,
    content text NOT NULL,
    sig bytea NOT NULL,
    received_at timestamptz DEFAULT now() NOT NULL,
    channel_id uuid,
    deleted_at timestamptz,
    d_tag text,
    not_before bigint,
    delivered_at bigint,
    search_tsv tsvector GENERATED ALWAYS AS (
CASE
    WHEN (kind = 30350) THEN NULL::tsvector
    ELSE
    CASE
        WHEN (kind = ANY (ARRAY[0, 9, 40002, 45001, 45003])) THEN to_tsvector('simple'::regconfig, content)
        ELSE NULL::tsvector
    END
END) STORED,
    CONSTRAINT events_p_future_pkey PRIMARY KEY (created_at, community_id, id),
    CONSTRAINT events_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: events_p_future_community_id_channel_id_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_future_community_id_channel_id_created_at_id_idx ON events_p_future (community_id, channel_id, created_at DESC, id);

--
-- Name: events_p_future_community_id_deleted_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_future_community_id_deleted_at_idx ON events_p_future (community_id, deleted_at);

--
-- Name: events_p_future_community_id_id_created_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_future_community_id_id_created_at_idx ON events_p_future (community_id, id, created_at DESC);

--
-- Name: events_p_future_community_id_kind_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_future_community_id_kind_created_at_id_idx ON events_p_future (community_id, kind, created_at DESC, id);

--
-- Name: events_p_future_community_id_kind_pubkey_channel_id_deleted_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_future_community_id_kind_pubkey_channel_id_deleted_idx ON events_p_future (community_id, kind, pubkey, channel_id, deleted_at);

--
-- Name: events_p_future_community_id_kind_pubkey_d_tag_created_at_i_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_future_community_id_kind_pubkey_d_tag_created_at_i_idx ON events_p_future (community_id, kind, pubkey, d_tag, created_at DESC, id) WHERE (d_tag IS NOT NULL) AND (deleted_at IS NULL);

--
-- Name: events_p_future_community_id_not_before_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_future_community_id_not_before_idx ON events_p_future (community_id, not_before) WHERE (not_before IS NOT NULL) AND (deleted_at IS NULL) AND (delivered_at IS NULL);

--
-- Name: events_p_future_community_id_pubkey_kind_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_future_community_id_pubkey_kind_created_at_id_idx ON events_p_future (community_id, pubkey, kind, created_at DESC, id);

--
-- Name: events_p_future_search_tsv_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_future_search_tsv_idx ON events_p_future USING gin (search_tsv);

--
-- Name: events_p_future_tags_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_future_tags_idx ON events_p_future USING gin (tags jsonb_path_ops);

--
-- Name: events_p_past; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS events_p_past (
    community_id uuid,
    id bytea,
    pubkey bytea NOT NULL,
    created_at timestamptz,
    kind integer NOT NULL,
    tags jsonb NOT NULL,
    content text NOT NULL,
    sig bytea NOT NULL,
    received_at timestamptz DEFAULT now() NOT NULL,
    channel_id uuid,
    deleted_at timestamptz,
    d_tag text,
    not_before bigint,
    delivered_at bigint,
    search_tsv tsvector GENERATED ALWAYS AS (
CASE
    WHEN (kind = 30350) THEN NULL::tsvector
    ELSE
    CASE
        WHEN (kind = ANY (ARRAY[0, 9, 40002, 45001, 45003])) THEN to_tsvector('simple'::regconfig, content)
        ELSE NULL::tsvector
    END
END) STORED,
    CONSTRAINT events_p_past_pkey PRIMARY KEY (created_at, community_id, id),
    CONSTRAINT events_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: events_p_past_community_id_channel_id_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_past_community_id_channel_id_created_at_id_idx ON events_p_past (community_id, channel_id, created_at DESC, id);

--
-- Name: events_p_past_community_id_deleted_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_past_community_id_deleted_at_idx ON events_p_past (community_id, deleted_at);

--
-- Name: events_p_past_community_id_id_created_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_past_community_id_id_created_at_idx ON events_p_past (community_id, id, created_at DESC);

--
-- Name: events_p_past_community_id_kind_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_past_community_id_kind_created_at_id_idx ON events_p_past (community_id, kind, created_at DESC, id);

--
-- Name: events_p_past_community_id_kind_pubkey_channel_id_deleted_a_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_past_community_id_kind_pubkey_channel_id_deleted_a_idx ON events_p_past (community_id, kind, pubkey, channel_id, deleted_at);

--
-- Name: events_p_past_community_id_kind_pubkey_d_tag_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_past_community_id_kind_pubkey_d_tag_created_at_id_idx ON events_p_past (community_id, kind, pubkey, d_tag, created_at DESC, id) WHERE (d_tag IS NOT NULL) AND (deleted_at IS NULL);

--
-- Name: events_p_past_community_id_not_before_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_past_community_id_not_before_idx ON events_p_past (community_id, not_before) WHERE (not_before IS NOT NULL) AND (deleted_at IS NULL) AND (delivered_at IS NULL);

--
-- Name: events_p_past_community_id_pubkey_kind_created_at_id_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_past_community_id_pubkey_kind_created_at_id_idx ON events_p_past (community_id, pubkey, kind, created_at DESC, id);

--
-- Name: events_p_past_search_tsv_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_past_search_tsv_idx ON events_p_past USING gin (search_tsv);

--
-- Name: events_p_past_tags_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS events_p_past_tags_idx ON events_p_past USING gin (tags jsonb_path_ops);

--
-- Name: git_repo_names; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS git_repo_names (
    community_id uuid,
    repo_id text,
    owner_pubkey text NOT NULL,
    created_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT git_repo_names_pkey PRIMARY KEY (community_id, repo_id),
    CONSTRAINT git_repo_names_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: idx_git_repo_names_owner; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_git_repo_names_owner ON git_repo_names (community_id, owner_pubkey);

--
-- Name: identity_enrollment_policies; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS identity_enrollment_policies (
    community_id uuid,
    policy_revision bigint,
    enrollment_mode smallint NOT NULL,
    policy_digest bytea NOT NULL,
    effective_at timestamptz NOT NULL,
    expires_at timestamptz,
    recorded_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    CONSTRAINT identity_enrollment_policies_pkey PRIMARY KEY (community_id, policy_revision),
    CONSTRAINT identity_enrollment_policies_community_id_policy_revision_e_key UNIQUE (community_id, policy_revision, enrollment_mode),
    CONSTRAINT identity_enrollment_policies_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT identity_enrollment_policies_check CHECK (expires_at IS NULL OR effective_at < expires_at),
    CONSTRAINT identity_enrollment_policies_enrollment_mode_check CHECK (enrollment_mode IN (1, 2, 3)),
    CONSTRAINT identity_enrollment_policies_policy_digest_check CHECK (octet_length(policy_digest) = 32),
    CONSTRAINT identity_enrollment_policies_policy_revision_check CHECK (policy_revision > 0)
);

--
-- Name: moderation_actions; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS moderation_actions (
    community_id uuid,
    id uuid DEFAULT gen_random_uuid(),
    actor_pubkey bytea NOT NULL,
    action text NOT NULL,
    target_pubkey bytea,
    target_event_id bytea,
    channel_id uuid,
    reason_code text,
    public_reason text,
    private_reason text,
    matched_principal text,
    created_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT moderation_actions_pkey PRIMARY KEY (community_id, id),
    CONSTRAINT moderation_actions_community_id_channel_id_fkey FOREIGN KEY (community_id, channel_id) REFERENCES channels (community_id, id),
    CONSTRAINT moderation_actions_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT moderation_actions_action_check CHECK (action IN ('delete_message'::text, 'kick'::text, 'ban'::text, 'unban'::text, 'timeout'::text, 'untimeout'::text, 'dismiss_report'::text, 'escalate'::text, 'resolve:delete'::text, 'resolve:kick'::text, 'resolve:ban'::text, 'resolve:timeout'::text)),
    CONSTRAINT moderation_actions_actor_pubkey_check CHECK (length(actor_pubkey) = 32),
    CONSTRAINT moderation_actions_matched_principal_check CHECK (matched_principal IS NULL OR (matched_principal IN ('self'::text, 'owner'::text))),
    CONSTRAINT moderation_actions_target_event_id_check CHECK (target_event_id IS NULL OR length(target_event_id) = 32),
    CONSTRAINT moderation_actions_target_pubkey_check CHECK (target_pubkey IS NULL OR length(target_pubkey) = 32)
);

--
-- Name: idx_moderation_actions_created; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_moderation_actions_created ON moderation_actions (community_id, created_at DESC);

--
-- Name: idx_moderation_actions_target_pubkey; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_moderation_actions_target_pubkey ON moderation_actions (community_id, target_pubkey) WHERE (target_pubkey IS NOT NULL);

--
-- Name: moderation_reports; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS moderation_reports (
    community_id uuid,
    id uuid DEFAULT gen_random_uuid(),
    report_event_id bytea NOT NULL,
    reporter_pubkey bytea NOT NULL,
    target_kind text NOT NULL,
    target_event_id bytea,
    target_pubkey bytea,
    target_blob_sha256 bytea,
    channel_id uuid,
    report_type text NOT NULL,
    note text,
    status text DEFAULT 'open' NOT NULL,
    resolved_by bytea,
    resolved_at timestamptz,
    action_id uuid,
    created_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT moderation_reports_pkey PRIMARY KEY (community_id, id),
    CONSTRAINT moderation_reports_community_id_action_id_fkey FOREIGN KEY (community_id, action_id) REFERENCES moderation_actions (community_id, id),
    CONSTRAINT moderation_reports_community_id_channel_id_fkey FOREIGN KEY (community_id, channel_id) REFERENCES channels (community_id, id),
    CONSTRAINT moderation_reports_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT moderation_reports_report_event_id_check CHECK (length(report_event_id) = 32),
    CONSTRAINT moderation_reports_reporter_pubkey_check CHECK (length(reporter_pubkey) = 32),
    CONSTRAINT moderation_reports_status_check CHECK (status IN ('open'::text, 'resolved'::text, 'dismissed'::text, 'escalated'::text)),
    CONSTRAINT moderation_reports_target_blob_sha256_check CHECK (target_blob_sha256 IS NULL OR length(target_blob_sha256) = 32),
    CONSTRAINT moderation_reports_target_event_id_check CHECK (target_event_id IS NULL OR length(target_event_id) = 32),
    CONSTRAINT moderation_reports_target_kind_check CHECK (target_kind IN ('event'::text, 'pubkey'::text, 'blob'::text)),
    CONSTRAINT moderation_reports_target_pubkey_check CHECK (target_pubkey IS NULL OR length(target_pubkey) = 32)
);

--
-- Name: idx_moderation_reports_event; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS idx_moderation_reports_event ON moderation_reports (community_id, report_event_id);

--
-- Name: idx_moderation_reports_status; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_moderation_reports_status ON moderation_reports (community_id, status, created_at DESC);

--
-- Name: idx_moderation_reports_target_event; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_moderation_reports_target_event ON moderation_reports (community_id, target_event_id) WHERE (target_event_id IS NOT NULL);

--
-- Name: idx_moderation_reports_target_pubkey; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_moderation_reports_target_pubkey ON moderation_reports (community_id, target_pubkey) WHERE (target_pubkey IS NOT NULL);

--
-- Name: parameterized_event_watermarks; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS parameterized_event_watermarks (
    community_id uuid,
    kind integer,
    pubkey bytea,
    d_tag text,
    created_at timestamptz NOT NULL,
    event_id bytea NOT NULL,
    CONSTRAINT parameterized_event_watermarks_pkey PRIMARY KEY (community_id, kind, pubkey, d_tag),
    CONSTRAINT parameterized_event_watermarks_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: product_feedback; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS product_feedback (
    id uuid DEFAULT gen_random_uuid(),
    community_id uuid NOT NULL,
    event_id bytea NOT NULL,
    submitter_pubkey bytea NOT NULL,
    category text,
    body text NOT NULL,
    tags jsonb DEFAULT '[]' NOT NULL,
    event_created_at timestamptz NOT NULL,
    received_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT product_feedback_pkey PRIMARY KEY (id),
    CONSTRAINT product_feedback_event_id_key UNIQUE (event_id),
    CONSTRAINT product_feedback_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT product_feedback_body_check CHECK (length(btrim(body)) > 0),
    CONSTRAINT product_feedback_category_check CHECK (category IN ('bug'::text, 'praise'::text, 'needs-work'::text)),
    CONSTRAINT product_feedback_event_id_check CHECK (length(event_id) = 32),
    CONSTRAINT product_feedback_submitter_pubkey_check CHECK (length(submitter_pubkey) = 32),
    CONSTRAINT product_feedback_tags_check CHECK (jsonb_typeof(tags) = 'array'::text)
);

--
-- Name: idx_product_feedback_community_received; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_product_feedback_community_received ON product_feedback (community_id, received_at DESC, id);

--
-- Name: idx_product_feedback_received; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_product_feedback_received ON product_feedback (received_at DESC, id);

--
-- Name: protected_publication_projection_outbox; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS protected_publication_projection_outbox (
    community_id uuid,
    operation_id uuid,
    request_fingerprint bytea NOT NULL,
    publication_result_digest bytea NOT NULL,
    object_kind smallint NOT NULL,
    object_key bytea NOT NULL,
    application_type bytea NOT NULL,
    application_version smallint NOT NULL,
    application_effect_digest bytea NOT NULL,
    authority_operation_id uuid,
    authority_request_fingerprint bytea,
    authority_epoch bigint,
    authority_fence bytea,
    projection_kind smallint,
    projection_key text NOT NULL,
    staged_object_key text NOT NULL,
    payload_digest bytea NOT NULL,
    delivery_state smallint DEFAULT 1 NOT NULL,
    attempt_count bigint DEFAULT 0 NOT NULL,
    next_attempt_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    last_attempt_at timestamptz,
    delivered_at timestamptz,
    failure_code smallint DEFAULT 0 NOT NULL,
    created_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    publication_sequence bigint NOT NULL,
    CONSTRAINT protected_publication_projection_outbox_pkey PRIMARY KEY (community_id, operation_id, projection_kind),
    CONSTRAINT protected_publication_project_community_id_publication_sequ_key UNIQUE (community_id, publication_sequence),
    CONSTRAINT protected_publication_project_community_id_operation_id_re_fkey FOREIGN KEY (community_id, operation_id, request_fingerprint) REFERENCES authorization_operation_receipts (community_id, operation_id, request_fingerprint) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT protected_publication_projection_outbox_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT protected_publication_projectio_application_effect_digest_check CHECK (octet_length(application_effect_digest) = 32 AND application_effect_digest <> decode(repeat('00'::text, 32), 'hex'::text)),
    CONSTRAINT protected_publication_projection_authority_binding CHECK ((authority_operation_id IS NULL AND authority_request_fingerprint IS NULL AND authority_epoch IS NULL AND authority_fence IS NULL) OR (authority_operation_id IS NOT NULL AND authority_operation_id <> '00000000-0000-0000-0000-000000000000'::uuid AND authority_request_fingerprint IS NOT NULL AND octet_length(authority_request_fingerprint) = 32 AND authority_epoch IS NOT NULL AND authority_epoch > 0 AND authority_fence IS NOT NULL AND octet_length(authority_fence) = 32 AND authority_fence <> decode(repeat('00'::text, 32), 'hex'::text))),
    CONSTRAINT protected_publication_projectio_publication_result_digest_check CHECK (octet_length(publication_result_digest) = 32 AND publication_result_digest <> decode(repeat('00'::text, 32), 'hex'::text)),
    CONSTRAINT protected_publication_projection_outb_application_version_check CHECK (application_version > 0),
    CONSTRAINT protected_publication_projection_outb_request_fingerprint_check CHECK (octet_length(request_fingerprint) = 32 AND request_fingerprint <> decode(repeat('00'::text, 32), 'hex'::text)),
    CONSTRAINT protected_publication_projection_outbox_application_type_check CHECK (octet_length(application_type) = 32 AND application_type <> decode(repeat('00'::text, 32), 'hex'::text)),
    CONSTRAINT protected_publication_projection_outbox_attempt_count_check CHECK (attempt_count >= 0),
    CONSTRAINT protected_publication_projection_outbox_check CHECK ((attempt_count = 0) = (last_attempt_at IS NULL)),
    CONSTRAINT protected_publication_projection_outbox_check1 CHECK (last_attempt_at IS NULL OR last_attempt_at >= created_at),
    CONSTRAINT protected_publication_projection_outbox_check2 CHECK (delivered_at IS NULL OR delivered_at >= created_at),
    CONSTRAINT protected_publication_projection_outbox_delivery_state_check CHECK (delivery_state IN (1, 2, 3)),
    CONSTRAINT protected_publication_projection_outbox_failure_code_check CHECK (failure_code >= 0 AND failure_code <= 6),
    CONSTRAINT protected_publication_projection_outbox_object_key_check CHECK (octet_length(object_key) = 32 AND object_key <> decode(repeat('00'::text, 32), 'hex'::text)),
    CONSTRAINT protected_publication_projection_outbox_object_kind_check CHECK (object_kind IN (3, 4)),
    CONSTRAINT protected_publication_projection_outbox_operation_id_check CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CONSTRAINT protected_publication_projection_outbox_payload_digest_check CHECK (octet_length(payload_digest) = 32 AND payload_digest <> decode(repeat('00'::text, 32), 'hex'::text)),
    CONSTRAINT protected_publication_projection_outbox_projection_key_check CHECK (octet_length(projection_key) >= 1 AND octet_length(projection_key) <= 2048),
    CONSTRAINT protected_publication_projection_outbox_projection_kind_check CHECK (projection_kind IN (1, 2, 3)),
    CONSTRAINT protected_publication_projection_outbox_staged_object_key_check CHECK (octet_length(staged_object_key) >= 1 AND octet_length(staged_object_key) <= 2048)
);

--
-- Name: protected_publication_projection_outbox_active_target; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS protected_publication_projection_outbox_active_target ON protected_publication_projection_outbox (community_id, projection_kind, projection_key) WHERE (delivery_state = 1);

--
-- Name: protected_publication_projection_outbox_pending; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS protected_publication_projection_outbox_pending ON protected_publication_projection_outbox (delivery_state, next_attempt_at, community_id, operation_id) WHERE (delivery_state = 1);

--
-- Name: protected_publication_projection_outbox_target_order; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS protected_publication_projection_outbox_target_order ON protected_publication_projection_outbox (community_id, object_kind, object_key, projection_kind, projection_key, publication_sequence);

--
-- Name: pubkey_allowlist; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS pubkey_allowlist (
    community_id uuid,
    pubkey bytea,
    added_by bytea,
    added_at timestamptz DEFAULT now() NOT NULL,
    note text,
    CONSTRAINT pubkey_allowlist_pkey PRIMARY KEY (community_id, pubkey),
    CONSTRAINT pubkey_allowlist_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: push_gateway_challenges; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS push_gateway_challenges (
    id uuid,
    challenge_hash bytea NOT NULL,
    expires_at timestamptz NOT NULL,
    created_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT push_gateway_challenges_pkey PRIMARY KEY (id),
    CONSTRAINT push_gateway_challenges_challenge_hash_check CHECK (length(challenge_hash) = 32)
);

--
-- Name: push_gateway_challenges_expiry; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS push_gateway_challenges_expiry ON push_gateway_challenges (expires_at);

--
-- Name: push_gateway_delivery_auth_replays; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS push_gateway_delivery_auth_replays (
    relay_pubkey bytea,
    auth_event_id bytea,
    expires_at timestamptz NOT NULL,
    CONSTRAINT push_gateway_delivery_auth_replays_pkey PRIMARY KEY (relay_pubkey, auth_event_id),
    CONSTRAINT push_gateway_delivery_auth_replays_auth_event_id_check CHECK (length(auth_event_id) = 32),
    CONSTRAINT push_gateway_delivery_auth_replays_relay_pubkey_check CHECK (length(relay_pubkey) = 32)
);

--
-- Name: push_gateway_delivery_auth_replays_expiry; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS push_gateway_delivery_auth_replays_expiry ON push_gateway_delivery_auth_replays (expires_at);

--
-- Name: push_gateway_delivery_request_replays; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS push_gateway_delivery_request_replays (
    relay_pubkey bytea,
    request_id uuid,
    expires_at timestamptz NOT NULL,
    CONSTRAINT push_gateway_delivery_request_replays_pkey PRIMARY KEY (relay_pubkey, request_id),
    CONSTRAINT push_gateway_delivery_request_replays_relay_pubkey_check CHECK (length(relay_pubkey) = 32)
);

--
-- Name: push_gateway_delivery_request_replays_expiry; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS push_gateway_delivery_request_replays_expiry ON push_gateway_delivery_request_replays (expires_at);

--
-- Name: push_gateway_endpoint_quotas; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS push_gateway_endpoint_quotas (
    token_fingerprint bytea,
    window_started_at timestamptz NOT NULL,
    admitted bigint NOT NULL,
    updated_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT push_gateway_endpoint_quotas_pkey PRIMARY KEY (token_fingerprint),
    CONSTRAINT push_gateway_endpoint_quotas_admitted_check CHECK (admitted >= 0),
    CONSTRAINT push_gateway_endpoint_quotas_token_fingerprint_check CHECK (length(token_fingerprint) = 32)
);

--
-- Name: push_gateway_endpoint_quotas_updated; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS push_gateway_endpoint_quotas_updated ON push_gateway_endpoint_quotas (updated_at);

--
-- Name: push_gateway_installations; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS push_gateway_installations (
    id uuid,
    app_attest_key_id bytea NOT NULL,
    app_attest_public_key bytea NOT NULL,
    assertion_counter bigint NOT NULL,
    app_profile text NOT NULL,
    token_ciphertext bytea NOT NULL,
    token_fingerprint bytea NOT NULL,
    endpoint_epoch bigint NOT NULL,
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    created_at timestamptz DEFAULT now() NOT NULL,
    updated_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT push_gateway_installations_pkey PRIMARY KEY (id),
    CONSTRAINT push_gateway_installations_app_attest_key_id_key UNIQUE (app_attest_key_id),
    CONSTRAINT push_gateway_installations_app_profile_token_fingerprint_key UNIQUE (app_profile, token_fingerprint),
    CONSTRAINT push_gateway_installations_app_attest_key_id_check CHECK (octet_length(app_attest_key_id) >= 1 AND octet_length(app_attest_key_id) <= 128),
    CONSTRAINT push_gateway_installations_app_attest_public_key_check CHECK (octet_length(app_attest_public_key) >= 33 AND octet_length(app_attest_public_key) <= 256),
    CONSTRAINT push_gateway_installations_app_profile_check CHECK (app_profile IN ('buzz-ios-production'::text, 'buzz-ios-sandbox'::text)),
    CONSTRAINT push_gateway_installations_assertion_counter_check CHECK (assertion_counter >= 0 AND assertion_counter <= '4294967295'::bigint),
    CONSTRAINT push_gateway_installations_endpoint_epoch_check CHECK (endpoint_epoch > 0),
    CONSTRAINT push_gateway_installations_token_ciphertext_check CHECK (octet_length(token_ciphertext) >= 1 AND octet_length(token_ciphertext) <= 2048),
    CONSTRAINT push_gateway_installations_token_fingerprint_check CHECK (length(token_fingerprint) = 32)
);

--
-- Name: push_gateway_installations_expiry; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS push_gateway_installations_expiry ON push_gateway_installations (expires_at) WHERE (revoked_at IS NULL);

--
-- Name: push_gateway_delegations; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS push_gateway_delegations (
    id uuid,
    installation_id uuid NOT NULL,
    relay_pubkey bytea NOT NULL,
    endpoint_epoch bigint NOT NULL,
    generation bigint NOT NULL,
    not_before timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    updated_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT push_gateway_delegations_pkey PRIMARY KEY (id),
    CONSTRAINT push_gateway_delegations_installation_id_relay_pubkey_key UNIQUE (installation_id, relay_pubkey),
    CONSTRAINT push_gateway_delegations_installation_id_fkey FOREIGN KEY (installation_id) REFERENCES push_gateway_installations (id),
    CONSTRAINT push_gateway_delegations_check CHECK (not_before < expires_at),
    CONSTRAINT push_gateway_delegations_endpoint_epoch_check CHECK (endpoint_epoch > 0),
    CONSTRAINT push_gateway_delegations_generation_check CHECK (generation > 0),
    CONSTRAINT push_gateway_delegations_relay_pubkey_check CHECK (length(relay_pubkey) = 32)
);

--
-- Name: push_gateway_delegations_expiry; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS push_gateway_delegations_expiry ON push_gateway_delegations (expires_at) WHERE (revoked_at IS NULL);

--
-- Name: push_leases; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS push_leases (
    community_id uuid,
    author bytea,
    installation_id text,
    source_event_id bytea NOT NULL,
    source_created_at bigint NOT NULL,
    generation bigint NOT NULL,
    active boolean NOT NULL,
    app_profile text,
    endpoint_hash bytea,
    endpoint_grant text,
    max_class text,
    subscriptions jsonb,
    expires_at bigint NOT NULL,
    updated_at timestamptz DEFAULT now() NOT NULL,
    endpoint_enabled boolean DEFAULT true NOT NULL,
    CONSTRAINT push_leases_pkey PRIMARY KEY (community_id, author, installation_id),
    CONSTRAINT push_leases_community_id_source_event_id_key UNIQUE (community_id, source_event_id),
    CONSTRAINT push_leases_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT push_leases_author_check CHECK (length(author) = 32),
    CONSTRAINT push_leases_endpoint_hash_check CHECK (endpoint_hash IS NULL OR length(endpoint_hash) = 32),
    CONSTRAINT push_leases_generation_check CHECK (generation > 0),
    CONSTRAINT push_leases_installation_id_check CHECK (octet_length(installation_id) >= 1 AND octet_length(installation_id) <= 64),
    CONSTRAINT push_leases_max_class_check CHECK (max_class IS NULL OR (max_class IN ('silent'::text, 'default'::text, 'time_sensitive'::text, 'urgent'::text))),
    CONSTRAINT push_leases_source_event_id_check CHECK (length(source_event_id) = 32)
);

--
-- Name: push_leases_endpoint_unique; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS push_leases_endpoint_unique ON push_leases (community_id, author, app_profile, endpoint_hash) WHERE (active);

--
-- Name: push_leases_expiry; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS push_leases_expiry ON push_leases (community_id, expires_at) WHERE (active);

--
-- Name: push_match_queue; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS push_match_queue (
    community_id uuid,
    event_id bytea,
    state text DEFAULT 'pending' NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    next_attempt_at timestamptz DEFAULT now() NOT NULL,
    lease_until timestamptz,
    claim_id uuid,
    created_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT push_match_queue_pkey PRIMARY KEY (community_id, event_id),
    CONSTRAINT push_match_queue_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT push_match_queue_attempts_check CHECK (attempts >= 0),
    CONSTRAINT push_match_queue_event_id_check CHECK (length(event_id) = 32),
    CONSTRAINT push_match_queue_state_check CHECK (state IN ('pending'::text, 'matching'::text))
);

--
-- Name: push_match_queue_due; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS push_match_queue_due ON push_match_queue (next_attempt_at, created_at) WHERE (state = 'pending'::text);

--
-- Name: push_match_queue_recovery; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS push_match_queue_recovery ON push_match_queue (lease_until) WHERE (state = 'matching'::text);

--
-- Name: push_wake_outbox; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS push_wake_outbox (
    community_id uuid,
    id uuid DEFAULT gen_random_uuid(),
    author bytea NOT NULL,
    installation_id text NOT NULL,
    lease_generation bigint NOT NULL,
    endpoint_hash bytea NOT NULL,
    event_id bytea NOT NULL,
    class text NOT NULL,
    expires_at bigint NOT NULL,
    state text DEFAULT 'pending' NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    next_attempt_at timestamptz DEFAULT now() NOT NULL,
    lease_until timestamptz,
    claim_id uuid,
    created_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT push_wake_outbox_pkey PRIMARY KEY (community_id, id),
    CONSTRAINT push_wake_outbox_community_id_endpoint_hash_event_id_key UNIQUE (community_id, endpoint_hash, event_id),
    CONSTRAINT push_wake_outbox_community_id_author_installation_id_fkey FOREIGN KEY (community_id, author, installation_id) REFERENCES push_leases (community_id, author, installation_id),
    CONSTRAINT push_wake_outbox_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT push_wake_outbox_attempts_check CHECK (attempts >= 0),
    CONSTRAINT push_wake_outbox_author_check CHECK (length(author) = 32),
    CONSTRAINT push_wake_outbox_class_check CHECK (class IN ('silent'::text, 'default'::text, 'time_sensitive'::text, 'urgent'::text)),
    CONSTRAINT push_wake_outbox_endpoint_hash_check CHECK (length(endpoint_hash) = 32),
    CONSTRAINT push_wake_outbox_event_id_check CHECK (length(event_id) = 32),
    CONSTRAINT push_wake_outbox_lease_generation_check CHECK (lease_generation > 0),
    CONSTRAINT push_wake_outbox_state_check CHECK (state IN ('pending'::text, 'sending'::text, 'delivered'::text, 'failed'::text))
);

--
-- Name: push_wake_outbox_due; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS push_wake_outbox_due ON push_wake_outbox (community_id, next_attempt_at) WHERE (state = 'pending'::text);

--
-- Name: push_wake_outbox_recovery; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS push_wake_outbox_recovery ON push_wake_outbox (community_id, lease_until) WHERE (state = 'sending'::text);

--
-- Name: rate_limit_violations; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS rate_limit_violations (
    id bigint GENERATED ALWAYS AS IDENTITY,
    community_id uuid,
    pubkey bytea,
    violation_at timestamptz DEFAULT now() NOT NULL,
    limit_type varchar(64),
    limit_value integer,
    actual_value integer,
    action_taken varchar(64),
    CONSTRAINT rate_limit_violations_pkey PRIMARY KEY (id)
);

--
-- Name: reactions; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS reactions (
    community_id uuid,
    event_created_at timestamptz,
    event_id bytea,
    pubkey bytea,
    emoji varchar(66),
    created_at timestamptz DEFAULT now() NOT NULL,
    removed_at timestamptz,
    reaction_event_id bytea,
    CONSTRAINT reactions_pkey PRIMARY KEY (community_id, event_created_at, event_id, pubkey, emoji),
    CONSTRAINT reactions_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: idx_reactions_event; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_reactions_event ON reactions (community_id, event_id, event_created_at);

--
-- Name: idx_reactions_pubkey; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_reactions_pubkey ON reactions (community_id, pubkey);

--
-- Name: idx_reactions_source_event; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS idx_reactions_source_event ON reactions (community_id, reaction_event_id) WHERE (reaction_event_id IS NOT NULL);

--
-- Name: relay_invites; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS relay_invites (
    community_id uuid,
    id uuid DEFAULT gen_random_uuid(),
    token_hash bytea NOT NULL,
    role text DEFAULT 'member' NOT NULL,
    max_uses integer,
    use_count integer DEFAULT 0 NOT NULL,
    expires_at timestamptz NOT NULL,
    created_by text NOT NULL,
    created_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT relay_invites_pkey PRIMARY KEY (community_id, id),
    CONSTRAINT relay_invites_community_id_token_hash_key UNIQUE (community_id, token_hash),
    CONSTRAINT relay_invites_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT relay_invites_check CHECK (max_uses IS NULL OR use_count <= max_uses),
    CONSTRAINT relay_invites_max_uses_check CHECK (max_uses >= 1 AND max_uses <= 10000),
    CONSTRAINT relay_invites_role_check CHECK (role = 'member'::text),
    CONSTRAINT relay_invites_token_hash_check CHECK (length(token_hash) = 32),
    CONSTRAINT relay_invites_use_count_check CHECK (use_count >= 0)
);

--
-- Name: relay_invites_expires_at_idx; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS relay_invites_expires_at_idx ON relay_invites (expires_at);

--
-- Name: relay_members; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS relay_members (
    community_id uuid,
    pubkey text,
    role text NOT NULL,
    added_by text,
    created_at timestamptz DEFAULT now() NOT NULL,
    updated_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT relay_members_pkey PRIMARY KEY (community_id, pubkey),
    CONSTRAINT relay_members_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT relay_members_role_check CHECK (role IN ('owner'::text, 'admin'::text, 'member'::text))
);

--
-- Name: idx_relay_members_role; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_relay_members_role ON relay_members (community_id, role);

--
-- Name: join_policy_acceptances; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS join_policy_acceptances (
    community_id uuid,
    pubkey text,
    policy_version text,
    accepted_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT join_policy_acceptances_pkey PRIMARY KEY (community_id, pubkey, policy_version),
    CONSTRAINT join_policy_acceptances_community_id_pubkey_fkey FOREIGN KEY (community_id, pubkey) REFERENCES relay_members (community_id, pubkey) ON DELETE CASCADE,
    CONSTRAINT join_policy_acceptances_policy_version_check CHECK (length(policy_version) = 64)
);

--
-- Name: replica_heartbeat; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS replica_heartbeat (
    id smallint,
    epoch uuid DEFAULT gen_random_uuid() NOT NULL,
    token bigint DEFAULT 0 NOT NULL,
    CONSTRAINT replica_heartbeat_pkey PRIMARY KEY (id),
    CONSTRAINT replica_heartbeat_id_check CHECK (id = 1)
);

--
-- Name: thread_metadata; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS thread_metadata (
    community_id uuid,
    event_created_at timestamptz,
    event_id bytea,
    channel_id uuid NOT NULL,
    parent_event_id bytea,
    parent_event_created_at timestamptz,
    root_event_id bytea,
    root_event_created_at timestamptz,
    depth integer DEFAULT 0 NOT NULL,
    reply_count integer DEFAULT 0 NOT NULL,
    descendant_count integer DEFAULT 0 NOT NULL,
    last_reply_at timestamptz,
    broadcast boolean DEFAULT false NOT NULL,
    CONSTRAINT thread_metadata_pkey PRIMARY KEY (community_id, event_created_at, event_id),
    CONSTRAINT thread_metadata_community_id_channel_id_fkey FOREIGN KEY (community_id, channel_id) REFERENCES channels (community_id, id),
    CONSTRAINT thread_metadata_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id)
);

--
-- Name: idx_thread_metadata_channel_depth; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_thread_metadata_channel_depth ON thread_metadata (community_id, channel_id, depth, event_created_at);

--
-- Name: idx_thread_metadata_event_id; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_thread_metadata_event_id ON thread_metadata (community_id, event_id);

--
-- Name: idx_thread_metadata_parent; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_thread_metadata_parent ON thread_metadata (community_id, parent_event_id);

--
-- Name: idx_thread_metadata_root; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_thread_metadata_root ON thread_metadata (community_id, root_event_id);

--
-- Name: users; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS users (
    community_id uuid,
    pubkey bytea,
    nip05_handle varchar(255),
    display_name varchar(255),
    avatar_url text,
    about text,
    agent_type varchar(255),
    capabilities jsonb,
    okta_user_id varchar(255),
    created_at timestamptz DEFAULT now() NOT NULL,
    updated_at timestamptz DEFAULT now() NOT NULL,
    deactivated_at timestamptz,
    metadata_event_id bytea,
    agent_owner_pubkey bytea,
    channel_add_policy channel_add_policy DEFAULT 'anyone'::channel_add_policy NOT NULL,
    CONSTRAINT users_pkey PRIMARY KEY (community_id, pubkey),
    CONSTRAINT users_community_id_agent_owner_pubkey_fkey FOREIGN KEY (community_id, agent_owner_pubkey) REFERENCES users (community_id, pubkey) ON DELETE SET NULL,
    CONSTRAINT users_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT chk_users_pubkey_len CHECK (length(pubkey) = 32)
);

--
-- Name: idx_users_nip05; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS idx_users_nip05 ON users (community_id, lower(nip05_handle::text)) WHERE (nip05_handle IS NOT NULL);

--
-- Name: idx_users_okta; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS idx_users_okta ON users (community_id, okta_user_id) WHERE (okta_user_id IS NOT NULL);

--
-- Name: api_tokens; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS api_tokens (
    community_id uuid,
    id uuid DEFAULT gen_random_uuid(),
    token_hash bytea NOT NULL,
    owner_pubkey bytea NOT NULL,
    name varchar(255) NOT NULL,
    scopes jsonb NOT NULL,
    channel_ids jsonb,
    created_at timestamptz DEFAULT now() NOT NULL,
    expires_at timestamptz,
    last_used_at timestamptz,
    revoked_at timestamptz,
    revoked_by bytea,
    created_by_self_mint boolean DEFAULT false NOT NULL,
    CONSTRAINT api_tokens_pkey PRIMARY KEY (community_id, id),
    CONSTRAINT api_tokens_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT api_tokens_community_id_owner_pubkey_fkey FOREIGN KEY (community_id, owner_pubkey) REFERENCES users (community_id, pubkey),
    CONSTRAINT chk_api_tokens_hash_len CHECK (length(token_hash) = 32)
);

--
-- Name: idx_api_tokens_hash; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS idx_api_tokens_hash ON api_tokens (community_id, token_hash);

--
-- Name: subscriptions; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS subscriptions (
    community_id uuid,
    id varchar(255),
    owner_pubkey bytea NOT NULL,
    filter_kinds jsonb,
    filter_authors jsonb,
    filter_channel_ids jsonb,
    filter_since timestamptz,
    filter_until timestamptz,
    delivery_method delivery_method DEFAULT 'webhook'::delivery_method NOT NULL,
    delivery_url text,
    status subscription_status DEFAULT 'active'::subscription_status NOT NULL,
    pause_reason pause_reason,
    delivered_count bigint DEFAULT 0 NOT NULL,
    error_count bigint DEFAULT 0 NOT NULL,
    created_at timestamptz DEFAULT now() NOT NULL,
    updated_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT subscriptions_pkey PRIMARY KEY (community_id, id),
    CONSTRAINT subscriptions_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT subscriptions_community_id_owner_pubkey_fkey FOREIGN KEY (community_id, owner_pubkey) REFERENCES users (community_id, pubkey)
);

--
-- Name: workflows; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS workflows (
    community_id uuid,
    id uuid DEFAULT gen_random_uuid(),
    name varchar(255) NOT NULL,
    owner_pubkey bytea NOT NULL,
    channel_id uuid,
    definition jsonb NOT NULL,
    definition_hash bytea NOT NULL,
    status workflow_status DEFAULT 'active'::workflow_status NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    created_at timestamptz DEFAULT now() NOT NULL,
    updated_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT workflows_pkey PRIMARY KEY (community_id, id),
    CONSTRAINT workflows_community_id_channel_id_fkey FOREIGN KEY (community_id, channel_id) REFERENCES channels (community_id, id),
    CONSTRAINT workflows_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT workflows_community_id_owner_pubkey_fkey FOREIGN KEY (community_id, owner_pubkey) REFERENCES users (community_id, pubkey)
);

--
-- Name: idx_workflows_channel_active; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_workflows_channel_active ON workflows (community_id, channel_id, status, enabled);

--
-- Name: idx_workflows_enabled; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_workflows_enabled ON workflows (enabled, status) WHERE (enabled);

--
-- Name: workflow_runs; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS workflow_runs (
    community_id uuid,
    id uuid DEFAULT gen_random_uuid(),
    workflow_id uuid NOT NULL,
    status run_status DEFAULT 'pending'::run_status NOT NULL,
    trigger_event_id bytea,
    current_step integer DEFAULT 0 NOT NULL,
    execution_trace jsonb DEFAULT '[]' NOT NULL,
    trigger_context jsonb,
    started_at timestamptz,
    completed_at timestamptz,
    error_message text,
    created_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT workflow_runs_pkey PRIMARY KEY (community_id, id),
    CONSTRAINT workflow_runs_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT workflow_runs_community_id_workflow_id_fkey FOREIGN KEY (community_id, workflow_id) REFERENCES workflows (community_id, id) ON DELETE CASCADE
);

--
-- Name: idx_workflow_runs_status; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_workflow_runs_status ON workflow_runs (community_id, status);

--
-- Name: idx_workflow_runs_workflow; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_workflow_runs_workflow ON workflow_runs (community_id, workflow_id);

--
-- Name: scheduled_workflow_fires; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS scheduled_workflow_fires (
    community_id uuid,
    workflow_id uuid,
    scheduled_for timestamptz,
    claimed_at timestamptz DEFAULT now() NOT NULL,
    workflow_run_id uuid,
    CONSTRAINT scheduled_workflow_fires_pkey PRIMARY KEY (community_id, workflow_id, scheduled_for),
    CONSTRAINT scheduled_workflow_fires_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT scheduled_workflow_fires_community_id_workflow_id_fkey FOREIGN KEY (community_id, workflow_id) REFERENCES workflows (community_id, id) ON DELETE CASCADE,
    CONSTRAINT scheduled_workflow_fires_community_id_workflow_run_id_fkey FOREIGN KEY (community_id, workflow_run_id) REFERENCES workflow_runs (community_id, id)
);

--
-- Name: idx_scheduled_fires_claimed_at; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_scheduled_fires_claimed_at ON scheduled_workflow_fires (claimed_at);

--
-- Name: workflow_approvals; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS workflow_approvals (
    community_id uuid,
    token bytea,
    workflow_id uuid NOT NULL,
    run_id uuid NOT NULL,
    step_id varchar(64) NOT NULL,
    step_index integer NOT NULL,
    approver_spec text NOT NULL,
    status approval_status DEFAULT 'pending'::approval_status NOT NULL,
    approver_pubkey bytea,
    note text,
    granted_at timestamptz,
    denied_at timestamptz,
    expires_at timestamptz NOT NULL,
    created_at timestamptz DEFAULT now() NOT NULL,
    CONSTRAINT workflow_approvals_pkey PRIMARY KEY (community_id, token),
    CONSTRAINT workflow_approvals_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT workflow_approvals_community_id_run_id_fkey FOREIGN KEY (community_id, run_id) REFERENCES workflow_runs (community_id, id) ON DELETE CASCADE,
    CONSTRAINT workflow_approvals_community_id_workflow_id_fkey FOREIGN KEY (community_id, workflow_id) REFERENCES workflows (community_id, id) ON DELETE CASCADE
);

--
-- Name: idx_workflow_approvals_run; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_workflow_approvals_run ON workflow_approvals (community_id, run_id);

--
-- Name: idx_workflow_approvals_status; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_workflow_approvals_status ON workflow_approvals (community_id, status);

--
-- Name: idx_workflow_approvals_workflow; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS idx_workflow_approvals_workflow ON workflow_approvals (community_id, workflow_id);

--
-- Name: identity_bindings; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS identity_bindings (
    community_id uuid,
    binding_id uuid,
    binding_version bigint GENERATED ALWAYS AS IDENTITY,
    issuer text NOT NULL,
    subject text NOT NULL,
    principal_fingerprint bytea NOT NULL,
    event_author_pubkey bytea NOT NULL,
    binding_state smallint NOT NULL,
    lifecycle_revision bigint NOT NULL,
    binding_provenance smallint NOT NULL,
    policy_revision bigint NOT NULL,
    enrollment_evidence_digest bytea NOT NULL,
    expires_at timestamptz,
    birth_history_id uuid NOT NULL,
    creation_operation_id uuid NOT NULL,
    creation_request_fingerprint bytea NOT NULL,
    retirement_history_id uuid,
    created_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    updated_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    CONSTRAINT identity_bindings_pkey PRIMARY KEY (community_id, binding_id),
    CONSTRAINT identity_bindings_community_id_binding_id_binding_version_key UNIQUE (community_id, binding_id, binding_version),
    CONSTRAINT identity_bindings_community_id_binding_version_key UNIQUE (community_id, binding_version),
    CONSTRAINT identity_bindings_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT identity_bindings_community_id_policy_revision_binding_pro_fkey FOREIGN KEY (community_id, policy_revision, binding_provenance) REFERENCES identity_enrollment_policies (community_id, policy_revision, enrollment_mode),
    CONSTRAINT identity_bindings_binding_id_check CHECK (binding_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CONSTRAINT identity_bindings_binding_provenance_check CHECK (binding_provenance IN (1, 2, 3)),
    CONSTRAINT identity_bindings_binding_state_check CHECK (binding_state IN (1, 2)),
    CONSTRAINT identity_bindings_binding_version_check CHECK (binding_version > 0),
    CONSTRAINT identity_bindings_birth_history_id_check CHECK (birth_history_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CONSTRAINT identity_bindings_check CHECK (expires_at IS NULL OR created_at < expires_at),
    CONSTRAINT identity_bindings_creation_operation_id_check CHECK (creation_operation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CONSTRAINT identity_bindings_creation_request_fingerprint_check CHECK (octet_length(creation_request_fingerprint) = 32),
    CONSTRAINT identity_bindings_enrollment_evidence_digest_check CHECK (octet_length(enrollment_evidence_digest) = 32),
    CONSTRAINT identity_bindings_event_author_pubkey_check CHECK (octet_length(event_author_pubkey) = 32),
    CONSTRAINT identity_bindings_issuer_check CHECK (octet_length(issuer) >= 1 AND octet_length(issuer) <= 2048),
    CONSTRAINT identity_bindings_lifecycle_revision_check CHECK (lifecycle_revision IN (1::bigint, 2::bigint)),
    CONSTRAINT identity_bindings_policy_revision_check CHECK (policy_revision > 0),
    CONSTRAINT identity_bindings_principal_fingerprint_check CHECK (octet_length(principal_fingerprint) = 32),
    CONSTRAINT identity_bindings_subject_check CHECK (octet_length(subject) >= 1 AND octet_length(subject) <= 2048)
);

--
-- Name: identity_bindings_active_event_author; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS identity_bindings_active_event_author ON identity_bindings (community_id, event_author_pubkey) WHERE (binding_state = 1);

--
-- Name: identity_bindings_active_principal; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS identity_bindings_active_principal ON identity_bindings (community_id, issuer, subject) WHERE (binding_state = 1);

--
-- Name: identity_bindings_current_lookup; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS identity_bindings_current_lookup ON identity_bindings (community_id, event_author_pubkey, binding_state, expires_at);

--
-- Name: identity_bindings_principal_fingerprint_lookup; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS identity_bindings_principal_fingerprint_lookup ON identity_bindings (community_id, principal_fingerprint) WHERE (binding_state = 1);

--
-- Name: identity_lifecycle_history; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS identity_lifecycle_history (
    community_id uuid,
    history_id uuid,
    transition_kind smallint NOT NULL,
    outcome_code smallint NOT NULL,
    old_binding_id uuid,
    old_binding_version bigint,
    old_prior_lifecycle_revision bigint,
    old_prior_state smallint,
    old_resulting_lifecycle_revision bigint,
    old_resulting_state smallint,
    successor_binding_id uuid,
    successor_binding_version bigint,
    successor_lifecycle_revision bigint,
    successor_state smallint,
    operation_id uuid NOT NULL,
    request_fingerprint bytea NOT NULL,
    transition_digest bytea NOT NULL,
    recorded_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    CONSTRAINT identity_lifecycle_history_pkey PRIMARY KEY (community_id, history_id),
    CONSTRAINT identity_lifecycle_history_community_id_history_id_old_bind_key UNIQUE (community_id, history_id, old_binding_id, old_binding_version, old_resulting_lifecycle_revision, old_resulting_state),
    CONSTRAINT identity_lifecycle_history_community_id_history_id_operatio_key UNIQUE (community_id, history_id, operation_id, request_fingerprint),
    CONSTRAINT identity_lifecycle_history_community_id_history_id_successo_key UNIQUE (community_id, history_id, successor_binding_id, successor_binding_version, operation_id, request_fingerprint),
    CONSTRAINT identity_lifecycle_history_community_id_operation_id_key UNIQUE (community_id, operation_id),
    CONSTRAINT identity_lifecycle_history_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT identity_lifecycle_history_community_id_old_binding_id_old_fkey FOREIGN KEY (community_id, old_binding_id, old_binding_version) REFERENCES identity_bindings (community_id, binding_id, binding_version) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT identity_lifecycle_history_community_id_operation_id_reque_fkey FOREIGN KEY (community_id, operation_id, request_fingerprint, transition_kind, outcome_code) REFERENCES authorization_operation_receipts (community_id, operation_id, request_fingerprint, operation_kind, outcome_code) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT identity_lifecycle_history_community_id_successor_binding__fkey FOREIGN KEY (community_id, successor_binding_id, successor_binding_version) REFERENCES identity_bindings (community_id, binding_id, binding_version) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT identity_lifecycle_history_check2 CHECK (old_binding_id IS NULL OR successor_binding_id IS NULL OR old_binding_id <> successor_binding_id),
    CONSTRAINT identity_lifecycle_history_check3 CHECK (old_binding_version IS NULL OR successor_binding_version IS NULL OR old_binding_version <> successor_binding_version),
    CONSTRAINT identity_lifecycle_history_check4 CHECK (old_binding_id IS NULL OR old_prior_lifecycle_revision = 1 AND old_prior_state = 1 AND old_resulting_lifecycle_revision = 2 AND old_resulting_state = 2 OR (transition_kind IN (7, 8)) AND old_prior_lifecycle_revision = 2 AND old_prior_state = 2 AND old_resulting_lifecycle_revision = 2 AND old_resulting_state = 2),
    CONSTRAINT identity_lifecycle_history_history_id_check CHECK (history_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CONSTRAINT identity_lifecycle_history_old_binding_version_check CHECK (old_binding_version IS NULL OR old_binding_version > 0),
    CONSTRAINT identity_lifecycle_history_old_prior_lifecycle_revision_check CHECK (old_prior_lifecycle_revision IS NULL OR (old_prior_lifecycle_revision IN (1::bigint, 2::bigint))),
    CONSTRAINT identity_lifecycle_history_old_prior_state_check CHECK (old_prior_state IS NULL OR (old_prior_state IN (1, 2))),
    CONSTRAINT identity_lifecycle_history_old_resulting_lifecycle_revisi_check CHECK (old_resulting_lifecycle_revision IS NULL OR (old_resulting_lifecycle_revision IN (1::bigint, 2::bigint))),
    CONSTRAINT identity_lifecycle_history_old_resulting_state_check CHECK (old_resulting_state IS NULL OR (old_resulting_state IN (1, 2))),
    CONSTRAINT identity_lifecycle_history_operation_id_check CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CONSTRAINT identity_lifecycle_history_outcome_code_check CHECK (outcome_code IN (1, 3)),
    CONSTRAINT identity_lifecycle_history_request_fingerprint_check CHECK (octet_length(request_fingerprint) = 32),
    CONSTRAINT identity_lifecycle_history_successor_binding_version_check CHECK (successor_binding_version IS NULL OR successor_binding_version > 0),
    CONSTRAINT identity_lifecycle_history_successor_lifecycle_revision_check CHECK (successor_lifecycle_revision IS NULL OR successor_lifecycle_revision = 1),
    CONSTRAINT identity_lifecycle_history_successor_state_check CHECK (successor_state IS NULL OR successor_state = 1),
    CONSTRAINT identity_lifecycle_history_transition_digest_check CHECK (octet_length(transition_digest) = 32),
    CONSTRAINT identity_lifecycle_history_transition_kind_check CHECK (transition_kind IN (1, 2, 3, 4, 5, 6, 7, 8, 9))
);

--
-- Name: identity_lifecycle_history_old_binding; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS identity_lifecycle_history_old_binding ON identity_lifecycle_history (community_id, old_binding_id, old_binding_version, recorded_at);

--
-- Name: identity_lifecycle_history_successor_binding; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS identity_lifecycle_history_successor_binding ON identity_lifecycle_history (community_id, successor_binding_id, successor_binding_version, recorded_at);

--
-- Name: identity_lifecycle_selectors; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS identity_lifecycle_selectors (
    community_id uuid,
    selector_id uuid,
    selector_kind smallint NOT NULL,
    selector_fingerprint bytea NOT NULL,
    fact_generation bigint NOT NULL,
    principal_fingerprint bytea,
    event_author_pubkey bytea,
    binding_id uuid,
    binding_version bigint,
    asserted_history_id uuid NOT NULL,
    selected_by_operation_id uuid NOT NULL,
    selected_by_request_fingerprint bytea NOT NULL,
    selected_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    CONSTRAINT identity_lifecycle_selectors_pkey PRIMARY KEY (community_id, selector_id),
    CONSTRAINT identity_lifecycle_selectors_community_id_selector_id_selec_key UNIQUE (community_id, selector_id, selector_kind),
    CONSTRAINT identity_lifecycle_selectors_community_id_selector_kind_sel_key UNIQUE (community_id, selector_kind, selector_fingerprint, fact_generation),
    CONSTRAINT identity_lifecycle_selectors_community_id_asserted_history_fkey FOREIGN KEY (community_id, asserted_history_id, selected_by_operation_id, selected_by_request_fingerprint) REFERENCES identity_lifecycle_history (community_id, history_id, operation_id, request_fingerprint) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT identity_lifecycle_selectors_community_id_binding_id_bindi_fkey FOREIGN KEY (community_id, binding_id, binding_version) REFERENCES identity_bindings (community_id, binding_id, binding_version) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT identity_lifecycle_selectors_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT identity_lifecycle_selectors_community_id_selected_by_oper_fkey FOREIGN KEY (community_id, selected_by_operation_id, selected_by_request_fingerprint) REFERENCES authorization_operation_receipts (community_id, operation_id, request_fingerprint) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT identity_lifecycle_selectors_binding_version_check CHECK (binding_version IS NULL OR binding_version > 0),
    CONSTRAINT identity_lifecycle_selectors_event_author_pubkey_check CHECK (event_author_pubkey IS NULL OR octet_length(event_author_pubkey) = 32),
    CONSTRAINT identity_lifecycle_selectors_fact_generation_check CHECK (fact_generation > 0),
    CONSTRAINT identity_lifecycle_selectors_principal_fingerprint_check CHECK (principal_fingerprint IS NULL OR octet_length(principal_fingerprint) = 32),
    CONSTRAINT identity_lifecycle_selectors_selected_by_request_fingerpr_check CHECK (octet_length(selected_by_request_fingerprint) = 32),
    CONSTRAINT identity_lifecycle_selectors_selector_fingerprint_check CHECK (octet_length(selector_fingerprint) = 32),
    CONSTRAINT identity_lifecycle_selectors_selector_id_check CHECK (selector_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CONSTRAINT identity_lifecycle_selectors_selector_kind_check CHECK (selector_kind IN (1, 2, 3, 4))
);

--
-- Name: identity_lifecycle_selectors_asserted_history; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS identity_lifecycle_selectors_asserted_history ON identity_lifecycle_selectors (community_id, asserted_history_id, selector_kind);

--
-- Name: identity_lifecycle_selectors_binding_lookup; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS identity_lifecycle_selectors_binding_lookup ON identity_lifecycle_selectors (community_id, selector_kind, binding_id, binding_version, fact_generation);

--
-- Name: identity_lifecycle_selectors_fact_generation; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS identity_lifecycle_selectors_fact_generation ON identity_lifecycle_selectors (community_id, selector_kind, principal_fingerprint, fact_generation) WHERE selector_kind IN (2, 4);

--
-- Name: identity_lifecycle_selectors_key_lookup; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS identity_lifecycle_selectors_key_lookup ON identity_lifecycle_selectors (community_id, selector_kind, event_author_pubkey, fact_generation);

--
-- Name: identity_lifecycle_selectors_permanent_key; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS identity_lifecycle_selectors_permanent_key ON identity_lifecycle_selectors (community_id, event_author_pubkey) WHERE (selector_kind = 3);

--
-- Name: identity_lifecycle_selectors_permanent_pair; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS identity_lifecycle_selectors_permanent_pair ON identity_lifecycle_selectors (community_id, binding_id, binding_version) WHERE (selector_kind = 1);

--
-- Name: identity_lifecycle_selectors_permanent_principal_key; Type: INDEX; Schema: -; Owner: -
--

CREATE UNIQUE INDEX IF NOT EXISTS identity_lifecycle_selectors_permanent_principal_key ON identity_lifecycle_selectors (community_id, principal_fingerprint, event_author_pubkey) WHERE (selector_kind = 1);

--
-- Name: identity_lifecycle_selectors_principal_lookup; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS identity_lifecycle_selectors_principal_lookup ON identity_lifecycle_selectors (community_id, selector_kind, principal_fingerprint, fact_generation);

--
-- Name: identity_lifecycle_selector_consumptions; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS identity_lifecycle_selector_consumptions (
    community_id uuid,
    selector_id uuid,
    selector_kind smallint NOT NULL,
    history_id uuid NOT NULL,
    operation_id uuid NOT NULL,
    request_fingerprint bytea NOT NULL,
    successor_binding_id uuid NOT NULL,
    successor_binding_version bigint NOT NULL,
    consumed_at timestamptz DEFAULT transaction_timestamp() NOT NULL,
    CONSTRAINT identity_lifecycle_selector_consumptions_pkey PRIMARY KEY (community_id, selector_id),
    CONSTRAINT identity_lifecycle_selector_c_community_id_history_id_succ_fkey FOREIGN KEY (community_id, history_id, successor_binding_id, successor_binding_version, operation_id, request_fingerprint) REFERENCES identity_lifecycle_history (community_id, history_id, successor_binding_id, successor_binding_version, operation_id, request_fingerprint) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT identity_lifecycle_selector_c_community_id_operation_id_re_fkey FOREIGN KEY (community_id, operation_id, request_fingerprint) REFERENCES authorization_operation_receipts (community_id, operation_id, request_fingerprint) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT identity_lifecycle_selector_c_community_id_selector_id_sel_fkey FOREIGN KEY (community_id, selector_id, selector_kind) REFERENCES identity_lifecycle_selectors (community_id, selector_id, selector_kind) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT identity_lifecycle_selector_consumptions_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT identity_lifecycle_selector_con_successor_binding_version_check CHECK (successor_binding_version > 0),
    CONSTRAINT identity_lifecycle_selector_consumpti_request_fingerprint_check CHECK (octet_length(request_fingerprint) = 32),
    CONSTRAINT identity_lifecycle_selector_consumptions_selector_kind_check CHECK (selector_kind IN (2, 4))
);

--
-- Name: identity_lifecycle_consumptions_history; Type: INDEX; Schema: -; Owner: -
--

CREATE INDEX IF NOT EXISTS identity_lifecycle_consumptions_history ON identity_lifecycle_selector_consumptions (community_id, history_id, selector_kind);

--
-- Name: protected_object_authority; Type: TABLE; Schema: -; Owner: -
--

CREATE TABLE IF NOT EXISTS protected_object_authority (
    community_id uuid,
    object_kind smallint,
    object_key bytea,
    capability smallint NOT NULL,
    actor_pubkey bytea NOT NULL,
    owner_pubkey bytea,
    binding_id uuid NOT NULL,
    binding_version bigint NOT NULL,
    delegated_relationship_id uuid,
    delegated_relationship_revision bigint,
    delegation_conditions_fingerprint bytea,
    policy_revision bigint NOT NULL,
    invalidation_generation bigint NOT NULL,
    authority_epoch bigint NOT NULL,
    fence bytea NOT NULL,
    issued_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    operation_id uuid NOT NULL,
    request_fingerprint bytea NOT NULL,
    CONSTRAINT protected_object_authority_pkey PRIMARY KEY (community_id, object_kind, object_key),
    CONSTRAINT protected_object_authority_community_id_binding_id_binding_fkey FOREIGN KEY (community_id, binding_id, binding_version) REFERENCES identity_bindings (community_id, binding_id, binding_version) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT protected_object_authority_community_id_fkey FOREIGN KEY (community_id) REFERENCES communities (id),
    CONSTRAINT protected_object_authority_community_id_object_kind_object_fkey FOREIGN KEY (community_id, object_kind, object_key, authority_epoch, fence, operation_id, request_fingerprint) REFERENCES authorization_authority_epochs (community_id, object_kind, object_key, authority_epoch, fence, operation_id, request_fingerprint) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT protected_object_authority_community_id_operation_id_reque_fkey FOREIGN KEY (community_id, operation_id, request_fingerprint) REFERENCES authorization_operation_receipts (community_id, operation_id, request_fingerprint) DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT protected_object_authority_actor_pubkey_check CHECK (octet_length(actor_pubkey) = 32),
    CONSTRAINT protected_object_authority_authority_epoch_check CHECK (authority_epoch > 0),
    CONSTRAINT protected_object_authority_binding_version_check CHECK (binding_version > 0),
    CONSTRAINT protected_object_authority_capability_check CHECK (capability IN (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29)),
    CONSTRAINT protected_object_authority_check CHECK (issued_at < expires_at),
    CONSTRAINT protected_object_authority_delegated_relationship_non_nil CHECK (delegated_relationship_id IS NULL OR delegated_relationship_id <> '00000000-0000-0000-0000-000000000000'::uuid),
    CONSTRAINT protected_object_authority_delegated_relationship_revisio_check CHECK (delegated_relationship_revision IS NULL OR delegated_relationship_revision > 0),
    CONSTRAINT protected_object_authority_delegation_conditions_fingerpr_check CHECK (delegation_conditions_fingerprint IS NULL OR octet_length(delegation_conditions_fingerprint) = 32),
    CONSTRAINT protected_object_authority_fence_check CHECK (octet_length(fence) = 32 AND fence <> decode(repeat('00'::text, 32), 'hex'::text)),
    CONSTRAINT protected_object_authority_invalidation_generation_check CHECK (invalidation_generation >= 0),
    CONSTRAINT protected_object_authority_object_key_check CHECK (octet_length(object_key) = 32),
    CONSTRAINT protected_object_authority_object_kind_check CHECK (object_kind IN (1, 2, 3, 4, 5, 6, 9)),
    CONSTRAINT protected_object_authority_owner_pubkey_check CHECK (owner_pubkey IS NULL OR octet_length(owner_pubkey) = 32),
    CONSTRAINT protected_object_authority_policy_revision_check CHECK (policy_revision > 0),
    CONSTRAINT protected_object_authority_request_fingerprint_check CHECK (octet_length(request_fingerprint) = 32)
);

--
-- Name: authorization_authority_epoch_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION authorization_authority_epoch_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    IF NEW IS NOT DISTINCT FROM OLD THEN
        RETURN NEW;
    END IF;
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.object_kind IS DISTINCT FROM OLD.object_kind
        OR NEW.object_key IS DISTINCT FROM OLD.object_key
        OR NEW.authority_epoch <= OLD.authority_epoch
        OR NEW.fence IS NOT DISTINCT FROM OLD.fence
        OR NEW.operation_id IS NOT DISTINCT FROM OLD.operation_id
        OR NEW.updated_at <= OLD.updated_at
    THEN
        RAISE EXCEPTION 'authorization authority epoch cannot move backward'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

--
-- Name: authorization_denial_bucket_record_v1(uuid, smallint, smallint, smallint); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION authorization_denial_bucket_record_v1(
    selected_community_id uuid,
    selected_surface_kind smallint,
    selected_denial_class smallint,
    selected_action_kind smallint
)
RETURNS bigint
LANGUAGE plpgsql
VOLATILE
STRICT
AS $$
DECLARE
    authoritative_now TIMESTAMPTZ := clock_timestamp();
    generation BIGINT;
    selected_slot SMALLINT;
    retained_count BIGINT;
BEGIN
    IF selected_surface_kind NOT BETWEEN 1 AND 3
        OR selected_denial_class NOT BETWEEN 1 AND 8
        OR selected_action_kind NOT BETWEEN 1 AND 8
        OR (selected_surface_kind = 2 AND selected_action_kind <> 1)
        OR (selected_surface_kind = 3 AND selected_action_kind <> 2)
    THEN
        RAISE EXCEPTION 'invalid denial bucket coordinate'
            USING ERRCODE = 'check_violation';
    END IF;
    generation := floor(extract(epoch FROM authoritative_now) / 300)::BIGINT;
    selected_slot := (generation % 12)::SMALLINT;
    INSERT INTO authorization_operator_denial_buckets (
        community_id, surface_kind, denial_class, action_kind, slot,
        window_generation, window_started_at, denial_count, lifetime_count,
        last_denied_at
    ) VALUES (
        selected_community_id, selected_surface_kind, selected_denial_class,
        selected_action_kind, selected_slot, generation,
        to_timestamp(generation * 300), 1, 1, authoritative_now
    )
    ON CONFLICT (
        community_id, surface_kind, denial_class, action_kind, slot
    ) DO UPDATE SET
        window_generation = GREATEST(
            authorization_operator_denial_buckets.window_generation,
            EXCLUDED.window_generation
        ),
        window_started_at = CASE
            WHEN authorization_operator_denial_buckets.window_generation
                <= EXCLUDED.window_generation
            THEN EXCLUDED.window_started_at
            ELSE authorization_operator_denial_buckets.window_started_at
        END,
        denial_count = CASE
            WHEN authorization_operator_denial_buckets.window_generation
                = EXCLUDED.window_generation
            THEN CASE
                WHEN authorization_operator_denial_buckets.denial_count
                    = 9223372036854775807
                THEN 9223372036854775807
                ELSE authorization_operator_denial_buckets.denial_count + 1
            END
            WHEN authorization_operator_denial_buckets.window_generation
                < EXCLUDED.window_generation
            THEN 1
            WHEN authorization_operator_denial_buckets.denial_count
                = 9223372036854775807
            THEN 9223372036854775807
            ELSE authorization_operator_denial_buckets.denial_count + 1
        END,
        lifetime_count = CASE
            WHEN authorization_operator_denial_buckets.lifetime_count
                = 9223372036854775807
            THEN 9223372036854775807
            ELSE authorization_operator_denial_buckets.lifetime_count + 1
        END,
        last_denied_at = GREATEST(
            authorization_operator_denial_buckets.last_denied_at,
            EXCLUDED.last_denied_at
        )
    RETURNING denial_count INTO retained_count;
    RETURN retained_count;
END;
$$;

--
-- Name: authorization_event_capacity_before_insert_v2(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION authorization_event_capacity_before_insert_v2()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
DECLARE
    policy authorization_event_capacity%ROWTYPE;
    envelope_bytes BIGINT;
    restrictive_event BOOLEAN;
    event_limit BIGINT;
    byte_limit BIGINT;
BEGIN
    SELECT * INTO policy
    FROM authorization_event_capacity
    WHERE community_id = NEW.community_id
    FOR UPDATE;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'authorization event capacity policy missing'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_event_capacity_policy_required';
    END IF;
    IF policy.health_state <> 1 THEN
        RAISE EXCEPTION 'authorization audit is unavailable'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_event_capacity_health';
    END IF;

    envelope_bytes := octet_length(NEW.canonical_envelope);
    restrictive_event := NEW.outcome_code <> 1
        OR NEW.event_kind IN (2, 3, 4, 5, 6, 7, 8, 9, 11, 14);
    IF restrictive_event THEN
        event_limit := policy.max_events_per_domain;
        byte_limit := policy.max_bytes_per_domain;
    ELSE
        event_limit := policy.max_events_per_domain
            - policy.restrictive_reserve_events;
        byte_limit := policy.max_bytes_per_domain
            - policy.restrictive_reserve_bytes;
    END IF;

    IF envelope_bytes > policy.max_envelope_bytes
        OR policy.retained_event_count + 1 > event_limit
        OR policy.retained_envelope_bytes + envelope_bytes > byte_limit
    THEN
        RAISE EXCEPTION 'authorization event capacity exhausted'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_event_capacity_exhausted';
    END IF;

    UPDATE authorization_event_capacity
    SET retained_event_count = retained_event_count + 1,
        retained_envelope_bytes = retained_envelope_bytes + envelope_bytes,
        updated_at = GREATEST(
            transaction_timestamp(),
            authorization_event_capacity.updated_at + INTERVAL '1 microsecond'
        )
    WHERE community_id = NEW.community_id;
    RETURN NEW;
END;
$$;

--
-- Name: authorization_event_capacity_guard_v2(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION authorization_event_capacity_guard_v2()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.configured_at IS DISTINCT FROM OLD.configured_at
        OR NEW.max_events_per_domain < OLD.max_events_per_domain
        OR NEW.max_bytes_per_domain < OLD.max_bytes_per_domain
        OR NEW.max_envelope_bytes < OLD.max_envelope_bytes
        OR NEW.restrictive_reserve_events < OLD.restrictive_reserve_events
        OR NEW.restrictive_reserve_bytes < OLD.restrictive_reserve_bytes
        OR NEW.retained_event_count < OLD.retained_event_count
        OR NEW.retained_envelope_bytes < OLD.retained_envelope_bytes
        OR NEW.failure_generation < OLD.failure_generation
        OR NEW.recovery_generation < OLD.recovery_generation
        OR NEW.updated_at <= OLD.updated_at
        OR NEW.failure_generation > OLD.failure_generation + 1
        OR NEW.recovery_generation > NEW.failure_generation
        OR (OLD.health_state = 1 AND NEW.health_state = 2 AND (
            NEW.failure_generation <> OLD.failure_generation + 1
            OR NEW.recovery_generation <> OLD.recovery_generation
            OR NEW.failure_code IS NULL
            OR NEW.failure_observed_at IS NULL
            OR NEW.recovered_at IS DISTINCT FROM OLD.recovered_at
        ))
        OR (OLD.health_state = 2 AND NEW.health_state = 1 AND (
            NEW.failure_generation <> OLD.failure_generation
            OR NEW.recovery_generation <> OLD.failure_generation
            OR NEW.failure_code IS NOT NULL
            OR NEW.failure_observed_at IS NOT NULL
            OR NEW.recovered_at IS NULL
            OR NEW.recovered_at <= OLD.failure_observed_at
        ))
        OR (OLD.health_state = NEW.health_state AND (
            NEW.failure_generation <> OLD.failure_generation
            OR NEW.recovery_generation <> OLD.recovery_generation
            OR NEW.failure_code IS DISTINCT FROM OLD.failure_code
            OR NEW.failure_observed_at IS DISTINCT FROM OLD.failure_observed_at
            OR NEW.recovered_at IS DISTINCT FROM OLD.recovered_at
        ))
    THEN
        RAISE EXCEPTION 'authorization event capacity transition is not monotonic'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_event_capacity_monotonic_v2';
    END IF;
    RETURN NEW;
END;
$$;

--
-- Name: authorization_event_capacity_recover_v2(uuid, bigint); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION authorization_event_capacity_recover_v2(
    selected_community_id uuid,
    expected_failure_generation bigint
)
RETURNS boolean
LANGUAGE plpgsql
VOLATILE
STRICT
AS $$
DECLARE
    recovered BOOLEAN;
BEGIN
    UPDATE authorization_event_capacity
    SET health_state = 1,
        failure_code = NULL,
        failure_observed_at = NULL,
        recovery_generation = failure_generation,
        recovered_at = transaction_timestamp(),
        updated_at = GREATEST(
            transaction_timestamp(),
            authorization_event_capacity.updated_at + INTERVAL '1 microsecond'
        )
    WHERE community_id = selected_community_id
      AND health_state = 2
      AND failure_generation = expected_failure_generation;
    recovered := FOUND;
    RETURN recovered;
END;
$$;

--
-- Name: authorization_event_capacity_report_failure_v2(uuid, smallint); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION authorization_event_capacity_report_failure_v2(
    selected_community_id uuid,
    selected_failure_code smallint
)
RETURNS bigint
LANGUAGE plpgsql
VOLATILE
STRICT
AS $$
DECLARE
    next_generation BIGINT;
BEGIN
    IF selected_failure_code NOT IN (1, 2, 3) THEN
        RAISE EXCEPTION 'invalid authorization audit failure code'
            USING ERRCODE = 'check_violation';
    END IF;
    UPDATE authorization_event_capacity
    SET health_state = 2,
        failure_code = selected_failure_code,
        failure_observed_at = transaction_timestamp(),
        failure_generation = failure_generation + 1,
        updated_at = GREATEST(
            transaction_timestamp(),
            authorization_event_capacity.updated_at + INTERVAL '1 microsecond'
        )
    WHERE community_id = selected_community_id AND health_state = 1
    RETURNING failure_generation INTO next_generation;
    IF next_generation IS NULL THEN
        SELECT failure_generation INTO next_generation
        FROM authorization_event_capacity
        WHERE community_id = selected_community_id AND health_state = 2;
    END IF;
    IF next_generation IS NULL THEN
        RAISE EXCEPTION 'authorization event capacity policy missing'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_event_capacity_policy_required';
    END IF;
    RETURN next_generation;
END;
$$;

--
-- Name: authorization_event_capacity_reserve_defaults_v2(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION authorization_event_capacity_reserve_defaults_v2()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    IF NEW.restrictive_reserve_events IS NULL THEN
        NEW.restrictive_reserve_events := CASE
            WHEN NEW.max_events_per_domain > 1
                THEN GREATEST(1, LEAST(64, NEW.max_events_per_domain / 8))
            ELSE 0
        END;
    END IF;
    IF NEW.restrictive_reserve_bytes IS NULL THEN
        NEW.restrictive_reserve_bytes := CASE
            WHEN NEW.max_bytes_per_domain > NEW.max_envelope_bytes
                THEN GREATEST(
                    NEW.max_envelope_bytes,
                    LEAST(262144, NEW.max_bytes_per_domain / 8)
                )
            ELSE 0
        END;
    END IF;
    RETURN NEW;
END;
$$;

--
-- Name: authorization_invalidation_domain_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION authorization_invalidation_domain_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    IF NEW IS NOT DISTINCT FROM OLD THEN
        RETURN NEW;
    END IF;
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.activated_at IS DISTINCT FROM OLD.activated_at
        OR NEW.current_generation <= OLD.current_generation
        OR NEW.updated_at <= OLD.updated_at
    THEN
        RAISE EXCEPTION 'authorization invalidation activation/generation cannot move backward'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

--
-- Name: authorization_invalidation_floor_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION authorization_invalidation_floor_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    IF NEW IS NOT DISTINCT FROM OLD THEN
        RETURN NEW;
    END IF;
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.selector_kind IS DISTINCT FROM OLD.selector_kind
        OR NEW.selector_fingerprint IS DISTINCT FROM OLD.selector_fingerprint
        OR NEW.floor_generation < OLD.floor_generation
        OR COALESCE(NEW.binding_version_floor, 0) < COALESCE(OLD.binding_version_floor, 0)
        OR COALESCE(NEW.relationship_revision_floor, 0)
            < COALESCE(OLD.relationship_revision_floor, 0)
        OR (
            NEW.floor_generation = OLD.floor_generation
            AND COALESCE(NEW.binding_version_floor, 0)
                = COALESCE(OLD.binding_version_floor, 0)
            AND COALESCE(NEW.relationship_revision_floor, 0)
                = COALESCE(OLD.relationship_revision_floor, 0)
        )
        OR NEW.operation_id IS NOT DISTINCT FROM OLD.operation_id
        OR NEW.updated_at <= OLD.updated_at
    THEN
        RAISE EXCEPTION 'authorization invalidation floor cannot move backward'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

--
-- Name: authorization_operation_receipt_event_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION authorization_operation_receipt_event_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
DECLARE
    receipt authorization_operation_receipts%ROWTYPE;
    expected_event_kind SMALLINT;
    matching_event_count BIGINT;
    expected_event_count BIGINT;
BEGIN
    IF TG_TABLE_NAME = 'authorization_operation_receipts' THEN
        receipt := NEW;
    ELSE
        SELECT * INTO receipt
        FROM authorization_operation_receipts
        WHERE community_id = NEW.community_id
          AND operation_id = NEW.operation_id;
        IF NOT FOUND THEN
            -- Credential-free pre-authentication denials intentionally have no
            -- canonical receipt. Their separate v30 FK/shape guards still run.
            RETURN NULL;
        END IF;
    END IF;

    IF receipt.operation_kind NOT BETWEEN 1 AND 9 THEN
        RETURN NULL;
    END IF;

    expected_event_kind := CASE receipt.operation_kind
        WHEN 1 THEN 1  -- enroll
        WHEN 2 THEN 1  -- provision
        WHEN 3 THEN 6  -- retire
        WHEN 4 THEN 7  -- disable
        WHEN 5 THEN 2  -- revoke
        WHEN 6 THEN 3  -- rotate
        WHEN 7 THEN 4  -- recover
        WHEN 8 THEN 5  -- enable
        WHEN 9 THEN 8  -- admission loss
    END;

    SELECT
        count(*),
        count(*) FILTER (WHERE event_kind = expected_event_kind)
    INTO matching_event_count, expected_event_count
    FROM authorization_events
    WHERE community_id = receipt.community_id
      AND operation_id = receipt.operation_id
      AND request_fingerprint = receipt.request_fingerprint;

    IF matching_event_count <> 1 OR expected_event_count <> 1 THEN
        RAISE EXCEPTION
            'lifecycle receipt requires exactly one event kind %, found % total and % expected',
            expected_event_kind, matching_event_count, expected_event_count
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_operation_receipt_event_cardinality';
    END IF;
    RETURN NULL;
END;
$$;

--
-- Name: authorization_operation_receipt_history_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION authorization_operation_receipt_history_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
DECLARE
    history_count BIGINT;
    expected_count BIGINT;
BEGIN
    SELECT count(*) INTO history_count
    FROM identity_lifecycle_history history
    WHERE history.community_id = NEW.community_id
      AND history.operation_id = NEW.operation_id;

    expected_count := CASE
        WHEN NEW.operation_kind BETWEEN 1 AND 9 AND NEW.outcome_code IN (1, 3) THEN 1
        ELSE 0
    END;
    IF history_count <> expected_count THEN
        RAISE EXCEPTION 'operation receipt requires % lifecycle history row, found %',
            expected_count, history_count
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_operation_receipt_history_cardinality';
    END IF;
    RETURN NULL;
END;
$$;

--
-- Name: authorization_operation_version_delta_cardinality_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION authorization_operation_version_delta_cardinality_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
DECLARE
    manifest authorization_operation_version_delta_manifests%ROWTYPE;
    actual_component_count BIGINT;
BEGIN
    IF TG_TABLE_NAME = 'authorization_operation_version_delta_manifests' THEN
        manifest := NEW;
    ELSE
        SELECT * INTO STRICT manifest
        FROM authorization_operation_version_delta_manifests
        WHERE community_id = NEW.community_id
          AND operation_id = NEW.operation_id
        FOR NO KEY UPDATE;
    END IF;

    SELECT count(*) INTO actual_component_count
    FROM authorization_operation_version_deltas
    WHERE community_id = manifest.community_id
      AND operation_id = manifest.operation_id;

    IF actual_component_count <> manifest.component_count THEN
        RAISE EXCEPTION 'operation version manifest declares % components, found %',
            manifest.component_count, actual_component_count
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_operation_version_delta_cardinality';
    END IF;
    RETURN NULL;
END;
$$;

--
-- Name: authorization_operator_denial_bucket_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION authorization_operator_denial_bucket_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.surface_kind IS DISTINCT FROM OLD.surface_kind
        OR NEW.denial_class IS DISTINCT FROM OLD.denial_class
        OR NEW.action_kind IS DISTINCT FROM OLD.action_kind
        OR NEW.slot IS DISTINCT FROM OLD.slot
        OR NEW.window_generation < OLD.window_generation
        OR NEW.lifetime_count IS DISTINCT FROM (CASE
            WHEN OLD.lifetime_count = 9223372036854775807
            THEN 9223372036854775807
            ELSE OLD.lifetime_count + 1
        END)
        OR (NEW.window_generation = OLD.window_generation AND (
            NEW.window_started_at IS DISTINCT FROM OLD.window_started_at
            OR NEW.denial_count IS DISTINCT FROM (CASE
                WHEN OLD.denial_count = 9223372036854775807
                THEN 9223372036854775807
                ELSE OLD.denial_count + 1
            END)
            OR NEW.last_denied_at < OLD.last_denied_at
        ))
        OR (NEW.window_generation > OLD.window_generation AND (
            NEW.denial_count <> 1
            OR NEW.window_started_at <= OLD.window_started_at
        ))
    THEN
        RAISE EXCEPTION 'operator denial bucket transition is invalid'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

--
-- Name: authorization_operator_denial_bucket_record_v1(uuid, smallint, smallint); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION authorization_operator_denial_bucket_record_v1(
    selected_community_id uuid,
    selected_denial_class smallint,
    selected_action_kind smallint
)
RETURNS bigint
LANGUAGE sql
VOLATILE
STRICT
AS $$
    SELECT authorization_denial_bucket_record_v1(
        selected_community_id, 1::SMALLINT,
        selected_denial_class, selected_action_kind
    );
$$;

--
-- Name: authorization_proxy_nonce_claim_retention_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION authorization_proxy_nonce_claim_retention_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    IF transaction_timestamp() <= OLD.retain_until THEN
        RAISE EXCEPTION 'trusted-proxy nonce claim retention is still active'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_proxy_nonce_claim_retention';
    END IF;
    RETURN OLD;
END;
$$;

--
-- Name: authorization_proxy_nonce_claim_stamp_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION authorization_proxy_nonce_claim_stamp_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    NEW.committed_at := transaction_timestamp();
    IF NEW.retain_until <= NEW.committed_at THEN
        RAISE EXCEPTION 'trusted-proxy nonce claim is already expired'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'authorization_proxy_nonce_claim_expired';
    END IF;
    RETURN NEW;
END;
$$;

--
-- Name: channels_community_id_immutable(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION channels_community_id_immutable()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    IF NEW.community_id IS DISTINCT FROM OLD.community_id THEN
        RAISE EXCEPTION 'channels.community_id is immutable (channel % cannot be re-tenanted)', OLD.id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

--
-- Name: enqueue_push_match_job(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION enqueue_push_match_job()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    -- Keep this allowlist identical to the relay's validated NIP-PL descriptor.
    IF NEW.kind IN (7, 9, 1059, 40007, 46010) THEN
        PERFORM pg_advisory_xact_lock_shared(
            hashtextextended('buzz_push_gate:' || NEW.community_id::text, 0));
        IF EXISTS (
            SELECT 1 FROM push_leases
            WHERE community_id = NEW.community_id
              AND active
              AND endpoint_enabled
              AND expires_at > EXTRACT(EPOCH FROM now())::bigint
        ) THEN
            INSERT INTO push_match_queue (community_id, event_id)
            VALUES (NEW.community_id, NEW.id)
            ON CONFLICT DO NOTHING;
        END IF;
    END IF;
    RETURN NEW;
END
$$;

--
-- Name: events_created_at_floor_guard(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION events_created_at_floor_guard()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
DECLARE
    floor_secs numeric := nullif(current_setting('buzz.created_at_floor', true), '')::numeric;
BEGIN
    IF floor_secs IS NOT NULL
       AND floor_secs > 0
       AND NEW.channel_id IS NOT NULL
       AND NEW.created_at < clock_timestamp() - make_interval(secs => floor_secs)
    THEN
        RAISE EXCEPTION
            'events.created_at % is more than % s before commit time %; below the replica-fence floor',
            NEW.created_at, floor_secs, clock_timestamp()
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END
$$;

--
-- Name: guard_event_mention_live(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION guard_event_mention_live()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    IF NEW.event_kind IS DISTINCT FROM 30078 THEN
        RETURN NEW;
    END IF;

    PERFORM 1
    FROM events
    WHERE community_id = NEW.community_id
      AND id = NEW.event_id
      AND created_at = NEW.event_created_at
      AND deleted_at IS NULL
    FOR KEY SHARE;

    IF NOT FOUND THEN
        RETURN NULL;
    END IF;

    RETURN NEW;
END;
$$;

--
-- Name: guard_nip_rs_hard_delete(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION guard_nip_rs_hard_delete()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    IF current_setting('buzz.nip_rs_hard_delete', true) IS DISTINCT FROM 'on' THEN
        RAISE EXCEPTION 'NIP-RS hard delete requires corrected writer opt-in'
            USING ERRCODE = 'check_violation';
    END IF;

    RETURN OLD;
END;
$$;

--
-- Name: guard_nip_rs_watermark(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION guard_nip_rs_watermark()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
DECLARE
    advanced BOOLEAN;
BEGIN
    IF NEW.kind = 30078
       AND NEW.d_tag ~ '^read-state:[0-9a-f]{32}$'
       AND (
           SELECT count(*)
           FROM jsonb_array_elements(CASE WHEN jsonb_typeof(NEW.tags) = 'array' THEN NEW.tags ELSE '[]'::jsonb END) tag
           WHERE jsonb_typeof(tag) = 'array'
             AND tag->0 = '"d"'::jsonb
       ) = 1
       AND EXISTS (
           SELECT 1
           FROM jsonb_array_elements(CASE WHEN jsonb_typeof(NEW.tags) = 'array' THEN NEW.tags ELSE '[]'::jsonb END) tag
           WHERE jsonb_typeof(tag) = 'array'
             AND jsonb_array_length(tag) >= 2
             AND jsonb_typeof(tag->1) = 'string'
             AND tag->>0 = 'd'
             AND tag->>1 = NEW.d_tag
       )
       AND (
           SELECT count(*)
           FROM jsonb_array_elements(CASE WHEN jsonb_typeof(NEW.tags) = 'array' THEN NEW.tags ELSE '[]'::jsonb END) tag
           WHERE tag = '["t", "read-state"]'::jsonb
       ) = 1 THEN
        INSERT INTO parameterized_event_watermarks
            (community_id, kind, pubkey, d_tag, created_at, event_id)
        VALUES
            (NEW.community_id, NEW.kind, NEW.pubkey, NEW.d_tag, NEW.created_at, NEW.id)
        ON CONFLICT (community_id, kind, pubkey, d_tag) DO UPDATE SET
            created_at = EXCLUDED.created_at,
            event_id = EXCLUDED.event_id
        WHERE EXCLUDED.created_at > parameterized_event_watermarks.created_at
           OR (EXCLUDED.created_at = parameterized_event_watermarks.created_at
               AND EXCLUDED.event_id < parameterized_event_watermarks.event_id)
        RETURNING TRUE INTO advanced;

        IF NOT COALESCE(advanced, FALSE) THEN
            IF EXISTS (
                SELECT 1
                FROM parameterized_event_watermarks
                WHERE community_id = NEW.community_id
                  AND kind = NEW.kind
                  AND pubkey = NEW.pubkey
                  AND d_tag = NEW.d_tag
                  AND created_at = NEW.created_at
                  AND event_id = NEW.id
            ) THEN
                RETURN NULL;
            END IF;

            RAISE EXCEPTION 'stale NIP-RS event rejected by durable watermark'
                USING ERRCODE = 'check_violation';
        END IF;
    END IF;

    RETURN NEW;
END;
$$;

--
-- Name: identity_binding_birth_eligibility_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION identity_binding_birth_eligibility_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM identity_lifecycle_selectors selector
        LEFT JOIN identity_lifecycle_selector_consumptions consumption
          ON consumption.community_id = selector.community_id
         AND consumption.selector_id = selector.selector_id
        WHERE selector.community_id = NEW.community_id
          AND (
            (selector.selector_kind = 1
                AND selector.principal_fingerprint = NEW.principal_fingerprint
                AND selector.event_author_pubkey = NEW.event_author_pubkey)
            OR (selector.selector_kind = 3
                AND selector.event_author_pubkey = NEW.event_author_pubkey)
            OR (selector.selector_kind IN (2, 4)
                AND selector.principal_fingerprint = NEW.principal_fingerprint
                AND consumption.selector_id IS NULL)
          )
    ) THEN
        RAISE EXCEPTION 'binding birth conflicts with an effective lifecycle selector'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_bindings_birth_eligibility';
    END IF;
    RETURN NULL;
END;
$$;

--
-- Name: identity_binding_history_semantics_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION identity_binding_history_semantics_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
DECLARE
    retirement identity_lifecycle_history%ROWTYPE;
BEGIN
    IF NEW.binding_state = 2 THEN
        SELECT * INTO STRICT retirement
        FROM identity_lifecycle_history
        WHERE community_id = NEW.community_id
          AND history_id = NEW.retirement_history_id
          AND old_binding_id = NEW.binding_id
          AND old_binding_version = NEW.binding_version;
        IF retirement.outcome_code <> 1
            OR retirement.old_prior_lifecycle_revision <> 1
            OR retirement.old_prior_state <> 1
            OR retirement.old_resulting_lifecycle_revision <> 2
            OR retirement.old_resulting_state <> 2
        THEN
            RAISE EXCEPTION 'retired binding must reference its exact Active-to-Retired transition'
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'identity_bindings_retirement_history_semantics';
        END IF;
    END IF;
    RETURN NULL;
END;
$$;

--
-- Name: identity_lifecycle_consumption_history_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION identity_lifecycle_consumption_history_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
DECLARE
    selector identity_lifecycle_selectors%ROWTYPE;
    successor identity_bindings%ROWTYPE;
    history identity_lifecycle_history%ROWTYPE;
BEGIN
    SELECT * INTO STRICT selector
    FROM identity_lifecycle_selectors
    WHERE community_id = NEW.community_id
      AND selector_id = NEW.selector_id
      AND selector_kind = NEW.selector_kind;
    SELECT * INTO STRICT successor
    FROM identity_bindings
    WHERE community_id = NEW.community_id
      AND binding_id = NEW.successor_binding_id
      AND binding_version = NEW.successor_binding_version;
    SELECT * INTO STRICT history
    FROM identity_lifecycle_history
    WHERE community_id = NEW.community_id
      AND history_id = NEW.history_id
      AND operation_id = NEW.operation_id
      AND request_fingerprint = NEW.request_fingerprint
      AND successor_binding_id = NEW.successor_binding_id
      AND successor_binding_version = NEW.successor_binding_version;

    IF successor.principal_fingerprint IS DISTINCT FROM selector.principal_fingerprint
        OR (NEW.selector_kind = 2 AND history.transition_kind <> 8)
        OR (NEW.selector_kind = 4 AND history.transition_kind NOT IN (7, 8))
        OR (NEW.selector_kind = 4 AND (
            history.old_binding_id IS DISTINCT FROM selector.binding_id
            OR history.old_binding_version IS DISTINCT FROM selector.binding_version
        ))
    THEN
        RAISE EXCEPTION 'selector consumption does not match its successor transition'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_lifecycle_consumption_history_semantics';
    END IF;
    RETURN NULL;
END;
$$;

--
-- Name: identity_lifecycle_history_insert_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION identity_lifecycle_history_insert_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    NEW.recorded_at := transaction_timestamp();
    RETURN NEW;
END;
$$;

--
-- Name: identity_lifecycle_lock_coordinates_v1(uuid, bytea, bytea); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION identity_lifecycle_lock_coordinates_v1(
    locked_community_id uuid,
    locked_principal_fingerprint bytea,
    locked_event_author_pubkey bytea
)
RETURNS void
LANGUAGE plpgsql
VOLATILE
AS $$
DECLARE
    principal_lock_key BIGINT;
    event_author_lock_key BIGINT;
BEGIN
    IF locked_principal_fingerprint IS NOT NULL THEN
        principal_lock_key := hashtextextended(
            'buzz:identity-lifecycle-coordinate:v1:principal:'
                || locked_community_id::text || ':'
                || encode(locked_principal_fingerprint, 'hex'),
            0
        );
    END IF;
    IF locked_event_author_pubkey IS NOT NULL THEN
        event_author_lock_key := hashtextextended(
            'buzz:identity-lifecycle-coordinate:v1:key:'
                || locked_community_id::text || ':'
                || encode(locked_event_author_pubkey, 'hex'),
            0
        );
    END IF;

    IF principal_lock_key IS NOT NULL AND event_author_lock_key IS NOT NULL THEN
        PERFORM pg_advisory_xact_lock(LEAST(principal_lock_key, event_author_lock_key));
        IF principal_lock_key <> event_author_lock_key THEN
            PERFORM pg_advisory_xact_lock(GREATEST(principal_lock_key, event_author_lock_key));
        END IF;
    ELSIF principal_lock_key IS NOT NULL THEN
        PERFORM pg_advisory_xact_lock(principal_lock_key);
    ELSIF event_author_lock_key IS NOT NULL THEN
        PERFORM pg_advisory_xact_lock(event_author_lock_key);
    END IF;
END;
$$;

--
-- Name: identity_bindings_insert_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION identity_bindings_insert_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    PERFORM identity_lifecycle_lock_coordinates_v1(
        NEW.community_id,
        NEW.principal_fingerprint,
        NEW.event_author_pubkey
    );
    IF NEW.binding_state <> 1
        OR NEW.lifecycle_revision <> 1
        OR NEW.retirement_history_id IS NOT NULL
    THEN
        RAISE EXCEPTION 'identity binding birth must be Active at lifecycle revision 1'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_bindings_birth_state';
    END IF;
    NEW.created_at := transaction_timestamp();
    NEW.updated_at := transaction_timestamp();
    RETURN NEW;
END;
$$;

--
-- Name: identity_bindings_transition_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION identity_bindings_transition_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    IF NEW IS NOT DISTINCT FROM OLD THEN
        RETURN NEW;
    END IF;
    PERFORM identity_lifecycle_lock_coordinates_v1(
        OLD.community_id,
        OLD.principal_fingerprint,
        OLD.event_author_pubkey
    );
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.binding_id IS DISTINCT FROM OLD.binding_id
        OR NEW.binding_version IS DISTINCT FROM OLD.binding_version
        OR NEW.issuer IS DISTINCT FROM OLD.issuer
        OR NEW.subject IS DISTINCT FROM OLD.subject
        OR NEW.principal_fingerprint IS DISTINCT FROM OLD.principal_fingerprint
        OR NEW.event_author_pubkey IS DISTINCT FROM OLD.event_author_pubkey
        OR NEW.binding_provenance IS DISTINCT FROM OLD.binding_provenance
        OR NEW.policy_revision IS DISTINCT FROM OLD.policy_revision
        OR NEW.enrollment_evidence_digest IS DISTINCT FROM OLD.enrollment_evidence_digest
        OR NEW.expires_at IS DISTINCT FROM OLD.expires_at
        OR NEW.birth_history_id IS DISTINCT FROM OLD.birth_history_id
        OR NEW.creation_operation_id IS DISTINCT FROM OLD.creation_operation_id
        OR NEW.creation_request_fingerprint IS DISTINCT FROM OLD.creation_request_fingerprint
        OR NEW.created_at IS DISTINCT FROM OLD.created_at
    THEN
        RAISE EXCEPTION 'identity binding generation coordinates are immutable'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_bindings_immutable_generation';
    END IF;
    IF OLD.binding_state <> 1
        OR OLD.lifecycle_revision <> 1
        OR OLD.retirement_history_id IS NOT NULL
        OR NEW.binding_state <> 2
        OR NEW.lifecycle_revision <> 2
        OR NEW.retirement_history_id IS NULL
        OR NEW.retirement_history_id = OLD.birth_history_id
    THEN
        RAISE EXCEPTION 'identity binding permits only Active/r1 to Retired/r2'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_bindings_active_to_retired';
    END IF;
    NEW.updated_at := transaction_timestamp();
    RETURN NEW;
END;
$$;

--
-- Name: identity_lifecycle_consumption_lock_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION identity_lifecycle_consumption_lock_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
DECLARE
    selector identity_lifecycle_selectors%ROWTYPE;
BEGIN
    SELECT * INTO STRICT selector
    FROM identity_lifecycle_selectors
    WHERE community_id = NEW.community_id
      AND selector_id = NEW.selector_id;
    PERFORM identity_lifecycle_lock_coordinates_v1(
        selector.community_id,
        selector.principal_fingerprint,
        NULL
    );
    NEW.consumed_at := transaction_timestamp();
    RETURN NEW;
END;
$$;

--
-- Name: identity_lifecycle_selector_history_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION identity_lifecycle_selector_history_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
DECLARE
    history identity_lifecycle_history%ROWTYPE;
    old_binding identity_bindings%ROWTYPE;
BEGIN
    SELECT * INTO STRICT history
    FROM identity_lifecycle_history
    WHERE community_id = NEW.community_id
      AND history_id = NEW.asserted_history_id
      AND operation_id = NEW.selected_by_operation_id
      AND request_fingerprint = NEW.selected_by_request_fingerprint;

    IF history.old_binding_id IS NOT NULL THEN
        SELECT * INTO STRICT old_binding
        FROM identity_bindings
        WHERE community_id = history.community_id
          AND binding_id = history.old_binding_id
          AND binding_version = history.old_binding_version;
    END IF;

    IF history.outcome_code <> 1
        OR (NEW.selector_kind = 1 AND (
            history.transition_kind NOT IN (3, 4, 5, 6, 9)
            OR history.old_binding_id IS DISTINCT FROM NEW.binding_id
            OR history.old_binding_version IS DISTINCT FROM NEW.binding_version
            OR old_binding.principal_fingerprint IS DISTINCT FROM NEW.principal_fingerprint
            OR old_binding.event_author_pubkey IS DISTINCT FROM NEW.event_author_pubkey
        ))
        OR (NEW.selector_kind = 2 AND (
            history.transition_kind <> 4
            OR (history.old_binding_id IS NOT NULL
                AND old_binding.principal_fingerprint
                    IS DISTINCT FROM NEW.principal_fingerprint)
        ))
        OR (NEW.selector_kind = 3 AND (
            history.transition_kind <> 5
            OR (history.old_binding_id IS NOT NULL
                AND old_binding.event_author_pubkey
                    IS DISTINCT FROM NEW.event_author_pubkey)
        ))
        OR (NEW.selector_kind = 4 AND (
            history.transition_kind NOT IN (3, 4, 5, 9)
            OR history.old_binding_id IS DISTINCT FROM NEW.binding_id
            OR history.old_binding_version IS DISTINCT FROM NEW.binding_version
            OR old_binding.principal_fingerprint IS DISTINCT FROM NEW.principal_fingerprint
            OR old_binding.event_author_pubkey IS DISTINCT FROM NEW.event_author_pubkey
        ))
    THEN
        RAISE EXCEPTION 'selector does not match its lifecycle transition'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_lifecycle_selector_history_semantics';
    END IF;
    RETURN NULL;
END;
$$;

--
-- Name: identity_lifecycle_selector_insert_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION identity_lifecycle_selector_insert_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
DECLARE
    prior_generation BIGINT;
BEGIN
    NEW.selected_at := transaction_timestamp();
    PERFORM identity_lifecycle_lock_coordinates_v1(
        NEW.community_id,
        CASE WHEN NEW.selector_kind IN (1, 2, 4) THEN NEW.principal_fingerprint END,
        CASE WHEN NEW.selector_kind IN (1, 3) THEN NEW.event_author_pubkey END
    );
    IF NEW.selector_kind NOT IN (2, 4) THEN
        RETURN NEW;
    END IF;
    SELECT max(selector.fact_generation) INTO prior_generation
    FROM identity_lifecycle_selectors selector
    WHERE selector.community_id = NEW.community_id
      AND selector.selector_kind = NEW.selector_kind
      AND selector.principal_fingerprint = NEW.principal_fingerprint;
    IF prior_generation IS NOT NULL AND NEW.fact_generation <= prior_generation THEN
        RAISE EXCEPTION 'selector fact generation must advance'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_lifecycle_selector_fact_generation';
    END IF;
    IF EXISTS (
        SELECT 1
        FROM identity_lifecycle_selectors selector
        LEFT JOIN identity_lifecycle_selector_consumptions consumption
          ON consumption.community_id = selector.community_id
         AND consumption.selector_id = selector.selector_id
        WHERE selector.community_id = NEW.community_id
          AND selector.selector_kind = NEW.selector_kind
          AND selector.principal_fingerprint = NEW.principal_fingerprint
          AND consumption.selector_id IS NULL
    ) THEN
        RAISE EXCEPTION 'an unconsumed selector fact already exists'
            USING ERRCODE = 'unique_violation',
                  CONSTRAINT = 'identity_lifecycle_selector_one_open_fact';
    END IF;
    RETURN NEW;
END;
$$;

--
-- Name: identity_lifecycle_transition_integrity_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION identity_lifecycle_transition_integrity_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
DECLARE
    transition identity_lifecycle_history%ROWTYPE;
    old_binding_state SMALLINT;
    asserted_p BIGINT;
    asserted_x BIGINT;
    asserted_y BIGINT;
    asserted_q BIGINT;
    consumed_x BIGINT;
    consumed_q BIGINT;
BEGIN
    IF TG_TABLE_NAME = 'identity_lifecycle_history' THEN
        transition := NEW;
    ELSIF TG_TABLE_NAME = 'identity_lifecycle_selectors' THEN
        SELECT * INTO STRICT transition
        FROM identity_lifecycle_history
        WHERE community_id = NEW.community_id
          AND history_id = NEW.asserted_history_id;
    ELSIF TG_TABLE_NAME = 'identity_lifecycle_selector_consumptions' THEN
        SELECT * INTO STRICT transition
        FROM identity_lifecycle_history
        WHERE community_id = NEW.community_id
          AND history_id = NEW.history_id;
    ELSE
        SELECT * INTO STRICT transition
        FROM identity_lifecycle_history
        WHERE community_id = NEW.community_id
          AND history_id = CASE
              WHEN NEW.binding_state = 2 THEN NEW.retirement_history_id
              ELSE NEW.birth_history_id
          END;
    END IF;

    SELECT
        count(*) FILTER (WHERE selector_kind = 1),
        count(*) FILTER (WHERE selector_kind = 2),
        count(*) FILTER (WHERE selector_kind = 3),
        count(*) FILTER (WHERE selector_kind = 4)
    INTO asserted_p, asserted_x, asserted_y, asserted_q
    FROM identity_lifecycle_selectors
    WHERE community_id = transition.community_id
      AND asserted_history_id = transition.history_id;

    SELECT
        count(*) FILTER (WHERE selector_kind = 2),
        count(*) FILTER (WHERE selector_kind = 4)
    INTO consumed_x, consumed_q
    FROM identity_lifecycle_selector_consumptions
    WHERE community_id = transition.community_id
      AND history_id = transition.history_id;

    IF transition.old_binding_id IS NOT NULL THEN
        SELECT binding_state INTO STRICT old_binding_state
        FROM identity_bindings
        WHERE community_id = transition.community_id
          AND binding_id = transition.old_binding_id
          AND binding_version = transition.old_binding_version;
        IF old_binding_state <> 2 THEN
            RAISE EXCEPTION 'lifecycle transition old binding must be retired at commit'
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'identity_lifecycle_transition_integrity';
        END IF;
    END IF;

    IF EXISTS (
        SELECT 1
        FROM identity_lifecycle_selectors selector
        LEFT JOIN identity_lifecycle_selector_consumptions consumption
          ON consumption.community_id = selector.community_id
         AND consumption.selector_id = selector.selector_id
        JOIN identity_bindings active
          ON active.community_id = selector.community_id
         AND active.binding_state = 1
         AND (
            (selector.selector_kind = 1
                AND active.principal_fingerprint = selector.principal_fingerprint
                AND active.event_author_pubkey = selector.event_author_pubkey)
            OR (selector.selector_kind IN (2, 4)
                AND consumption.selector_id IS NULL
                AND active.principal_fingerprint = selector.principal_fingerprint)
            OR (selector.selector_kind = 3
                AND active.event_author_pubkey = selector.event_author_pubkey)
         )
        WHERE selector.community_id = transition.community_id
    ) THEN
        RAISE EXCEPTION 'effective lifecycle selector conflicts with an active binding'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_lifecycle_transition_integrity';
    END IF;

    IF transition.outcome_code = 3 THEN
        IF asserted_p + asserted_x + asserted_y + asserted_q + consumed_x + consumed_q <> 0 THEN
            RAISE EXCEPTION 'no-op lifecycle transition cannot create or consume selector facts'
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'identity_lifecycle_transition_integrity';
        END IF;
        RETURN NULL;
    END IF;

    IF (transition.transition_kind IN (1, 2)
            AND (asserted_p, asserted_x, asserted_y, asserted_q, consumed_x, consumed_q)
                <> (0, 0, 0, 0, 0, 0))
        OR (transition.transition_kind = 3
            AND (asserted_p, asserted_x, asserted_y, asserted_q, consumed_x, consumed_q)
                <> (1, 0, 0, 1, 0, 0))
        OR (transition.transition_kind = 4 AND (
            (transition.old_binding_id IS NOT NULL
                AND (asserted_p, asserted_x, asserted_y, asserted_q, consumed_x, consumed_q)
                    <> (1, 1, 0, 1, 0, 0))
            OR (transition.old_binding_id IS NULL
                AND (asserted_p, asserted_x, asserted_y, asserted_q, consumed_x, consumed_q)
                    <> (0, 1, 0, 0, 0, 0))
        ))
        OR (transition.transition_kind = 5 AND (
            (transition.old_binding_id IS NOT NULL
                AND (asserted_p, asserted_x, asserted_y, asserted_q, consumed_x, consumed_q)
                    <> (1, 0, 1, 1, 0, 0))
            OR (transition.old_binding_id IS NULL
                AND (asserted_p, asserted_x, asserted_y, asserted_q, consumed_x, consumed_q)
                    <> (0, 0, 1, 0, 0, 0))
        ))
        OR (transition.transition_kind = 6
            AND (asserted_p, asserted_x, asserted_y, asserted_q, consumed_x, consumed_q)
                <> (1, 0, 0, 0, 0, 0))
        OR (transition.transition_kind = 7
            AND (asserted_p, asserted_x, asserted_y, asserted_q, consumed_x, consumed_q)
                <> (0, 0, 0, 0, 0, 1))
        OR (transition.transition_kind = 8 AND (
            (asserted_p, asserted_x, asserted_y, asserted_q, consumed_x)
                <> (0, 0, 0, 0, 1)
            OR consumed_q NOT IN (0, 1)
            OR (transition.old_binding_id IS NULL AND consumed_q <> 0)
            OR (transition.old_binding_id IS NOT NULL AND consumed_q <> 1)
        ))
        OR (transition.transition_kind = 9
            AND (asserted_p, asserted_x, asserted_y, asserted_q, consumed_x, consumed_q)
                <> (1, 0, 0, 1, 0, 0))
    THEN
        RAISE EXCEPTION 'lifecycle transition has incomplete or forbidden selector companions'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'identity_lifecycle_transition_integrity';
    END IF;
    RETURN NULL;
END;
$$;

--
-- Name: nip_fi_lock_authorization_operation_v1(uuid, uuid); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION nip_fi_lock_authorization_operation_v1(
    locked_community_id uuid,
    locked_operation_id uuid
)
RETURNS void
LANGUAGE plpgsql
VOLATILE
STRICT
AS $$
BEGIN
    IF locked_community_id = '00000000-0000-0000-0000-000000000000'::uuid
        OR locked_operation_id = '00000000-0000-0000-0000-000000000000'::uuid
    THEN
        RAISE EXCEPTION 'invalid NIP-FI operation lock coordinate'
            USING ERRCODE = 'check_violation';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended(
        'buzz:authorization-operation:v1:' || locked_community_id::text
            || ':' || locked_operation_id::text,
        0
    ));
END;
$$;

--
-- Name: nip_fi_reject_row_mutation_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION nip_fi_reject_row_mutation_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    RAISE EXCEPTION '% is immutable', TG_TABLE_NAME
        USING ERRCODE = 'check_violation';
END;
$$;

--
-- Name: nip_fi_reject_truncate_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION nip_fi_reject_truncate_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    RAISE EXCEPTION '% cannot be truncated', TG_TABLE_NAME
        USING ERRCODE = 'check_violation';
END;
$$;

--
-- Name: protected_object_authority_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION protected_object_authority_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    IF NEW IS NOT DISTINCT FROM OLD THEN
        RETURN NEW;
    END IF;
    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.object_kind IS DISTINCT FROM OLD.object_kind
        OR NEW.object_key IS DISTINCT FROM OLD.object_key
        OR NEW.authority_epoch <= OLD.authority_epoch
        OR NEW.fence IS NOT DISTINCT FROM OLD.fence
        OR NEW.operation_id IS NOT DISTINCT FROM OLD.operation_id
        OR NEW.issued_at <= OLD.issued_at
    THEN
        RAISE EXCEPTION 'protected authority replacement requires a new operation and epoch'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

--
-- Name: protected_publication_projection_insert_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION protected_publication_projection_insert_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    IF NEW.delivery_state <> 1
        OR NEW.attempt_count <> 0
        OR NEW.last_attempt_at IS NOT NULL
        OR NEW.delivered_at IS NOT NULL
        OR NEW.failure_code <> 0
        OR NEW.authority_operation_id IS NOT NULL
        OR NEW.authority_request_fingerprint IS NOT NULL
        OR NEW.authority_epoch IS NOT NULL
        OR NEW.authority_fence IS NOT NULL
    THEN
        RAISE EXCEPTION 'protected publication projection must start pending'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'protected_publication_projection_initial_state';
    END IF;
    -- Serialize only publications for the exact protected projection target.
    -- Sequence allocation happens after acquiring this transaction lock, so a
    -- later publication cannot become visible or deliver before an earlier
    -- allocator commits or aborts. Hash collisions only over-serialize two
    -- independent targets; they cannot weaken ordering.
    PERFORM pg_advisory_xact_lock(
        hashtextextended(
            jsonb_build_array(
                'buzz:nip-fi:protected-publication-projection:v1',
                NEW.community_id::TEXT,
                NEW.projection_kind,
                NEW.projection_key
            )::TEXT,
            0
        )
    );
    NEW.publication_sequence := nextval('protected_publication_projection_sequence_v1');
    NEW.created_at := transaction_timestamp();
    NEW.next_attempt_at := transaction_timestamp();
    RETURN NEW;
END;
$$;

--
-- Name: protected_publication_projection_receipt_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION protected_publication_projection_receipt_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
DECLARE
    canonical_authority_operation_id UUID;
    canonical_authority_request_fingerprint BYTEA;
    canonical_authority_epoch BIGINT;
    canonical_authority_fence BYTEA;
BEGIN
    SELECT authority.operation_id,
           authority.request_fingerprint,
           authority.authority_epoch,
           authority.fence
      INTO canonical_authority_operation_id,
           canonical_authority_request_fingerprint,
           canonical_authority_epoch,
           canonical_authority_fence
        FROM authorization_operation_receipts receipt
        JOIN authorization_admission_results admission
          ON admission.community_id = receipt.community_id
         AND admission.operation_id = receipt.operation_id
         AND admission.request_fingerprint = receipt.request_fingerprint
        JOIN protected_object_authority authority
          ON authority.community_id = admission.community_id
         AND authority.object_kind = admission.object_kind
         AND authority.object_key = admission.object_key
        JOIN authorization_authority_epochs epoch
          ON epoch.community_id = authority.community_id
         AND epoch.object_kind = authority.object_kind
         AND epoch.object_key = authority.object_key
         AND epoch.authority_epoch = authority.authority_epoch
         AND epoch.fence = authority.fence
         AND epoch.operation_id = authority.operation_id
         AND epoch.request_fingerprint = authority.request_fingerprint
        JOIN authorization_operation_receipts authority_receipt
          ON authority_receipt.community_id = authority.community_id
         AND authority_receipt.operation_id = authority.operation_id
         AND authority_receipt.request_fingerprint = authority.request_fingerprint
        WHERE receipt.community_id = NEW.community_id
          AND receipt.operation_id = NEW.operation_id
          AND receipt.request_fingerprint = NEW.request_fingerprint
          -- First-use enrollment may atomically carry this protected effect;
          -- every later established-binding mutation uses kind 11.
          AND receipt.operation_kind IN (1, 11)
          AND receipt.outcome_code = 1
          AND admission.object_kind = NEW.object_kind
          AND admission.object_key = NEW.object_key
          AND admission.application_type = NEW.application_type
          AND admission.application_version = NEW.application_version
          AND admission.application_effect_digest = NEW.application_effect_digest
          AND admission.application_result_digest = NEW.publication_result_digest
          AND authority_receipt.operation_kind IN (1, 11)
          AND authority_receipt.outcome_code = 1
        FOR SHARE OF authority, epoch, authority_receipt;

    IF canonical_authority_operation_id IS NULL THEN
        RAISE EXCEPTION 'protected publication projection requires exact applied receipt'
            USING ERRCODE = 'foreign_key_violation',
                  CONSTRAINT = 'protected_publication_projection_receipt';
    END IF;

    UPDATE protected_publication_projection_outbox projection
       SET authority_operation_id = canonical_authority_operation_id,
           authority_request_fingerprint = canonical_authority_request_fingerprint,
           authority_epoch = canonical_authority_epoch,
           authority_fence = canonical_authority_fence
     WHERE projection.community_id = NEW.community_id
       AND projection.operation_id = NEW.operation_id
       AND projection.projection_kind = NEW.projection_kind
       AND projection.authority_operation_id IS NULL
       AND projection.authority_request_fingerprint IS NULL
       AND projection.authority_epoch IS NULL
       AND projection.authority_fence IS NULL;

    IF NOT EXISTS (
        SELECT 1
          FROM protected_publication_projection_outbox projection
         WHERE projection.community_id = NEW.community_id
           AND projection.operation_id = NEW.operation_id
           AND projection.projection_kind = NEW.projection_kind
           AND projection.authority_operation_id = canonical_authority_operation_id
           AND projection.authority_request_fingerprint =
               canonical_authority_request_fingerprint
           AND projection.authority_epoch = canonical_authority_epoch
           AND projection.authority_fence = canonical_authority_fence
    ) THEN
        RAISE EXCEPTION 'protected publication projection authority binding is invalid'
            USING ERRCODE = 'foreign_key_violation',
                  CONSTRAINT = 'protected_publication_projection_receipt';
    END IF;
    RETURN NEW;
END;
$$;

--
-- Name: protected_publication_projection_state_guard_v1(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION protected_publication_projection_state_guard_v1()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    -- The deferred receipt guard owns the sole transition from an unbound
    -- in-transaction row to its immutable canonical authority snapshot.
    IF pg_trigger_depth() = 2
        AND OLD.authority_operation_id IS NULL
        AND OLD.authority_request_fingerprint IS NULL
        AND OLD.authority_epoch IS NULL
        AND OLD.authority_fence IS NULL
        AND NEW.authority_operation_id IS NOT NULL
        AND NEW.authority_request_fingerprint IS NOT NULL
        AND NEW.authority_epoch IS NOT NULL
        AND NEW.authority_fence IS NOT NULL
        AND NEW.community_id IS NOT DISTINCT FROM OLD.community_id
        AND NEW.operation_id IS NOT DISTINCT FROM OLD.operation_id
        AND NEW.request_fingerprint IS NOT DISTINCT FROM OLD.request_fingerprint
        AND NEW.publication_result_digest IS NOT DISTINCT FROM OLD.publication_result_digest
        AND NEW.object_kind IS NOT DISTINCT FROM OLD.object_kind
        AND NEW.object_key IS NOT DISTINCT FROM OLD.object_key
        AND NEW.application_type IS NOT DISTINCT FROM OLD.application_type
        AND NEW.application_version IS NOT DISTINCT FROM OLD.application_version
        AND NEW.application_effect_digest IS NOT DISTINCT FROM OLD.application_effect_digest
        AND NEW.projection_kind IS NOT DISTINCT FROM OLD.projection_kind
        AND NEW.projection_key IS NOT DISTINCT FROM OLD.projection_key
        AND NEW.staged_object_key IS NOT DISTINCT FROM OLD.staged_object_key
        AND NEW.payload_digest IS NOT DISTINCT FROM OLD.payload_digest
        AND NEW.delivery_state IS NOT DISTINCT FROM OLD.delivery_state
        AND NEW.attempt_count IS NOT DISTINCT FROM OLD.attempt_count
        AND NEW.next_attempt_at IS NOT DISTINCT FROM OLD.next_attempt_at
        AND NEW.last_attempt_at IS NOT DISTINCT FROM OLD.last_attempt_at
        AND NEW.delivered_at IS NOT DISTINCT FROM OLD.delivered_at
        AND NEW.failure_code IS NOT DISTINCT FROM OLD.failure_code
        AND NEW.created_at IS NOT DISTINCT FROM OLD.created_at
        AND NEW.publication_sequence IS NOT DISTINCT FROM OLD.publication_sequence
    THEN
        RETURN NEW;
    END IF;

    IF NEW.community_id IS DISTINCT FROM OLD.community_id
        OR NEW.operation_id IS DISTINCT FROM OLD.operation_id
        OR NEW.request_fingerprint IS DISTINCT FROM OLD.request_fingerprint
        OR NEW.publication_result_digest IS DISTINCT FROM OLD.publication_result_digest
        OR NEW.object_kind IS DISTINCT FROM OLD.object_kind
        OR NEW.object_key IS DISTINCT FROM OLD.object_key
        OR NEW.application_type IS DISTINCT FROM OLD.application_type
        OR NEW.application_version IS DISTINCT FROM OLD.application_version
        OR NEW.application_effect_digest IS DISTINCT FROM OLD.application_effect_digest
        OR NEW.authority_operation_id IS DISTINCT FROM OLD.authority_operation_id
        OR NEW.authority_request_fingerprint IS DISTINCT FROM OLD.authority_request_fingerprint
        OR NEW.authority_epoch IS DISTINCT FROM OLD.authority_epoch
        OR NEW.authority_fence IS DISTINCT FROM OLD.authority_fence
        OR NEW.projection_kind IS DISTINCT FROM OLD.projection_kind
        OR NEW.projection_key IS DISTINCT FROM OLD.projection_key
        OR NEW.staged_object_key IS DISTINCT FROM OLD.staged_object_key
        OR NEW.payload_digest IS DISTINCT FROM OLD.payload_digest
        OR NEW.created_at IS DISTINCT FROM OLD.created_at
        OR NEW.publication_sequence IS DISTINCT FROM OLD.publication_sequence
        OR OLD.delivery_state <> 1
        OR NEW.delivery_state NOT IN (1, 2, 3)
        OR NEW.attempt_count <> OLD.attempt_count + 1
        OR NEW.next_attempt_at < OLD.next_attempt_at
        OR (NEW.delivery_state = 1 AND NEW.failure_code NOT IN (1, 2))
        OR (
            NEW.delivery_state = 2
            AND EXISTS (
                SELECT 1
                FROM protected_publication_projection_outbox earlier
                WHERE earlier.community_id = OLD.community_id
                  AND earlier.object_kind = OLD.object_kind
                  AND earlier.object_key = OLD.object_key
                  AND earlier.projection_kind = OLD.projection_kind
                  AND earlier.projection_key = OLD.projection_key
                  AND earlier.publication_sequence < OLD.publication_sequence
                  AND earlier.delivery_state = 1
            )
        )
    THEN
        RAISE EXCEPTION 'protected publication projection transition is invalid'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'protected_publication_projection_transition';
    END IF;
    NEW.last_attempt_at := transaction_timestamp();
    IF NEW.delivery_state = 2 THEN
        NEW.delivered_at := transaction_timestamp();
        NEW.failure_code := 0;
    ELSE
        NEW.delivered_at := NULL;
    END IF;
    RETURN NEW;
END;
$$;

--
-- Name: purge_soft_deleted_buzz_mesh_status(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION purge_soft_deleted_buzz_mesh_status()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    IF OLD.deleted_at IS NULL
       AND NEW.deleted_at IS NOT NULL
       AND NEW.kind = 30003
       AND NEW.d_tag LIKE 'buzz-mesh-member-status:%'
       AND NEW.tags @> '[["k", "buzz-mesh-status"]]'::jsonb THEN
        DELETE FROM events
        WHERE community_id = NEW.community_id
          AND created_at = NEW.created_at
          AND id = NEW.id;

        DELETE FROM event_mentions
        WHERE community_id = NEW.community_id AND event_id = NEW.id;
    END IF;

    RETURN NULL;
END;
$$;

--
-- Name: purge_soft_deleted_nip_rs(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION purge_soft_deleted_nip_rs()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
BEGIN
    IF OLD.deleted_at IS NULL
       AND NEW.deleted_at IS NOT NULL
       AND NEW.kind = 30078
       AND NEW.d_tag ~ '^read-state:[0-9a-f]{32}$'
       AND (
           SELECT count(*)
           FROM jsonb_array_elements(CASE WHEN jsonb_typeof(NEW.tags) = 'array' THEN NEW.tags ELSE '[]'::jsonb END) tag
           WHERE jsonb_typeof(tag) = 'array'
             AND tag->0 = '"d"'::jsonb
       ) = 1
       AND EXISTS (
           SELECT 1
           FROM jsonb_array_elements(CASE WHEN jsonb_typeof(NEW.tags) = 'array' THEN NEW.tags ELSE '[]'::jsonb END) tag
           WHERE jsonb_typeof(tag) = 'array'
             AND jsonb_array_length(tag) >= 2
             AND jsonb_typeof(tag->1) = 'string'
             AND tag->>0 = 'd'
             AND tag->>1 = NEW.d_tag
       )
       AND (
           SELECT count(*)
           FROM jsonb_array_elements(CASE WHEN jsonb_typeof(NEW.tags) = 'array' THEN NEW.tags ELSE '[]'::jsonb END) tag
           WHERE tag = '["t", "read-state"]'::jsonb
       ) = 1 THEN
        PERFORM set_config('buzz.nip_rs_hard_delete', 'on', true);

        DELETE FROM events
        WHERE community_id = NEW.community_id
          AND created_at = NEW.created_at
          AND id = NEW.id;

        DELETE FROM event_mentions
        WHERE community_id = NEW.community_id AND event_id = NEW.id;
    END IF;

    RETURN NULL;
END;
$$;

--
-- Name: refresh_channel_ttl_after_event_insert(); Type: FUNCTION; Schema: -; Owner: -
--

CREATE OR REPLACE FUNCTION refresh_channel_ttl_after_event_insert()
RETURNS trigger
LANGUAGE plpgsql
VOLATILE
AS $$
DECLARE
    channel_ttl INTEGER;
BEGIN
    -- Kind 9007 creates the channel and initializes its deadline itself.
    IF NEW.channel_id IS NOT NULL AND NEW.kind <> 9007 THEN
        BEGIN
            PERFORM pg_advisory_xact_lock_shared(hashtextextended(
                'buzz_channel_ttl:' || NEW.community_id::text || ':' || NEW.channel_id::text, 0));

            SELECT ttl_seconds INTO channel_ttl
            FROM channels
            WHERE community_id = NEW.community_id AND id = NEW.channel_id;

            IF channel_ttl IS NOT NULL THEN
                UPDATE channels
                SET ttl_deadline = clock_timestamp() + make_interval(secs => ttl_seconds)
                WHERE community_id = NEW.community_id
                  AND id = NEW.channel_id
                  AND ttl_seconds IS NOT NULL
                  AND archived_at IS NULL
                  AND deleted_at IS NULL;
            END IF;
        EXCEPTION WHEN OTHERS THEN
            -- Preserve the existing best-effort contract: a TTL refresh failure
            -- must not reject an otherwise valid durable event.
            RAISE WARNING 'channel TTL refresh failed for community %, channel %: %',
                NEW.community_id, NEW.channel_id, SQLERRM;
        END;
    END IF;
    RETURN NULL;
END
$$;

--
-- Name: identity_bindings_exact_birth_history_fk; Type: CONSTRAINT; Schema: -; Owner: -
--

ALTER TABLE identity_bindings
ADD CONSTRAINT identity_bindings_exact_birth_history_fk FOREIGN KEY (community_id, birth_history_id, binding_id, binding_version, creation_operation_id, creation_request_fingerprint) REFERENCES identity_lifecycle_history (community_id, history_id, successor_binding_id, successor_binding_version, operation_id, request_fingerprint) DEFERRABLE INITIALLY DEFERRED;

--
-- Name: identity_bindings_exact_retirement_history_fk; Type: CONSTRAINT; Schema: -; Owner: -
--

ALTER TABLE identity_bindings
ADD CONSTRAINT identity_bindings_exact_retirement_history_fk FOREIGN KEY (community_id, retirement_history_id, binding_id, binding_version, lifecycle_revision, binding_state) REFERENCES identity_lifecycle_history (community_id, history_id, old_binding_id, old_binding_version, old_resulting_lifecycle_revision, old_resulting_state) DEFERRABLE INITIALLY DEFERRED;

--
-- Name: authorization_admission_results_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_admission_results_no_truncate
    BEFORE TRUNCATE ON authorization_admission_results
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: authorization_admission_results_no_update; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_admission_results_no_update
    BEFORE UPDATE OR DELETE ON authorization_admission_results
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: authorization_authentication_denial_attempts_immutable; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_authentication_denial_attempts_immutable
    BEFORE UPDATE OR DELETE ON authorization_authentication_denial_attempts
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: authorization_authentication_denial_attempts_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_authentication_denial_attempts_no_truncate
    BEFORE TRUNCATE ON authorization_authentication_denial_attempts
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: authorization_authority_epochs_monotonic; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_authority_epochs_monotonic
    BEFORE UPDATE ON authorization_authority_epochs
    FOR EACH ROW
    EXECUTE FUNCTION authorization_authority_epoch_guard_v1();

--
-- Name: authorization_authority_epochs_no_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_authority_epochs_no_delete
    BEFORE DELETE ON authorization_authority_epochs
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: authorization_authority_epochs_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_authority_epochs_no_truncate
    BEFORE TRUNCATE ON authorization_authority_epochs
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: authorization_event_capacity_monotonic; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_event_capacity_monotonic
    BEFORE UPDATE ON authorization_event_capacity
    FOR EACH ROW
    EXECUTE FUNCTION authorization_event_capacity_guard_v2();

--
-- Name: authorization_event_capacity_no_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_event_capacity_no_delete
    BEFORE DELETE ON authorization_event_capacity
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: authorization_event_capacity_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_event_capacity_no_truncate
    BEFORE TRUNCATE ON authorization_event_capacity
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: authorization_event_capacity_reserve_defaults; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_event_capacity_reserve_defaults
    BEFORE INSERT ON authorization_event_capacity
    FOR EACH ROW
    EXECUTE FUNCTION authorization_event_capacity_reserve_defaults_v2();

--
-- Name: authorization_event_receipt_cardinality; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER authorization_event_receipt_cardinality
    AFTER INSERT ON authorization_events
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION authorization_operation_receipt_event_guard_v1();

--
-- Name: authorization_events_capacity; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_events_capacity
    BEFORE INSERT ON authorization_events
    FOR EACH ROW
    EXECUTE FUNCTION authorization_event_capacity_before_insert_v2();

--
-- Name: authorization_events_immutable; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_events_immutable
    BEFORE UPDATE OR DELETE ON authorization_events
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: authorization_events_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_events_no_truncate
    BEFORE TRUNCATE ON authorization_events
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: authorization_invalidation_domains_monotonic; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_invalidation_domains_monotonic
    BEFORE UPDATE ON authorization_invalidation_domains
    FOR EACH ROW
    EXECUTE FUNCTION authorization_invalidation_domain_guard_v1();

--
-- Name: authorization_invalidation_domains_no_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_invalidation_domains_no_delete
    BEFORE DELETE ON authorization_invalidation_domains
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: authorization_invalidation_domains_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_invalidation_domains_no_truncate
    BEFORE TRUNCATE ON authorization_invalidation_domains
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: authorization_invalidation_floors_monotonic; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_invalidation_floors_monotonic
    BEFORE UPDATE ON authorization_invalidation_floors
    FOR EACH ROW
    EXECUTE FUNCTION authorization_invalidation_floor_guard_v1();

--
-- Name: authorization_invalidation_floors_no_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_invalidation_floors_no_delete
    BEFORE DELETE ON authorization_invalidation_floors
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: authorization_invalidation_floors_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_invalidation_floors_no_truncate
    BEFORE TRUNCATE ON authorization_invalidation_floors
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: authorization_operation_receipt_event_cardinality; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER authorization_operation_receipt_event_cardinality
    AFTER INSERT ON authorization_operation_receipts
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION authorization_operation_receipt_event_guard_v1();

--
-- Name: authorization_operation_receipt_history_cardinality; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER authorization_operation_receipt_history_cardinality
    AFTER INSERT ON authorization_operation_receipts
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION authorization_operation_receipt_history_guard_v1();

--
-- Name: authorization_operation_receipts_immutable; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_operation_receipts_immutable
    BEFORE UPDATE OR DELETE ON authorization_operation_receipts
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: authorization_operation_receipts_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_operation_receipts_no_truncate
    BEFORE TRUNCATE ON authorization_operation_receipts
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: authorization_operation_version_delta_component_cardinality; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER authorization_operation_version_delta_component_cardinality
    AFTER INSERT ON authorization_operation_version_deltas
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION authorization_operation_version_delta_cardinality_guard_v1();

--
-- Name: authorization_operation_version_delta_manifest_cardinality; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER authorization_operation_version_delta_manifest_cardinality
    AFTER INSERT ON authorization_operation_version_delta_manifests
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION authorization_operation_version_delta_cardinality_guard_v1();

--
-- Name: authorization_operation_version_delta_manifests_immutable; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_operation_version_delta_manifests_immutable
    BEFORE UPDATE OR DELETE ON authorization_operation_version_delta_manifests
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: authorization_operation_version_delta_manifests_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_operation_version_delta_manifests_no_truncate
    BEFORE TRUNCATE ON authorization_operation_version_delta_manifests
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: authorization_operation_version_deltas_immutable; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_operation_version_deltas_immutable
    BEFORE UPDATE OR DELETE ON authorization_operation_version_deltas
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: authorization_operation_version_deltas_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_operation_version_deltas_no_truncate
    BEFORE TRUNCATE ON authorization_operation_version_deltas
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: authorization_operator_denial_buckets_monotonic; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_operator_denial_buckets_monotonic
    BEFORE UPDATE ON authorization_operator_denial_buckets
    FOR EACH ROW
    EXECUTE FUNCTION authorization_operator_denial_bucket_guard_v1();

--
-- Name: authorization_operator_denial_buckets_no_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_operator_denial_buckets_no_delete
    BEFORE DELETE ON authorization_operator_denial_buckets
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: authorization_operator_denial_buckets_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_operator_denial_buckets_no_truncate
    BEFORE TRUNCATE ON authorization_operator_denial_buckets
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: authorization_proxy_nonce_claims_immutable; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_proxy_nonce_claims_immutable
    BEFORE UPDATE ON authorization_proxy_nonce_claims
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: authorization_proxy_nonce_claims_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_proxy_nonce_claims_no_truncate
    BEFORE TRUNCATE ON authorization_proxy_nonce_claims
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: authorization_proxy_nonce_claims_retained; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_proxy_nonce_claims_retained
    BEFORE DELETE ON authorization_proxy_nonce_claims
    FOR EACH ROW
    EXECUTE FUNCTION authorization_proxy_nonce_claim_retention_v1();

--
-- Name: authorization_proxy_nonce_claims_stamp; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER authorization_proxy_nonce_claims_stamp
    BEFORE INSERT ON authorization_proxy_nonce_claims
    FOR EACH ROW
    EXECUTE FUNCTION authorization_proxy_nonce_claim_stamp_v1();

--
-- Name: events_created_at_floor; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_created_at_floor
    AFTER INSERT OR UPDATE OF created_at, channel_id ON events_p_past
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION events_created_at_floor_guard();

--
-- Name: events_created_at_floor; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_created_at_floor
    AFTER INSERT OR UPDATE OF created_at, channel_id ON events_p2026_01
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION events_created_at_floor_guard();

--
-- Name: events_created_at_floor; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_created_at_floor
    AFTER INSERT OR UPDATE OF created_at, channel_id ON events
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION events_created_at_floor_guard();

--
-- Name: events_created_at_floor; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_created_at_floor
    AFTER INSERT OR UPDATE OF created_at, channel_id ON events_p_future
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION events_created_at_floor_guard();

--
-- Name: events_created_at_floor; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_created_at_floor
    AFTER INSERT OR UPDATE OF created_at, channel_id ON events_p2026_06
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION events_created_at_floor_guard();

--
-- Name: events_created_at_floor; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_created_at_floor
    AFTER INSERT OR UPDATE OF created_at, channel_id ON events_p2026_05
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION events_created_at_floor_guard();

--
-- Name: events_created_at_floor; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_created_at_floor
    AFTER INSERT OR UPDATE OF created_at, channel_id ON events_p2026_04
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION events_created_at_floor_guard();

--
-- Name: events_created_at_floor; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_created_at_floor
    AFTER INSERT OR UPDATE OF created_at, channel_id ON events_p2026_03
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION events_created_at_floor_guard();

--
-- Name: events_created_at_floor; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_created_at_floor
    AFTER INSERT OR UPDATE OF created_at, channel_id ON events_p2026_02
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION events_created_at_floor_guard();

--
-- Name: events_enqueue_push_match; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER events_enqueue_push_match
    AFTER INSERT ON events_p2026_01
    FOR EACH ROW
    EXECUTE FUNCTION enqueue_push_match_job();

--
-- Name: events_enqueue_push_match; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER events_enqueue_push_match
    AFTER INSERT ON events_p2026_06
    FOR EACH ROW
    EXECUTE FUNCTION enqueue_push_match_job();

--
-- Name: events_enqueue_push_match; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER events_enqueue_push_match
    AFTER INSERT ON events_p2026_04
    FOR EACH ROW
    EXECUTE FUNCTION enqueue_push_match_job();

--
-- Name: events_enqueue_push_match; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER events_enqueue_push_match
    AFTER INSERT ON events_p_past
    FOR EACH ROW
    EXECUTE FUNCTION enqueue_push_match_job();

--
-- Name: events_enqueue_push_match; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER events_enqueue_push_match
    AFTER INSERT ON events_p2026_05
    FOR EACH ROW
    EXECUTE FUNCTION enqueue_push_match_job();

--
-- Name: events_enqueue_push_match; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER events_enqueue_push_match
    AFTER INSERT ON events_p_future
    FOR EACH ROW
    EXECUTE FUNCTION enqueue_push_match_job();

--
-- Name: events_enqueue_push_match; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER events_enqueue_push_match
    AFTER INSERT ON events
    FOR EACH ROW
    EXECUTE FUNCTION enqueue_push_match_job();

--
-- Name: events_enqueue_push_match; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER events_enqueue_push_match
    AFTER INSERT ON events_p2026_03
    FOR EACH ROW
    EXECUTE FUNCTION enqueue_push_match_job();

--
-- Name: events_enqueue_push_match; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER events_enqueue_push_match
    AFTER INSERT ON events_p2026_02
    FOR EACH ROW
    EXECUTE FUNCTION enqueue_push_match_job();

--
-- Name: events_refresh_channel_ttl; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_refresh_channel_ttl
    AFTER INSERT ON events
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION refresh_channel_ttl_after_event_insert();

--
-- Name: events_refresh_channel_ttl; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_refresh_channel_ttl
    AFTER INSERT ON events_p_future
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION refresh_channel_ttl_after_event_insert();

--
-- Name: events_refresh_channel_ttl; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_refresh_channel_ttl
    AFTER INSERT ON events_p_past
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION refresh_channel_ttl_after_event_insert();

--
-- Name: events_refresh_channel_ttl; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_refresh_channel_ttl
    AFTER INSERT ON events_p2026_04
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION refresh_channel_ttl_after_event_insert();

--
-- Name: events_refresh_channel_ttl; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_refresh_channel_ttl
    AFTER INSERT ON events_p2026_01
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION refresh_channel_ttl_after_event_insert();

--
-- Name: events_refresh_channel_ttl; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_refresh_channel_ttl
    AFTER INSERT ON events_p2026_02
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION refresh_channel_ttl_after_event_insert();

--
-- Name: events_refresh_channel_ttl; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_refresh_channel_ttl
    AFTER INSERT ON events_p2026_06
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION refresh_channel_ttl_after_event_insert();

--
-- Name: events_refresh_channel_ttl; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_refresh_channel_ttl
    AFTER INSERT ON events_p2026_03
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION refresh_channel_ttl_after_event_insert();

--
-- Name: events_refresh_channel_ttl; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER events_refresh_channel_ttl
    AFTER INSERT ON events_p2026_05
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION refresh_channel_ttl_after_event_insert();

--
-- Name: identity_bindings_birth_eligibility; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER identity_bindings_birth_eligibility
    AFTER INSERT ON identity_bindings
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION identity_binding_birth_eligibility_guard_v1();

--
-- Name: identity_bindings_history_semantics; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER identity_bindings_history_semantics
    AFTER INSERT OR UPDATE ON identity_bindings
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION identity_binding_history_semantics_guard_v1();

--
-- Name: identity_bindings_insert_guard; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER identity_bindings_insert_guard
    BEFORE INSERT ON identity_bindings
    FOR EACH ROW
    EXECUTE FUNCTION identity_bindings_insert_guard_v1();

--
-- Name: identity_bindings_no_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER identity_bindings_no_delete
    BEFORE DELETE ON identity_bindings
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: identity_bindings_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER identity_bindings_no_truncate
    BEFORE TRUNCATE ON identity_bindings
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: identity_bindings_transition_guard; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER identity_bindings_transition_guard
    BEFORE UPDATE ON identity_bindings
    FOR EACH ROW
    EXECUTE FUNCTION identity_bindings_transition_guard_v1();

--
-- Name: identity_bindings_transition_integrity; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER identity_bindings_transition_integrity
    AFTER INSERT OR UPDATE ON identity_bindings
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION identity_lifecycle_transition_integrity_guard_v1();

--
-- Name: identity_enrollment_policies_immutable; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER identity_enrollment_policies_immutable
    BEFORE UPDATE OR DELETE ON identity_enrollment_policies
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: identity_enrollment_policies_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER identity_enrollment_policies_no_truncate
    BEFORE TRUNCATE ON identity_enrollment_policies
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: identity_lifecycle_consumption_history_semantics; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER identity_lifecycle_consumption_history_semantics
    AFTER INSERT ON identity_lifecycle_selector_consumptions
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION identity_lifecycle_consumption_history_guard_v1();

--
-- Name: identity_lifecycle_consumption_lock; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER identity_lifecycle_consumption_lock
    BEFORE INSERT ON identity_lifecycle_selector_consumptions
    FOR EACH ROW
    EXECUTE FUNCTION identity_lifecycle_consumption_lock_v1();

--
-- Name: identity_lifecycle_consumption_transition_integrity; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER identity_lifecycle_consumption_transition_integrity
    AFTER INSERT ON identity_lifecycle_selector_consumptions
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION identity_lifecycle_transition_integrity_guard_v1();

--
-- Name: identity_lifecycle_history_immutable; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER identity_lifecycle_history_immutable
    BEFORE UPDATE OR DELETE ON identity_lifecycle_history
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: identity_lifecycle_history_insert_guard; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER identity_lifecycle_history_insert_guard
    BEFORE INSERT ON identity_lifecycle_history
    FOR EACH ROW
    EXECUTE FUNCTION identity_lifecycle_history_insert_guard_v1();

--
-- Name: identity_lifecycle_history_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER identity_lifecycle_history_no_truncate
    BEFORE TRUNCATE ON identity_lifecycle_history
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: identity_lifecycle_selector_consumptions_immutable; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER identity_lifecycle_selector_consumptions_immutable
    BEFORE UPDATE OR DELETE ON identity_lifecycle_selector_consumptions
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: identity_lifecycle_selector_consumptions_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER identity_lifecycle_selector_consumptions_no_truncate
    BEFORE TRUNCATE ON identity_lifecycle_selector_consumptions
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: identity_lifecycle_selector_history_semantics; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER identity_lifecycle_selector_history_semantics
    AFTER INSERT ON identity_lifecycle_selectors
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION identity_lifecycle_selector_history_guard_v1();

--
-- Name: identity_lifecycle_selector_insert_guard; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER identity_lifecycle_selector_insert_guard
    BEFORE INSERT ON identity_lifecycle_selectors
    FOR EACH ROW
    EXECUTE FUNCTION identity_lifecycle_selector_insert_guard_v1();

--
-- Name: identity_lifecycle_selector_transition_integrity; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER identity_lifecycle_selector_transition_integrity
    AFTER INSERT ON identity_lifecycle_selectors
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION identity_lifecycle_transition_integrity_guard_v1();

--
-- Name: identity_lifecycle_selectors_immutable; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER identity_lifecycle_selectors_immutable
    BEFORE UPDATE OR DELETE ON identity_lifecycle_selectors
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: identity_lifecycle_selectors_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER identity_lifecycle_selectors_no_truncate
    BEFORE TRUNCATE ON identity_lifecycle_selectors
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: identity_lifecycle_transition_integrity; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER identity_lifecycle_transition_integrity
    AFTER INSERT ON identity_lifecycle_history
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION identity_lifecycle_transition_integrity_guard_v1();

--
-- Name: protected_object_authority_no_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER protected_object_authority_no_delete
    BEFORE DELETE ON protected_object_authority
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: protected_object_authority_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER protected_object_authority_no_truncate
    BEFORE TRUNCATE ON protected_object_authority
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: protected_object_authority_strict_replacement; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER protected_object_authority_strict_replacement
    BEFORE UPDATE ON protected_object_authority
    FOR EACH ROW
    EXECUTE FUNCTION protected_object_authority_guard_v1();

--
-- Name: protected_publication_projection_initial_state; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER protected_publication_projection_initial_state
    BEFORE INSERT ON protected_publication_projection_outbox
    FOR EACH ROW
    EXECUTE FUNCTION protected_publication_projection_insert_guard_v1();

--
-- Name: protected_publication_projection_no_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER protected_publication_projection_no_delete
    BEFORE DELETE ON protected_publication_projection_outbox
    FOR EACH ROW
    EXECUTE FUNCTION nip_fi_reject_row_mutation_v1();

--
-- Name: protected_publication_projection_no_truncate; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER protected_publication_projection_no_truncate
    BEFORE TRUNCATE ON protected_publication_projection_outbox
    FOR EACH STATEMENT
    EXECUTE FUNCTION nip_fi_reject_truncate_v1();

--
-- Name: protected_publication_projection_receipt; Type: TRIGGER; Schema: -; Owner: -
--

CREATE CONSTRAINT TRIGGER protected_publication_projection_receipt
    AFTER INSERT ON protected_publication_projection_outbox
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION protected_publication_projection_receipt_guard_v1();

--
-- Name: protected_publication_projection_state; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER protected_publication_projection_state
    BEFORE UPDATE ON protected_publication_projection_outbox
    FOR EACH ROW
    EXECUTE FUNCTION protected_publication_projection_state_guard_v1();

--
-- Name: trg_channels_community_id_immutable; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_channels_community_id_immutable
    BEFORE UPDATE ON channels
    FOR EACH ROW
    EXECUTE FUNCTION channels_community_id_immutable();

--
-- Name: trg_event_mentions_require_live_event; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_event_mentions_require_live_event
    BEFORE INSERT ON event_mentions
    FOR EACH ROW
    EXECUTE FUNCTION guard_event_mention_live();

--
-- Name: trg_events_guard_nip_rs_hard_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_guard_nip_rs_hard_delete
    BEFORE DELETE ON events_p_past
    FOR EACH ROW
    WHEN ((((OLD.kind = 30078) AND (OLD.d_tag ~ '^read-state:[0-9a-f]{32}$'::text))))
    EXECUTE FUNCTION guard_nip_rs_hard_delete();

--
-- Name: trg_events_guard_nip_rs_hard_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_guard_nip_rs_hard_delete
    BEFORE DELETE ON events_p2026_02
    FOR EACH ROW
    WHEN ((((OLD.kind = 30078) AND (OLD.d_tag ~ '^read-state:[0-9a-f]{32}$'::text))))
    EXECUTE FUNCTION guard_nip_rs_hard_delete();

--
-- Name: trg_events_guard_nip_rs_hard_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_guard_nip_rs_hard_delete
    BEFORE DELETE ON events_p2026_04
    FOR EACH ROW
    WHEN ((((OLD.kind = 30078) AND (OLD.d_tag ~ '^read-state:[0-9a-f]{32}$'::text))))
    EXECUTE FUNCTION guard_nip_rs_hard_delete();

--
-- Name: trg_events_guard_nip_rs_hard_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_guard_nip_rs_hard_delete
    BEFORE DELETE ON events
    FOR EACH ROW
    WHEN ((((OLD.kind = 30078) AND (OLD.d_tag ~ '^read-state:[0-9a-f]{32}$'::text))))
    EXECUTE FUNCTION guard_nip_rs_hard_delete();

--
-- Name: trg_events_guard_nip_rs_hard_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_guard_nip_rs_hard_delete
    BEFORE DELETE ON events_p2026_05
    FOR EACH ROW
    WHEN ((((OLD.kind = 30078) AND (OLD.d_tag ~ '^read-state:[0-9a-f]{32}$'::text))))
    EXECUTE FUNCTION guard_nip_rs_hard_delete();

--
-- Name: trg_events_guard_nip_rs_hard_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_guard_nip_rs_hard_delete
    BEFORE DELETE ON events_p2026_01
    FOR EACH ROW
    WHEN ((((OLD.kind = 30078) AND (OLD.d_tag ~ '^read-state:[0-9a-f]{32}$'::text))))
    EXECUTE FUNCTION guard_nip_rs_hard_delete();

--
-- Name: trg_events_guard_nip_rs_hard_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_guard_nip_rs_hard_delete
    BEFORE DELETE ON events_p2026_03
    FOR EACH ROW
    WHEN ((((OLD.kind = 30078) AND (OLD.d_tag ~ '^read-state:[0-9a-f]{32}$'::text))))
    EXECUTE FUNCTION guard_nip_rs_hard_delete();

--
-- Name: trg_events_guard_nip_rs_hard_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_guard_nip_rs_hard_delete
    BEFORE DELETE ON events_p_future
    FOR EACH ROW
    WHEN ((((OLD.kind = 30078) AND (OLD.d_tag ~ '^read-state:[0-9a-f]{32}$'::text))))
    EXECUTE FUNCTION guard_nip_rs_hard_delete();

--
-- Name: trg_events_guard_nip_rs_hard_delete; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_guard_nip_rs_hard_delete
    BEFORE DELETE ON events_p2026_06
    FOR EACH ROW
    WHEN ((((OLD.kind = 30078) AND (OLD.d_tag ~ '^read-state:[0-9a-f]{32}$'::text))))
    EXECUTE FUNCTION guard_nip_rs_hard_delete();

--
-- Name: trg_events_nip_rs_watermark; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_nip_rs_watermark
    BEFORE INSERT ON events_p2026_04
    FOR EACH ROW
    EXECUTE FUNCTION guard_nip_rs_watermark();

--
-- Name: trg_events_nip_rs_watermark; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_nip_rs_watermark
    BEFORE INSERT ON events_p2026_06
    FOR EACH ROW
    EXECUTE FUNCTION guard_nip_rs_watermark();

--
-- Name: trg_events_nip_rs_watermark; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_nip_rs_watermark
    BEFORE INSERT ON events_p2026_01
    FOR EACH ROW
    EXECUTE FUNCTION guard_nip_rs_watermark();

--
-- Name: trg_events_nip_rs_watermark; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_nip_rs_watermark
    BEFORE INSERT ON events_p2026_02
    FOR EACH ROW
    EXECUTE FUNCTION guard_nip_rs_watermark();

--
-- Name: trg_events_nip_rs_watermark; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_nip_rs_watermark
    BEFORE INSERT ON events_p_future
    FOR EACH ROW
    EXECUTE FUNCTION guard_nip_rs_watermark();

--
-- Name: trg_events_nip_rs_watermark; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_nip_rs_watermark
    BEFORE INSERT ON events_p2026_05
    FOR EACH ROW
    EXECUTE FUNCTION guard_nip_rs_watermark();

--
-- Name: trg_events_nip_rs_watermark; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_nip_rs_watermark
    BEFORE INSERT ON events
    FOR EACH ROW
    EXECUTE FUNCTION guard_nip_rs_watermark();

--
-- Name: trg_events_nip_rs_watermark; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_nip_rs_watermark
    BEFORE INSERT ON events_p2026_03
    FOR EACH ROW
    EXECUTE FUNCTION guard_nip_rs_watermark();

--
-- Name: trg_events_nip_rs_watermark; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_nip_rs_watermark
    BEFORE INSERT ON events_p_past
    FOR EACH ROW
    EXECUTE FUNCTION guard_nip_rs_watermark();

--
-- Name: trg_events_purge_soft_deleted_buzz_mesh_status; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_buzz_mesh_status
    AFTER UPDATE OF deleted_at ON events_p2026_04
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_buzz_mesh_status();

--
-- Name: trg_events_purge_soft_deleted_buzz_mesh_status; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_buzz_mesh_status
    AFTER UPDATE OF deleted_at ON events_p2026_06
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_buzz_mesh_status();

--
-- Name: trg_events_purge_soft_deleted_buzz_mesh_status; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_buzz_mesh_status
    AFTER UPDATE OF deleted_at ON events_p_past
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_buzz_mesh_status();

--
-- Name: trg_events_purge_soft_deleted_buzz_mesh_status; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_buzz_mesh_status
    AFTER UPDATE OF deleted_at ON events_p2026_03
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_buzz_mesh_status();

--
-- Name: trg_events_purge_soft_deleted_buzz_mesh_status; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_buzz_mesh_status
    AFTER UPDATE OF deleted_at ON events
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_buzz_mesh_status();

--
-- Name: trg_events_purge_soft_deleted_buzz_mesh_status; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_buzz_mesh_status
    AFTER UPDATE OF deleted_at ON events_p2026_05
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_buzz_mesh_status();

--
-- Name: trg_events_purge_soft_deleted_buzz_mesh_status; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_buzz_mesh_status
    AFTER UPDATE OF deleted_at ON events_p_future
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_buzz_mesh_status();

--
-- Name: trg_events_purge_soft_deleted_buzz_mesh_status; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_buzz_mesh_status
    AFTER UPDATE OF deleted_at ON events_p2026_02
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_buzz_mesh_status();

--
-- Name: trg_events_purge_soft_deleted_buzz_mesh_status; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_buzz_mesh_status
    AFTER UPDATE OF deleted_at ON events_p2026_01
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_buzz_mesh_status();

--
-- Name: trg_events_purge_soft_deleted_nip_rs; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_nip_rs
    AFTER UPDATE OF deleted_at ON events_p_future
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_nip_rs();

--
-- Name: trg_events_purge_soft_deleted_nip_rs; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_nip_rs
    AFTER UPDATE OF deleted_at ON events_p2026_01
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_nip_rs();

--
-- Name: trg_events_purge_soft_deleted_nip_rs; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_nip_rs
    AFTER UPDATE OF deleted_at ON events_p2026_05
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_nip_rs();

--
-- Name: trg_events_purge_soft_deleted_nip_rs; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_nip_rs
    AFTER UPDATE OF deleted_at ON events_p2026_06
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_nip_rs();

--
-- Name: trg_events_purge_soft_deleted_nip_rs; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_nip_rs
    AFTER UPDATE OF deleted_at ON events_p2026_02
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_nip_rs();

--
-- Name: trg_events_purge_soft_deleted_nip_rs; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_nip_rs
    AFTER UPDATE OF deleted_at ON events
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_nip_rs();

--
-- Name: trg_events_purge_soft_deleted_nip_rs; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_nip_rs
    AFTER UPDATE OF deleted_at ON events_p2026_04
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_nip_rs();

--
-- Name: trg_events_purge_soft_deleted_nip_rs; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_nip_rs
    AFTER UPDATE OF deleted_at ON events_p_past
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_nip_rs();

--
-- Name: trg_events_purge_soft_deleted_nip_rs; Type: TRIGGER; Schema: -; Owner: -
--

CREATE OR REPLACE TRIGGER trg_events_purge_soft_deleted_nip_rs
    AFTER UPDATE OF deleted_at ON events_p2026_03
    FOR EACH ROW
    EXECUTE FUNCTION purge_soft_deleted_nip_rs();

-- pgschema 1.7.4 omits disjunctive CHECK constraints from dumps. The
-- post-apply catalog-closure script installs and verifies these exact checks.
ALTER TABLE ONLY authorization_event_capacity
    ADD CONSTRAINT authorization_event_capacity_check3 CHECK (((health_state = 1) AND (failure_code IS NULL) AND (failure_observed_at IS NULL)) OR ((health_state = 2) AND (failure_code IS NOT NULL) AND (failure_observed_at IS NOT NULL)));

ALTER TABLE ONLY authorization_events
    ADD CONSTRAINT authorization_events_check CHECK ((((actor_kind = 4) AND (event_kind = 9) AND (request_fingerprint IS NULL)) OR ((actor_kind = ANY (ARRAY[1, 2, 3])) AND (request_fingerprint IS NOT NULL))));

ALTER TABLE ONLY authorization_events
    ADD CONSTRAINT authorization_events_check1 CHECK ((((actor_kind = 4) AND (actor_fingerprint IS NULL) AND (subject_fingerprint IS NULL)) OR ((actor_kind = ANY (ARRAY[1, 2, 3])) AND (actor_fingerprint IS NOT NULL))));

ALTER TABLE ONLY authorization_invalidation_floors
    ADD CONSTRAINT authorization_invalidation_floors_check CHECK ((((selector_kind = 3) AND (binding_version_floor IS NOT NULL) AND (relationship_revision_floor IS NULL)) OR ((selector_kind = 7) AND (binding_version_floor IS NULL) AND (relationship_revision_floor IS NOT NULL)) OR ((selector_kind <> ALL (ARRAY[3, 7])) AND (binding_version_floor IS NULL) AND (relationship_revision_floor IS NULL))));

ALTER TABLE ONLY identity_bindings
    ADD CONSTRAINT identity_bindings_check1 CHECK ((((binding_state = 1) AND (lifecycle_revision = 1) AND (retirement_history_id IS NULL)) OR ((binding_state = 2) AND (lifecycle_revision = 2) AND (retirement_history_id IS NOT NULL))));

ALTER TABLE ONLY identity_lifecycle_history
    ADD CONSTRAINT identity_lifecycle_history_check CHECK ((((old_binding_id IS NULL) AND (old_binding_version IS NULL) AND (old_prior_lifecycle_revision IS NULL) AND (old_prior_state IS NULL) AND (old_resulting_lifecycle_revision IS NULL) AND (old_resulting_state IS NULL)) OR ((old_binding_id IS NOT NULL) AND (old_binding_version IS NOT NULL) AND (old_prior_lifecycle_revision IS NOT NULL) AND (old_prior_state IS NOT NULL) AND (old_resulting_lifecycle_revision IS NOT NULL) AND (old_resulting_state IS NOT NULL))));

ALTER TABLE ONLY identity_lifecycle_history
    ADD CONSTRAINT identity_lifecycle_history_check1 CHECK ((((successor_binding_id IS NULL) AND (successor_binding_version IS NULL) AND (successor_lifecycle_revision IS NULL) AND (successor_state IS NULL)) OR ((successor_binding_id IS NOT NULL) AND (successor_binding_version IS NOT NULL) AND (successor_lifecycle_revision = 1) AND (successor_state = 1))));

ALTER TABLE ONLY identity_lifecycle_history
    ADD CONSTRAINT identity_lifecycle_history_check5 CHECK ((((outcome_code = 3) AND (old_binding_id IS NULL) AND (successor_binding_id IS NULL)) OR ((outcome_code = 1) AND ((((transition_kind = ANY (ARRAY[1, 2])) AND (old_binding_id IS NULL) AND (successor_binding_id IS NOT NULL)) OR ((transition_kind = 3) AND (old_binding_id IS NOT NULL) AND (successor_binding_id IS NULL)) OR ((transition_kind = ANY (ARRAY[4, 5])) AND (successor_binding_id IS NULL)) OR ((transition_kind = 6) AND (old_binding_id IS NOT NULL) AND (successor_binding_id IS NOT NULL)) OR ((transition_kind = 7) AND (old_binding_id IS NOT NULL) AND (successor_binding_id IS NOT NULL)) OR ((transition_kind = 8) AND (successor_binding_id IS NOT NULL)) OR ((transition_kind = 9) AND (old_binding_id IS NOT NULL) AND (successor_binding_id IS NULL)))))));

ALTER TABLE ONLY identity_lifecycle_selectors
    ADD CONSTRAINT identity_lifecycle_selectors_check CHECK ((((selector_kind = 1) AND (fact_generation = 1) AND (principal_fingerprint IS NOT NULL) AND (event_author_pubkey IS NOT NULL) AND (binding_id IS NOT NULL) AND (binding_version IS NOT NULL)) OR ((selector_kind = 2) AND (principal_fingerprint IS NOT NULL) AND (event_author_pubkey IS NULL) AND (binding_id IS NULL) AND (binding_version IS NULL)) OR ((selector_kind = 3) AND (fact_generation = 1) AND (principal_fingerprint IS NULL) AND (event_author_pubkey IS NOT NULL) AND (binding_id IS NULL) AND (binding_version IS NULL)) OR ((selector_kind = 4) AND (principal_fingerprint IS NOT NULL) AND (event_author_pubkey IS NOT NULL) AND (binding_id IS NOT NULL) AND (binding_version IS NOT NULL))));

ALTER TABLE ONLY moderation_reports
    ADD CONSTRAINT moderation_reports_check CHECK ((((target_kind = 'event'::text) AND (target_event_id IS NOT NULL) AND (target_pubkey IS NULL) AND (target_blob_sha256 IS NULL)) OR ((target_kind = 'pubkey'::text) AND (target_event_id IS NULL) AND (target_pubkey IS NOT NULL) AND (target_blob_sha256 IS NULL)) OR ((target_kind = 'blob'::text) AND (target_event_id IS NULL) AND (target_pubkey IS NULL) AND (target_blob_sha256 IS NOT NULL))));

ALTER TABLE ONLY protected_object_authority
    ADD CONSTRAINT protected_object_authority_check1 CHECK ((((owner_pubkey IS NULL) AND (delegated_relationship_id IS NULL) AND (delegated_relationship_revision IS NULL) AND (delegation_conditions_fingerprint IS NULL)) OR ((owner_pubkey IS NOT NULL) AND (delegated_relationship_id IS NOT NULL) AND (delegated_relationship_revision IS NOT NULL) AND (delegation_conditions_fingerprint IS NOT NULL))));

ALTER TABLE ONLY protected_publication_projection_outbox
    ADD CONSTRAINT protected_publication_projection_outbox_check3 CHECK ((((delivery_state = 1) AND (delivered_at IS NULL) AND (failure_code = ANY (ARRAY[0, 1, 2]))) OR ((delivery_state = 2) AND (delivered_at IS NOT NULL) AND (failure_code = 0)) OR ((delivery_state = 3) AND (delivered_at IS NULL) AND (failure_code >= 3) AND (failure_code <= 6))));

ALTER TABLE ONLY push_leases
    ADD CONSTRAINT push_leases_check CHECK (((active AND (app_profile IS NOT NULL) AND (endpoint_hash IS NOT NULL) AND (endpoint_grant IS NOT NULL) AND (max_class IS NOT NULL) AND (subscriptions IS NOT NULL)) OR ((NOT active) AND (app_profile IS NULL) AND (endpoint_hash IS NULL) AND (endpoint_grant IS NULL) AND (max_class IS NULL) AND (subscriptions IS NULL))));
