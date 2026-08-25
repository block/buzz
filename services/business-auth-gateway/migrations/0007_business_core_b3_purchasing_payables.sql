-- Business Core B3: purchasing, provisional receipt costing, operational
-- payables, supplier payments and append-only settlement facts.

ALTER TABLE business_suppliers
    ADD COLUMN payment_terms_days INTEGER NOT NULL DEFAULT 30
        CHECK (payment_terms_days BETWEEN 0 AND 3650);

CREATE SEQUENCE business_purchase_order_number_seq;
CREATE SEQUENCE business_goods_receipt_number_seq;
CREATE SEQUENCE business_trade_payable_number_seq;
CREATE SEQUENCE business_supplier_payment_number_seq;

CREATE TABLE purchase_orders (
    id UUID PRIMARY KEY,
    purchase_order_number TEXT NOT NULL UNIQUE,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    supplier_id UUID NOT NULL REFERENCES business_suppliers(id) ON DELETE RESTRICT,
    buyer_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    business_unit_id UUID NOT NULL REFERENCES business_units(id) ON DELETE RESTRICT,
    department_id UUID REFERENCES business_departments(id) ON DELETE RESTRICT,
    brand_id UUID REFERENCES business_brands(id) ON DELETE RESTRICT,
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    order_date DATE NOT NULL,
    expected_delivery_date DATE,
    payment_terms_days INTEGER NOT NULL CHECK (payment_terms_days BETWEEN 0 AND 3650),
    payment_terms_snapshot JSONB NOT NULL,
    lifecycle_status TEXT NOT NULL DEFAULT 'draft'
        CHECK (lifecycle_status IN ('draft','confirmed','completed','cancelled')),
    receiving_status TEXT NOT NULL DEFAULT 'unreceived'
        CHECK (receiving_status IN ('unreceived','partially_received','received','cancelled')),
    subtotal_amount NUMERIC(24,6) NOT NULL CHECK (subtotal_amount >= 0),
    discount_amount NUMERIC(24,6) NOT NULL CHECK (discount_amount >= 0),
    net_amount NUMERIC(24,6) NOT NULL CHECK (net_amount >= 0),
    tax_amount NUMERIC(24,6) NOT NULL CHECK (tax_amount >= 0),
    gross_amount NUMERIC(24,6) NOT NULL CHECK (gross_amount >= 0),
    supplier_reference TEXT CHECK (supplier_reference IS NULL OR char_length(supplier_reference) <= 120),
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
    CHECK (net_amount = subtotal_amount - discount_amount),
    CHECK (gross_amount = net_amount + tax_amount),
    CHECK (expected_delivery_date IS NULL OR expected_delivery_date >= order_date)
);
CREATE INDEX purchase_orders_scope_idx ON purchase_orders
    (legal_entity_id, supplier_id, order_date DESC);
CREATE INDEX purchase_orders_status_idx ON purchase_orders
    (lifecycle_status, receiving_status, updated_at DESC);

CREATE TABLE purchase_order_lines (
    id UUID PRIMARY KEY,
    purchase_order_id UUID NOT NULL REFERENCES purchase_orders(id) ON DELETE RESTRICT,
    line_number INTEGER NOT NULL CHECK (line_number > 0),
    sku_id UUID NOT NULL REFERENCES business_skus(id) ON DELETE RESTRICT,
    warehouse_id UUID NOT NULL REFERENCES business_warehouses(id) ON DELETE RESTRICT,
    unit_of_measure_id UUID NOT NULL REFERENCES business_units_of_measure(id) ON DELETE RESTRICT,
    ordered_quantity NUMERIC(24,6) NOT NULL CHECK (ordered_quantity > 0),
    cancelled_quantity NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (cancelled_quantity >= 0),
    received_quantity NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (received_quantity >= 0),
    unit_price NUMERIC(24,6) NOT NULL CHECK (unit_price >= 0),
    discount_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (discount_amount >= 0),
    net_amount NUMERIC(24,6) NOT NULL CHECK (net_amount >= 0),
    tax_rate NUMERIC(12,8) NOT NULL DEFAULT 0 CHECK (tax_rate >= 0 AND tax_rate <= 1),
    tax_amount NUMERIC(24,6) NOT NULL CHECK (tax_amount >= 0),
    gross_amount NUMERIC(24,6) NOT NULL CHECK (gross_amount >= 0),
    received_net_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (received_net_amount >= 0),
    received_tax_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (received_tax_amount >= 0),
    received_gross_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (received_gross_amount >= 0),
    received_inventory_cost_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (received_inventory_cost_amount >= 0),
    provisional_inventory_cost_amount NUMERIC(24,6) NOT NULL CHECK (provisional_inventory_cost_amount >= 0),
    cost_basis_type TEXT NOT NULL DEFAULT 'po_net_price' CHECK (cost_basis_type = 'po_net_price'),
    business_unit_id UUID NOT NULL REFERENCES business_units(id) ON DELETE RESTRICT,
    department_id UUID REFERENCES business_departments(id) ON DELETE RESTRICT,
    brand_id UUID REFERENCES business_brands(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    UNIQUE (purchase_order_id, line_number),
    CHECK (received_quantity + cancelled_quantity <= ordered_quantity),
    CHECK (gross_amount = net_amount + tax_amount),
    CHECK (received_net_amount <= net_amount AND received_tax_amount <= tax_amount
        AND received_gross_amount <= gross_amount)
);
CREATE INDEX purchase_order_lines_receiving_idx ON purchase_order_lines
    (warehouse_id, sku_id, purchase_order_id);

CREATE TABLE purchase_order_events (
    id UUID PRIMARY KEY,
    purchase_order_id UUID NOT NULL REFERENCES purchase_orders(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL,
    order_version BIGINT NOT NULL CHECK (order_version > 0),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    trace_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE goods_receipts (
    id UUID PRIMARY KEY,
    goods_receipt_number TEXT NOT NULL UNIQUE,
    purchase_order_id UUID NOT NULL REFERENCES purchase_orders(id) ON DELETE RESTRICT,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    supplier_id UUID NOT NULL REFERENCES business_suppliers(id) ON DELETE RESTRICT,
    warehouse_id UUID NOT NULL REFERENCES business_warehouses(id) ON DELETE RESTRICT,
    receipt_date DATE NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','confirmed','reversed','cancelled')),
    net_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (net_amount >= 0),
    tax_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (tax_amount >= 0),
    gross_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (gross_amount >= 0),
    inventory_cost_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (inventory_cost_amount >= 0),
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    confirmed_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at TIMESTAMPTZ,
    reversed_at TIMESTAMPTZ,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    trace_id UUID NOT NULL,
    CHECK (gross_amount = net_amount + tax_amount)
);
CREATE INDEX goods_receipts_order_idx ON goods_receipts (purchase_order_id, receipt_date DESC);
CREATE INDEX goods_receipts_scope_idx ON goods_receipts
    (legal_entity_id, supplier_id, warehouse_id, receipt_date DESC);

CREATE TABLE goods_receipt_lines (
    id UUID PRIMARY KEY,
    goods_receipt_id UUID NOT NULL REFERENCES goods_receipts(id) ON DELETE RESTRICT,
    purchase_order_line_id UUID NOT NULL REFERENCES purchase_order_lines(id) ON DELETE RESTRICT,
    sku_id UUID NOT NULL REFERENCES business_skus(id) ON DELETE RESTRICT,
    unit_of_measure_id UUID NOT NULL REFERENCES business_units_of_measure(id) ON DELETE RESTRICT,
    received_quantity NUMERIC(24,6) NOT NULL CHECK (received_quantity > 0),
    base_quantity NUMERIC(24,6) NOT NULL CHECK (base_quantity > 0),
    net_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (net_amount >= 0),
    tax_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (tax_amount >= 0),
    gross_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (gross_amount >= 0),
    provisional_unit_cost NUMERIC(24,6),
    provisional_total_cost NUMERIC(24,6),
    cost_basis_type TEXT NOT NULL DEFAULT 'po_net_price' CHECK (cost_basis_type = 'po_net_price'),
    cost_status TEXT NOT NULL DEFAULT 'provisional' CHECK (cost_status = 'provisional'),
    cost_snapshot_at TIMESTAMPTZ,
    inventory_movement_id UUID REFERENCES inventory_movements(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    UNIQUE (goods_receipt_id, purchase_order_line_id),
    CHECK (gross_amount = net_amount + tax_amount),
    CHECK ((provisional_unit_cost IS NULL AND provisional_total_cost IS NULL AND cost_snapshot_at IS NULL)
        OR (provisional_unit_cost IS NOT NULL AND provisional_total_cost IS NOT NULL
            AND provisional_unit_cost >= 0 AND provisional_total_cost >= 0 AND cost_snapshot_at IS NOT NULL))
);

CREATE TABLE goods_receipt_events (
    id UUID PRIMARY KEY,
    goods_receipt_id UUID NOT NULL REFERENCES goods_receipts(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL,
    receipt_version BIGINT NOT NULL CHECK (receipt_version > 0),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    trace_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE trade_payables (
    id UUID PRIMARY KEY,
    payable_number TEXT NOT NULL UNIQUE,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    supplier_id UUID NOT NULL REFERENCES business_suppliers(id) ON DELETE RESTRICT,
    purchase_order_id UUID NOT NULL REFERENCES purchase_orders(id) ON DELETE RESTRICT,
    goods_receipt_id UUID NOT NULL UNIQUE REFERENCES goods_receipts(id) ON DELETE RESTRICT,
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    original_amount NUMERIC(24,6) NOT NULL CHECK (original_amount >= 0),
    settled_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (settled_amount >= 0),
    open_amount NUMERIC(24,6) NOT NULL CHECK (open_amount >= 0),
    recognized_at TIMESTAMPTZ NOT NULL,
    due_date DATE NOT NULL,
    payment_terms_days INTEGER NOT NULL CHECK (payment_terms_days BETWEEN 0 AND 3650),
    payment_terms_snapshot JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','partially_settled','settled','reversed')),
    reversal_of_payable_id UUID REFERENCES trade_payables(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    trace_id UUID NOT NULL,
    CHECK (settled_amount + open_amount = original_amount)
);
CREATE INDEX trade_payables_supplier_idx ON trade_payables
    (legal_entity_id, supplier_id, due_date, status);

CREATE TABLE trade_payable_events (
    id UUID PRIMARY KEY,
    payable_id UUID NOT NULL REFERENCES trade_payables(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL,
    amount NUMERIC(24,6) NOT NULL CHECK (amount >= 0),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    trace_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE supplier_payments (
    id UUID PRIMARY KEY,
    supplier_payment_number TEXT NOT NULL UNIQUE,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    supplier_id UUID NOT NULL REFERENCES business_suppliers(id) ON DELETE RESTRICT,
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    payment_date DATE NOT NULL,
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
    CHECK (allocated_amount + unapplied_amount = CASE
        WHEN status IN ('draft','cancelled','reversed') THEN 0 ELSE amount END)
);
CREATE INDEX supplier_payments_supplier_idx ON supplier_payments
    (legal_entity_id, supplier_id, payment_date DESC);

CREATE TABLE supplier_payment_events (
    id UUID PRIMARY KEY,
    supplier_payment_id UUID NOT NULL REFERENCES supplier_payments(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL,
    amount NUMERIC(24,6) NOT NULL CHECK (amount >= 0),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    trace_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE payable_allocations (
    id UUID PRIMARY KEY,
    supplier_payment_id UUID NOT NULL REFERENCES supplier_payments(id) ON DELETE RESTRICT,
    payable_id UUID NOT NULL REFERENCES trade_payables(id) ON DELETE RESTRICT,
    allocation_type TEXT NOT NULL DEFAULT 'apply' CHECK (allocation_type IN ('apply','reversal')),
    amount NUMERIC(24,6) NOT NULL CHECK (amount > 0),
    reverses_allocation_id UUID REFERENCES payable_allocations(id) ON DELETE RESTRICT,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','reversed')),
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    trace_id UUID NOT NULL,
    CHECK ((allocation_type='apply' AND reverses_allocation_id IS NULL)
        OR (allocation_type='reversal' AND reverses_allocation_id IS NOT NULL))
);
CREATE UNIQUE INDEX payable_allocations_one_reversal
    ON payable_allocations (reverses_allocation_id) WHERE reverses_allocation_id IS NOT NULL;
CREATE INDEX payable_allocations_payment_idx ON payable_allocations (supplier_payment_id, created_at);
CREATE INDEX payable_allocations_payable_idx ON payable_allocations (payable_id, created_at);

ALTER TABLE inventory_movements DROP CONSTRAINT inventory_movements_movement_type_check;
ALTER TABLE inventory_movements ADD CONSTRAINT inventory_movements_movement_type_check
    CHECK (movement_type IN ('opening_balance','opening_balance_reversal','sales_shipment',
        'sales_shipment_reversal','purchase_receipt','purchase_receipt_reversal'));

CREATE TRIGGER purchase_order_events_append_only BEFORE UPDATE OR DELETE ON purchase_order_events
    FOR EACH ROW EXECUTE FUNCTION business_core_append_only();
CREATE TRIGGER goods_receipt_events_append_only BEFORE UPDATE OR DELETE ON goods_receipt_events
    FOR EACH ROW EXECUTE FUNCTION business_core_append_only();
CREATE TRIGGER trade_payable_events_append_only BEFORE UPDATE OR DELETE ON trade_payable_events
    FOR EACH ROW EXECUTE FUNCTION business_core_append_only();
CREATE TRIGGER supplier_payment_events_append_only BEFORE UPDATE OR DELETE ON supplier_payment_events
    FOR EACH ROW EXECUTE FUNCTION business_core_append_only();
CREATE TRIGGER payable_allocations_append_only BEFORE UPDATE OR DELETE ON payable_allocations
    FOR EACH ROW EXECUTE FUNCTION business_core_append_only();

CREATE TRIGGER purchase_orders_touch BEFORE UPDATE ON purchase_orders
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER purchase_order_lines_touch BEFORE UPDATE ON purchase_order_lines
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER goods_receipts_touch BEFORE UPDATE ON goods_receipts
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER trade_payables_touch BEFORE UPDATE ON trade_payables
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER supplier_payments_touch BEFORE UPDATE ON supplier_payments
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();

CREATE VIEW payable_balance_reconciliation AS
WITH applied AS (
    SELECT payable_id,
           COALESCE(sum(CASE WHEN allocation_type='apply' THEN amount ELSE -amount END),0)::NUMERIC(24,6) expected_settled,
           count(*) source_event_count,max(created_at) last_event_at
    FROM payable_allocations GROUP BY payable_id
)
SELECT p.id payable_id,p.original_amount,
       COALESCE(a.expected_settled,0) expected_settled,p.settled_amount actual_settled,
       COALESCE(a.expected_settled,0)-p.settled_amount settled_difference,
       p.original_amount-COALESCE(a.expected_settled,0) expected_open,p.open_amount actual_open,
       (p.original_amount-COALESCE(a.expected_settled,0))-p.open_amount open_difference,
       COALESCE(a.source_event_count,0) source_event_count,a.last_event_at
FROM trade_payables p LEFT JOIN applied a ON a.payable_id=p.id
WHERE p.status <> 'reversed';

CREATE VIEW supplier_payment_reconciliation AS
WITH applied AS (
    SELECT supplier_payment_id,
           COALESCE(sum(CASE WHEN allocation_type='apply' THEN amount ELSE -amount END),0)::NUMERIC(24,6) expected_allocated,
           count(*) source_event_count,max(created_at) last_event_at
    FROM payable_allocations GROUP BY supplier_payment_id
)
SELECT p.id supplier_payment_id,p.amount,
       COALESCE(a.expected_allocated,0) expected_allocated,p.allocated_amount actual_allocated,
       COALESCE(a.expected_allocated,0)-p.allocated_amount allocated_difference,
       CASE WHEN p.status IN ('draft','cancelled','reversed') THEN 0
            ELSE p.amount-COALESCE(a.expected_allocated,0) END expected_unapplied,
       p.unapplied_amount actual_unapplied,
       CASE WHEN p.status IN ('draft','cancelled','reversed') THEN -p.unapplied_amount
            ELSE p.amount-COALESCE(a.expected_allocated,0)-p.unapplied_amount END unapplied_difference,
       COALESCE(a.source_event_count,0) source_event_count,a.last_event_at
FROM supplier_payments p LEFT JOIN applied a ON a.supplier_payment_id=p.id;
