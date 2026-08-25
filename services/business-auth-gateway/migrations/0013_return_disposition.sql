-- Return inspection, quarantine, supplier acknowledgment and operational analytics.

ALTER TABLE inventory_balances
    ADD COLUMN quarantined_quantity NUMERIC(24,6) NOT NULL DEFAULT 0
        CHECK (quarantined_quantity >= 0),
    ADD CONSTRAINT inventory_quarantine_within_on_hand
        CHECK (reserved_quantity + quarantined_quantity <= on_hand_quantity);

ALTER TABLE sales_returns
    ADD COLUMN inspection_status TEXT NOT NULL DEFAULT 'not_required'
        CHECK (inspection_status IN ('not_required','pending','completed')),
    ADD COLUMN inspected_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    ADD COLUMN inspected_at TIMESTAMPTZ,
    ADD COLUMN inspection_date DATE,
    ADD COLUMN inspection_note TEXT CHECK (inspection_note IS NULL OR char_length(inspection_note) <= 1000),
    ADD COLUMN scrap_cost_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (scrap_cost_amount >= 0);

ALTER TABLE sales_return_lines
    ADD COLUMN accepted_quantity NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (accepted_quantity >= 0),
    ADD COLUMN scrap_quantity NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (scrap_quantity >= 0),
    ADD COLUMN scrap_cost_amount NUMERIC(24,6) NOT NULL DEFAULT 0 CHECK (scrap_cost_amount >= 0),
    ADD CONSTRAINT sales_return_disposition_quantity_check
        CHECK (accepted_quantity + scrap_quantity <= quantity);

ALTER TABLE purchase_returns
    ADD COLUMN logistics_status TEXT NOT NULL DEFAULT 'not_dispatched'
        CHECK (logistics_status IN ('not_dispatched','dispatched','supplier_acknowledged')),
    ADD COLUMN dispatch_date DATE,
    ADD COLUMN carrier TEXT CHECK (carrier IS NULL OR char_length(carrier) <= 120),
    ADD COLUMN tracking_number TEXT CHECK (tracking_number IS NULL OR char_length(tracking_number) <= 120),
    ADD COLUMN dispatched_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    ADD COLUMN dispatched_at TIMESTAMPTZ,
    ADD COLUMN supplier_acknowledged_date DATE,
    ADD COLUMN supplier_acknowledgment_note TEXT CHECK (supplier_acknowledgment_note IS NULL OR char_length(supplier_acknowledgment_note) <= 1000),
    ADD COLUMN supplier_acknowledged_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    ADD COLUMN supplier_acknowledged_at TIMESTAMPTZ;

ALTER TABLE inventory_movements DROP CONSTRAINT inventory_movements_movement_type_check;
ALTER TABLE inventory_movements ADD CONSTRAINT inventory_movements_movement_type_check
    CHECK (movement_type IN ('opening_balance','opening_balance_reversal',
        'sales_shipment','sales_shipment_reversal','purchase_receipt',
        'purchase_receipt_reversal','sales_return','purchase_return',
        'sales_return_scrap'));

CREATE VIEW return_operating_metrics AS
WITH shipment_period AS (
    SELECT legal_entity_id,currency,date_trunc('month',shipment_date)::date management_period,
           sum(sales_amount)::numeric(24,6) shipped_sales_amount
    FROM shipments WHERE status='confirmed' GROUP BY 1,2,3
), sales_return_period AS (
    SELECT legal_entity_id,currency,date_trunc('month',return_date)::date management_period,
           count(*) return_count,sum(sales_amount)::numeric(24,6) return_sales_amount,
           sum(GREATEST(sales_amount-cost_amount+scrap_cost_amount,0))::numeric(24,6) return_loss_amount,
           sum(scrap_cost_amount)::numeric(24,6) scrap_cost_amount
    FROM sales_returns WHERE status='confirmed' GROUP BY 1,2,3
), receipt_period AS (
    SELECT legal_entity_id,currency,date_trunc('month',receipt_date)::date management_period,
           sum(gross_amount)::numeric(24,6) received_purchase_amount
    FROM goods_receipts WHERE status='confirmed' GROUP BY 1,2,3
), purchase_return_period AS (
    SELECT legal_entity_id,currency,date_trunc('month',return_date)::date management_period,
           count(*) return_count,sum(gross_amount)::numeric(24,6) return_purchase_amount
    FROM purchase_returns WHERE status='confirmed' GROUP BY 1,2,3
), keys AS (
    SELECT legal_entity_id,currency,management_period FROM shipment_period
    UNION SELECT legal_entity_id,currency,management_period FROM sales_return_period
    UNION SELECT legal_entity_id,currency,management_period FROM receipt_period
    UNION SELECT legal_entity_id,currency,management_period FROM purchase_return_period
)
SELECT k.legal_entity_id,k.currency::text currency,k.management_period,
       COALESCE(s.shipped_sales_amount,0)::numeric(24,6) shipped_sales_amount,
       COALESCE(sr.return_count,0)::bigint sales_return_count,
       COALESCE(sr.return_sales_amount,0)::numeric(24,6) sales_return_amount,
       CASE WHEN COALESCE(s.shipped_sales_amount,0)=0 THEN NULL
            ELSE (COALESCE(sr.return_sales_amount,0)/s.shipped_sales_amount)::numeric(24,8) END sales_return_rate,
       COALESCE(sr.return_loss_amount,0)::numeric(24,6) return_loss_amount,
       COALESCE(sr.scrap_cost_amount,0)::numeric(24,6) scrap_cost_amount,
       COALESCE(r.received_purchase_amount,0)::numeric(24,6) received_purchase_amount,
       COALESCE(pr.return_count,0)::bigint purchase_return_count,
       COALESCE(pr.return_purchase_amount,0)::numeric(24,6) purchase_return_amount,
       CASE WHEN COALESCE(r.received_purchase_amount,0)=0 THEN NULL
            ELSE (COALESCE(pr.return_purchase_amount,0)/r.received_purchase_amount)::numeric(24,8) END purchase_return_rate
FROM keys k
LEFT JOIN shipment_period s USING(legal_entity_id,currency,management_period)
LEFT JOIN sales_return_period sr USING(legal_entity_id,currency,management_period)
LEFT JOIN receipt_period r USING(legal_entity_id,currency,management_period)
LEFT JOIN purchase_return_period pr USING(legal_entity_id,currency,management_period);
