-- Supplier delivery commitments, live purchase delivery risk and operational scorecards.

CREATE TABLE purchase_delivery_commitments (
    id UUID PRIMARY KEY,
    purchase_order_id UUID NOT NULL REFERENCES purchase_orders(id) ON DELETE RESTRICT,
    revision BIGINT NOT NULL CHECK (revision > 0),
    promised_delivery_date DATE NOT NULL,
    commitment_note TEXT CHECK (commitment_note IS NULL OR char_length(commitment_note) <= 1000),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','superseded')),
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    superseded_at TIMESTAMPTZ,
    trace_id UUID NOT NULL,
    UNIQUE (purchase_order_id, revision)
);
CREATE UNIQUE INDEX purchase_delivery_commitment_active_idx
    ON purchase_delivery_commitments(purchase_order_id) WHERE status='active';
CREATE INDEX purchase_delivery_commitment_date_idx
    ON purchase_delivery_commitments(promised_delivery_date) WHERE status='active';

CREATE TABLE purchase_delivery_commitment_events (
    id UUID PRIMARY KEY,
    purchase_delivery_commitment_id UUID NOT NULL
        REFERENCES purchase_delivery_commitments(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL CHECK (event_type IN ('committed','recommitted')),
    commitment_revision BIGINT NOT NULL CHECK (commitment_revision > 0),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    trace_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TRIGGER purchase_delivery_commitment_events_append_only
    BEFORE UPDATE OR DELETE ON purchase_delivery_commitment_events
    FOR EACH ROW EXECUTE FUNCTION business_core_append_only();

CREATE VIEW purchase_delivery_current AS
WITH line_totals AS (
    SELECT purchase_order_id,
           sum(ordered_quantity)::numeric(24,6) ordered_quantity,
           sum(received_quantity)::numeric(24,6) received_quantity,
           sum(cancelled_quantity)::numeric(24,6) cancelled_quantity
    FROM purchase_order_lines GROUP BY purchase_order_id
), receipt_dates AS (
    SELECT purchase_order_id,
           min(receipt_date) first_receipt_date,
           max(receipt_date) last_receipt_date,
           count(*) receipt_count
    FROM goods_receipts WHERE status='confirmed' GROUP BY purchase_order_id
), active_commitment AS (
    SELECT purchase_order_id,id commitment_id,revision commitment_revision,
           promised_delivery_date,commitment_note,created_at commitment_recorded_at
    FROM purchase_delivery_commitments WHERE status='active'
)
SELECT o.id purchase_order_id,o.purchase_order_number,o.legal_entity_id,
       o.supplier_id,s.code supplier_code,s.name supplier_name,o.buyer_user_id,
       o.order_date,o.expected_delivery_date,
       COALESCE(c.promised_delivery_date,o.expected_delivery_date) promised_delivery_date,
       CASE WHEN c.commitment_id IS NULL THEN 'planned' ELSE 'supplier_commitment' END commitment_source,
       c.commitment_id,COALESCE(c.commitment_revision,0)::bigint commitment_revision,
       c.commitment_note,c.commitment_recorded_at,
       o.lifecycle_status,o.receiving_status,o.currency::text,o.gross_amount,
       l.ordered_quantity,l.received_quantity,l.cancelled_quantity,
       GREATEST(l.ordered_quantity-l.received_quantity-l.cancelled_quantity,0)::numeric(24,6) open_quantity,
       COALESCE(r.receipt_count,0)::bigint receipt_count,r.first_receipt_date,r.last_receipt_date,
       CASE
         WHEN o.lifecycle_status='cancelled' OR o.receiving_status='cancelled' THEN 'cancelled'
         WHEN COALESCE(c.promised_delivery_date,o.expected_delivery_date) IS NULL THEN 'unscheduled'
         WHEN GREATEST(l.ordered_quantity-l.received_quantity-l.cancelled_quantity,0)=0
           AND r.last_receipt_date<=COALESCE(c.promised_delivery_date,o.expected_delivery_date)
           THEN 'completed_on_time'
         WHEN GREATEST(l.ordered_quantity-l.received_quantity-l.cancelled_quantity,0)=0
           THEN 'completed_late'
         WHEN COALESCE(c.promised_delivery_date,o.expected_delivery_date)<CURRENT_DATE THEN 'overdue'
         WHEN COALESCE(c.promised_delivery_date,o.expected_delivery_date)=CURRENT_DATE THEN 'due_today'
         WHEN COALESCE(c.promised_delivery_date,o.expected_delivery_date)<=CURRENT_DATE+3 THEN 'due_soon'
         ELSE 'on_track'
       END delivery_status,
       CASE
         WHEN COALESCE(c.promised_delivery_date,o.expected_delivery_date) IS NULL THEN NULL
         WHEN GREATEST(l.ordered_quantity-l.received_quantity-l.cancelled_quantity,0)=0
           THEN r.last_receipt_date-COALESCE(c.promised_delivery_date,o.expected_delivery_date)
         ELSE CURRENT_DATE-COALESCE(c.promised_delivery_date,o.expected_delivery_date)
       END delivery_variance_days,
       o.updated_at,o.version
FROM purchase_orders o
JOIN business_suppliers s ON s.id=o.supplier_id
JOIN line_totals l ON l.purchase_order_id=o.id
LEFT JOIN receipt_dates r ON r.purchase_order_id=o.id
LEFT JOIN active_commitment c ON c.purchase_order_id=o.id;

INSERT INTO business_role_permissions(role_id,permission_key)
SELECT role.id,permission.permission_key
FROM business_roles role
CROSS JOIN (VALUES
    ('purchase_delivery:read'),
    ('purchase_delivery_commitment:manage'),
    ('supplier_delivery_performance:read')
) permission(permission_key)
WHERE role.role_key='s1_operator'
ON CONFLICT DO NOTHING;
