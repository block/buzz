CREATE SCHEMA business_iam;

CREATE TABLE business_iam.principals (
  id uuid PRIMARY KEY,
  kind text NOT NULL CHECK (kind IN ('human','independent_agent','proxy_agent')),
  external_id text NOT NULL CHECK (length(external_id) BETWEEN 1 AND 200),
  display_name text NOT NULL CHECK (length(display_name) BETWEEN 1 AND 200),
  status text NOT NULL DEFAULT 'active' CHECK (status IN ('active','disabled')),
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(metadata) = 'object'),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  disabled_at timestamptz,
  version bigint NOT NULL DEFAULT 1 CHECK (version > 0),
  UNIQUE (kind, external_id),
  CHECK ((status = 'disabled') = (disabled_at IS NOT NULL))
);

CREATE UNIQUE INDEX business_iam_principals_active_external_id
  ON business_iam.principals (external_id) WHERE status = 'active';

CREATE TABLE business_iam.roles (
  id uuid PRIMARY KEY,
  code text NOT NULL UNIQUE CHECK (code ~ '^[a-z][a-z0-9_.:-]{2,127}$'),
  name text NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
  status text NOT NULL DEFAULT 'active' CHECK (status IN ('active','disabled')),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version bigint NOT NULL DEFAULT 1 CHECK (version > 0)
);

CREATE TABLE business_iam.permissions (
  id uuid PRIMARY KEY,
  capability text NOT NULL UNIQUE CHECK (capability ~ '^[a-z0-9_.-]+:[a-z0-9_.-]+$'),
  resource_type text NOT NULL CHECK (resource_type ~ '^[a-z][a-z0-9_.-]{1,127}$'),
  action text NOT NULL CHECK (action ~ '^[a-z][a-z0-9_.-]{1,63}$'),
  default_data_scope jsonb NOT NULL DEFAULT '{"mode":"unrestricted"}'::jsonb
    CHECK (jsonb_typeof(default_data_scope) = 'object'),
  obligations jsonb NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(obligations) = 'array'),
  risk_level text NOT NULL DEFAULT 'low' CHECK (risk_level IN ('low','medium','high','critical')),
  status text NOT NULL DEFAULT 'active' CHECK (status IN ('active','disabled')),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  version bigint NOT NULL DEFAULT 1 CHECK (version > 0),
  CHECK (capability = resource_type || ':' || action)
);

CREATE TABLE business_iam.role_permissions (
  role_id uuid NOT NULL REFERENCES business_iam.roles(id),
  permission_id uuid NOT NULL REFERENCES business_iam.permissions(id),
  data_scope jsonb NOT NULL DEFAULT '{"mode":"unrestricted"}'::jsonb
    CHECK (jsonb_typeof(data_scope) = 'object'),
  obligations jsonb NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(obligations) = 'array'),
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (role_id, permission_id)
);

CREATE TABLE business_iam.principal_roles (
  principal_id uuid NOT NULL REFERENCES business_iam.principals(id),
  role_id uuid NOT NULL REFERENCES business_iam.roles(id),
  valid_from timestamptz NOT NULL DEFAULT now(),
  valid_until timestamptz,
  granted_by uuid REFERENCES business_iam.principals(id),
  reason text,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (principal_id, role_id),
  CHECK (valid_until IS NULL OR valid_until > valid_from)
);

CREATE TABLE business_iam.principal_permissions (
  principal_id uuid NOT NULL REFERENCES business_iam.principals(id),
  permission_id uuid NOT NULL REFERENCES business_iam.permissions(id),
  data_scope jsonb NOT NULL DEFAULT '{"mode":"unrestricted"}'::jsonb
    CHECK (jsonb_typeof(data_scope) = 'object'),
  obligations jsonb NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(obligations) = 'array'),
  valid_from timestamptz NOT NULL DEFAULT now(),
  valid_until timestamptz,
  granted_by uuid REFERENCES business_iam.principals(id),
  reason text,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (principal_id, permission_id),
  CHECK (valid_until IS NULL OR valid_until > valid_from)
);

CREATE TABLE business_iam.authorization_decisions (
  id uuid PRIMARY KEY,
  decided_at timestamptz NOT NULL DEFAULT now(),
  human_principal_id uuid REFERENCES business_iam.principals(id),
  agent_principal_id uuid NOT NULL REFERENCES business_iam.principals(id),
  agent_kind text NOT NULL CHECK (agent_kind IN ('independent_agent','proxy_agent')),
  task_id text NOT NULL CHECK (length(task_id) BETWEEN 1 AND 128),
  requested_capabilities text[] NOT NULL CHECK (cardinality(requested_capabilities) > 0),
  allowed_capabilities text[] NOT NULL,
  denied_capabilities text[] NOT NULL,
  effective_grants jsonb NOT NULL CHECK (jsonb_typeof(effective_grants) = 'array'),
  result text NOT NULL CHECK (result IN ('allow','partial','deny')),
  reason_code text NOT NULL,
  trace_id uuid NOT NULL,
  policy_version text NOT NULL DEFAULT 'business-iam-v1',
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(metadata) = 'object')
);

CREATE INDEX business_iam_decisions_trace_idx
  ON business_iam.authorization_decisions (trace_id);
CREATE INDEX business_iam_decisions_agent_time_idx
  ON business_iam.authorization_decisions (agent_principal_id, decided_at DESC);
CREATE INDEX business_iam_decisions_human_time_idx
  ON business_iam.authorization_decisions (human_principal_id, decided_at DESC)
  WHERE human_principal_id IS NOT NULL;

CREATE OR REPLACE FUNCTION business_iam.deny_decision_mutation() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'business_iam.authorization_decisions is append-only'; END $$;
CREATE TRIGGER business_iam_decisions_no_update_delete
  BEFORE UPDATE OR DELETE ON business_iam.authorization_decisions
  FOR EACH ROW EXECUTE FUNCTION business_iam.deny_decision_mutation();

INSERT INTO business_iam.permissions (id, capability, resource_type, action)
VALUES
  (gen_random_uuid(), 'sales_order:read', 'sales_order', 'read'),
  (gen_random_uuid(), 'purchase_order:read', 'purchase_order', 'read'),
  (gen_random_uuid(), 'inventory:read', 'inventory', 'read'),
  (gen_random_uuid(), 'receivable:read', 'receivable', 'read'),
  (gen_random_uuid(), 'payable:read', 'payable', 'read'),
  (gen_random_uuid(), 'order_profit:read', 'order_profit', 'read'),
  (gen_random_uuid(), 'business_anomaly:read', 'business_anomaly', 'read'),
  (gen_random_uuid(), 'business_action:read', 'business_action', 'read');

ALTER TABLE agent_read_delegations
  ADD COLUMN iam_decision_id uuid REFERENCES business_iam.authorization_decisions(id),
  ADD COLUMN agent_principal_id uuid REFERENCES business_iam.principals(id),
  ADD COLUMN effective_grants jsonb NOT NULL DEFAULT '[]'::jsonb
    CHECK (jsonb_typeof(effective_grants) = 'array');

CREATE INDEX agent_read_delegations_iam_decision_idx
  ON agent_read_delegations (iam_decision_id) WHERE iam_decision_id IS NOT NULL;

CREATE OR REPLACE FUNCTION business_iam.revoke_delegations_for_principal(affected_principal uuid)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
  INSERT INTO security_audit_events(
    id,event_type,result,reason_code,enterprise_user_id,identity_binding_id,
    delegation_id,agent_id,agent_turn_id,source_buzz_event_id,source_channel_id,trace_id)
  SELECT
    gen_random_uuid(),'AGENT_DELEGATION_REVOKED','success','iam_authority_changed',
    delegation.enterprise_user_id,delegation.identity_binding_id,delegation.id,
    delegation.agent_id,delegation.agent_turn_id,delegation.source_buzz_event_id,
    delegation.source_channel_id,delegation.trace_id
  FROM agent_read_delegations delegation
  JOIN business_iam.authorization_decisions decision
    ON decision.id=delegation.iam_decision_id
  WHERE delegation.status IN ('active','exhausted')
    AND (decision.human_principal_id=affected_principal
      OR decision.agent_principal_id=affected_principal);

  UPDATE agent_read_delegations delegation
  SET status='revoked',revoked_at=now(),version=version+1
  FROM business_iam.authorization_decisions decision
  WHERE decision.id=delegation.iam_decision_id
    AND delegation.status IN ('active','exhausted')
    AND (decision.human_principal_id=affected_principal
      OR decision.agent_principal_id=affected_principal);
END $$;

CREATE OR REPLACE FUNCTION business_iam.revoke_for_principal_update()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF OLD.status IS DISTINCT FROM NEW.status THEN
    PERFORM business_iam.revoke_delegations_for_principal(NEW.id);
  END IF;
  RETURN NEW;
END $$;

CREATE TRIGGER business_iam_principal_status_revokes_delegations
  AFTER UPDATE OF status ON business_iam.principals
  FOR EACH ROW EXECUTE FUNCTION business_iam.revoke_for_principal_update();

CREATE OR REPLACE FUNCTION business_iam.revoke_for_principal_assignment()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE affected_principal uuid;
BEGIN
  affected_principal := CASE WHEN TG_OP='DELETE' THEN OLD.principal_id ELSE NEW.principal_id END;
  PERFORM business_iam.revoke_delegations_for_principal(affected_principal);
  RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
END $$;

CREATE TRIGGER business_iam_direct_permission_revokes_delegations
  AFTER INSERT OR UPDATE OR DELETE ON business_iam.principal_permissions
  FOR EACH ROW EXECUTE FUNCTION business_iam.revoke_for_principal_assignment();
CREATE TRIGGER business_iam_role_assignment_revokes_delegations
  AFTER INSERT OR UPDATE OR DELETE ON business_iam.principal_roles
  FOR EACH ROW EXECUTE FUNCTION business_iam.revoke_for_principal_assignment();

CREATE OR REPLACE FUNCTION business_iam.revoke_for_role_change()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE affected_role uuid;
DECLARE assigned_principal record;
BEGIN
  affected_role := CASE WHEN TG_OP='DELETE' THEN OLD.role_id ELSE NEW.role_id END;
  FOR assigned_principal IN
    SELECT principal_id FROM business_iam.principal_roles WHERE role_id=affected_role
  LOOP
    PERFORM business_iam.revoke_delegations_for_principal(assigned_principal.principal_id);
  END LOOP;
  RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
END $$;

CREATE TRIGGER business_iam_role_permission_revokes_delegations
  AFTER INSERT OR UPDATE OR DELETE ON business_iam.role_permissions
  FOR EACH ROW EXECUTE FUNCTION business_iam.revoke_for_role_change();

CREATE OR REPLACE FUNCTION business_iam.revoke_for_role_record()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE assigned_principal record;
BEGIN
  FOR assigned_principal IN
    SELECT principal_id FROM business_iam.principal_roles WHERE role_id=NEW.id
  LOOP
    PERFORM business_iam.revoke_delegations_for_principal(assigned_principal.principal_id);
  END LOOP;
  RETURN NEW;
END $$;

CREATE TRIGGER business_iam_role_update_revokes_delegations
  AFTER UPDATE ON business_iam.roles
  FOR EACH ROW EXECUTE FUNCTION business_iam.revoke_for_role_record();

CREATE OR REPLACE FUNCTION business_iam.revoke_for_permission_record()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE assigned_principal record;
BEGIN
  FOR assigned_principal IN
    SELECT principal_id
    FROM business_iam.principal_permissions
    WHERE permission_id=NEW.id
    UNION
    SELECT principal_role.principal_id
    FROM business_iam.role_permissions role_permission
    JOIN business_iam.principal_roles principal_role
      ON principal_role.role_id=role_permission.role_id
    WHERE role_permission.permission_id=NEW.id
  LOOP
    PERFORM business_iam.revoke_delegations_for_principal(assigned_principal.principal_id);
  END LOOP;
  RETURN NEW;
END $$;

CREATE TRIGGER business_iam_permission_update_revokes_delegations
  AFTER UPDATE ON business_iam.permissions
  FOR EACH ROW EXECUTE FUNCTION business_iam.revoke_for_permission_record();
