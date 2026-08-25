-- Business Core S1.3: index-supported operating dashboards and stability reads.
-- These indexes cover business operations only; they do not introduce
-- invoices, journals, ledgers, tax, or statutory reporting.

CREATE INDEX sales_orders_operating_dashboard_idx ON sales_orders
    (currency, order_date, legal_entity_id, customer_id, business_unit_id)
    INCLUDE (brand_id, gross_amount, lifecycle_status, fulfillment_status, hold_status);

CREATE INDEX shipments_operating_dashboard_idx ON shipments
    (currency, shipment_date, legal_entity_id, customer_id, warehouse_id)
    INCLUDE (sales_order_id, status, sales_amount, cost_amount);

CREATE INDEX purchase_orders_operating_dashboard_idx ON purchase_orders
    (currency, order_date, legal_entity_id, supplier_id, business_unit_id)
    INCLUDE (brand_id, gross_amount, receiving_status);

CREATE INDEX purchase_order_lines_dashboard_idx ON purchase_order_lines
    (purchase_order_id, warehouse_id)
    INCLUDE (ordered_quantity, received_quantity, cancelled_quantity);

CREATE INDEX profit_facts_operating_dashboard_idx ON profit_facts
    (management_period, currency, legal_entity_id, customer_id, business_unit_id)
    INCLUDE (brand_id, warehouse_id, metric_type, direction, amount, fact_sequence);

CREATE INDEX profit_projection_failures_pending_idx ON profit_projection_failures
    (aggregate_id, last_failed_at DESC)
    WHERE status = 'pending';

CREATE INDEX business_core_outbox_profit_projection_idx ON business_core_outbox
    (created_at, id)
    WHERE topic IN ('shipment_confirmed', 'shipment_reversed');
