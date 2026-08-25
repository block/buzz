-- Operational safety-stock policies, replenishment suggestions and purchase requisitions.

CREATE SEQUENCE business_purchase_requisition_number_seq;

CREATE TABLE inventory_replenishment_policies (
    id UUID PRIMARY KEY,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    warehouse_id UUID NOT NULL REFERENCES business_warehouses(id) ON DELETE RESTRICT,
    sku_id UUID NOT NULL REFERENCES business_skus(id) ON DELETE RESTRICT,
    preferred_supplier_id UUID NOT NULL REFERENCES business_suppliers(id) ON DELETE RESTRICT,
    unit_of_measure_id UUID NOT NULL REFERENCES business_units_of_measure(id) ON DELETE RESTRICT,
    safety_stock NUMERIC(24,6) NOT NULL CHECK (safety_stock >= 0),
    reorder_point NUMERIC(24,6) NOT NULL CHECK (reorder_point >= safety_stock),
    target_stock NUMERIC(24,6) NOT NULL CHECK (target_stock > reorder_point),
    minimum_order_quantity NUMERIC(24,6) NOT NULL DEFAULT 1 CHECK (minimum_order_quantity > 0),
    order_multiple NUMERIC(24,6) NOT NULL DEFAULT 1 CHECK (order_multiple > 0),
    lead_time_days INTEGER NOT NULL DEFAULT 7 CHECK (lead_time_days BETWEEN 0 AND 3650),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','paused')),
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    updated_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    trace_id UUID NOT NULL,
    UNIQUE (legal_entity_id,warehouse_id,sku_id)
);
CREATE INDEX inventory_replenishment_policy_supplier_idx
    ON inventory_replenishment_policies(preferred_supplier_id,status);

CREATE TABLE purchase_requisitions (
    id UUID PRIMARY KEY,
    requisition_number TEXT NOT NULL UNIQUE,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    warehouse_id UUID NOT NULL REFERENCES business_warehouses(id) ON DELETE RESTRICT,
    supplier_id UUID NOT NULL REFERENCES business_suppliers(id) ON DELETE RESTRICT,
    request_date DATE NOT NULL,
    required_date DATE NOT NULL,
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','confirmed','converted','cancelled')),
    purchase_order_id UUID UNIQUE REFERENCES purchase_orders(id) ON DELETE RESTRICT,
    business_note TEXT CHECK (business_note IS NULL OR char_length(business_note) <= 1000),
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    confirmed_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    cancelled_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    converted_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    converted_at TIMESTAMPTZ,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    trace_id UUID NOT NULL,
    CHECK (required_date >= request_date)
);
CREATE INDEX purchase_requisitions_scope_idx ON purchase_requisitions
    (legal_entity_id,warehouse_id,supplier_id,request_date DESC);

CREATE TABLE purchase_requisition_lines (
    id UUID PRIMARY KEY,
    purchase_requisition_id UUID NOT NULL REFERENCES purchase_requisitions(id) ON DELETE RESTRICT,
    replenishment_policy_id UUID NOT NULL REFERENCES inventory_replenishment_policies(id) ON DELETE RESTRICT,
    sku_id UUID NOT NULL REFERENCES business_skus(id) ON DELETE RESTRICT,
    unit_of_measure_id UUID NOT NULL REFERENCES business_units_of_measure(id) ON DELETE RESTRICT,
    requested_quantity NUMERIC(24,6) NOT NULL CHECK (requested_quantity > 0),
    snapshot_available_quantity NUMERIC(24,6) NOT NULL,
    snapshot_inbound_quantity NUMERIC(24,6) NOT NULL CHECK (snapshot_inbound_quantity >= 0),
    snapshot_open_requisition_quantity NUMERIC(24,6) NOT NULL CHECK (snapshot_open_requisition_quantity >= 0),
    snapshot_reorder_point NUMERIC(24,6) NOT NULL CHECK (snapshot_reorder_point >= 0),
    snapshot_target_stock NUMERIC(24,6) NOT NULL CHECK (snapshot_target_stock >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (purchase_requisition_id,sku_id)
);

CREATE TABLE purchase_requisition_events (
    id UUID PRIMARY KEY,
    purchase_requisition_id UUID NOT NULL REFERENCES purchase_requisitions(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL,
    requisition_version BIGINT NOT NULL CHECK (requisition_version > 0),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    trace_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TRIGGER inventory_replenishment_policies_touch
    BEFORE UPDATE ON inventory_replenishment_policies
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER purchase_requisitions_touch
    BEFORE UPDATE ON purchase_requisitions
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER purchase_requisition_events_append_only
    BEFORE UPDATE OR DELETE ON purchase_requisition_events
    FOR EACH ROW EXECUTE FUNCTION business_core_append_only();

CREATE VIEW inventory_replenishment_current AS
WITH inbound AS (
    SELECT o.legal_entity_id,l.warehouse_id,l.sku_id,
        sum(l.ordered_quantity-l.received_quantity-l.cancelled_quantity)::numeric(24,6) quantity
    FROM purchase_orders o JOIN purchase_order_lines l ON l.purchase_order_id=o.id
    WHERE o.lifecycle_status='confirmed' AND o.receiving_status IN ('unreceived','partially_received')
    GROUP BY 1,2,3
), open_requisitions AS (
    SELECT r.legal_entity_id,r.warehouse_id,l.sku_id,
        sum(l.requested_quantity)::numeric(24,6) quantity
    FROM purchase_requisitions r JOIN purchase_requisition_lines l ON l.purchase_requisition_id=r.id
    WHERE r.status IN ('draft','confirmed')
    GROUP BY 1,2,3
), base AS (
    SELECT p.*,s.code sku_code,s.name sku_name,w.code warehouse_code,w.name warehouse_name,
        supplier.code supplier_code,supplier.name supplier_name,e.functional_currency::text currency,
        COALESCE(b.on_hand_quantity,0)::numeric(24,6) on_hand_quantity,
        COALESCE(b.reserved_quantity,0)::numeric(24,6) reserved_quantity,
        COALESCE(b.quarantined_quantity,0)::numeric(24,6) quarantined_quantity,
        (COALESCE(b.on_hand_quantity,0)-COALESCE(b.reserved_quantity,0)-COALESCE(b.quarantined_quantity,0))::numeric(24,6) available_quantity,
        COALESCE(i.quantity,0)::numeric(24,6) inbound_quantity,
        COALESCE(r.quantity,0)::numeric(24,6) open_requisition_quantity,
        COALESCE(b.inventory_value,0)::numeric(24,6) inventory_value,
        b.average_unit_cost
    FROM inventory_replenishment_policies p
    JOIN business_legal_entities e ON e.id=p.legal_entity_id
    JOIN business_warehouses w ON w.id=p.warehouse_id
    JOIN business_skus s ON s.id=p.sku_id
    JOIN business_suppliers supplier ON supplier.id=p.preferred_supplier_id
    LEFT JOIN inventory_balances b ON b.legal_entity_id=p.legal_entity_id AND b.warehouse_id=p.warehouse_id AND b.sku_id=p.sku_id
    LEFT JOIN inbound i ON i.legal_entity_id=p.legal_entity_id AND i.warehouse_id=p.warehouse_id AND i.sku_id=p.sku_id
    LEFT JOIN open_requisitions r ON r.legal_entity_id=p.legal_entity_id AND r.warehouse_id=p.warehouse_id AND r.sku_id=p.sku_id
), projected AS (
    SELECT base.*,(available_quantity+inbound_quantity+open_requisition_quantity)::numeric(24,6) projected_quantity
    FROM base
)
SELECT projected.*,
    CASE
        WHEN status='paused' THEN 'paused'
        WHEN open_requisition_quantity>0 THEN 'requisition_open'
        WHEN inbound_quantity>0 AND available_quantity<=reorder_point THEN 'inbound_covered'
        WHEN available_quantity<=safety_stock THEN 'critical'
        WHEN available_quantity<=reorder_point THEN 'warning'
        ELSE 'healthy'
    END risk_state,
    CASE WHEN status='active' AND projected_quantity<=reorder_point THEN
        GREATEST(minimum_order_quantity,
            ceil(GREATEST(target_stock-projected_quantity,0)/order_multiple)*order_multiple)::numeric(24,6)
        ELSE 0::numeric(24,6)
    END suggested_quantity,
    (CURRENT_DATE+lead_time_days)::date suggested_required_date
FROM projected;
