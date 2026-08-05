-- Additive identity-binding state projection.
--
-- Migrations 0027/0028 are a frozen compatibility boundary. This migration
-- never renames or removes their columns, constraints, indexes, rows, or
-- lifecycle selectors. In particular, every legacy identity_revoked_keys row
-- remains authoritative because rotation-created and explicitly strengthened
-- tombstones cannot be distinguished after the fact.

-- Materialize every retained legacy identity row before projecting any new
-- state. A storage/decoding/read-policy failure must abort this migration;
-- treating an unreadable binding, principal denial, or key tombstone as
-- absent could otherwise invent authority. This block is read-only and runs
-- inside the migration transaction before the first persisted write.
WITH legacy_rows AS MATERIALIZED (
    SELECT to_jsonb(row_value) AS payload FROM identity_bindings row_value
    UNION ALL
    SELECT to_jsonb(row_value) AS payload FROM identity_principals row_value
    UNION ALL
    SELECT to_jsonb(row_value) AS payload FROM identity_revoked_keys row_value
)
SELECT COUNT(payload) FROM legacy_rows;

-- Current verifier-owned enrollment policy. The table is intentionally empty
-- after migration: installing a policy is a separately authorized server
-- configuration action, so the disabled candidate fails closed by default.
CREATE TABLE identity_enrollment_policies (
    community_id   UUID NOT NULL PRIMARY KEY REFERENCES communities(id),
    policy_id      UUID NOT NULL,
    policy_epoch   BIGINT NOT NULL,
    requirement    TEXT NOT NULL,
    effective_from BIGINT NOT NULL,
    effective_until BIGINT NOT NULL,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (policy_id <> '00000000-0000-0000-0000-000000000000'::UUID),
    CHECK (policy_epoch > 0),
    CHECK (requirement IN ('not_required', 'attested_key', 'provisioned', 'tofu')),
    CHECK (effective_from >= 0),
    CHECK (effective_from < effective_until)
);

-- The policy UUID is a stable namespace and every semantic replacement must
-- advance its positive epoch. This makes an ID/epoch comparison inside the
-- binding transaction sufficient to detect requirement or interval drift.
CREATE FUNCTION enforce_identity_enrollment_policy_lineage()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.community_id <> OLD.community_id
       OR NEW.policy_id <> OLD.policy_id
       OR NEW.policy_epoch <= OLD.policy_epoch THEN
        RAISE EXCEPTION 'identity enrollment policy lineage must advance monotonically';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER identity_enrollment_policy_lineage_guard
BEFORE UPDATE ON identity_enrollment_policies
FOR EACH ROW EXECUTE FUNCTION enforce_identity_enrollment_policy_lineage();

ALTER TABLE identity_bindings
    ADD COLUMN binding_id UUID,
    ADD COLUMN binding_version BIGINT,
    ADD COLUMN binding_state TEXT,
    ADD COLUMN binding_provenance TEXT,
    ADD COLUMN replacement_binding_id UUID,
    ADD COLUMN created_by BYTEA,
    ADD COLUMN created_policy_version TEXT,
    ADD COLUMN expires_at TIMESTAMPTZ,
    ADD COLUMN creation_attribution_kind TEXT,
    ADD COLUMN archived_at TIMESTAMPTZ,
    ADD COLUMN archived_by BYTEA,
    ADD COLUMN archived_reason TEXT;

-- Every row receives a stable persisted identifier. Unique legacy exact pairs
-- use a reproducible length-prefixed hash. Byte-identical duplicate rows lack
-- a legacy row identifier, so they retain random persisted IDs and their exact
-- principal is quarantined below.
UPDATE identity_bindings
SET binding_id = gen_random_uuid();

WITH unique_pairs AS (
    SELECT
        community_id,
        issuer,
        uid,
        pubkey,
        (
            substr(fingerprint, 1, 8) || '-' ||
            substr(fingerprint, 9, 4) || '-' ||
            substr(fingerprint, 13, 4) || '-' ||
            substr(fingerprint, 17, 4) || '-' ||
            substr(fingerprint, 21, 12)
        )::UUID AS stable_id
    FROM (
        SELECT
            community_id,
            issuer,
            uid,
            pubkey,
            encode(
                digest(
                    E'\\x01'::BYTEA ||
                    uuid_send(community_id) ||
                    int8send(octet_length(convert_to(issuer, 'UTF8'))::BIGINT) ||
                    convert_to(issuer, 'UTF8') ||
                    int8send(octet_length(convert_to(uid, 'UTF8'))::BIGINT) ||
                    convert_to(uid, 'UTF8') ||
                    int8send(octet_length(pubkey)::BIGINT) ||
                    pubkey,
                    'sha256'
                ),
                'hex'
            ) AS fingerprint
        FROM identity_bindings
        GROUP BY community_id, issuer, uid, pubkey
        HAVING COUNT(*) = 1
    ) fingerprints
)
UPDATE identity_bindings binding
SET binding_id = unique_pairs.stable_id
FROM unique_pairs
WHERE binding.community_id = unique_pairs.community_id
  AND binding.issuer = unique_pairs.issuer
  AND binding.uid = unique_pairs.uid
  AND binding.pubkey = unique_pairs.pubkey;

UPDATE identity_bindings
SET binding_state = CASE
        WHEN rotation_completed_at IS NOT NULL THEN 'rotated'
        WHEN revoked_at IS NOT NULL THEN 'revoked'
        ELSE 'active'
    END,
    binding_provenance = CASE source
        WHEN 'jwt_npub' THEN 'attested_key'
        ELSE 'tofu'
    END,
    creation_attribution_kind = 'legacy_unknown';

ALTER TABLE identity_bindings
    ALTER COLUMN binding_id SET NOT NULL,
    ALTER COLUMN binding_id SET DEFAULT gen_random_uuid(),
    ALTER COLUMN binding_state SET NOT NULL,
    ALTER COLUMN binding_state SET DEFAULT 'active',
    ALTER COLUMN binding_provenance SET NOT NULL,
    ALTER COLUMN binding_provenance SET DEFAULT 'tofu',
    ALTER COLUMN creation_attribution_kind SET NOT NULL,
    ADD CONSTRAINT identity_bindings_binding_id_unique
        UNIQUE (community_id, binding_id),
    ADD CONSTRAINT chk_identity_bindings_id_not_nil
        CHECK (binding_id <> '00000000-0000-0000-0000-000000000000'::UUID),
    ADD CONSTRAINT chk_identity_bindings_state
        CHECK (binding_state IN ('active', 'revoked', 'rotated', 'archived')),
    ADD CONSTRAINT chk_identity_bindings_provenance
        CHECK (binding_provenance IN ('attested_key', 'provisioned', 'tofu')),
    ADD CONSTRAINT chk_identity_bindings_created_by_len
        CHECK (created_by IS NULL OR length(created_by) = 32),
    ADD CONSTRAINT chk_identity_bindings_policy_version
        CHECK (created_policy_version IS NULL OR length(created_policy_version) > 0),
    ADD CONSTRAINT chk_identity_bindings_expiry
        CHECK (expires_at IS NULL OR expires_at > TIMESTAMPTZ 'epoch'),
    ADD CONSTRAINT chk_identity_bindings_creation_attribution
        CHECK (
            (creation_attribution_kind = 'legacy_unknown'
                AND created_by IS NULL AND created_policy_version IS NULL)
            OR
            (creation_attribution_kind IN ('authenticated_key', 'operator')
                AND created_by IS NOT NULL AND length(created_by) = 32
                AND created_policy_version IS NOT NULL
                AND length(created_policy_version) > 0)
        ),
    ADD CONSTRAINT chk_identity_bindings_authority_state
        CHECK ((binding_state = 'active') = (revoked_at IS NULL)),
    ADD CONSTRAINT chk_identity_bindings_archive_attribution
        CHECK (
            (binding_state <> 'archived'
                AND archived_at IS NULL AND archived_by IS NULL AND archived_reason IS NULL)
            OR
            (binding_state = 'archived'
                AND archived_at IS NOT NULL
                AND archived_by IS NOT NULL AND length(archived_by) = 32
                AND archived_reason IS NOT NULL AND length(archived_reason) > 0)
        );

-- Preserve the checksum-frozen legacy indexes while making the authoritative
-- predicate explicit in the current catalog as well as in every authority read.
CREATE UNIQUE INDEX idx_identity_bindings_authoritative_principal
    ON identity_bindings (community_id, issuer, uid)
    WHERE binding_state = 'active' AND revoked_at IS NULL;
CREATE UNIQUE INDEX idx_identity_bindings_authoritative_pubkey
    ON identity_bindings (community_id, pubkey)
    WHERE binding_state = 'active' AND revoked_at IS NULL;

-- Invalid legacy graphs remain stored verbatim but are not usable as binding
-- authority. The binding subsystem exposes no operation that clears these
-- migration denials.
CREATE TABLE identity_migration_denials (
    community_id UUID NOT NULL REFERENCES communities(id),
    issuer       TEXT NOT NULL,
    subject      TEXT NOT NULL,
    reason       TEXT NOT NULL,
    detected_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, issuer, subject),
    CHECK (length(issuer) > 0),
    CHECK (length(subject) > 0),
    CHECK (length(reason) > 0)
);

INSERT INTO identity_migration_denials (community_id, issuer, subject, reason)
SELECT community_id, issuer, uid, 'duplicate legacy exact-pair rows'
FROM identity_bindings
GROUP BY community_id, issuer, uid, pubkey
HAVING COUNT(*) > 1
ON CONFLICT (community_id, issuer, subject) DO NOTHING;

-- A valid legacy principal is one complete, non-branching, acyclic chain. Edge
-- candidates include inactive replacements. Equality is intentional because
-- NOW() is transaction-stable in the legacy rotation helper; an older row can
-- never be selected as a successor.
WITH RECURSIVE
candidate_edges AS (
    SELECT
        predecessor.community_id,
        predecessor.issuer,
        predecessor.uid,
        predecessor.binding_id AS predecessor_id,
        successor.binding_id AS successor_id,
        COUNT(*) OVER (
            PARTITION BY predecessor.community_id, predecessor.binding_id
        ) AS outgoing_candidates
    FROM identity_bindings predecessor
    JOIN identity_bindings successor
      ON successor.community_id = predecessor.community_id
     AND successor.issuer = predecessor.issuer
     AND successor.uid = predecessor.uid
     AND successor.pubkey = predecessor.rotated_to_pubkey
     AND successor.binding_id <> predecessor.binding_id
     AND successor.created_at >= predecessor.rotation_completed_at
    WHERE predecessor.rotation_completed_at IS NOT NULL
),
resolved_edges AS (
    SELECT community_id, issuer, uid, predecessor_id, successor_id
    FROM candidate_edges
    WHERE outgoing_candidates = 1
),
principals AS (
    SELECT community_id, issuer, uid, COUNT(*)::BIGINT AS node_count
    FROM identity_bindings
    GROUP BY community_id, issuer, uid
),
roots AS (
    SELECT node.community_id, node.issuer, node.uid, node.binding_id
    FROM identity_bindings node
    WHERE NOT EXISTS (
        SELECT 1
        FROM resolved_edges edge
        WHERE edge.community_id = node.community_id
          AND edge.successor_id = node.binding_id
    )
),
reachable AS (
    SELECT root.community_id, root.issuer, root.uid, root.binding_id
    FROM roots root
    UNION
    SELECT edge.community_id, edge.issuer, edge.uid, edge.successor_id
    FROM reachable current_node
    JOIN resolved_edges edge
      ON edge.community_id = current_node.community_id
     AND edge.predecessor_id = current_node.binding_id
),
graph_stats AS (
    SELECT
        principal.community_id,
        principal.issuer,
        principal.uid,
        principal.node_count,
        COUNT(DISTINCT edge.predecessor_id)::BIGINT AS edge_count,
        COUNT(DISTINCT root.binding_id)::BIGINT AS root_count,
        COUNT(DISTINCT reached.binding_id)::BIGINT AS reached_count,
        COALESCE(MAX(incoming.incoming_count), 0)::BIGINT AS max_incoming
    FROM principals principal
    LEFT JOIN resolved_edges edge
      ON edge.community_id = principal.community_id
     AND edge.issuer = principal.issuer
     AND edge.uid = principal.uid
    LEFT JOIN roots root
      ON root.community_id = principal.community_id
     AND root.issuer = principal.issuer
     AND root.uid = principal.uid
    LEFT JOIN reachable reached
      ON reached.community_id = principal.community_id
     AND reached.issuer = principal.issuer
     AND reached.uid = principal.uid
    LEFT JOIN (
        SELECT community_id, issuer, uid, successor_id, COUNT(*)::BIGINT AS incoming_count
        FROM resolved_edges
        GROUP BY community_id, issuer, uid, successor_id
    ) incoming
      ON incoming.community_id = principal.community_id
     AND incoming.issuer = principal.issuer
     AND incoming.uid = principal.uid
    GROUP BY principal.community_id, principal.issuer, principal.uid, principal.node_count
),
unresolved_rotations AS (
    SELECT predecessor.community_id, predecessor.issuer, predecessor.uid
    FROM identity_bindings predecessor
    LEFT JOIN candidate_edges edge
      ON edge.community_id = predecessor.community_id
     AND edge.predecessor_id = predecessor.binding_id
    WHERE predecessor.rotation_completed_at IS NOT NULL
    GROUP BY predecessor.community_id, predecessor.issuer, predecessor.uid, predecessor.binding_id
    HAVING COUNT(edge.successor_id) <> 1
       OR COALESCE(MAX(edge.outgoing_candidates), 0) <> 1
),
invalid_principals AS (
    SELECT community_id, issuer, uid FROM unresolved_rotations
    UNION
    SELECT community_id, issuer, uid
    FROM graph_stats
    WHERE edge_count <> node_count - 1
       OR root_count <> 1
       OR reached_count <> node_count
       OR max_incoming > 1
)
INSERT INTO identity_migration_denials (community_id, issuer, subject, reason)
SELECT community_id, issuer, uid, 'ambiguous legacy replacement lineage'
FROM invalid_principals
ON CONFLICT (community_id, issuer, subject) DO NOTHING;

-- Principal quarantine alone is insufficient: a missing or ambiguous target
-- key must not be resurrected under a different principal in the same domain.
-- Retain a domain-key denial for every stored or referenced key implicated by
-- an invalid legacy graph.
CREATE TABLE identity_migration_denied_keys (
    community_id UUID NOT NULL REFERENCES communities(id),
    pubkey       BYTEA NOT NULL,
    reason       TEXT NOT NULL,
    detected_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, pubkey),
    CHECK (length(pubkey) = 32),
    CHECK (length(reason) > 0)
);

INSERT INTO identity_migration_denied_keys (community_id, pubkey, reason)
SELECT DISTINCT binding.community_id, binding.pubkey,
       'key implicated by ambiguous legacy replacement lineage'
FROM identity_bindings binding
JOIN identity_migration_denials denial
  ON denial.community_id = binding.community_id
 AND denial.issuer = binding.issuer
 AND denial.subject = binding.uid
UNION
SELECT DISTINCT binding.community_id, binding.rotated_to_pubkey,
       'key implicated by ambiguous legacy replacement lineage'
FROM identity_bindings binding
JOIN identity_migration_denials denial
  ON denial.community_id = binding.community_id
 AND denial.issuer = binding.issuer
 AND denial.subject = binding.uid
WHERE binding.rotated_to_pubkey IS NOT NULL;

-- A version belongs to one stable binding ID. Imported rows are snapshots of
-- distinct legacy binding IDs, so each starts at version 1; later transitions
-- increment only the binding ID whose authorization state changes.
UPDATE identity_bindings SET binding_version = 1;

ALTER TABLE identity_bindings
    ALTER COLUMN binding_version SET NOT NULL,
    ALTER COLUMN binding_version SET DEFAULT 1,
    ADD CONSTRAINT chk_identity_bindings_version_positive
        CHECK (binding_version > 0);

CREATE TABLE identity_binding_lineage (
    community_id          UUID NOT NULL REFERENCES communities(id),
    predecessor_binding_id UUID NOT NULL,
    successor_binding_id   UUID NOT NULL,
    imported_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, predecessor_binding_id),
    UNIQUE (community_id, successor_binding_id),
    FOREIGN KEY (community_id, predecessor_binding_id)
        REFERENCES identity_bindings (community_id, binding_id),
    FOREIGN KEY (community_id, successor_binding_id)
        REFERENCES identity_bindings (community_id, binding_id),
    CHECK (predecessor_binding_id <> successor_binding_id)
);

WITH candidate_edges AS (
    SELECT
        predecessor.community_id,
        predecessor.issuer,
        predecessor.uid,
        predecessor.binding_id AS predecessor_id,
        successor.binding_id AS successor_id,
        COUNT(*) OVER (
            PARTITION BY predecessor.community_id, predecessor.binding_id
        ) AS outgoing_candidates
    FROM identity_bindings predecessor
    JOIN identity_bindings successor
      ON successor.community_id = predecessor.community_id
     AND successor.issuer = predecessor.issuer
     AND successor.uid = predecessor.uid
     AND successor.pubkey = predecessor.rotated_to_pubkey
     AND successor.binding_id <> predecessor.binding_id
     AND successor.created_at >= predecessor.rotation_completed_at
    WHERE predecessor.rotation_completed_at IS NOT NULL
)
INSERT INTO identity_binding_lineage
    (community_id, predecessor_binding_id, successor_binding_id)
SELECT edge.community_id, edge.predecessor_id, edge.successor_id
FROM candidate_edges edge
WHERE edge.outgoing_candidates = 1
  AND NOT EXISTS (
      SELECT 1 FROM identity_migration_denials denial
      WHERE denial.community_id = edge.community_id
        AND denial.issuer = edge.issuer
        AND denial.subject = edge.uid
  );

UPDATE identity_bindings predecessor
SET replacement_binding_id = lineage.successor_binding_id
FROM identity_binding_lineage lineage
WHERE lineage.community_id = predecessor.community_id
  AND lineage.predecessor_binding_id = predecessor.binding_id;

-- A legacy row whose successor could not be proven is inactive but not a
-- completed rotation. Keep its legacy fields and quarantine intact while
-- projecting the truthful binding state as revoked.
UPDATE identity_bindings
SET binding_state = 'revoked'
WHERE binding_state = 'rotated' AND replacement_binding_id IS NULL;

ALTER TABLE identity_bindings
    ADD CONSTRAINT identity_bindings_replacement_fk
        FOREIGN KEY (community_id, replacement_binding_id)
        REFERENCES identity_bindings (community_id, binding_id)
        DEFERRABLE INITIALLY DEFERRED,
    ADD CONSTRAINT chk_identity_bindings_rotated_lineage
        CHECK (binding_state <> 'rotated' OR replacement_binding_id IS NOT NULL);

CREATE TABLE identity_retired_pairs (
    community_id           UUID NOT NULL REFERENCES communities(id),
    issuer                 TEXT NOT NULL,
    subject                TEXT NOT NULL,
    pubkey                 BYTEA NOT NULL,
    retired_binding_id     UUID,
    retired_binding_version BIGINT,
    retired_at             TIMESTAMPTZ NOT NULL,
    retired_by             BYTEA,
    reason                 TEXT NOT NULL,
    PRIMARY KEY (community_id, issuer, subject, pubkey),
    UNIQUE (
        community_id, issuer, subject, pubkey,
        retired_binding_id, retired_binding_version
    ),
    FOREIGN KEY (community_id, retired_binding_id)
        REFERENCES identity_bindings (community_id, binding_id),
    CHECK (length(issuer) > 0),
    CHECK (length(subject) > 0),
    CHECK (length(pubkey) = 32),
    CHECK (retired_binding_version IS NULL OR retired_binding_version > 0),
    CHECK (
        (retired_binding_id IS NULL AND retired_binding_version IS NULL)
        OR
        (retired_binding_id IS NOT NULL AND retired_binding_version IS NOT NULL)
    ),
    CHECK (retired_by IS NULL OR length(retired_by) = 32),
    CHECK (length(reason) > 0)
);

INSERT INTO identity_retired_pairs
    (community_id, issuer, subject, pubkey, retired_binding_id,
     retired_binding_version, retired_at, retired_by, reason)
SELECT
    community_id,
    issuer,
    uid,
    pubkey,
    CASE WHEN COUNT(*) = 1 THEN (array_agg(binding_id))[1] END,
    CASE WHEN COUNT(*) = 1 THEN (array_agg(binding_version))[1] END,
    MIN(COALESCE(revoked_at, rotation_completed_at, updated_at)),
    CASE WHEN COUNT(*) = 1 THEN (array_agg(COALESCE(rotation_by, revoked_by)))[1] END,
    MIN(COALESCE(NULLIF(rotation_reason, ''), NULLIF(revoked_reason, ''), 'legacy pair retirement'))
FROM identity_bindings
WHERE revoked_at IS NOT NULL
GROUP BY community_id, issuer, uid, pubkey;

-- Q is append-only history. cleared_at marks selector absence while retaining
-- the selector version so recreation of the same tuple cannot cause ABA.
CREATE TABLE identity_pending_replacements (
    community_id           UUID NOT NULL REFERENCES communities(id),
    issuer                 TEXT NOT NULL,
    subject                TEXT NOT NULL,
    selector_version       BIGINT NOT NULL,
    retired_pubkey         BYTEA NOT NULL,
    retired_binding_id     UUID NOT NULL,
    retired_binding_version BIGINT NOT NULL,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_operation_id   UUID,
    cleared_at             TIMESTAMPTZ,
    cleared_operation_id   UUID,
    PRIMARY KEY (community_id, issuer, subject, selector_version),
    FOREIGN KEY (
        community_id, issuer, subject, retired_pubkey,
        retired_binding_id, retired_binding_version
    ) REFERENCES identity_retired_pairs (
        community_id, issuer, subject, pubkey,
        retired_binding_id, retired_binding_version
    ),
    CHECK (length(issuer) > 0),
    CHECK (length(subject) > 0),
    CHECK (selector_version > 0),
    CHECK (length(retired_pubkey) = 32),
    CHECK (retired_binding_version > 0),
    CHECK (
        (cleared_at IS NULL AND cleared_operation_id IS NULL)
        OR
        (cleared_at IS NOT NULL AND cleared_operation_id IS NOT NULL)
    )
);

CREATE UNIQUE INDEX idx_identity_pending_replacements_active
    ON identity_pending_replacements (community_id, issuer, subject)
    WHERE cleared_at IS NULL;

INSERT INTO identity_pending_replacements
    (community_id, issuer, subject, selector_version, retired_pubkey,
     retired_binding_id, retired_binding_version)
SELECT
    terminal.community_id,
    terminal.issuer,
    terminal.uid,
    1,
    terminal.pubkey,
    terminal.binding_id,
    terminal.binding_version
FROM identity_bindings terminal
WHERE terminal.revoked_at IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM identity_bindings active
      WHERE active.community_id = terminal.community_id
        AND active.issuer = terminal.issuer
        AND active.uid = terminal.uid
        AND active.binding_state = 'active'
        AND active.revoked_at IS NULL
  )
  AND NOT EXISTS (
      SELECT 1 FROM identity_binding_lineage lineage
      WHERE lineage.community_id = terminal.community_id
        AND lineage.predecessor_binding_id = terminal.binding_id
  )
  AND NOT EXISTS (
      SELECT 1 FROM identity_migration_denials denial
      WHERE denial.community_id = terminal.community_id
        AND denial.issuer = terminal.issuer
        AND denial.subject = terminal.uid
  );

CREATE TABLE identity_binding_history (
    community_id          UUID NOT NULL REFERENCES communities(id),
    history_id            UUID NOT NULL DEFAULT gen_random_uuid(),
    binding_id            UUID NOT NULL,
    binding_version       BIGINT NOT NULL,
    issuer                TEXT NOT NULL,
    subject               TEXT NOT NULL,
    pubkey                BYTEA NOT NULL,
    binding_state         TEXT NOT NULL,
    binding_provenance    TEXT NOT NULL,
    transition_kind       TEXT NOT NULL,
    replacement_binding_id UUID,
    operation_id          UUID,
    actor                 BYTEA,
    reason                TEXT NOT NULL,
    recorded_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, history_id),
    UNIQUE (community_id, binding_id, binding_version, transition_kind),
    FOREIGN KEY (community_id, binding_id)
        REFERENCES identity_bindings (community_id, binding_id),
    FOREIGN KEY (community_id, replacement_binding_id)
        REFERENCES identity_bindings (community_id, binding_id)
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (history_id <> '00000000-0000-0000-0000-000000000000'::UUID),
    CHECK (binding_version > 0),
    CHECK (length(issuer) > 0),
    CHECK (length(subject) > 0),
    CHECK (length(pubkey) = 32),
    CHECK (binding_state IN ('active', 'revoked', 'rotated', 'archived')),
    CHECK (binding_provenance IN ('attested_key', 'provisioned', 'tofu')),
    CHECK (transition_kind IN (
        'legacy_import', 'enroll', 'provision', 'provenance_strengthened',
        'retire_pair', 'disable_identity', 'revoke_key', 'rotate',
        'recover', 'enable_identity', 'archive'
    )),
    CHECK (actor IS NULL OR length(actor) = 32),
    CHECK (length(reason) > 0)
);

CREATE INDEX idx_identity_binding_history_principal
    ON identity_binding_history (community_id, issuer, subject, recorded_at);

INSERT INTO identity_binding_history
    (community_id, binding_id, binding_version, issuer, subject, pubkey,
     binding_state, binding_provenance, transition_kind,
     replacement_binding_id, actor, reason, recorded_at)
SELECT
    community_id,
    binding_id,
    binding_version,
    issuer,
    uid,
    pubkey,
    binding_state,
    binding_provenance,
    'legacy_import',
    replacement_binding_id,
    COALESCE(rotation_by, revoked_by),
    COALESCE(NULLIF(rotation_reason, ''), NULLIF(revoked_reason, ''), 'legacy import'),
    updated_at
FROM identity_bindings;

-- Idempotency and local state history only. Authorization and complete
-- operator audit authority remain outside the binding persistence layer.
CREATE TABLE identity_lifecycle_operations (
    community_id          UUID NOT NULL REFERENCES communities(id),
    operation_id          UUID NOT NULL,
    operation_kind        TEXT NOT NULL,
    request_fingerprint   BYTEA NOT NULL,
    issuer                TEXT,
    subject               TEXT,
    pubkey                BYTEA,
    replacement_pubkey    BYTEA,
    binding_id            UUID,
    replacement_binding_id UUID,
    binding_version       BIGINT,
    replacement_binding_version BIGINT,
    selector_version      BIGINT,
    actor                 BYTEA NOT NULL,
    reason                TEXT NOT NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, operation_id),
    FOREIGN KEY (community_id, binding_id)
        REFERENCES identity_bindings (community_id, binding_id),
    FOREIGN KEY (community_id, replacement_binding_id)
        REFERENCES identity_bindings (community_id, binding_id),
    CHECK (operation_id <> '00000000-0000-0000-0000-000000000000'::UUID),
    CHECK (operation_kind IN (
        'provision', 'retire_pair', 'disable_identity', 'revoke_key',
        'rotate', 'recover', 'enable_identity', 'archive'
    )),
    CHECK (length(request_fingerprint) = 32),
    CHECK (issuer IS NULL OR length(issuer) > 0),
    CHECK (subject IS NULL OR length(subject) > 0),
    CHECK (pubkey IS NULL OR length(pubkey) = 32),
    CHECK (replacement_pubkey IS NULL OR length(replacement_pubkey) = 32),
    CHECK (binding_version IS NULL OR binding_version > 0),
    CHECK (replacement_binding_version IS NULL OR replacement_binding_version > 0),
    CHECK (selector_version IS NULL OR selector_version > 0),
    CHECK (length(actor) = 32),
    CHECK (length(reason) > 0)
);

CREATE INDEX idx_identity_lifecycle_operations_principal
    ON identity_lifecycle_operations (community_id, issuer, subject, created_at);

CREATE INDEX idx_identity_lifecycle_operations_key
    ON identity_lifecycle_operations (community_id, pubkey, created_at);
