-- Period-aware, scope-aware counters for governed numbering rules.

ALTER TABLE business_numbering_rules
    ADD COLUMN reset_period TEXT NOT NULL DEFAULT 'never'
        CHECK (reset_period IN ('never','yearly','monthly','daily')),
    ADD COLUMN scope_dimension TEXT NOT NULL DEFAULT 'global'
        CHECK (scope_dimension IN ('global','legal_entity','business_unit'));

CREATE TABLE business_numbering_sequence_pools (
    rule_id UUID NOT NULL REFERENCES business_numbering_rules(id) ON DELETE CASCADE,
    scope_key TEXT NOT NULL,
    period_key TEXT NOT NULL,
    current_value BIGINT NOT NULL CHECK (current_value >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (rule_id, scope_key, period_key)
);

CREATE INDEX business_numbering_sequence_pools_updated
    ON business_numbering_sequence_pools(updated_at DESC);

-- Default rules use a global, never-reset pool. Carry forward each legacy
-- PostgreSQL sequence watermark so an in-place upgrade cannot reuse a number.
INSERT INTO business_numbering_sequence_pools(rule_id,scope_key,period_key,current_value)
SELECT '20000000-0000-0000-0000-000000000001'::uuid,'global','*',CASE WHEN is_called THEN last_value ELSE 0 END FROM business_sales_order_number_seq
UNION ALL SELECT '20000000-0000-0000-0000-000000000002'::uuid,'global','*',CASE WHEN is_called THEN last_value ELSE 0 END FROM business_shipment_number_seq
UNION ALL SELECT '20000000-0000-0000-0000-000000000003'::uuid,'global','*',CASE WHEN is_called THEN last_value ELSE 0 END FROM business_receivable_number_seq
UNION ALL SELECT '20000000-0000-0000-0000-000000000004'::uuid,'global','*',CASE WHEN is_called THEN last_value ELSE 0 END FROM business_customer_receipt_number_seq
UNION ALL SELECT '20000000-0000-0000-0000-000000000005'::uuid,'global','*',CASE WHEN is_called THEN last_value ELSE 0 END FROM business_inventory_opening_number_seq
UNION ALL SELECT '20000000-0000-0000-0000-000000000006'::uuid,'global','*',CASE WHEN is_called THEN last_value ELSE 0 END FROM business_purchase_order_number_seq
UNION ALL SELECT '20000000-0000-0000-0000-000000000007'::uuid,'global','*',CASE WHEN is_called THEN last_value ELSE 0 END FROM business_goods_receipt_number_seq
UNION ALL SELECT '20000000-0000-0000-0000-000000000008'::uuid,'global','*',CASE WHEN is_called THEN last_value ELSE 0 END FROM business_trade_payable_number_seq
UNION ALL SELECT '20000000-0000-0000-0000-000000000009'::uuid,'global','*',CASE WHEN is_called THEN last_value ELSE 0 END FROM business_supplier_payment_number_seq
UNION ALL SELECT '20000000-0000-0000-0000-000000000010'::uuid,'global','*',CASE WHEN is_called THEN last_value ELSE 0 END FROM business_sales_return_number_seq
UNION ALL SELECT '20000000-0000-0000-0000-000000000011'::uuid,'global','*',CASE WHEN is_called THEN last_value ELSE 0 END FROM business_purchase_return_number_seq
UNION ALL SELECT '20000000-0000-0000-0000-000000000012'::uuid,'global','*',CASE WHEN is_called THEN last_value ELSE 0 END FROM business_inventory_count_number_seq
UNION ALL SELECT '20000000-0000-0000-0000-000000000013'::uuid,'global','*',CASE WHEN is_called THEN last_value ELSE 0 END FROM business_purchase_requisition_number_seq
UNION ALL SELECT '20000000-0000-0000-0000-000000000014'::uuid,'global','*',CASE WHEN is_called THEN last_value ELSE 0 END FROM business_profit_adjustment_number_seq
UNION ALL SELECT '20000000-0000-0000-0000-000000000015'::uuid,'global','*',CASE WHEN is_called THEN last_value ELSE 0 END FROM business_management_report_number_seq;
