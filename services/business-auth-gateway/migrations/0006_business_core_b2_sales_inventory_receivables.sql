-- Business Core B2: sales, inventory, operational receivables and settlement.
-- The database remains the single-customer-group boundary; deliberately no
-- tenant_id/client_group_id columns are introduced.

ALTER TABLE business_customers
    ADD COLUMN payment_terms_days INTEGER NOT NULL DEFAULT 30
        CHECK (payment_terms_days BETWEEN 0 AND 3650);
ALTER TABLE business_products
    ADD COLUMN allow_zero_cost BOOLEAN NOT NULL DEFAULT FALSE;

CREATE SEQUENCE business_sales_order_number_seq;
CREATE SEQUENCE business_shipment_number_seq;
CREATE SEQUENCE business_receivable_number_seq;
CREATE SEQUENCE business_customer_receipt_number_seq;
CREATE SEQUENCE business_inventory_opening_number_seq;

CREATE TABLE business_command_idempotency (
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    operation TEXT NOT NULL CHECK (operation ~ '^[a-z][a-z0-9:_-]{1,95}$'),
    idempotency_key TEXT NOT NULL CHECK (char_length(idempotency_key) BETWEEN 8 AND 128),
    request_hash CHAR(64) NOT NULL CHECK (request_hash ~ '^[a-f0-9]{64}$'),
    response JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    PRIMARY KEY (actor_user_id, operation, idempotency_key),
    CHECK ((response IS NULL) = (completed_at IS NULL))
);

CREATE TABLE sales_orders (
    id UUID PRIMARY KEY,
    order_number TEXT NOT NULL UNIQUE,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    customer_id UUID NOT NULL REFERENCES business_customers(id) ON DELETE RESTRICT,
    salesperson_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    business_unit_id UUID NOT NULL REFERENCES business_units(id) ON DELETE RESTRICT,
    department_id UUID REFERENCES business_departments(id) ON DELETE RESTRICT,
    brand_id UUID REFERENCES business_brands(id) ON DELETE RESTRICT,
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    order_date DATE NOT NULL,
    requested_delivery_date DATE,
    payment_terms_days INTEGER NOT NULL CHECK (payment_terms_days BETWEEN 0 AND 3650),
    payment_terms_snapshot JSONB NOT NULL,
    lifecycle_status TEXT NOT NULL DEFAULT 'draft'
        CHECK (lifecycle_status IN ('draft','confirmed','completed','cancelled')),
    hold_status TEXT NOT NULL DEFAULT 'none'
        CHECK (hold_status IN ('none','manual_review_hold')),
    fulfillment_status TEXT NOT NULL DEFAULT 'unreserved'
        CHECK (fulfillment_status IN ('unreserved','reserved','partially_shipped','shipped','cancelled')),
    subtotal_amount NUMERIC(24,6) NOT NULL CHECK (subtotal_amount >= 0),
    discount_amount NUMERIC(24,6) NOT NULL CHECK (discount_amount >= 0),
    net_amount NUMERIC(24,6) NOT NULL CHECK (net_amount >= 0),
    tax_amount NUMERIC(24,6) NOT NULL CHECK (tax_amount >= 0),
    gross_amount NUMERIC(24,6) NOT NULL CHECK (gross_amount >= 0),
    customer_reference TEXT CHECK (customer_reference IS NULL OR char_length(customer_reference) <= 120),
    business_note TEXT CHECK (business_note IS NULL OR char_length(business_note) <= 1000),
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    updated_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    trace_id UUID NOT NULL,
    CHECK (gross_amount = net_amount + tax_amount),
    CHECK (net_amount = subtotal_amount - discount_amount),
    CHECK (requested_delivery_date IS NULL OR requested_delivery_date >= order_date)
);
CREATE INDEX sales_orders_scope_idx ON sales_orders (legal_entity_id, customer_id, order_date DESC);
CREATE INDEX sales_orders_status_idx ON sales_orders (lifecycle_status, fulfillment_status, updated_at DESC);

CREATE TABLE sales_order_lines (
    id UUID PRIMARY KEY,
    sales_order_id UUID NOT NULL REFERENCES sales_orders(id) ON DELETE RESTRICT,
    line_number INTEGER NOT NULL CHECK (line_number > 0),
    sku_id UUID NOT NULL REFERENCES business_skus(id) ON DELETE RESTRICT,
    warehouse_id UUID NOT NULL REFERENCES business_warehouses(id) ON DELETE RESTRICT,
    unit_of_measure_id UUID NOT NULL REFERENCES business_units_of_measure(id) ON DELETE RESTRICT,
    ordered_quantity NUMERIC(24,6) NOT NULL CHECK (ordered_quantity > 0),
    cancelled_quantity NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (cancelled_quantity >= 0),
    reserved_quantity NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (reserved_quantity >= 0),
    shipped_quantity NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (shipped_quantity >= 0),
    unit_price NUMERIC(24,6) NOT NULL CHECK (unit_price >= 0),
    discount_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (discount_amount >= 0),
    net_amount NUMERIC(24,6) NOT NULL CHECK (net_amount >= 0),
    tax_rate NUMERIC(12,8) NOT NULL DEFAULT 0 CHECK (tax_rate >= 0 AND tax_rate <= 1),
    tax_amount NUMERIC(24,6) NOT NULL CHECK (tax_amount >= 0),
    gross_amount NUMERIC(24,6) NOT NULL CHECK (gross_amount >= 0),
    business_unit_id UUID NOT NULL REFERENCES business_units(id) ON DELETE RESTRICT,
    department_id UUID REFERENCES business_departments(id) ON DELETE RESTRICT,
    brand_id UUID REFERENCES business_brands(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    UNIQUE (sales_order_id, line_number),
    CHECK (shipped_quantity + cancelled_quantity <= ordered_quantity),
    CHECK (reserved_quantity + shipped_quantity + cancelled_quantity <= ordered_quantity),
    CHECK (gross_amount = net_amount + tax_amount)
);
CREATE INDEX sales_order_lines_fulfillment_idx ON sales_order_lines (warehouse_id, sku_id, sales_order_id);

CREATE TABLE sales_order_events (
    id UUID PRIMARY KEY,
    sales_order_id UUID NOT NULL REFERENCES sales_orders(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL,
    order_version BIGINT NOT NULL CHECK (order_version > 0),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    trace_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (sales_order_id, order_version, event_type)
);

CREATE TABLE inventory_opening_batches (
    id UUID PRIMARY KEY,
    batch_number TEXT NOT NULL UNIQUE,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    business_date DATE NOT NULL,
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','posted','reversed')),
    reversal_of_batch_id UUID REFERENCES inventory_opening_batches(id) ON DELETE RESTRICT,
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    posted_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    posted_at TIMESTAMPTZ,
    reversed_at TIMESTAMPTZ,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    trace_id UUID NOT NULL
);
CREATE INDEX inventory_opening_batches_scope_idx ON inventory_opening_batches (legal_entity_id, business_date DESC);

CREATE TABLE inventory_opening_lines (
    id UUID PRIMARY KEY,
    batch_id UUID NOT NULL REFERENCES inventory_opening_batches(id) ON DELETE RESTRICT,
    line_number INTEGER NOT NULL CHECK (line_number > 0),
    warehouse_id UUID NOT NULL REFERENCES business_warehouses(id) ON DELETE RESTRICT,
    sku_id UUID NOT NULL REFERENCES business_skus(id) ON DELETE RESTRICT,
    quantity NUMERIC(24,6) NOT NULL CHECK (quantity > 0),
    unit_cost NUMERIC(24,6) NOT NULL CHECK (unit_cost >= 0),
    total_cost NUMERIC(24,6) NOT NULL CHECK (total_cost >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (batch_id, line_number),
    UNIQUE (batch_id, warehouse_id, sku_id)
);

CREATE TABLE inventory_balances (
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    warehouse_id UUID NOT NULL REFERENCES business_warehouses(id) ON DELETE RESTRICT,
    sku_id UUID NOT NULL REFERENCES business_skus(id) ON DELETE RESTRICT,
    on_hand_quantity NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (on_hand_quantity >= 0),
    reserved_quantity NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (reserved_quantity >= 0),
    inventory_value NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (inventory_value >= 0),
    average_unit_cost NUMERIC(24,6),
    last_movement_id UUID,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    PRIMARY KEY (legal_entity_id, warehouse_id, sku_id),
    CHECK (reserved_quantity <= on_hand_quantity),
    CHECK ((on_hand_quantity = 0 AND average_unit_cost IS NULL AND inventory_value = 0)
        OR (on_hand_quantity > 0 AND average_unit_cost IS NOT NULL))
);
CREATE INDEX inventory_balances_lookup_idx ON inventory_balances (warehouse_id, sku_id);

CREATE TABLE inventory_movements (
    id UUID PRIMARY KEY,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    warehouse_id UUID NOT NULL REFERENCES business_warehouses(id) ON DELETE RESTRICT,
    sku_id UUID NOT NULL REFERENCES business_skus(id) ON DELETE RESTRICT,
    movement_type TEXT NOT NULL CHECK (movement_type IN ('opening_balance','opening_balance_reversal','sales_shipment','sales_shipment_reversal')),
    quantity NUMERIC(24,6) NOT NULL CHECK (quantity <> 0),
    unit_cost NUMERIC(24,6) NOT NULL CHECK (unit_cost >= 0),
    total_cost NUMERIC(24,6) NOT NULL,
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    source_type TEXT NOT NULL,
    source_id UUID NOT NULL,
    source_line_id UUID NOT NULL,
    business_date DATE NOT NULL,
    posted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    reversal_of_movement_id UUID REFERENCES inventory_movements(id) ON DELETE RESTRICT,
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    trace_id UUID NOT NULL,
    UNIQUE (source_type, source_line_id, movement_type),
    CHECK ((quantity > 0 AND total_cost >= 0) OR (quantity < 0 AND total_cost <= 0))
);
ALTER TABLE inventory_balances ADD CONSTRAINT inventory_balances_last_movement_fk
    FOREIGN KEY (last_movement_id) REFERENCES inventory_movements(id) ON DELETE RESTRICT;
CREATE INDEX inventory_movements_ledger_idx ON inventory_movements (legal_entity_id, warehouse_id, sku_id, posted_at, id);

CREATE TABLE inventory_reservations (
    id UUID PRIMARY KEY,
    sales_order_id UUID NOT NULL REFERENCES sales_orders(id) ON DELETE RESTRICT,
    sales_order_line_id UUID NOT NULL REFERENCES sales_order_lines(id) ON DELETE RESTRICT,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    warehouse_id UUID NOT NULL REFERENCES business_warehouses(id) ON DELETE RESTRICT,
    sku_id UUID NOT NULL REFERENCES business_skus(id) ON DELETE RESTRICT,
    reserved_quantity NUMERIC(24,6) NOT NULL CHECK (reserved_quantity > 0),
    consumed_quantity NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (consumed_quantity >= 0),
    released_quantity NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (released_quantity >= 0),
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active','partially_consumed','consumed','released','cancelled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    trace_id UUID NOT NULL,
    UNIQUE (sales_order_line_id),
    CHECK (consumed_quantity + released_quantity <= reserved_quantity)
);
CREATE INDEX inventory_reservations_stock_idx ON inventory_reservations (legal_entity_id, warehouse_id, sku_id, status);

CREATE TABLE inventory_reservation_events (
    id UUID PRIMARY KEY,
    reservation_id UUID NOT NULL REFERENCES inventory_reservations(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL,
    quantity NUMERIC(24,6) NOT NULL CHECK (quantity > 0),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    trace_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE shipments (
    id UUID PRIMARY KEY,
    shipment_number TEXT NOT NULL UNIQUE,
    sales_order_id UUID NOT NULL REFERENCES sales_orders(id) ON DELETE RESTRICT,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    warehouse_id UUID NOT NULL REFERENCES business_warehouses(id) ON DELETE RESTRICT,
    customer_id UUID NOT NULL REFERENCES business_customers(id) ON DELETE RESTRICT,
    shipment_date DATE NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','confirmed','reversed','cancelled')),
    sales_amount NUMERIC(24,6) NOT NULL CHECK (sales_amount >= 0),
    cost_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (cost_amount >= 0),
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    confirmed_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at TIMESTAMPTZ,
    reversed_at TIMESTAMPTZ,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    trace_id UUID NOT NULL
);
CREATE INDEX shipments_order_idx ON shipments (sales_order_id, status, shipment_date DESC);
CREATE INDEX shipments_scope_idx ON shipments (legal_entity_id, warehouse_id, customer_id, shipment_date DESC);

CREATE TABLE shipment_lines (
    id UUID PRIMARY KEY,
    shipment_id UUID NOT NULL REFERENCES shipments(id) ON DELETE RESTRICT,
    sales_order_line_id UUID NOT NULL REFERENCES sales_order_lines(id) ON DELETE RESTRICT,
    sku_id UUID NOT NULL REFERENCES business_skus(id) ON DELETE RESTRICT,
    quantity NUMERIC(24,6) NOT NULL CHECK (quantity > 0),
    sales_amount NUMERIC(24,6) NOT NULL CHECK (sales_amount >= 0),
    unit_cost NUMERIC(24,6),
    total_cost NUMERIC(24,6),
    cost_snapshot_at TIMESTAMPTZ,
    inventory_reservation_id UUID NOT NULL REFERENCES inventory_reservations(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    UNIQUE (shipment_id, sales_order_line_id),
    CHECK ((unit_cost IS NULL AND total_cost IS NULL AND cost_snapshot_at IS NULL)
        OR (unit_cost IS NOT NULL AND total_cost IS NOT NULL AND total_cost >= 0 AND cost_snapshot_at IS NOT NULL))
);

CREATE TABLE shipment_events (
    id UUID PRIMARY KEY,
    shipment_id UUID NOT NULL REFERENCES shipments(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL,
    shipment_version BIGINT NOT NULL CHECK (shipment_version > 0),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    trace_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE trade_receivables (
    id UUID PRIMARY KEY,
    receivable_number TEXT NOT NULL UNIQUE,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    customer_id UUID NOT NULL REFERENCES business_customers(id) ON DELETE RESTRICT,
    sales_order_id UUID NOT NULL REFERENCES sales_orders(id) ON DELETE RESTRICT,
    shipment_id UUID NOT NULL UNIQUE REFERENCES shipments(id) ON DELETE RESTRICT,
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    original_amount NUMERIC(24,6) NOT NULL CHECK (original_amount >= 0),
    settled_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (settled_amount >= 0),
    open_amount NUMERIC(24,6) NOT NULL CHECK (open_amount >= 0),
    recognized_at TIMESTAMPTZ NOT NULL,
    due_date DATE NOT NULL,
    payment_terms_days INTEGER NOT NULL CHECK (payment_terms_days BETWEEN 0 AND 3650),
    payment_terms_snapshot JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','partially_settled','settled','reversed')),
    reversal_of_receivable_id UUID REFERENCES trade_receivables(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    trace_id UUID NOT NULL,
    CHECK (settled_amount + open_amount = original_amount)
);
CREATE INDEX trade_receivables_customer_idx ON trade_receivables (legal_entity_id, customer_id, due_date, status);

CREATE TABLE trade_receivable_events (
    id UUID PRIMARY KEY,
    receivable_id UUID NOT NULL REFERENCES trade_receivables(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL,
    amount NUMERIC(24,6) NOT NULL CHECK (amount >= 0),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    trace_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE customer_receipts (
    id UUID PRIMARY KEY,
    receipt_number TEXT NOT NULL UNIQUE,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    customer_id UUID NOT NULL REFERENCES business_customers(id) ON DELETE RESTRICT,
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    receipt_date DATE NOT NULL,
    amount NUMERIC(24,6) NOT NULL CHECK (amount > 0),
    allocated_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (allocated_amount >= 0),
    unapplied_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (unapplied_amount >= 0),
    payment_method TEXT NOT NULL CHECK (payment_method IN ('bank_transfer','cash','card','other')),
    external_reference TEXT CHECK (external_reference IS NULL OR char_length(external_reference) <= 120),
    business_note TEXT CHECK (business_note IS NULL OR char_length(business_note) <= 1000),
    status TEXT NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft','confirmed','partially_allocated','fully_allocated','reversed','cancelled')),
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    confirmed_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at TIMESTAMPTZ,
    reversed_at TIMESTAMPTZ,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    trace_id UUID NOT NULL,
    CHECK (allocated_amount + unapplied_amount = CASE WHEN status IN ('draft','cancelled','reversed') THEN 0 ELSE amount END)
);
CREATE INDEX customer_receipts_customer_idx ON customer_receipts (legal_entity_id, customer_id, receipt_date DESC);

CREATE TABLE customer_receipt_events (
    id UUID PRIMARY KEY,
    receipt_id UUID NOT NULL REFERENCES customer_receipts(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL,
    amount NUMERIC(24,6) NOT NULL CHECK (amount >= 0),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    trace_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE receivable_allocations (
    id UUID PRIMARY KEY,
    receipt_id UUID NOT NULL REFERENCES customer_receipts(id) ON DELETE RESTRICT,
    receivable_id UUID NOT NULL REFERENCES trade_receivables(id) ON DELETE RESTRICT,
    allocation_type TEXT NOT NULL DEFAULT 'apply' CHECK (allocation_type IN ('apply','reversal')),
    amount NUMERIC(24,6) NOT NULL CHECK (amount > 0),
    reverses_allocation_id UUID REFERENCES receivable_allocations(id) ON DELETE RESTRICT,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','reversed')),
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    trace_id UUID NOT NULL,
    CHECK ((allocation_type='apply' AND reverses_allocation_id IS NULL)
        OR (allocation_type='reversal' AND reverses_allocation_id IS NOT NULL))
);
CREATE UNIQUE INDEX receivable_allocations_one_reversal
    ON receivable_allocations (reverses_allocation_id) WHERE reverses_allocation_id IS NOT NULL;
CREATE INDEX receivable_allocations_receipt_idx ON receivable_allocations (receipt_id, created_at);
CREATE INDEX receivable_allocations_receivable_idx ON receivable_allocations (receivable_id, created_at);

CREATE OR REPLACE FUNCTION business_core_append_only() RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION '% is append-only', TG_TABLE_NAME;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER sales_order_events_append_only BEFORE UPDATE OR DELETE ON sales_order_events
    FOR EACH ROW EXECUTE FUNCTION business_core_append_only();
CREATE TRIGGER inventory_movements_append_only BEFORE UPDATE OR DELETE ON inventory_movements
    FOR EACH ROW EXECUTE FUNCTION business_core_append_only();
CREATE TRIGGER inventory_reservation_events_append_only BEFORE UPDATE OR DELETE ON inventory_reservation_events
    FOR EACH ROW EXECUTE FUNCTION business_core_append_only();
CREATE TRIGGER shipment_events_append_only BEFORE UPDATE OR DELETE ON shipment_events
    FOR EACH ROW EXECUTE FUNCTION business_core_append_only();
CREATE TRIGGER trade_receivable_events_append_only BEFORE UPDATE OR DELETE ON trade_receivable_events
    FOR EACH ROW EXECUTE FUNCTION business_core_append_only();
CREATE TRIGGER customer_receipt_events_append_only BEFORE UPDATE OR DELETE ON customer_receipt_events
    FOR EACH ROW EXECUTE FUNCTION business_core_append_only();
CREATE TRIGGER receivable_allocations_append_only BEFORE UPDATE OR DELETE ON receivable_allocations
    FOR EACH ROW EXECUTE FUNCTION business_core_append_only();

CREATE TRIGGER sales_orders_touch BEFORE UPDATE ON sales_orders
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER sales_order_lines_touch BEFORE UPDATE ON sales_order_lines
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER inventory_opening_batches_touch BEFORE UPDATE ON inventory_opening_batches
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER inventory_balances_touch BEFORE UPDATE ON inventory_balances
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER inventory_reservations_touch BEFORE UPDATE ON inventory_reservations
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER shipments_touch BEFORE UPDATE ON shipments
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER trade_receivables_touch BEFORE UPDATE ON trade_receivables
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER customer_receipts_touch BEFORE UPDATE ON customer_receipts
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();

CREATE VIEW inventory_balance_reconciliation AS
WITH movement AS (
    SELECT legal_entity_id, warehouse_id, sku_id,
           COALESCE(sum(quantity),0)::NUMERIC(24,6) expected_on_hand,
           COALESCE(sum(total_cost),0)::NUMERIC(24,6) expected_value,
           count(*) source_event_count, max(posted_at) last_event_at
    FROM inventory_movements GROUP BY legal_entity_id, warehouse_id, sku_id
), reservation AS (
    SELECT legal_entity_id, warehouse_id, sku_id,
           COALESCE(sum(reserved_quantity-consumed_quantity-released_quantity),0)::NUMERIC(24,6) expected_reserved
    FROM inventory_reservations
    GROUP BY legal_entity_id, warehouse_id, sku_id
)
SELECT b.legal_entity_id,b.warehouse_id,b.sku_id,
       m.expected_on_hand,b.on_hand_quantity actual_on_hand,
       m.expected_on_hand-b.on_hand_quantity on_hand_difference,
       COALESCE(r.expected_reserved,0) expected_reserved,b.reserved_quantity actual_reserved,
       COALESCE(r.expected_reserved,0)-b.reserved_quantity reserved_difference,
       m.expected_value,b.inventory_value actual_value,m.expected_value-b.inventory_value value_difference,
       m.source_event_count,m.last_event_at
FROM inventory_balances b
JOIN movement m USING (legal_entity_id,warehouse_id,sku_id)
LEFT JOIN reservation r USING (legal_entity_id,warehouse_id,sku_id);

CREATE VIEW receivable_balance_reconciliation AS
WITH applied AS (
    SELECT receivable_id,
           COALESCE(sum(CASE WHEN allocation_type='apply' THEN amount ELSE -amount END),0)::NUMERIC(24,6) expected_settled,
           count(*) source_event_count,max(created_at) last_event_at
    FROM receivable_allocations GROUP BY receivable_id
)
SELECT r.id receivable_id,r.original_amount,
       COALESCE(a.expected_settled,0) expected_settled,r.settled_amount actual_settled,
       COALESCE(a.expected_settled,0)-r.settled_amount settled_difference,
       r.original_amount-COALESCE(a.expected_settled,0) expected_open,r.open_amount actual_open,
       (r.original_amount-COALESCE(a.expected_settled,0))-r.open_amount open_difference,
       COALESCE(a.source_event_count,0) source_event_count,a.last_event_at
FROM trade_receivables r LEFT JOIN applied a ON a.receivable_id=r.id
WHERE r.status <> 'reversed';
