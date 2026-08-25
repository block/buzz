-- Add only the six draft-creation capabilities exposed through the fixed,
-- turn-scoped Business Agent tool surface. Confirmation, approval, reversal,
-- allocation, posting and payment execution remain excluded.
INSERT INTO business_iam.permissions(
  id,capability,resource_type,action,risk_level
)
VALUES
  (gen_random_uuid(),'sales_order:create','sales_order','create','medium'),
  (gen_random_uuid(),'shipment:create','shipment','create','medium'),
  (gen_random_uuid(),'purchase_order:create','purchase_order','create','medium'),
  (gen_random_uuid(),'goods_receipt:create','goods_receipt','create','medium'),
  (gen_random_uuid(),'customer_receipt:create','customer_receipt','create','medium'),
  (gen_random_uuid(),'supplier_payment:create','supplier_payment','create','medium');
