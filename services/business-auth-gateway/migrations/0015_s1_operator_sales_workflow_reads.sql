-- The S1 operator owns the end-to-end operating view. Keep its read access in
-- sync with the sales workflow page, whose stages span B2 services.

INSERT INTO business_role_permissions (role_id, permission_key)
SELECT role.id, permission.permission_key
FROM business_roles AS role
CROSS JOIN (
    VALUES
        ('receivable:read'),
        ('customer_receipt:read')
) AS permission(permission_key)
WHERE role.role_key = 's1_operator'
ON CONFLICT DO NOTHING;
