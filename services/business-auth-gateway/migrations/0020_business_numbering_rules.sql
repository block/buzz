-- Governed, record-specific numbering rules used by Business Core commands.

CREATE TABLE business_numbering_rules (
    id UUID PRIMARY KEY,
    record_type TEXT NOT NULL UNIQUE CHECK (record_type IN (
        'sales_order','shipment','receivable','receipt','opening',
        'purchase_order','goods_receipt','payable','supplier_payment',
        'sales_return','purchase_return','inventory_count',
        'purchase_requisition','profit_adjustment','management_report'
    )),
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
    segments JSONB NOT NULL CHECK (jsonb_typeof(segments) = 'array'),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    updated_by_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TRIGGER business_numbering_rules_touch
    BEFORE UPDATE ON business_numbering_rules
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();

INSERT INTO business_numbering_rules(id,record_type,name,segments) VALUES
('20000000-0000-0000-0000-000000000001','sales_order','销售订单编码','[{"type":"fixed","value":"SO-"},{"type":"date","format":"YYYYMM"},{"type":"fixed","value":"-"},{"type":"sequence","width":6}]'),
('20000000-0000-0000-0000-000000000002','shipment','销售出库编码','[{"type":"fixed","value":"SHP-"},{"type":"date","format":"YYYYMM"},{"type":"fixed","value":"-"},{"type":"sequence","width":6}]'),
('20000000-0000-0000-0000-000000000003','receivable','经营应收编码','[{"type":"fixed","value":"AR-"},{"type":"date","format":"YYYYMM"},{"type":"fixed","value":"-"},{"type":"sequence","width":6}]'),
('20000000-0000-0000-0000-000000000004','receipt','客户收款编码','[{"type":"fixed","value":"RCPT-"},{"type":"date","format":"YYYYMM"},{"type":"fixed","value":"-"},{"type":"sequence","width":6}]'),
('20000000-0000-0000-0000-000000000005','opening','库存期初编码','[{"type":"fixed","value":"OPEN-"},{"type":"date","format":"YYYYMM"},{"type":"fixed","value":"-"},{"type":"sequence","width":6}]'),
('20000000-0000-0000-0000-000000000006','purchase_order','采购订单编码','[{"type":"fixed","value":"PO-"},{"type":"date","format":"YYYYMM"},{"type":"fixed","value":"-"},{"type":"sequence","width":6}]'),
('20000000-0000-0000-0000-000000000007','goods_receipt','采购入库编码','[{"type":"fixed","value":"GR-"},{"type":"date","format":"YYYYMM"},{"type":"fixed","value":"-"},{"type":"sequence","width":6}]'),
('20000000-0000-0000-0000-000000000008','payable','经营应付编码','[{"type":"fixed","value":"AP-"},{"type":"date","format":"YYYYMM"},{"type":"fixed","value":"-"},{"type":"sequence","width":6}]'),
('20000000-0000-0000-0000-000000000009','supplier_payment','供应商付款编码','[{"type":"fixed","value":"PAY-"},{"type":"date","format":"YYYYMM"},{"type":"fixed","value":"-"},{"type":"sequence","width":6}]'),
('20000000-0000-0000-0000-000000000010','sales_return','销售退货编码','[{"type":"fixed","value":"SRET-"},{"type":"date","format":"YYYYMM"},{"type":"fixed","value":"-"},{"type":"sequence","width":6}]'),
('20000000-0000-0000-0000-000000000011','purchase_return','采购退货编码','[{"type":"fixed","value":"PRET-"},{"type":"date","format":"YYYYMM"},{"type":"fixed","value":"-"},{"type":"sequence","width":6}]'),
('20000000-0000-0000-0000-000000000012','inventory_count','库存盘点编码','[{"type":"fixed","value":"CNT-"},{"type":"date","format":"YYYYMM"},{"type":"fixed","value":"-"},{"type":"sequence","width":6}]'),
('20000000-0000-0000-0000-000000000013','purchase_requisition','采购申请编码','[{"type":"fixed","value":"PRQ-"},{"type":"date","format":"YYYYMM"},{"type":"fixed","value":"-"},{"type":"sequence","width":6}]'),
('20000000-0000-0000-0000-000000000014','profit_adjustment','经营调整编码','[{"type":"fixed","value":"ADJ-"},{"type":"date","format":"YYYYMM"},{"type":"fixed","value":"-"},{"type":"sequence","width":6}]'),
('20000000-0000-0000-0000-000000000015','management_report','管理报表快照编码','[{"type":"fixed","value":"MGR-"},{"type":"date","format":"YYYYMM"},{"type":"fixed","value":"-"},{"type":"sequence","width":6}]');

INSERT INTO business_role_permissions(role_id,permission_key)
SELECT role.id,permission.permission_key
FROM business_roles role
CROSS JOIN (VALUES ('business_numbering_rules:read'),('business_numbering_rules:manage')) permission(permission_key)
WHERE role.role_key IN ('business_admin','s1_operator')
ON CONFLICT DO NOTHING;
