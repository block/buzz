-- Business Core returns: append-only sales and purchase return documents.

CREATE SEQUENCE business_sales_return_number_seq;
CREATE SEQUENCE business_purchase_return_number_seq;

CREATE TABLE sales_returns (
    id UUID PRIMARY KEY,
    return_number TEXT NOT NULL UNIQUE,
    shipment_id UUID NOT NULL REFERENCES shipments(id) ON DELETE RESTRICT,
    sales_order_id UUID NOT NULL REFERENCES sales_orders(id) ON DELETE RESTRICT,
    receivable_id UUID NOT NULL REFERENCES trade_receivables(id) ON DELETE RESTRICT,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    warehouse_id UUID NOT NULL REFERENCES business_warehouses(id) ON DELETE RESTRICT,
    customer_id UUID NOT NULL REFERENCES business_customers(id) ON DELETE RESTRICT,
    return_date DATE NOT NULL,
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    reason_code TEXT NOT NULL CHECK (char_length(reason_code) BETWEEN 1 AND 64),
    business_note TEXT CHECK (business_note IS NULL OR char_length(business_note) <= 1000),
    sales_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (sales_amount >= 0),
    cost_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (cost_amount >= 0),
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','confirmed','cancelled')),
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    confirmed_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at TIMESTAMPTZ,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    trace_id UUID NOT NULL
);
CREATE INDEX sales_returns_scope_idx ON sales_returns
    (legal_entity_id, customer_id, return_date DESC);

CREATE TABLE sales_return_lines (
    id UUID PRIMARY KEY,
    sales_return_id UUID NOT NULL REFERENCES sales_returns(id) ON DELETE RESTRICT,
    shipment_line_id UUID NOT NULL REFERENCES shipment_lines(id) ON DELETE RESTRICT,
    sku_id UUID NOT NULL REFERENCES business_skus(id) ON DELETE RESTRICT,
    quantity NUMERIC(24,6) NOT NULL CHECK (quantity > 0),
    sales_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (sales_amount >= 0),
    unit_cost NUMERIC(24,6),
    total_cost NUMERIC(24,6),
    inventory_movement_id UUID REFERENCES inventory_movements(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (sales_return_id, shipment_line_id)
);

CREATE TABLE sales_return_events (
    id UUID PRIMARY KEY,
    sales_return_id UUID NOT NULL REFERENCES sales_returns(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL,
    return_version BIGINT NOT NULL CHECK (return_version > 0),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    trace_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE purchase_returns (
    id UUID PRIMARY KEY,
    return_number TEXT NOT NULL UNIQUE,
    goods_receipt_id UUID NOT NULL REFERENCES goods_receipts(id) ON DELETE RESTRICT,
    purchase_order_id UUID NOT NULL REFERENCES purchase_orders(id) ON DELETE RESTRICT,
    payable_id UUID NOT NULL REFERENCES trade_payables(id) ON DELETE RESTRICT,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    warehouse_id UUID NOT NULL REFERENCES business_warehouses(id) ON DELETE RESTRICT,
    supplier_id UUID NOT NULL REFERENCES business_suppliers(id) ON DELETE RESTRICT,
    return_date DATE NOT NULL,
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    reason_code TEXT NOT NULL CHECK (char_length(reason_code) BETWEEN 1 AND 64),
    business_note TEXT CHECK (business_note IS NULL OR char_length(business_note) <= 1000),
    net_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (net_amount >= 0),
    tax_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (tax_amount >= 0),
    gross_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (gross_amount >= 0),
    inventory_cost_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (inventory_cost_amount >= 0),
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','confirmed','cancelled')),
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    confirmed_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at TIMESTAMPTZ,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    trace_id UUID NOT NULL,
    CHECK (gross_amount = net_amount + tax_amount)
);
CREATE INDEX purchase_returns_scope_idx ON purchase_returns
    (legal_entity_id, supplier_id, return_date DESC);

CREATE TABLE purchase_return_lines (
    id UUID PRIMARY KEY,
    purchase_return_id UUID NOT NULL REFERENCES purchase_returns(id) ON DELETE RESTRICT,
    goods_receipt_line_id UUID NOT NULL REFERENCES goods_receipt_lines(id) ON DELETE RESTRICT,
    sku_id UUID NOT NULL REFERENCES business_skus(id) ON DELETE RESTRICT,
    quantity NUMERIC(24,6) NOT NULL CHECK (quantity > 0),
    net_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (net_amount >= 0),
    tax_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (tax_amount >= 0),
    gross_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (gross_amount >= 0),
    unit_cost NUMERIC(24,6),
    total_cost NUMERIC(24,6),
    inventory_movement_id UUID REFERENCES inventory_movements(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (purchase_return_id, goods_receipt_line_id),
    CHECK (gross_amount = net_amount + tax_amount)
);

CREATE TABLE purchase_return_events (
    id UUID PRIMARY KEY,
    purchase_return_id UUID NOT NULL REFERENCES purchase_returns(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL,
    return_version BIGINT NOT NULL CHECK (return_version > 0),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    trace_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE inventory_movements DROP CONSTRAINT inventory_movements_movement_type_check;
ALTER TABLE inventory_movements ADD CONSTRAINT inventory_movements_movement_type_check
    CHECK (movement_type IN ('opening_balance','opening_balance_reversal',
        'sales_shipment','sales_shipment_reversal','purchase_receipt',
        'purchase_receipt_reversal','sales_return','purchase_return'));

DROP VIEW profit_projection_reconciliation;
CREATE VIEW profit_projection_reconciliation AS
WITH returned AS (
 SELECT l.shipment_line_id,sum(l.sales_amount) sales_amount,sum(l.total_cost) total_cost
 FROM sales_return_lines l JOIN sales_returns r ON r.id=l.sales_return_id
 WHERE r.status='confirmed' GROUP BY l.shipment_line_id
), expected AS (
 SELECT sl.id shipment_line_id,s.id shipment_id,s.sales_order_id,s.status,
   CASE WHEN s.status='confirmed' THEN sl.sales_amount-COALESCE(r.sales_amount,0) ELSE 0 END expected_revenue,
   CASE WHEN s.status='confirmed' THEN sl.total_cost-COALESCE(r.total_cost,0) ELSE 0 END expected_cost
 FROM shipments s JOIN shipment_lines sl ON sl.shipment_id=s.id
 LEFT JOIN returned r ON r.shipment_line_id=sl.id
 WHERE s.status IN ('confirmed','reversed')
), actual AS (
 SELECT shipment_line_id,
   COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER (WHERE metric_type='net_revenue'),0) actual_revenue,
   COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER (WHERE metric_type='product_cost'),0) actual_cost,
   count(*) fact_count,max(fact_sequence) last_fact_sequence
 FROM profit_facts WHERE source_type IN ('shipment','sales_return') GROUP BY shipment_line_id
)
SELECT e.*,COALESCE(a.actual_revenue,0)::NUMERIC(24,6) actual_revenue,
 (e.expected_revenue-COALESCE(a.actual_revenue,0))::NUMERIC(24,6) revenue_difference,
 COALESCE(a.actual_cost,0)::NUMERIC(24,6) actual_cost,
 (COALESCE(e.expected_cost,0)-COALESCE(a.actual_cost,0))::NUMERIC(24,6) cost_difference,
 COALESCE(a.fact_count,0) fact_count,a.last_fact_sequence
FROM expected e LEFT JOIN actual a USING(shipment_line_id);
