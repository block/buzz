# Action catalog

`trade-action-v1.0` is a server-owned, effective-dated catalog. Each entry has a deterministic ID and configuration hash, anomaly/resource allowlists, due-day policy, allowed assignee roles, optional draft type, and mandatory explicit-confirmation flag.

The bundled version contains 21 codes: `review_order_pricing`, `review_product_cost`, `review_freight_and_commission`, `review_customer_rebate_terms`, `request_margin_review`, `request_collection_plan`, `review_customer_credit`, `review_future_shipment_risk`, `request_sales_finance_joint_review`, `review_replenishment_plan`, `review_slow_moving_inventory`, `review_open_purchase_orders`, `review_inventory_data_quality`, `review_supplier_price_change`, `review_payment_terms`, `review_receipt_invoice_progress`, `request_supplier_review`, `request_cost_completion`, `request_relation_correction`, `request_status_reconciliation`, and `request_currency_or_unit_review`.

Models may explain returned entries but cannot invent a code, title template, or draft type. Startup rejects an unreadable path or mismatched catalog version.
