CREATE TABLE enterprise_users (
  id uuid PRIMARY KEY,
  oidc_issuer text NOT NULL,
  oidc_subject text NOT NULL,
  email text,
  display_name text NOT NULL,
  status text NOT NULL DEFAULT 'active' CHECK (status IN ('active','disabled')),
  oidc_sid text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  last_login_at timestamptz NOT NULL DEFAULT now(),
  version bigint NOT NULL DEFAULT 1 CHECK (version > 0),
  UNIQUE (oidc_issuer, oidc_subject)
);

CREATE TABLE workbench_sessions (
  id uuid PRIMARY KEY,
  enterprise_user_id uuid NOT NULL REFERENCES enterprise_users(id),
  oidc_sid text,
  status text NOT NULL DEFAULT 'active' CHECK (status IN ('active','revoked','expired')),
  created_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz NOT NULL,
  last_seen_at timestamptz NOT NULL DEFAULT now(),
  revoked_at timestamptz,
  trace_id uuid NOT NULL
);

CREATE TABLE buzz_identity_bindings (
  id uuid PRIMARY KEY,
  enterprise_user_id uuid NOT NULL REFERENCES enterprise_users(id),
  buzz_pubkey text NOT NULL CHECK (buzz_pubkey ~ '^[0-9a-f]{64}$'),
  device_id text NOT NULL CHECK (length(device_id) BETWEEN 8 AND 200),
  device_name text NOT NULL CHECK (length(device_name) BETWEEN 1 AND 200),
  device_platform text NOT NULL CHECK (device_platform IN ('macos','windows','linux','web')),
  status text NOT NULL DEFAULT 'active' CHECK (status IN ('active','revoked')),
  bound_at timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz NOT NULL DEFAULT now(),
  revoked_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version bigint NOT NULL DEFAULT 1 CHECK (version > 0)
);
CREATE UNIQUE INDEX buzz_identity_bindings_active_pubkey
  ON buzz_identity_bindings (buzz_pubkey) WHERE status = 'active';
CREATE UNIQUE INDEX buzz_identity_bindings_active_device
  ON buzz_identity_bindings (enterprise_user_id, device_id) WHERE status = 'active';

CREATE TABLE identity_binding_challenges (
  id uuid PRIMARY KEY,
  enterprise_user_id uuid NOT NULL REFERENCES enterprise_users(id),
  requested_pubkey text NOT NULL CHECK (requested_pubkey ~ '^[0-9a-f]{64}$'),
  device_id text NOT NULL,
  device_name text NOT NULL,
  device_platform text NOT NULL CHECK (device_platform IN ('macos','windows','linux','web')),
  challenge_hash bytea NOT NULL UNIQUE CHECK (octet_length(challenge_hash) = 32),
  canonical_payload text NOT NULL,
  audience text NOT NULL CHECK (audience = 'bizfin-workbench-device-binding'),
  status text NOT NULL DEFAULT 'active' CHECK (status IN ('active','consumed','expired','revoked')),
  created_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz NOT NULL,
  consumed_at timestamptz,
  failed_attempts integer NOT NULL DEFAULT 0 CHECK (failed_attempts >= 0),
  created_ip inet,
  trace_id uuid NOT NULL
);

CREATE TABLE embed_sessions (
  id uuid PRIMARY KEY,
  code_hash bytea NOT NULL UNIQUE CHECK (octet_length(code_hash) = 32),
  enterprise_user_id uuid NOT NULL REFERENCES enterprise_users(id),
  identity_binding_id uuid NOT NULL REFERENCES buzz_identity_bindings(id),
  workbench_session_id uuid NOT NULL REFERENCES workbench_sessions(id),
  oidc_sid text,
  audience text NOT NULL CHECK (audience = 'business-dock'),
  deployment_id text NOT NULL,
  target_path text NOT NULL,
  target_resource_type text NOT NULL,
  target_resource_id text NOT NULL,
  status text NOT NULL DEFAULT 'active' CHECK (status IN ('active','consumed','expired','revoked')),
  created_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz NOT NULL,
  consumed_at timestamptz,
  revoked_at timestamptz,
  created_ip inet,
  consumed_ip inet,
  user_agent_hash bytea CHECK (user_agent_hash IS NULL OR octet_length(user_agent_hash) = 32),
  trace_id uuid NOT NULL,
  version bigint NOT NULL DEFAULT 1 CHECK (version > 0)
);

CREATE TABLE business_sessions (
  id uuid PRIMARY KEY,
  session_token_hash bytea NOT NULL UNIQUE CHECK (octet_length(session_token_hash) = 32),
  csrf_token_hash bytea NOT NULL CHECK (octet_length(csrf_token_hash) = 32),
  enterprise_user_id uuid NOT NULL REFERENCES enterprise_users(id),
  identity_binding_id uuid NOT NULL REFERENCES buzz_identity_bindings(id),
  workbench_session_id uuid NOT NULL REFERENCES workbench_sessions(id),
  embed_session_id uuid NOT NULL UNIQUE REFERENCES embed_sessions(id),
  oidc_sid text,
  status text NOT NULL DEFAULT 'active' CHECK (status IN ('active','expired','revoked')),
  created_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz NOT NULL,
  last_seen_at timestamptz NOT NULL DEFAULT now(),
  revoked_at timestamptz,
  created_ip inet,
  user_agent_hash bytea CHECK (user_agent_hash IS NULL OR octet_length(user_agent_hash) = 32),
  trace_id uuid NOT NULL
);

CREATE TABLE security_audit_events (
  id uuid PRIMARY KEY,
  occurred_at timestamptz NOT NULL DEFAULT now(),
  event_type text NOT NULL,
  result text NOT NULL CHECK (result IN ('success','failure')),
  reason_code text,
  enterprise_user_id uuid REFERENCES enterprise_users(id),
  oidc_issuer text,
  oidc_subject text,
  identity_binding_id uuid REFERENCES buzz_identity_bindings(id),
  buzz_pubkey_short text,
  device_id text,
  workbench_session_id uuid REFERENCES workbench_sessions(id),
  embed_session_id uuid REFERENCES embed_sessions(id),
  business_session_id uuid REFERENCES business_sessions(id),
  target_resource_type text,
  target_resource_id text,
  source_ip inet,
  user_agent_hash bytea,
  trace_id uuid NOT NULL,
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(metadata) = 'object')
);

CREATE INDEX enterprise_users_oidc_sid_idx ON enterprise_users (oidc_sid) WHERE oidc_sid IS NOT NULL;
CREATE INDEX workbench_sessions_expiry_idx ON workbench_sessions (expires_at) WHERE status = 'active';
CREATE INDEX workbench_sessions_sid_idx ON workbench_sessions (oidc_sid) WHERE oidc_sid IS NOT NULL;
CREATE INDEX identity_binding_challenges_expiry_idx ON identity_binding_challenges (expires_at) WHERE status = 'active';
CREATE INDEX embed_sessions_expiry_idx ON embed_sessions (expires_at) WHERE status = 'active';
CREATE INDEX embed_sessions_workbench_idx ON embed_sessions (workbench_session_id, status);
CREATE INDEX business_sessions_expiry_idx ON business_sessions (expires_at) WHERE status = 'active';
CREATE INDEX business_sessions_binding_idx ON business_sessions (identity_binding_id, status);
CREATE INDEX security_audit_events_time_idx ON security_audit_events (occurred_at DESC);
CREATE INDEX security_audit_events_trace_idx ON security_audit_events (trace_id);

CREATE OR REPLACE FUNCTION deny_security_audit_mutation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'security_audit_events is append-only'; END $$;
CREATE TRIGGER security_audit_events_no_update_delete
  BEFORE UPDATE OR DELETE ON security_audit_events
  FOR EACH ROW EXECUTE FUNCTION deny_security_audit_mutation();
