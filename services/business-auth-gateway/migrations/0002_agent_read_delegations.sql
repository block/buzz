CREATE TABLE agent_read_delegations (
  id uuid PRIMARY KEY,
  token_hash bytea NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
  enterprise_user_id uuid NOT NULL REFERENCES enterprise_users(id),
  identity_binding_id uuid NOT NULL REFERENCES buzz_identity_bindings(id),
  agent_id text NOT NULL CHECK (length(agent_id) BETWEEN 1 AND 128),
  agent_turn_id text NOT NULL CHECK (length(agent_turn_id) BETWEEN 1 AND 128),
  source_buzz_event_id text NOT NULL CHECK (source_buzz_event_id ~ '^[0-9a-f]{64}$'),
  source_channel_id text NOT NULL CHECK (length(source_channel_id) BETWEEN 1 AND 200),
  audience text NOT NULL CHECK (audience = 'business-read-mcp'),
  scopes text[] NOT NULL CHECK (cardinality(scopes) BETWEEN 1 AND 6),
  data_scope_hash bytea NOT NULL CHECK (octet_length(data_scope_hash) = 32),
  status text NOT NULL DEFAULT 'active' CHECK (status IN ('active','expired','revoked','exhausted')),
  max_calls integer NOT NULL CHECK (max_calls BETWEEN 1 AND 100),
  used_calls integer NOT NULL DEFAULT 0 CHECK (used_calls BETWEEN 0 AND max_calls),
  created_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz NOT NULL,
  last_used_at timestamptz,
  revoked_at timestamptz,
  trace_id uuid NOT NULL,
  version bigint NOT NULL DEFAULT 1 CHECK (version > 0),
  UNIQUE (source_buzz_event_id, agent_id)
);

CREATE INDEX agent_read_delegations_expiry_idx
  ON agent_read_delegations (expires_at) WHERE status = 'active';
CREATE INDEX agent_read_delegations_binding_idx
  ON agent_read_delegations (identity_binding_id, status);
CREATE INDEX agent_read_delegations_turn_idx
  ON agent_read_delegations (agent_turn_id, status);

ALTER TABLE security_audit_events
  ADD COLUMN delegation_id uuid REFERENCES agent_read_delegations(id),
  ADD COLUMN agent_id text,
  ADD COLUMN agent_turn_id text,
  ADD COLUMN source_buzz_event_id text,
  ADD COLUMN source_channel_id text,
  ADD COLUMN tool_name text,
  ADD COLUMN result_count integer,
  ADD COLUMN duration_ms bigint;

CREATE INDEX security_audit_events_delegation_idx
  ON security_audit_events (delegation_id) WHERE delegation_id IS NOT NULL;
