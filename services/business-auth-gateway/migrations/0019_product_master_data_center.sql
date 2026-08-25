-- Controlled product master data and product-specific unit conversions.

CREATE TABLE business_product_uom_conversions (
    id UUID PRIMARY KEY,
    product_id UUID NOT NULL REFERENCES business_products(id) ON DELETE RESTRICT,
    unit_of_measure_id UUID NOT NULL REFERENCES business_units_of_measure(id) ON DELETE RESTRICT,
    factor_to_base NUMERIC(24,8) NOT NULL CHECK (factor_to_base > 0),
    usage_scope TEXT NOT NULL DEFAULT 'both' CHECK (usage_scope IN ('sales','purchase','both')),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','disabled')),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(product_id, unit_of_measure_id)
);

CREATE TRIGGER business_product_uom_conversions_touch
BEFORE UPDATE ON business_product_uom_conversions
FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();

CREATE VIEW product_master_data_maintenance AS
SELECT 'unit_of_measure'::text resource_type,u.id,u.code,u.name,u.status,
       NULL::uuid product_id,NULL::text product_code,NULL::text product_name,
       NULL::uuid category_id,NULL::text category_code,NULL::text category_name,
       NULL::uuid parent_category_id,NULL::text parent_category_code,NULL::text parent_category_name,
       NULL::uuid brand_id,NULL::text brand_code,NULL::text brand_name,
       u.id unit_of_measure_id,u.code unit_of_measure_code,u.name unit_of_measure_name,
       NULL::text barcode,u.precision_scale,NULL::boolean allow_zero_cost,
       NULL::numeric factor_to_base,NULL::text usage_scope,u.version,u.updated_at
FROM business_units_of_measure u
UNION ALL
SELECT 'product_category',c.id,c.code,c.name,c.status,
       NULL,NULL,NULL,c.id,c.code,c.name,p.id,p.code,p.name,
       NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,c.version,c.updated_at
FROM business_product_categories c LEFT JOIN business_product_categories p ON p.id=c.parent_id
UNION ALL
SELECT 'brand',b.id,b.code,b.name,b.status,
       NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,b.id,b.code,b.name,
       NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,b.version,b.updated_at
FROM business_brands b
UNION ALL
SELECT 'product',p.id,p.code,p.name,p.status,
       p.id,p.code,p.name,c.id,c.code,c.name,c.parent_id,pc.code,pc.name,
       b.id,b.code,b.name,u.id,u.code,u.name,NULL,u.precision_scale,p.allow_zero_cost,
       NULL,NULL,p.version,p.updated_at
FROM business_products p JOIN business_product_categories c ON c.id=p.category_id
LEFT JOIN business_product_categories pc ON pc.id=c.parent_id
LEFT JOIN business_brands b ON b.id=p.brand_id
JOIN business_units_of_measure u ON u.id=p.base_uom_id
UNION ALL
SELECT 'sku',s.id,s.code,s.name,s.status,
       p.id,p.code,p.name,c.id,c.code,c.name,c.parent_id,pc.code,pc.name,
       b.id,b.code,b.name,u.id,u.code,u.name,s.barcode,u.precision_scale,p.allow_zero_cost,
       NULL,NULL,s.version,s.updated_at
FROM business_skus s JOIN business_products p ON p.id=s.product_id
JOIN business_product_categories c ON c.id=p.category_id
LEFT JOIN business_product_categories pc ON pc.id=c.parent_id
LEFT JOIN business_brands b ON b.id=p.brand_id
JOIN business_units_of_measure u ON u.id=p.base_uom_id
UNION ALL
SELECT 'uom_conversion',x.id,p.code||':'||u.code,p.name||' / '||u.name,x.status,
       p.id,p.code,p.name,c.id,c.code,c.name,c.parent_id,pc.code,pc.name,
       b.id,b.code,b.name,u.id,u.code,u.name,NULL,u.precision_scale,p.allow_zero_cost,
       x.factor_to_base,x.usage_scope,x.version,x.updated_at
FROM business_product_uom_conversions x JOIN business_products p ON p.id=x.product_id
JOIN business_product_categories c ON c.id=p.category_id
LEFT JOIN business_product_categories pc ON pc.id=c.parent_id
LEFT JOIN business_brands b ON b.id=p.brand_id
JOIN business_units_of_measure u ON u.id=x.unit_of_measure_id;

INSERT INTO business_role_permissions(role_id,permission_key)
SELECT role.id,permission.permission_key
FROM business_roles role
CROSS JOIN (VALUES ('business_product_master:read'),('business_product_master:manage')) permission(permission_key)
WHERE role.role_key IN ('business_admin','s1_operator')
ON CONFLICT DO NOTHING;
