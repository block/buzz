-- Domain-oriented maintenance projection and permissions for core master data.

CREATE VIEW core_master_data_maintenance AS
SELECT 'legal_entity'::text resource_type,e.id,e.code,e.name,e.status,
       e.id legal_entity_id,e.code legal_entity_code,e.name legal_entity_name,
       NULL::uuid business_unit_id,NULL::text business_unit_code,NULL::text business_unit_name,
       e.country_code::text,e.functional_currency::text,e.registration_number,
       NULL::text address,NULL::text credit_currency,NULL::bigint credit_limit_minor,
       NULL::integer payment_terms_days,e.version,e.updated_at
FROM business_legal_entities e
UNION ALL
SELECT 'business_unit',u.id,u.code,u.name,u.status,
       e.id,e.code,e.name,u.id,u.code,u.name,
       NULL,NULL,NULL,NULL,NULL,NULL,NULL,u.version,u.updated_at
FROM business_units u JOIN business_legal_entities e ON e.id=u.legal_entity_id
UNION ALL
SELECT 'customer',c.id,c.code,c.name,c.status,
       e.id,e.code,e.name,u.id,u.code,u.name,
       NULL,NULL,NULL,NULL,c.credit_currency::text,c.credit_limit_minor,c.payment_terms_days,
       c.version,c.updated_at
FROM business_customers c JOIN business_legal_entities e ON e.id=c.legal_entity_id
JOIN business_units u ON u.id=c.business_unit_id
UNION ALL
SELECT 'supplier',s.id,s.code,s.name,s.status,
       e.id,e.code,e.name,u.id,u.code,u.name,
       NULL,NULL,NULL,NULL,NULL,NULL,s.payment_terms_days,s.version,s.updated_at
FROM business_suppliers s JOIN business_legal_entities e ON e.id=s.legal_entity_id
JOIN business_units u ON u.id=s.business_unit_id
UNION ALL
SELECT 'warehouse',w.id,w.code,w.name,w.status,
       e.id,e.code,e.name,u.id,u.code,u.name,
       NULL,NULL,NULL,w.address,NULL,NULL,NULL,w.version,w.updated_at
FROM business_warehouses w JOIN business_legal_entities e ON e.id=w.legal_entity_id
JOIN business_units u ON u.id=w.business_unit_id;

INSERT INTO business_role_permissions(role_id,permission_key)
SELECT role.id,permission.permission_key
FROM business_roles role
CROSS JOIN (VALUES ('business_master_data:read'),('business_master_data:manage')) permission(permission_key)
WHERE role.role_key IN ('business_admin','s1_operator')
ON CONFLICT DO NOTHING;
