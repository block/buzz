ALTER TABLE agent_read_delegations
  DROP CONSTRAINT agent_read_delegations_scopes_check,
  ADD CONSTRAINT agent_read_delegations_scopes_check
    CHECK (cardinality(scopes) BETWEEN 1 AND 7);

ALTER TABLE security_audit_events
  ADD COLUMN response_buzz_event_id text,
  ADD COLUMN finding_count integer,
  ADD COLUMN resource_ref_count integer,
  ADD COLUMN rule_set_version text,
  ADD COLUMN anomaly_run_id uuid;

CREATE INDEX security_audit_events_anomaly_run_idx
  ON security_audit_events (anomaly_run_id)
  WHERE anomaly_run_id IS NOT NULL;
