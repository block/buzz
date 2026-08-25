-- Business Core B4: rebuildable management-profit facts, deterministic
-- operational adjustments, profitability projections and immutable reports.
-- These are management facts, not invoices, bank records or general ledger.

CREATE SEQUENCE business_profit_adjustment_number_seq;
CREATE SEQUENCE business_management_report_number_seq;
CREATE SEQUENCE business_profit_fact_sequence;

CREATE TABLE profit_facts (
    id UUID PRIMARY KEY,
    fact_sequence BIGINT NOT NULL DEFAULT nextval('business_profit_fact_sequence') UNIQUE,
    metric_type TEXT NOT NULL CHECK (metric_type IN (
        'net_revenue','product_cost','outbound_freight','sales_commission',
        'platform_fee','customer_rebate','supplier_rebate','other_direct_cost',
        'allocated_operating_expense')),
    direction TEXT NOT NULL CHECK (direction IN ('normal','reversal')),
    amount NUMERIC(24,6) NOT NULL CHECK (amount >= 0),
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    quantity NUMERIC(24,6) CHECK (quantity IS NULL OR quantity >= 0),
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    sales_order_id UUID NOT NULL REFERENCES sales_orders(id) ON DELETE RESTRICT,
    sales_order_line_id UUID REFERENCES sales_order_lines(id) ON DELETE RESTRICT,
    shipment_id UUID REFERENCES shipments(id) ON DELETE RESTRICT,
    shipment_line_id UUID REFERENCES shipment_lines(id) ON DELETE RESTRICT,
    customer_id UUID NOT NULL REFERENCES business_customers(id) ON DELETE RESTRICT,
    sku_id UUID REFERENCES business_skus(id) ON DELETE RESTRICT,
    product_category_id UUID REFERENCES business_product_categories(id) ON DELETE RESTRICT,
    brand_id UUID REFERENCES business_brands(id) ON DELETE RESTRICT,
    salesperson_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    business_unit_id UUID NOT NULL REFERENCES business_units(id) ON DELETE RESTRICT,
    department_id UUID REFERENCES business_departments(id) ON DELETE RESTRICT,
    warehouse_id UUID REFERENCES business_warehouses(id) ON DELETE RESTRICT,
    business_date DATE NOT NULL,
    management_period CHAR(7) NOT NULL CHECK (management_period ~ '^[0-9]{4}-(0[1-9]|1[0-2])$'),
    source_system TEXT NOT NULL CHECK (source_system IN ('business_core_b2','business_core_b4')),
    source_type TEXT NOT NULL CHECK (source_type IN ('shipment','operational_adjustment')),
    source_id UUID NOT NULL,
    source_line_id UUID NOT NULL,
    source_event_id UUID NOT NULL,
    source_event_version BIGINT NOT NULL CHECK (source_event_version > 0),
    data_as_of TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    trace_id UUID NOT NULL,
    UNIQUE (source_event_id, metric_type, source_line_id, direction)
);
CREATE INDEX profit_facts_order_idx ON profit_facts (sales_order_id, fact_sequence);
CREATE INDEX profit_facts_period_entity_idx ON profit_facts (management_period, legal_entity_id, currency, fact_sequence);
CREATE INDEX profit_facts_customer_idx ON profit_facts (customer_id, management_period, currency);
CREATE INDEX profit_facts_sku_idx ON profit_facts (sku_id, management_period, currency) WHERE sku_id IS NOT NULL;
CREATE INDEX profit_facts_brand_idx ON profit_facts (brand_id, management_period, currency) WHERE brand_id IS NOT NULL;
CREATE INDEX profit_facts_salesperson_idx ON profit_facts (salesperson_user_id, management_period, currency);

CREATE TABLE profit_projection_offsets (
    consumer_name TEXT PRIMARY KEY CHECK (consumer_name ~ '^[a-z][a-z0-9_-]{2,63}$'),
    last_outbox_created_at TIMESTAMPTZ,
    last_outbox_event_id UUID,
    last_fact_sequence BIGINT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0)
);

CREATE TABLE profit_projection_failures (
    id UUID PRIMARY KEY,
    outbox_event_id UUID NOT NULL REFERENCES business_core_outbox(id) ON DELETE RESTRICT,
    topic TEXT NOT NULL,
    aggregate_id UUID NOT NULL,
    error_code TEXT NOT NULL,
    retry_count INTEGER NOT NULL DEFAULT 0 CHECK (retry_count >= 0),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','resolved')),
    error_summary TEXT NOT NULL CHECK (char_length(error_summary) <= 500),
    first_failed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_failed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ,
    trace_id UUID NOT NULL,
    UNIQUE (outbox_event_id)
);

CREATE TABLE operational_adjustment_batches (
    id UUID PRIMARY KEY,
    adjustment_number TEXT NOT NULL UNIQUE,
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    management_period CHAR(7) NOT NULL CHECK (management_period ~ '^[0-9]{4}-(0[1-9]|1[0-2])$'),
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','previewed','posted','reversed','cancelled')),
    rule_version TEXT NOT NULL DEFAULT 'profit-allocation-v1',
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    updated_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    previewed_at TIMESTAMPTZ,
    posted_at TIMESTAMPTZ,
    reversed_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    trace_id UUID NOT NULL
);
CREATE INDEX operational_adjustment_batches_scope_idx ON operational_adjustment_batches (legal_entity_id, management_period, currency, status);

CREATE TABLE operational_adjustment_lines (
    id UUID PRIMARY KEY,
    batch_id UUID NOT NULL REFERENCES operational_adjustment_batches(id) ON DELETE RESTRICT,
    line_number INTEGER NOT NULL CHECK (line_number > 0),
    metric_type TEXT NOT NULL CHECK (metric_type IN (
        'outbound_freight','sales_commission','platform_fee','customer_rebate',
        'supplier_rebate','other_direct_cost','allocated_operating_expense')),
    amount NUMERIC(24,6) NOT NULL CHECK (amount > 0),
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    business_date DATE NOT NULL,
    management_period CHAR(7) NOT NULL CHECK (management_period ~ '^[0-9]{4}-(0[1-9]|1[0-2])$'),
    legal_entity_id UUID NOT NULL REFERENCES business_legal_entities(id) ON DELETE RESTRICT,
    direct_sales_order_id UUID REFERENCES sales_orders(id) ON DELETE RESTRICT,
    direct_sales_order_line_id UUID REFERENCES sales_order_lines(id) ON DELETE RESTRICT,
    direct_shipment_id UUID REFERENCES shipments(id) ON DELETE RESTRICT,
    customer_id UUID REFERENCES business_customers(id) ON DELETE RESTRICT,
    sku_id UUID REFERENCES business_skus(id) ON DELETE RESTRICT,
    brand_id UUID REFERENCES business_brands(id) ON DELETE RESTRICT,
    salesperson_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    business_unit_id UUID REFERENCES business_units(id) ON DELETE RESTRICT,
    department_id UUID REFERENCES business_departments(id) ON DELETE RESTRICT,
    warehouse_id UUID REFERENCES business_warehouses(id) ON DELETE RESTRICT,
    allocation_basis TEXT NOT NULL CHECK (allocation_basis IN ('direct','net_revenue','product_cost','shipped_quantity','fixed_weight')),
    allocation_scope JSONB NOT NULL DEFAULT '{}'::jsonb,
    source_reference TEXT CHECK (source_reference IS NULL OR char_length(source_reference) <= 120),
    reason_code TEXT NOT NULL CHECK (reason_code ~ '^[A-Z][A-Z0-9_]{1,63}$'),
    business_note TEXT CHECK (business_note IS NULL OR char_length(business_note) <= 1000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    UNIQUE (batch_id, line_number),
    CHECK ((allocation_basis = 'direct' AND direct_sales_order_id IS NOT NULL)
        OR (allocation_basis <> 'direct' AND direct_sales_order_id IS NULL))
);

CREATE TABLE operational_adjustment_previews (
    id UUID PRIMARY KEY,
    batch_id UUID NOT NULL REFERENCES operational_adjustment_batches(id) ON DELETE RESTRICT,
    preview_hash CHAR(64) NOT NULL CHECK (preview_hash ~ '^[a-f0-9]{64}$'),
    source_hash CHAR(64) NOT NULL CHECK (source_hash ~ '^[a-f0-9]{64}$'),
    source_watermark BIGINT NOT NULL CHECK (source_watermark >= 0),
    batch_version BIGINT NOT NULL CHECK (batch_version > 0),
    total_amount NUMERIC(24,6) NOT NULL CHECK (total_amount >= 0),
    allocated_amount NUMERIC(24,6) NOT NULL CHECK (allocated_amount >= 0),
    unallocated_amount NUMERIC(24,6) NOT NULL CHECK (unallocated_amount >= 0),
    payload JSONB NOT NULL,
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    trace_id UUID NOT NULL,
    UNIQUE (batch_id, preview_hash)
);

CREATE TABLE operational_adjustment_allocations (
    id UUID PRIMARY KEY,
    batch_id UUID NOT NULL REFERENCES operational_adjustment_batches(id) ON DELETE RESTRICT,
    adjustment_line_id UUID NOT NULL REFERENCES operational_adjustment_lines(id) ON DELETE RESTRICT,
    preview_id UUID NOT NULL REFERENCES operational_adjustment_previews(id) ON DELETE RESTRICT,
    sales_order_id UUID NOT NULL REFERENCES sales_orders(id) ON DELETE RESTRICT,
    weight NUMERIC(36,12) NOT NULL CHECK (weight >= 0),
    allocated_amount NUMERIC(24,6) NOT NULL CHECK (allocated_amount >= 0),
    remainder_rank INTEGER NOT NULL CHECK (remainder_rank >= 0),
    profit_fact_id UUID UNIQUE REFERENCES profit_facts(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    trace_id UUID NOT NULL,
    UNIQUE (adjustment_line_id, sales_order_id)
);
CREATE INDEX operational_adjustment_allocations_order_idx ON operational_adjustment_allocations (sales_order_id, batch_id);

CREATE TABLE operational_adjustment_events (
    id UUID PRIMARY KEY,
    batch_id UUID NOT NULL REFERENCES operational_adjustment_batches(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL CHECK (event_type IN ('created','updated','previewed','posted','reversed','cancelled')),
    batch_version BIGINT NOT NULL CHECK (batch_version > 0),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    trace_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (batch_id, batch_version, event_type)
);

CREATE TABLE management_report_snapshots (
    id UUID PRIMARY KEY,
    snapshot_number TEXT NOT NULL UNIQUE,
    report_type TEXT NOT NULL CHECK (report_type IN ('management_profit_statement','profitability_by_dimension')),
    management_period CHAR(7) NOT NULL CHECK (management_period ~ '^[0-9]{4}-(0[1-9]|1[0-2])$'),
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    scope JSONB NOT NULL,
    scope_hash CHAR(64) NOT NULL CHECK (scope_hash ~ '^[a-f0-9]{64}$'),
    rule_version TEXT NOT NULL,
    source_watermark BIGINT NOT NULL CHECK (source_watermark >= 0),
    source_hash CHAR(64) NOT NULL CHECK (source_hash ~ '^[a-f0-9]{64}$'),
    status TEXT NOT NULL DEFAULT 'generated' CHECK (status IN ('generated','superseded')),
    generated_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    generated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    supersedes_snapshot_id UUID REFERENCES management_report_snapshots(id) ON DELETE RESTRICT,
    data_as_of TIMESTAMPTZ NOT NULL,
    trace_id UUID NOT NULL,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    UNIQUE (report_type, management_period, currency, scope_hash, rule_version, source_watermark)
);
CREATE INDEX management_report_snapshots_lookup_idx ON management_report_snapshots (management_period, currency, generated_at DESC);

CREATE TABLE management_report_snapshot_rows (
    id UUID PRIMARY KEY,
    snapshot_id UUID NOT NULL REFERENCES management_report_snapshots(id) ON DELETE RESTRICT,
    row_key TEXT NOT NULL,
    dimension_type TEXT,
    dimension_id TEXT,
    amounts JSONB NOT NULL,
    data_quality_status TEXT NOT NULL CHECK (data_quality_status IN ('complete','partial','blocked')),
    warnings JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (snapshot_id, row_key)
);

CREATE TABLE management_report_snapshot_evidence (
    id UUID PRIMARY KEY,
    snapshot_id UUID NOT NULL REFERENCES management_report_snapshots(id) ON DELETE RESTRICT,
    evidence_type TEXT NOT NULL,
    source_id UUID,
    source_watermark BIGINT NOT NULL,
    source_hash CHAR(64) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TRIGGER profit_facts_append_only BEFORE UPDATE OR DELETE ON profit_facts
    FOR EACH ROW EXECUTE FUNCTION business_core_audit_append_only();
CREATE TRIGGER operational_adjustment_previews_append_only BEFORE UPDATE OR DELETE ON operational_adjustment_previews
    FOR EACH ROW EXECUTE FUNCTION business_core_audit_append_only();
CREATE TRIGGER operational_adjustment_allocations_append_only BEFORE UPDATE OR DELETE ON operational_adjustment_allocations
    FOR EACH ROW EXECUTE FUNCTION business_core_audit_append_only();
CREATE TRIGGER operational_adjustment_events_append_only BEFORE UPDATE OR DELETE ON operational_adjustment_events
    FOR EACH ROW EXECUTE FUNCTION business_core_audit_append_only();
CREATE TRIGGER management_report_snapshots_append_only BEFORE UPDATE OR DELETE ON management_report_snapshots
    FOR EACH ROW EXECUTE FUNCTION business_core_audit_append_only();
CREATE TRIGGER management_report_snapshot_rows_append_only BEFORE UPDATE OR DELETE ON management_report_snapshot_rows
    FOR EACH ROW EXECUTE FUNCTION business_core_audit_append_only();
CREATE TRIGGER management_report_snapshot_evidence_append_only BEFORE UPDATE OR DELETE ON management_report_snapshot_evidence
    FOR EACH ROW EXECUTE FUNCTION business_core_audit_append_only();

CREATE TRIGGER operational_adjustment_batches_touch BEFORE UPDATE ON operational_adjustment_batches
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER operational_adjustment_lines_touch BEFORE UPDATE ON operational_adjustment_lines
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
CREATE TRIGGER profit_projection_offsets_touch BEFORE UPDATE ON profit_projection_offsets
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();

CREATE VIEW order_profit_component_current AS
SELECT sales_order_id, legal_entity_id, customer_id, currency, metric_type,
       sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END)::NUMERIC(24,6) amount,
       max(fact_sequence) last_fact_sequence, max(data_as_of) data_as_of
FROM profit_facts
GROUP BY sales_order_id, legal_entity_id, customer_id, currency, metric_type;

CREATE VIEW order_profit_current AS
WITH p AS (
  SELECT sales_order_id,legal_entity_id,customer_id,currency,
    COALESCE(sum(signed_amount) FILTER (WHERE metric_type='net_revenue'),0)::NUMERIC(24,6) net_revenue,
    COALESCE(sum(signed_amount) FILTER (WHERE metric_type='product_cost'),0)::NUMERIC(24,6) product_cost,
    COALESCE(sum(signed_amount) FILTER (WHERE metric_type='outbound_freight'),0)::NUMERIC(24,6) outbound_freight,
    COALESCE(sum(signed_amount) FILTER (WHERE metric_type='sales_commission'),0)::NUMERIC(24,6) sales_commission,
    COALESCE(sum(signed_amount) FILTER (WHERE metric_type='platform_fee'),0)::NUMERIC(24,6) platform_fee,
    COALESCE(sum(signed_amount) FILTER (WHERE metric_type='customer_rebate'),0)::NUMERIC(24,6) customer_rebate,
    COALESCE(sum(signed_amount) FILTER (WHERE metric_type='supplier_rebate'),0)::NUMERIC(24,6) supplier_rebate,
    COALESCE(sum(signed_amount) FILTER (WHERE metric_type='other_direct_cost'),0)::NUMERIC(24,6) other_direct_cost,
    COALESCE(sum(signed_amount) FILTER (WHERE metric_type='allocated_operating_expense'),0)::NUMERIC(24,6) allocated_operating_expense,
    max(fact_sequence) last_fact_sequence,max(data_as_of) data_as_of
  FROM (SELECT *,CASE direction WHEN 'normal' THEN amount ELSE -amount END signed_amount FROM profit_facts) signed
  GROUP BY sales_order_id,legal_entity_id,customer_id,currency
), q AS (
  SELECT p.*,(net_revenue-product_cost)::NUMERIC(24,6) gross_profit,
    (net_revenue-product_cost-outbound_freight-sales_commission-platform_fee-customer_rebate-other_direct_cost+supplier_rebate)::NUMERIC(24,6) contribution_profit
  FROM p
)
SELECT q.*,(contribution_profit-allocated_operating_expense)::NUMERIC(24,6) management_operating_profit,
  CASE WHEN net_revenue=0 THEN NULL ELSE round(gross_profit/net_revenue,8) END gross_margin_rate,
  CASE WHEN net_revenue=0 THEN NULL ELSE round(contribution_profit/net_revenue,8) END contribution_margin_rate,
  CASE WHEN net_revenue=0 THEN NULL ELSE round((contribution_profit-allocated_operating_expense)/net_revenue,8) END management_operating_margin_rate,
  CASE WHEN EXISTS(SELECT 1 FROM profit_projection_failures f WHERE f.status='pending') THEN 'blocked' ELSE 'complete' END data_quality_status
FROM q;

CREATE VIEW profit_projection_reconciliation AS
WITH expected AS (
 SELECT sl.id shipment_line_id,s.id shipment_id,s.sales_order_id,s.status,
   CASE WHEN s.status='confirmed' THEN sl.sales_amount ELSE 0 END expected_revenue,
   CASE WHEN s.status='confirmed' THEN sl.total_cost ELSE 0 END expected_cost
 FROM shipments s JOIN shipment_lines sl ON sl.shipment_id=s.id
 WHERE s.status IN ('confirmed','reversed')
), actual AS (
 SELECT shipment_line_id,
   COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER (WHERE metric_type='net_revenue'),0) actual_revenue,
   COALESCE(sum(CASE direction WHEN 'normal' THEN amount ELSE -amount END) FILTER (WHERE metric_type='product_cost'),0) actual_cost,
   count(*) fact_count,max(fact_sequence) last_fact_sequence
 FROM profit_facts WHERE source_type='shipment' GROUP BY shipment_line_id
)
SELECT e.*,COALESCE(a.actual_revenue,0)::NUMERIC(24,6) actual_revenue,
 (e.expected_revenue-COALESCE(a.actual_revenue,0))::NUMERIC(24,6) revenue_difference,
 COALESCE(a.actual_cost,0)::NUMERIC(24,6) actual_cost,
 (COALESCE(e.expected_cost,0)-COALESCE(a.actual_cost,0))::NUMERIC(24,6) cost_difference,
 COALESCE(a.fact_count,0) fact_count,a.last_fact_sequence
FROM expected e LEFT JOIN actual a USING(shipment_line_id);
