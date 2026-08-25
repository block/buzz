import React from "react";
import {
  type MasterDataList,
  type MasterDataRecord,
  type PurchaseOrderEntryOptions,
  request,
} from "./api";
import { formatAmount } from "./formatters";

type LineDraft = {
  key: string;
  skuId: string;
  warehouseId: string;
  unitOfMeasureId: string;
  quantity: string;
  unitPrice: string;
  discountAmount: string;
  taxPercent: string;
};

type Catalog = {
  legalEntities: MasterDataRecord[];
  suppliers: MasterDataRecord[];
  businessUnits: MasterDataRecord[];
  skus: MasterDataRecord[];
  warehouses: MasterDataRecord[];
  units: MasterDataRecord[];
};

const emptyCatalog: Catalog = {
  legalEntities: [],
  suppliers: [],
  businessUnits: [],
  skus: [],
  warehouses: [],
  units: [],
};

export function PurchaseOrderEntry({
  orderId,
  onDone,
}: {
  orderId?: string;
  onDone: () => void;
}) {
  const [catalog, setCatalog] = React.useState<Catalog>(emptyCatalog);
  const [options, setOptions] = React.useState<PurchaseOrderEntryOptions>();
  const [loading, setLoading] = React.useState(true);
  const [legalEntityId, setLegalEntityId] = React.useState("");
  const [supplierId, setSupplierId] = React.useState("");
  const [businessUnitId, setBusinessUnitId] = React.useState("");
  const [orderDate, setOrderDate] = React.useState(today());
  const [expectedDeliveryDate, setExpectedDeliveryDate] = React.useState("");
  const [paymentTermsDays, setPaymentTermsDays] = React.useState("30");
  const [supplierReference, setSupplierReference] = React.useState("");
  const [businessNote, setBusinessNote] = React.useState("");
  const [version, setVersion] = React.useState(1);
  const [lines, setLines] = React.useState<LineDraft[]>([newLine()]);
  const [busy, setBusy] = React.useState(false);
  const [notice, setNotice] = React.useState("");

  React.useEffect(() => {
    let active = true;
    const contextPath = orderId
      ? `/api/v1/purchase-orders/entry-options?orderId=${encodeURIComponent(orderId)}`
      : "/api/v1/purchase-orders/entry-options";
    Promise.all([
      request<PurchaseOrderEntryOptions>(contextPath),
      loadMaster("legal_entity"),
      loadMaster("supplier"),
      loadMaster("business_unit"),
      loadMaster("sku"),
      loadMaster("warehouse"),
      loadMaster("unit_of_measure"),
    ])
      .then(([entry, legalEntities, suppliers, businessUnits, skus, warehouses, units]) => {
        if (!active) return;
        const nextCatalog = { legalEntities, suppliers, businessUnits, skus, warehouses, units };
        setCatalog(nextCatalog);
        setOptions(entry);
        if (entry.draft) {
          setLegalEntityId(entry.draft.legalEntityId);
          setSupplierId(entry.draft.supplierId);
          setBusinessUnitId(entry.draft.businessUnitId);
          setOrderDate(entry.draft.orderDate);
          setExpectedDeliveryDate(entry.draft.expectedDeliveryDate ?? "");
          setPaymentTermsDays(String(entry.draft.paymentTermsDays));
          setSupplierReference(entry.draft.supplierReference ?? "");
          setBusinessNote(entry.draft.businessNote ?? "");
          setVersion(entry.draft.version);
          setLines(
            entry.draft.lines.map((line) => ({
              key: crypto.randomUUID(),
              skuId: line.skuId,
              warehouseId: line.warehouseId,
              unitOfMeasureId: line.unitOfMeasureId,
              quantity: line.quantity,
              unitPrice: line.unitPrice,
              discountAmount: line.discountAmount,
              taxPercent: String(amount(line.taxRate) * 100),
            })),
          );
        } else {
          const legalEntity = legalEntities[0]?.id ?? "";
          setLegalEntityId(legalEntity);
          setSupplierId(scoped(suppliers, legalEntity)[0]?.id ?? "");
          setBusinessUnitId(scoped(businessUnits, legalEntity)[0]?.id ?? "");
          setLines([newLine(skus[0]?.id, scoped(warehouses, legalEntity)[0]?.id, units[0]?.id)]);
        }
      })
      .catch((error: Error) => active && setNotice(error.message))
      .finally(() => active && setLoading(false));
    return () => {
      active = false;
    };
  }, [orderId]);

  const availableSuppliers = scoped(catalog.suppliers, legalEntityId);
  const availableBusinessUnits = scoped(catalog.businessUnits, legalEntityId);
  const availableWarehouses = scoped(catalog.warehouses, legalEntityId);
  const allowed = orderId ? options?.canUpdate : options?.canCreate;

  function changeLegalEntity(value: string) {
    setLegalEntityId(value);
    setSupplierId(scoped(catalog.suppliers, value)[0]?.id ?? "");
    setBusinessUnitId(scoped(catalog.businessUnits, value)[0]?.id ?? "");
    const warehouseId = scoped(catalog.warehouses, value)[0]?.id ?? "";
    setLines((current) => current.map((line) => ({ ...line, warehouseId })));
  }

  function updateLine(key: string, field: keyof LineDraft, value: string) {
    setLines((current) => current.map((line) => (line.key === key ? { ...line, [field]: value } : line)));
  }

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setNotice("");
    if (!allowed) return;
    if (!legalEntityId || !supplierId || !businessUnitId) {
      setNotice("请选择法律主体、供应商和业务单元。");
      return;
    }
    if (expectedDeliveryDate && expectedDeliveryDate < orderDate) {
      setNotice("预计交付日不能早于订单日期。");
      return;
    }
    const terms = Number(paymentTermsDays);
    if (!Number.isInteger(terms) || terms < 0 || terms > 3650) {
      setNotice("账期天数必须是 0–3650 的整数。");
      return;
    }
    if (lines.some((line) => !validLine(line))) {
      setNotice("请补全商品行；数量须大于 0，单价、折扣和税率不能为负，税率不能超过 100%。");
      return;
    }
    if (new Set(lines.map((line) => `${line.skuId}:${line.warehouseId}`)).size !== lines.length) {
      setNotice("同一商品和仓库只能出现一次。");
      return;
    }
    if (lines.some((line) => amount(line.discountAmount) > amount(line.quantity) * amount(line.unitPrice))) {
      setNotice("折扣不能超过该行数量与单价的乘积。");
      return;
    }
    setBusy(true);
    try {
      const payload = {
        ...(orderId ? { expectedVersion: version } : {}),
        legalEntityId,
        supplierId,
        businessUnitId,
        currency: "CNY",
        orderDate,
        expectedDeliveryDate: expectedDeliveryDate || undefined,
        paymentTermsDays: terms,
        supplierReference: supplierReference.trim() || undefined,
        businessNote: businessNote.trim() || undefined,
        lines: lines.map((line) => ({
          skuId: line.skuId,
          warehouseId: line.warehouseId,
          unitOfMeasureId: line.unitOfMeasureId,
          quantity: line.quantity,
          unitPrice: line.unitPrice,
          discountAmount: line.discountAmount,
          taxRate: decimalRate(line.taxPercent),
        })),
      };
      const result = await request<{ number: string; version: number }>(
        orderId ? `/api/v1/purchase-orders/${orderId}` : "/api/v1/purchase-orders",
        { method: orderId ? "PUT" : "POST", body: JSON.stringify(payload) },
      );
      setVersion(result.version);
      setNotice(`采购订单 ${result.number} 已保存为草稿。`);
      if (!orderId) {
        setSupplierReference("");
        setBusinessNote("");
        setLines([newLine(catalog.skus[0]?.id, availableWarehouses[0]?.id, catalog.units[0]?.id)]);
      }
      onDone();
    } catch (error) {
      setNotice((error as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const totals = lines.reduce(
    (sum, line) => {
      const subtotal = amount(line.quantity) * amount(line.unitPrice);
      const discount = Math.min(subtotal, amount(line.discountAmount));
      const net = Math.max(0, subtotal - discount);
      const tax = net * (amount(line.taxPercent) / 100);
      return { subtotal: sum.subtotal + subtotal, discount: sum.discount + discount, net: sum.net + net, tax: sum.tax + tax, gross: sum.gross + net + tax };
    },
    { subtotal: 0, discount: 0, net: 0, tax: 0, gross: 0 },
  );

  return (
    <section className="sales-entry purchase-entry" aria-labelledby="purchase-entry-title">
      <header>
        <div>
          <span>SUPPLY COMMITMENT</span>
          <h2 id="purchase-entry-title">{orderId ? "编辑采购订单草稿" : "录入采购订单"}</h2>
          <p>保存草稿不会增加库存；确认后才形成采购承诺。</p>
        </div>
        <strong>{options?.draft?.purchaseOrderNumber ?? "采购草稿"}</strong>
      </header>
      {loading ? (
        <p className="entry-loading">正在加载供应商、商品与收货仓库…</p>
      ) : (
        <form onSubmit={submit}>
          {!allowed && (
            <div className="shipment-gate">
              当前角色没有{orderId ? "编辑采购订单草稿" : "创建采购订单"}的权限，可继续查看订单记录。
            </div>
          )}
          <div className="entry-fields purchase-fields">
            <Field label="法律主体"><select value={legalEntityId} onChange={(event) => changeLegalEntity(event.target.value)} disabled={Boolean(orderId) || !allowed} required>{catalog.legalEntities.map(option)}</select></Field>
            <Field label="供应商"><select value={supplierId} onChange={(event) => setSupplierId(event.target.value)} disabled={!allowed} required>{availableSuppliers.map(option)}</select></Field>
            <Field label="业务单元"><select value={businessUnitId} onChange={(event) => setBusinessUnitId(event.target.value)} disabled={!allowed} required>{availableBusinessUnits.map(option)}</select></Field>
            <Field label="订单日期"><input type="date" value={orderDate} onChange={(event) => setOrderDate(event.target.value)} disabled={!allowed} required /></Field>
            <Field label="预计交付日"><input type="date" min={orderDate} value={expectedDeliveryDate} onChange={(event) => setExpectedDeliveryDate(event.target.value)} disabled={!allowed} /></Field>
            <Field label="账期天数"><input type="number" min="0" max="3650" step="1" value={paymentTermsDays} onChange={(event) => setPaymentTermsDays(event.target.value)} disabled={!allowed} required /></Field>
            <Field label="供应商参考号"><input value={supplierReference} maxLength={120} onChange={(event) => setSupplierReference(event.target.value)} disabled={!allowed} placeholder="报价单或合同参考号（可选）" /></Field>
          </div>

          <div className="supplier-strip">
            <span>供应方</span>
            <strong>{availableSuppliers.find((item) => item.id === supplierId)?.name ?? "请选择供应商"}</strong>
            <small>交付至所选仓库 · 币种 CNY</small>
          </div>

          <div className="entry-lines purchase-lines">
            <div className="entry-line-head"><span>商品</span><span>收货仓库</span><span>单位</span><span>数量</span><span>未税单价</span><span>折扣</span><span>税率 %</span><span /></div>
            {lines.map((line, index) => (
              <div className="entry-line" key={line.key}>
                <LineSelect label={`第 ${index + 1} 行商品`} value={line.skuId} disabled={!allowed} onChange={(value) => updateLine(line.key, "skuId", value)}>{catalog.skus.map(option)}</LineSelect>
                <LineSelect label={`第 ${index + 1} 行收货仓库`} value={line.warehouseId} disabled={!allowed} onChange={(value) => updateLine(line.key, "warehouseId", value)}>{availableWarehouses.map(option)}</LineSelect>
                <LineSelect label={`第 ${index + 1} 行单位`} value={line.unitOfMeasureId} disabled={!allowed} onChange={(value) => updateLine(line.key, "unitOfMeasureId", value)}>{catalog.units.map(option)}</LineSelect>
                {(["quantity", "unitPrice", "discountAmount", "taxPercent"] as const).map((field) => (
                  <label key={field}><span>{lineLabel(field)}</span><input aria-label={`第 ${index + 1} 行${lineLabel(field)}`} type="number" min="0" max={field === "taxPercent" ? "100" : undefined} step="0.000001" value={line[field]} disabled={!allowed} onChange={(event) => updateLine(line.key, field, event.target.value)} required /></label>
                ))}
                <button type="button" className="line-remove secondary" disabled={!allowed || lines.length === 1} onClick={() => setLines((current) => current.filter((item) => item.key !== line.key))} aria-label={`删除第 ${index + 1} 行`}>×</button>
              </div>
            ))}
          </div>

          <div className="entry-foot">
            <div>
              <button type="button" className="secondary" disabled={!allowed} onClick={() => setLines((current) => [...current, newLine(catalog.skus[0]?.id, availableWarehouses[0]?.id, catalog.units[0]?.id)])}>+ 添加采购行</button>
              <label className="entry-note"><span>采购备注</span><input value={businessNote} maxLength={1000} disabled={!allowed} onChange={(event) => setBusinessNote(event.target.value)} placeholder="包装、交期或验收要求（可选）" /></label>
            </div>
            <dl className="entry-total purchase-total">
              <div><dt>商品原额</dt><dd>¥ {formatAmount(totals.subtotal)}</dd></div>
              <div><dt>折扣</dt><dd>− ¥ {formatAmount(totals.discount)}</dd></div>
              <div><dt>采购净额</dt><dd>¥ {formatAmount(totals.net)}</dd></div>
              <div><dt>税额</dt><dd>¥ {formatAmount(totals.tax)}</dd></div>
              <div className="grand"><dt>含税合计</dt><dd>CNY {formatAmount(totals.gross)}</dd></div>
            </dl>
          </div>
          {notice && <p className="entry-notice">{notice}</p>}
          <button className="entry-save" type="submit" disabled={!allowed || busy}>{busy ? "正在保存…" : orderId ? "保存采购订单修改" : "保存采购订单草稿"}</button>
        </form>
      )}
    </section>
  );
}

function Field({ label, children }: React.PropsWithChildren<{ label: string }>) {
  const id = React.useId();
  const control = React.Children.only(children) as React.ReactElement<{ id?: string }>;
  return <div className="entry-field"><label htmlFor={id}>{label}</label>{React.cloneElement(control, { id })}</div>;
}

function LineSelect({ label, value, disabled, onChange, children }: React.PropsWithChildren<{ label: string; value: string; disabled: boolean; onChange: (value: string) => void }>) {
  return <label><span>{label}</span><select aria-label={label} value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)} required>{children}</select></label>;
}

function option(item: MasterDataRecord) {
  return <option value={item.id} key={item.id}>{item.code} · {item.name}</option>;
}

async function loadMaster(resource: string) {
  const response = await request<MasterDataList>(`/api/v1/master-data/${resource}?limit=200`);
  return response.items.filter((item) => item.status === "active");
}

function scoped(items: MasterDataRecord[], legalEntityId: string) {
  return items.filter((item) => !item.legalEntityId || item.legalEntityId === legalEntityId);
}

function newLine(skuId = "", warehouseId = "", unitOfMeasureId = ""): LineDraft {
  return { key: crypto.randomUUID(), skuId, warehouseId, unitOfMeasureId, quantity: "1", unitPrice: "0", discountAmount: "0", taxPercent: "0" };
}

function validLine(line: LineDraft) {
  return Boolean(line.skuId && line.warehouseId && line.unitOfMeasureId) && amount(line.quantity) > 0 && amount(line.unitPrice) >= 0 && amount(line.discountAmount) >= 0 && amount(line.taxPercent) >= 0 && amount(line.taxPercent) <= 100;
}

function lineLabel(field: "quantity" | "unitPrice" | "discountAmount" | "taxPercent") {
  return { quantity: "数量", unitPrice: "未税单价", discountAmount: "折扣", taxPercent: "税率 %" }[field];
}

function amount(value: string) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function decimalRate(percent: string) {
  return (amount(percent) / 100).toFixed(6).replace(/\.?0+$/, "");
}

function today() {
  const value = new Date();
  return `${value.getFullYear()}-${String(value.getMonth() + 1).padStart(2, "0")}-${String(value.getDate()).padStart(2, "0")}`;
}
