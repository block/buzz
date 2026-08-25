ALTER TABLE agent_read_delegations
  DROP CONSTRAINT agent_read_delegations_scopes_check,
  ADD CONSTRAINT agent_read_delegations_scopes_check
    CHECK (cardinality(scopes) BETWEEN 1 AND 8);

CREATE TABLE business_action_state (
  singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
  state jsonb NOT NULL CHECK (jsonb_typeof(state) = 'object'),
  version bigint NOT NULL DEFAULT 1 CHECK (version > 0),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE business_anomaly_findings (
  id uuid PRIMARY KEY,
  finding_key text NOT NULL UNIQUE CHECK (finding_key ~ '^[0-9a-f]{64}$'),
  legal_entity_id text NOT NULL,
  scope_hash text NOT NULL CHECK (scope_hash ~ '^[0-9a-f]{64}$'),
  rule_set_version text NOT NULL,
  condition_status text NOT NULL CHECK (condition_status IN ('active','cleared')),
  review_status text NOT NULL CHECK (review_status IN ('unreviewed','acknowledged','in_progress','resolved','dismissed','reopened')),
  occurrence_count bigint NOT NULL CHECK (occurrence_count > 0),
  first_seen_at timestamptz NOT NULL,
  last_seen_at timestamptz NOT NULL,
  cleared_at timestamptz,
  resolved_at timestamptz,
  dismissed_at timestamptz,
  review_after timestamptz,
  finding_snapshot_hash text NOT NULL CHECK (finding_snapshot_hash ~ '^[0-9a-f]{64}$'),
  version bigint NOT NULL CHECK (version > 0),
  trace_id uuid NOT NULL,
  payload jsonb NOT NULL CHECK (jsonb_typeof(payload) = 'object')
);

CREATE TABLE business_action_catalog_versions (
  version text PRIMARY KEY,
  config_hash text NOT NULL CHECK (config_hash ~ '^[0-9a-f]{64}$'),
  effective_from timestamptz NOT NULL,
  effective_to timestamptz,
  enabled boolean NOT NULL,
  payload jsonb NOT NULL CHECK (jsonb_typeof(payload) = 'array')
);

CREATE TABLE business_action_proposals (
  id uuid PRIMARY KEY,
  finding_id uuid NOT NULL REFERENCES business_anomaly_findings(id),
  action_catalog_version text NOT NULL REFERENCES business_action_catalog_versions(version),
  action_code text NOT NULL CHECK (action_code ~ '^[a-z][a-z_]{1,79}$'),
  status text NOT NULL CHECK (status IN ('suggested','accepted','dismissed','expired','superseded')),
  finding_version bigint NOT NULL CHECK (finding_version > 0),
  finding_snapshot_hash text NOT NULL CHECK (finding_snapshot_hash ~ '^[0-9a-f]{64}$'),
  proposal_hash text NOT NULL CHECK (proposal_hash ~ '^[0-9a-f]{64}$'),
  created_at timestamptz NOT NULL,
  expires_at timestamptz NOT NULL,
  trace_id uuid NOT NULL,
  version bigint NOT NULL CHECK (version > 0),
  payload jsonb NOT NULL CHECK (jsonb_typeof(payload) = 'object'),
  UNIQUE (finding_id, action_code, finding_version, action_catalog_version)
);

CREATE TABLE business_work_item_drafts (
  id uuid PRIMARY KEY,
  proposal_id uuid NOT NULL REFERENCES business_action_proposals(id),
  finding_id uuid NOT NULL REFERENCES business_anomaly_findings(id),
  status text NOT NULL CHECK (status IN ('prepared','consumed','expired','revoked')),
  preview_hash text NOT NULL CHECK (preview_hash ~ '^[0-9a-f]{64}$'),
  finding_snapshot_hash text NOT NULL CHECK (finding_snapshot_hash ~ '^[0-9a-f]{64}$'),
  created_by_user_id uuid NOT NULL REFERENCES enterprise_users(id),
  created_at timestamptz NOT NULL,
  expires_at timestamptz NOT NULL,
  trace_id uuid NOT NULL,
  payload jsonb NOT NULL CHECK (jsonb_typeof(payload) = 'object')
);

CREATE TABLE business_work_items (
  id uuid PRIMARY KEY,
  work_item_number text NOT NULL UNIQUE CHECK (work_item_number ~ '^WI-[0-9]{6,}$'),
  finding_id uuid NOT NULL REFERENCES business_anomaly_findings(id),
  proposal_id uuid NOT NULL REFERENCES business_action_proposals(id),
  action_code text NOT NULL CHECK (action_code ~ '^[a-z][a-z_]{1,79}$'),
  status text NOT NULL CHECK (status IN ('open','in_progress','blocked','ready_for_review','completed','cancelled','reopened')),
  assignee_user_id uuid REFERENCES enterprise_users(id),
  assignee_role_key text,
  created_by_user_id uuid NOT NULL REFERENCES enterprise_users(id),
  due_at timestamptz NOT NULL,
  source_condition_status text NOT NULL CHECK (source_condition_status IN ('active','cleared')),
  finding_snapshot_hash text NOT NULL CHECK (finding_snapshot_hash ~ '^[0-9a-f]{64}$'),
  created_at timestamptz NOT NULL,
  updated_at timestamptz NOT NULL,
  version bigint NOT NULL CHECK (version > 0),
  trace_id uuid NOT NULL,
  payload jsonb NOT NULL CHECK (jsonb_typeof(payload) = 'object')
);

CREATE UNIQUE INDEX business_work_items_one_active_action
  ON business_work_items(finding_id, action_code)
  WHERE status NOT IN ('completed','cancelled');

CREATE TABLE business_work_item_events (
  id uuid PRIMARY KEY,
  work_item_id uuid NOT NULL REFERENCES business_work_items(id),
  event_type text NOT NULL CHECK (event_type IN ('created','assigned','started','blocked','unblocked','ready_for_review','completed','cancelled','reopened','comment_added','source_condition_changed','source_reactivated')),
  actor_user_id uuid REFERENCES enterprise_users(id),
  occurred_at timestamptz NOT NULL,
  trace_id uuid NOT NULL,
  payload jsonb NOT NULL CHECK (jsonb_typeof(payload) = 'object')
);

CREATE OR REPLACE FUNCTION deny_business_work_item_event_mutation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'business_work_item_events is append-only'; END $$;
CREATE TRIGGER business_work_item_events_no_update_delete
  BEFORE UPDATE OR DELETE ON business_work_item_events
  FOR EACH ROW EXECUTE FUNCTION deny_business_work_item_event_mutation();

CREATE TABLE business_approval_draft_previews (
  id uuid PRIMARY KEY,
  work_item_id uuid NOT NULL REFERENCES business_work_items(id),
  finding_id uuid NOT NULL REFERENCES business_anomaly_findings(id),
  preview_hash text NOT NULL CHECK (preview_hash ~ '^[0-9a-f]{64}$'),
  created_by_user_id uuid NOT NULL REFERENCES enterprise_users(id),
  created_at timestamptz NOT NULL,
  expires_at timestamptz NOT NULL,
  consumed boolean NOT NULL DEFAULT false,
  trace_id uuid NOT NULL,
  payload jsonb NOT NULL CHECK (jsonb_typeof(payload) = 'object')
);

CREATE TABLE business_approval_drafts (
  id uuid PRIMARY KEY,
  approval_draft_number text NOT NULL UNIQUE CHECK (approval_draft_number ~ '^AD-[0-9]{6,}$'),
  work_item_id uuid NOT NULL REFERENCES business_work_items(id),
  finding_id uuid NOT NULL REFERENCES business_anomaly_findings(id),
  action_code text NOT NULL CHECK (action_code ~ '^[a-z][a-z_]{1,79}$'),
  draft_type text NOT NULL,
  status text NOT NULL CHECK (status IN ('draft','ready_for_review','withdrawn','expired','superseded')),
  draft_only boolean NOT NULL DEFAULT true CHECK (draft_only),
  source_snapshot_hash text NOT NULL CHECK (source_snapshot_hash ~ '^[0-9a-f]{64}$'),
  draft_hash text NOT NULL CHECK (draft_hash ~ '^[0-9a-f]{64}$'),
  created_by_user_id uuid NOT NULL REFERENCES enterprise_users(id),
  created_at timestamptz NOT NULL,
  updated_at timestamptz NOT NULL,
  expires_at timestamptz NOT NULL,
  version bigint NOT NULL CHECK (version > 0),
  trace_id uuid NOT NULL,
  payload jsonb NOT NULL CHECK (jsonb_typeof(payload) = 'object')
);

CREATE TABLE business_approval_draft_evidence (
  id uuid PRIMARY KEY,
  approval_draft_id uuid NOT NULL REFERENCES business_approval_drafts(id),
  evidence_type text NOT NULL,
  source_snapshot_hash text NOT NULL CHECK (source_snapshot_hash ~ '^[0-9a-f]{64}$'),
  payload jsonb NOT NULL CHECK (jsonb_typeof(payload) = 'object')
);

CREATE TABLE business_action_idempotency (
  enterprise_user_id uuid NOT NULL REFERENCES enterprise_users(id),
  idempotency_key text NOT NULL CHECK (length(idempotency_key) BETWEEN 16 AND 128),
  request_hash text NOT NULL CHECK (request_hash ~ '^[0-9a-f]{64}$'),
  entity_id uuid NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (enterprise_user_id, idempotency_key)
);

CREATE TABLE business_action_audit_events (
  id uuid PRIMARY KEY,
  occurred_at timestamptz NOT NULL,
  event_type text NOT NULL,
  result text NOT NULL CHECK (result IN ('success','failure')),
  entity_id uuid,
  action_code text,
  status text,
  entity_hash text CHECK (entity_hash IS NULL OR entity_hash ~ '^[0-9a-f]{64}$'),
  enterprise_user_id uuid REFERENCES enterprise_users(id),
  reason_code text,
  entity_version bigint,
  trace_id uuid NOT NULL,
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(metadata) = 'object')
);

CREATE INDEX business_anomaly_findings_status_idx ON business_anomaly_findings(condition_status, review_status);
CREATE INDEX business_anomaly_findings_last_seen_idx ON business_anomaly_findings(last_seen_at DESC);
CREATE INDEX business_action_proposals_expiry_idx ON business_action_proposals(expires_at) WHERE status='suggested';
CREATE INDEX business_work_item_drafts_expiry_idx ON business_work_item_drafts(expires_at) WHERE status='prepared';
CREATE INDEX business_work_items_assignee_idx ON business_work_items(assignee_user_id, status);
CREATE INDEX business_work_items_due_idx ON business_work_items(due_at) WHERE status NOT IN ('completed','cancelled');
CREATE INDEX business_approval_drafts_expiry_idx ON business_approval_drafts(expires_at) WHERE status IN ('draft','ready_for_review');
CREATE INDEX business_action_audit_trace_idx ON business_action_audit_events(trace_id);

CREATE OR REPLACE FUNCTION deny_business_action_audit_mutation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'business_action_audit_events is append-only'; END $$;
CREATE TRIGGER business_action_audit_no_update_delete
  BEFORE UPDATE OR DELETE ON business_action_audit_events
  FOR EACH ROW EXECUTE FUNCTION deny_business_action_audit_mutation();
