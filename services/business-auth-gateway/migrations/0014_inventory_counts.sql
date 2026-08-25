-- Inventory cycle counts, database-enforced scope freeze and operating metrics.

CREATE SEQUENCE business_inventory_count_number_seq;

CREATE TABLE inventory_count_tasks (
    id UUID PRIMARY KEY,
    count_number TEXT NOT NULL UNIQUE,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    warehouse_id UUID NOT NULL REFERENCES business_warehouses(id) ON DELETE RESTRICT,
    count_date DATE NOT NULL,
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    status TEXT NOT NULL DEFAULT 'counting'
        CHECK (status IN ('counting','counted','posted','cancelled')),
    business_note TEXT CHECK (business_note IS NULL OR char_length(business_note) <= 1000),
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    counted_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    posted_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    cancelled_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    counted_at TIMESTAMPTZ,
    posted_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    trace_id UUID NOT NULL
);
CREATE INDEX inventory_count_tasks_scope_idx ON inventory_count_tasks
    (legal_entity_id,warehouse_id,count_date DESC);

CREATE TABLE inventory_count_lines (
    id UUID PRIMARY KEY,
    inventory_count_id UUID NOT NULL REFERENCES inventory_count_tasks(id) ON DELETE RESTRICT,
    sku_id UUID NOT NULL REFERENCES business_skus(id) ON DELETE RESTRICT,
    snapshot_on_hand_quantity NUMERIC(24,6) NOT NULL CHECK (snapshot_on_hand_quantity >= 0),
    snapshot_reserved_quantity NUMERIC(24,6) NOT NULL CHECK (snapshot_reserved_quantity >= 0),
    snapshot_quarantined_quantity NUMERIC(24,6) NOT NULL CHECK (snapshot_quarantined_quantity >= 0),
    snapshot_inventory_value NUMERIC(24,6) NOT NULL CHECK (snapshot_inventory_value >= 0),
    snapshot_average_unit_cost NUMERIC(24,6),
    actual_on_hand_quantity NUMERIC(24,6) CHECK (actual_on_hand_quantity IS NULL OR actual_on_hand_quantity >= 0),
    surplus_unit_cost NUMERIC(24,6) CHECK (surplus_unit_cost IS NULL OR surplus_unit_cost >= 0),
    variance_quantity NUMERIC(24,6),
    variance_value NUMERIC(24,6),
    inventory_movement_id UUID REFERENCES inventory_movements(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (inventory_count_id,sku_id),
    CHECK (snapshot_reserved_quantity + snapshot_quarantined_quantity <= snapshot_on_hand_quantity)
);

CREATE TABLE inventory_count_events (
    id UUID PRIMARY KEY,
    inventory_count_id UUID NOT NULL REFERENCES inventory_count_tasks(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL,
    count_version BIGINT NOT NULL CHECK (count_version > 0),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    trace_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TRIGGER inventory_count_tasks_touch BEFORE UPDATE ON inventory_count_tasks
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER inventory_count_events_append_only BEFORE UPDATE OR DELETE ON inventory_count_events
    FOR EACH ROW EXECUTE FUNCTION business_core_append_only();

ALTER TABLE inventory_movements DROP CONSTRAINT inventory_movements_movement_type_check;
ALTER TABLE inventory_movements ADD CONSTRAINT inventory_movements_movement_type_check
    CHECK (movement_type IN ('opening_balance','opening_balance_reversal',
        'sales_shipment','sales_shipment_reversal','purchase_receipt',
        'purchase_receipt_reversal','sales_return','purchase_return',
        'sales_return_scrap','inventory_count_adjustment'));

CREATE OR REPLACE FUNCTION enforce_inventory_count_freeze() RETURNS TRIGGER AS $$
BEGIN
    IF NEW.movement_type = 'inventory_count_adjustment' THEN
        RETURN NEW;
    END IF;
    IF EXISTS (
        SELECT 1 FROM inventory_count_tasks t
        JOIN inventory_count_lines l ON l.inventory_count_id=t.id
        WHERE t.status IN ('counting','counted')
          AND t.legal_entity_id=NEW.legal_entity_id
          AND t.warehouse_id=NEW.warehouse_id
          AND l.sku_id=NEW.sku_id
    ) THEN
        RAISE EXCEPTION USING ERRCODE='P0001',
            MESSAGE='inventory count scope is frozen';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER inventory_movement_count_freeze
    BEFORE INSERT ON inventory_movements
    FOR EACH ROW EXECUTE FUNCTION enforce_inventory_count_freeze();

CREATE OR REPLACE FUNCTION enforce_inventory_balance_count_freeze() RETURNS TRIGGER AS $$
DECLARE active_count UUID;
BEGIN
    SELECT t.id INTO active_count
    FROM inventory_count_tasks t
    JOIN inventory_count_lines l ON l.inventory_count_id=t.id
    WHERE t.status IN ('counting','counted')
      AND t.legal_entity_id=NEW.legal_entity_id
      AND t.warehouse_id=NEW.warehouse_id
      AND l.sku_id=NEW.sku_id
    LIMIT 1;
    IF active_count IS NOT NULL
       AND COALESCE(current_setting('business.inventory_count_adjustment',true),'') <> active_count::text THEN
        RAISE EXCEPTION USING ERRCODE='P0001',
            MESSAGE='inventory count scope is frozen';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER inventory_balance_count_freeze
    BEFORE UPDATE ON inventory_balances
    FOR EACH ROW EXECUTE FUNCTION enforce_inventory_balance_count_freeze();

CREATE VIEW inventory_aging_current AS
SELECT b.legal_entity_id,b.warehouse_id,b.sku_id,s.code sku_code,s.name sku_name,
       b.on_hand_quantity,b.reserved_quantity,b.quarantined_quantity,
       b.inventory_value,b.average_unit_cost,m.currency::text currency,
       issue.last_issue_date,
       GREATEST(current_date-COALESCE(issue.last_issue_date,first_in.first_inbound_date,current_date),0)::integer days_without_issue,
       CASE
         WHEN current_date-COALESCE(issue.last_issue_date,first_in.first_inbound_date,current_date) <= 30 THEN '0_30'
         WHEN current_date-COALESCE(issue.last_issue_date,first_in.first_inbound_date,current_date) <= 60 THEN '31_60'
         WHEN current_date-COALESCE(issue.last_issue_date,first_in.first_inbound_date,current_date) <= 90 THEN '61_90'
         ELSE 'over_90' END aging_bucket
FROM inventory_balances b
JOIN business_skus s ON s.id=b.sku_id
LEFT JOIN inventory_movements m ON m.id=b.last_movement_id
LEFT JOIN LATERAL (
  SELECT max(business_date) last_issue_date FROM inventory_movements i
  WHERE i.legal_entity_id=b.legal_entity_id AND i.warehouse_id=b.warehouse_id
    AND i.sku_id=b.sku_id AND i.quantity<0
) issue ON true
LEFT JOIN LATERAL (
  SELECT min(business_date) first_inbound_date FROM inventory_movements i
  WHERE i.legal_entity_id=b.legal_entity_id AND i.warehouse_id=b.warehouse_id
    AND i.sku_id=b.sku_id AND i.quantity>0
) first_in ON true
WHERE b.on_hand_quantity>0;
