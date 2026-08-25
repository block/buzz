-- Business Core B1 owns one customer group per deployment. Deliberately no
-- tenant_id/client_group_id columns: the database is the isolation boundary.

CREATE TABLE business_group_profile (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    id UUID NOT NULL UNIQUE,
    code TEXT NOT NULL UNIQUE CHECK (code ~ '^[A-Z0-9][A-Z0-9_-]{1,31}$'),
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 160),
    base_currency CHAR(3) NOT NULL CHECK (base_currency ~ '^[A-Z]{3}$'),
    timezone TEXT NOT NULL CHECK (char_length(timezone) BETWEEN 1 AND 64),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE business_legal_entities (
    id UUID PRIMARY KEY,
    code TEXT NOT NULL UNIQUE CHECK (code ~ '^[A-Z0-9][A-Z0-9_-]{1,31}$'),
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 160),
    country_code CHAR(2) NOT NULL CHECK (country_code ~ '^[A-Z]{2}$'),
    functional_currency CHAR(3) NOT NULL CHECK (functional_currency ~ '^[A-Z]{3}$'),
    registration_number TEXT,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE business_ledger_books (
    id UUID PRIMARY KEY,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    code TEXT NOT NULL,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 160),
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    fiscal_year_start_month SMALLINT NOT NULL CHECK (fiscal_year_start_month BETWEEN 1 AND 12),
    is_primary BOOLEAN NOT NULL DEFAULT FALSE,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (legal_entity_id, code)
);
CREATE UNIQUE INDEX business_ledger_books_one_primary
    ON business_ledger_books (legal_entity_id) WHERE is_primary;

CREATE TABLE business_units (
    id UUID PRIMARY KEY,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 160),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE business_departments (
    id UUID PRIMARY KEY,
    business_unit_id UUID NOT NULL REFERENCES business_units(id) ON DELETE RESTRICT,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 160),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE business_units_of_measure (
    id UUID PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
    precision_scale SMALLINT NOT NULL DEFAULT 2 CHECK (precision_scale BETWEEN 0 AND 6),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE business_product_categories (
    id UUID PRIMARY KEY,
    parent_id UUID REFERENCES business_product_categories(id) ON DELETE RESTRICT,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 120),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (parent_id IS NULL OR parent_id <> id)
);

CREATE TABLE business_brands (
    id UUID PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 120),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE business_warehouses (
    id UUID PRIMARY KEY,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    business_unit_id UUID NOT NULL REFERENCES business_units(id) ON DELETE RESTRICT,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 160),
    address TEXT,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE business_customers (
    id UUID PRIMARY KEY,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    business_unit_id UUID NOT NULL REFERENCES business_units(id) ON DELETE RESTRICT,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 200),
    credit_currency CHAR(3) NOT NULL CHECK (credit_currency ~ '^[A-Z]{3}$'),
    credit_limit_minor BIGINT NOT NULL DEFAULT 0 CHECK (credit_limit_minor >= 0),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE business_suppliers (
    id UUID PRIMARY KEY,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    business_unit_id UUID NOT NULL REFERENCES business_units(id) ON DELETE RESTRICT,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 200),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE business_products (
    id UUID PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 200),
    category_id UUID NOT NULL REFERENCES business_product_categories(id) ON DELETE RESTRICT,
    brand_id UUID REFERENCES business_brands(id) ON DELETE RESTRICT,
    base_uom_id UUID NOT NULL REFERENCES business_units_of_measure(id) ON DELETE RESTRICT,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE business_skus (
    id UUID PRIMARY KEY,
    product_id UUID NOT NULL REFERENCES business_products(id) ON DELETE RESTRICT,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 200),
    barcode TEXT UNIQUE,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE business_salespeople (
    id UUID PRIMARY KEY,
    enterprise_user_id UUID NOT NULL UNIQUE REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    business_unit_id UUID NOT NULL REFERENCES business_units(id) ON DELETE RESTRICT,
    department_id UUID REFERENCES business_departments(id) ON DELETE RESTRICT,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 160),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE business_roles (
    id UUID PRIMARY KEY,
    role_key TEXT NOT NULL UNIQUE CHECK (role_key ~ '^[a-z][a-z0-9:_-]{1,63}$'),
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 120),
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE business_role_permissions (
    role_id UUID NOT NULL REFERENCES business_roles(id) ON DELETE CASCADE,
    permission_key TEXT NOT NULL CHECK (permission_key ~ '^[a-z][a-z0-9:_-]{1,95}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (role_id, permission_key)
);

CREATE TABLE business_user_roles (
    enterprise_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE CASCADE,
    role_id UUID NOT NULL REFERENCES business_roles(id) ON DELETE CASCADE,
    assigned_by UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (enterprise_user_id, role_id)
);

CREATE TABLE business_legal_entity_scopes (
    enterprise_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE CASCADE,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE CASCADE,
    granted_by UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY (enterprise_user_id, legal_entity_id)
);
CREATE TABLE business_warehouse_scopes (
    enterprise_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE CASCADE,
    warehouse_id UUID NOT NULL REFERENCES business_warehouses(id) ON DELETE CASCADE,
    granted_by UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY (enterprise_user_id, warehouse_id)
);
CREATE TABLE business_customer_scopes (
    enterprise_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE CASCADE,
    customer_id UUID NOT NULL REFERENCES business_customers(id) ON DELETE CASCADE,
    granted_by UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY (enterprise_user_id, customer_id)
);
CREATE TABLE business_supplier_scopes (
    enterprise_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE CASCADE,
    supplier_id UUID NOT NULL REFERENCES business_suppliers(id) ON DELETE CASCADE,
    granted_by UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY (enterprise_user_id, supplier_id)
);
CREATE TABLE business_brand_scopes (
    enterprise_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE CASCADE,
    brand_id UUID NOT NULL REFERENCES business_brands(id) ON DELETE CASCADE,
    granted_by UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY (enterprise_user_id, brand_id)
);
CREATE TABLE business_unit_scopes (
    enterprise_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE CASCADE,
    business_unit_id UUID NOT NULL REFERENCES business_units(id) ON DELETE CASCADE,
    granted_by UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY (enterprise_user_id, business_unit_id)
);

CREATE TABLE business_assignment_policies (
    action_code TEXT PRIMARY KEY CHECK (action_code ~ '^[a-z][a-z0-9:_-]{1,95}$'),
    required_permission TEXT NOT NULL,
    eligible_role_keys TEXT[] NOT NULL CHECK (cardinality(eligible_role_keys) > 0),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE business_approval_policies (
    action_code TEXT PRIMARY KEY CHECK (action_code ~ '^[a-z][a-z0-9:_-]{1,95}$'),
    required_permission TEXT NOT NULL,
    eligible_role_keys TEXT[] NOT NULL CHECK (cardinality(eligible_role_keys) > 0),
    min_approvers SMALLINT NOT NULL DEFAULT 1 CHECK (min_approvers BETWEEN 1 AND 10),
    allow_self_approval BOOLEAN NOT NULL DEFAULT FALSE,
    require_distinct_business_unit BOOLEAN NOT NULL DEFAULT FALSE,
    step_up_amount_minor BIGINT CHECK (step_up_amount_minor IS NULL OR step_up_amount_minor >= 0),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE business_authorization_revision (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    revision BIGINT NOT NULL DEFAULT 1 CHECK (revision > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO business_authorization_revision(singleton) VALUES (TRUE);

CREATE TABLE business_core_audit_events (
    id BIGSERIAL PRIMARY KEY,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    trace_id UUID NOT NULL,
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    operation TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    details JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE TABLE business_core_outbox (
    id UUID PRIMARY KEY,
    topic TEXT NOT NULL,
    aggregate_type TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_at TIMESTAMPTZ
);

CREATE OR REPLACE FUNCTION business_core_touch_updated_at() RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    NEW.version = OLD.version + 1;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION business_core_bump_authorization_revision() RETURNS TRIGGER AS $$
BEGIN
    UPDATE business_authorization_revision SET revision = revision + 1, updated_at = now() WHERE singleton;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION business_core_audit_append_only() RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'business_core_audit_events is append-only';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER business_group_profile_touch BEFORE UPDATE ON business_group_profile
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER business_legal_entities_touch BEFORE UPDATE ON business_legal_entities
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER business_ledger_books_touch BEFORE UPDATE ON business_ledger_books
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER business_units_touch BEFORE UPDATE ON business_units
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER business_departments_touch BEFORE UPDATE ON business_departments
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER business_uom_touch BEFORE UPDATE ON business_units_of_measure
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER business_categories_touch BEFORE UPDATE ON business_product_categories
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER business_brands_touch BEFORE UPDATE ON business_brands
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER business_warehouses_touch BEFORE UPDATE ON business_warehouses
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER business_customers_touch BEFORE UPDATE ON business_customers
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER business_suppliers_touch BEFORE UPDATE ON business_suppliers
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER business_products_touch BEFORE UPDATE ON business_products
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER business_skus_touch BEFORE UPDATE ON business_skus
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER business_salespeople_touch BEFORE UPDATE ON business_salespeople
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER business_roles_touch BEFORE UPDATE ON business_roles
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER business_assignment_policies_touch BEFORE UPDATE ON business_assignment_policies
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER business_approval_policies_touch BEFORE UPDATE ON business_approval_policies
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();

CREATE TRIGGER business_user_roles_revision AFTER INSERT OR UPDATE OR DELETE ON business_user_roles
    FOR EACH ROW EXECUTE FUNCTION business_core_bump_authorization_revision();
CREATE TRIGGER business_permissions_revision AFTER INSERT OR UPDATE OR DELETE ON business_role_permissions
    FOR EACH ROW EXECUTE FUNCTION business_core_bump_authorization_revision();
CREATE TRIGGER business_legal_entity_scope_revision AFTER INSERT OR UPDATE OR DELETE ON business_legal_entity_scopes
    FOR EACH ROW EXECUTE FUNCTION business_core_bump_authorization_revision();
CREATE TRIGGER business_warehouse_scope_revision AFTER INSERT OR UPDATE OR DELETE ON business_warehouse_scopes
    FOR EACH ROW EXECUTE FUNCTION business_core_bump_authorization_revision();
CREATE TRIGGER business_customer_scope_revision AFTER INSERT OR UPDATE OR DELETE ON business_customer_scopes
    FOR EACH ROW EXECUTE FUNCTION business_core_bump_authorization_revision();
CREATE TRIGGER business_supplier_scope_revision AFTER INSERT OR UPDATE OR DELETE ON business_supplier_scopes
    FOR EACH ROW EXECUTE FUNCTION business_core_bump_authorization_revision();
CREATE TRIGGER business_brand_scope_revision AFTER INSERT OR UPDATE OR DELETE ON business_brand_scopes
    FOR EACH ROW EXECUTE FUNCTION business_core_bump_authorization_revision();
CREATE TRIGGER business_unit_scope_revision AFTER INSERT OR UPDATE OR DELETE ON business_unit_scopes
    FOR EACH ROW EXECUTE FUNCTION business_core_bump_authorization_revision();
CREATE TRIGGER business_assignment_policy_revision AFTER INSERT OR UPDATE OR DELETE ON business_assignment_policies
    FOR EACH ROW EXECUTE FUNCTION business_core_bump_authorization_revision();
CREATE TRIGGER business_approval_policy_revision AFTER INSERT OR UPDATE OR DELETE ON business_approval_policies
    FOR EACH ROW EXECUTE FUNCTION business_core_bump_authorization_revision();

CREATE TRIGGER business_core_audit_no_update BEFORE UPDATE OR DELETE ON business_core_audit_events
    FOR EACH ROW EXECUTE FUNCTION business_core_audit_append_only();

CREATE VIEW business_master_data_directory AS
SELECT 'legal_entity'::TEXT resource_type, id, code, name, status, id legal_entity_id,
       NULL::UUID warehouse_id, NULL::UUID customer_id, NULL::UUID supplier_id,
       NULL::UUID brand_id, NULL::UUID business_unit_id, version
FROM business_legal_entities
UNION ALL SELECT 'ledger_book', id, code, name, status, legal_entity_id, NULL, NULL, NULL, NULL, NULL, version FROM business_ledger_books
UNION ALL SELECT 'business_unit', id, code, name, status, legal_entity_id, NULL, NULL, NULL, NULL, id, version FROM business_units
UNION ALL SELECT 'department', d.id, d.code, d.name, d.status, u.legal_entity_id, NULL, NULL, NULL, NULL, d.business_unit_id, d.version FROM business_departments d JOIN business_units u ON u.id=d.business_unit_id
UNION ALL SELECT 'unit_of_measure', id, code, name, status, NULL, NULL, NULL, NULL, NULL, NULL, version FROM business_units_of_measure
UNION ALL SELECT 'product_category', id, code, name, status, NULL, NULL, NULL, NULL, NULL, NULL, version FROM business_product_categories
UNION ALL SELECT 'brand', id, code, name, status, NULL, NULL, NULL, NULL, id, NULL, version FROM business_brands
UNION ALL SELECT 'warehouse', id, code, name, status, legal_entity_id, id, NULL, NULL, NULL, business_unit_id, version FROM business_warehouses
UNION ALL SELECT 'customer', id, code, name, status, legal_entity_id, NULL, id, NULL, NULL, business_unit_id, version FROM business_customers
UNION ALL SELECT 'supplier', id, code, name, status, legal_entity_id, NULL, NULL, id, NULL, business_unit_id, version FROM business_suppliers
UNION ALL SELECT 'product', p.id, p.code, p.name, p.status, NULL, NULL, NULL, NULL, p.brand_id, NULL, p.version FROM business_products p
UNION ALL SELECT 'sku', s.id, s.code, s.name, s.status, NULL, NULL, NULL, NULL, p.brand_id, NULL, s.version FROM business_skus s JOIN business_products p ON p.id=s.product_id
UNION ALL SELECT 'salesperson', s.id, s.code, s.name, s.status, u.legal_entity_id, NULL, NULL, NULL, NULL, s.business_unit_id, s.version FROM business_salespeople s JOIN business_units u ON u.id=s.business_unit_id;
