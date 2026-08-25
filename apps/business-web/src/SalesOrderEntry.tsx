import React from "react";
import { type MasterDataList, type MasterDataRecord, request } from "./api";
import { formatAmount } from "./formatters";

type LineDraft = {
  key: string;
  skuId: string;
  warehouseId: string;
  unitOfMeasureId: string;
  quantity: string;
  unitPrice: string;
  discountAmount: string;
  taxRate: string;
};

type Catalog = {
  legalEntities: MasterDataRecord[];
  customers: MasterDataRecord[];
  businessUnits: MasterDataRecord[];
  skus: MasterDataRecord[];
  warehouses: MasterDataRecord[];
  units: MasterDataRecord[];
};

const emptyCatalog: Catalog = {
  legalEntities: [],
  customers: [],
  businessUnits: [],
  skus: [],
  warehouses: [],
  units: [],
};

export function SalesOrderEntry({ onDone }: { onDone: () => void }) {
  const [catalog, setCatalog] = React.useState<Catalog>(emptyCatalog);
  const [loading, setLoading] = React.useState(true);
  const [legalEntityId, setLegalEntityId] = React.useState("");
  const [customerId, setCustomerId] = React.useState("");
  const [businessUnitId, setBusinessUnitId] = React.useState("");
  const [orderDate, setOrderDate] = React.useState(today());
  const [requestedDeliveryDate, setRequestedDeliveryDate] = React.useState("");
  const [customerReference, setCustomerReference] = React.useState("");
  const [businessNote, setBusinessNote] = React.useState("");
  const [lines, setLines] = React.useState<LineDraft[]>([newLine()]);
  const [busy, setBusy] = React.useState(false);
  const [notice, setNotice] = React.useState<string | null>(null);

  React.useEffect(() => {
    let active = true;
    Promise.all([
      loadMaster("legal_entity"),
      loadMaster("customer"),
      loadMaster("business_unit"),
      loadMaster("sku"),
      loadMaster("warehouse"),
      loadMaster("unit_of_measure"),
    ])
      .then(
        ([
          legalEntities,
          customers,
          businessUnits,
          skus,
          warehouses,
          units,
        ]) => {
          if (!active) return;
          const next = {
            legalEntities,
            customers,
            businessUnits,
            skus,
            warehouses,
            units,
          };
          setCatalog(next);
          setLegalEntityId(legalEntities[0]?.id ?? "");
          setCustomerId(customers[0]?.id ?? "");
          setBusinessUnitId(businessUnits[0]?.id ?? "");
          setLines([newLine(skus[0]?.id, warehouses[0]?.id, units[0]?.id)]);
        },
      )
      .catch((error: Error) => active && setNotice(error.message))
      .finally(() => active && setLoading(false));
    return () => {
      active = false;
    };
  }, []);

  const availableCustomers = catalog.customers.filter(
    (item) => !item.legalEntityId || item.legalEntityId === legalEntityId,
  );
  const availableUnits = catalog.businessUnits.filter(
    (item) => !item.legalEntityId || item.legalEntityId === legalEntityId,
  );
  const availableWarehouses = catalog.warehouses.filter(
    (item) => !item.legalEntityId || item.legalEntityId === legalEntityId,
  );

  function changeLegalEntity(value: string) {
    setLegalEntityId(value);
    const customers = catalog.customers.filter(
      (item) => !item.legalEntityId || item.legalEntityId === value,
    );
    const units = catalog.businessUnits.filter(
      (item) => !item.legalEntityId || item.legalEntityId === value,
    );
    const warehouses = catalog.warehouses.filter(
      (item) => !item.legalEntityId || item.legalEntityId === value,
    );
    setCustomerId(customers[0]?.id ?? "");
    setBusinessUnitId(units[0]?.id ?? "");
    setLines((current) =>
      current.map((line) => ({
        ...line,
        warehouseId: warehouses[0]?.id ?? "",
      })),
    );
  }

  function updateLine(key: string, field: keyof LineDraft, value: string) {
    setLines((current) =>
      current.map((line) =>
        line.key === key ? { ...line, [field]: value } : line,
      ),
    );
  }

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setNotice(null);
    if (!legalEntityId || !customerId || !businessUnitId) {
      setNotice("请选择法律主体、客户和业务单元。");
      return;
    }
    if (
      lines.some(
        (line) =>
          !line.skuId ||
          !line.warehouseId ||
          !line.unitOfMeasureId ||
          Number(line.quantity) <= 0 ||
          Number(line.unitPrice) < 0,
      )
    ) {
      setNotice("请补全商品行，并确保数量大于 0、单价不小于 0。");
      return;
    }
    setBusy(true);
    try {
      const output = await request<{ number: string }>("/api/v1/sales-orders", {
        method: "POST",
        body: JSON.stringify({
          legalEntityId,
          customerId,
          businessUnitId,
          currency: "CNY",
          orderDate,
          requestedDeliveryDate: requestedDeliveryDate || undefined,
          customerReference: customerReference.trim() || undefined,
          businessNote: businessNote.trim() || undefined,
          lines: lines.map(({ key: _key, ...line }) => line),
        }),
      });
      setNotice(`销售订单 ${output.number} 已保存为草稿。`);
      setCustomerReference("");
      setBusinessNote("");
      setLines([
        newLine(
          catalog.skus[0]?.id,
          availableWarehouses[0]?.id,
          catalog.units[0]?.id,
        ),
      ]);
      onDone();
    } catch (error) {
      setNotice((error as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const totals = lines.reduce(
    (sum, line) => {
      const base = amount(line.quantity) * amount(line.unitPrice);
      const discount = amount(line.discountAmount);
      const net = Math.max(0, base - discount);
      const tax = net * (amount(line.taxRate) / 100);
      return {
        subtotal: sum.subtotal + base,
        discount: sum.discount + discount,
        tax: sum.tax + tax,
        gross: sum.gross + net + tax,
      };
    },
    { subtotal: 0, discount: 0, tax: 0, gross: 0 },
  );

  return (
    <section className="sales-entry" aria-labelledby="sales-entry-title">
      <header>
        <div>
          <span>NEW SALES ORDER</span>
          <h2 id="sales-entry-title">录入销售订单</h2>
          <p>先保存草稿，再进入订单详情核对库存并执行确认。</p>
        </div>
        <strong>草稿</strong>
      </header>
      {loading ? (
        <p className="entry-loading">正在加载可用客户、商品与仓库…</p>
      ) : (
        <form onSubmit={submit}>
          <div className="entry-fields">
            <Field label="法律主体">
              <select
                value={legalEntityId}
                onChange={(event) => changeLegalEntity(event.target.value)}
                required
              >
                {catalog.legalEntities.map(option)}
              </select>
            </Field>
            <Field label="客户">
              <select
                value={customerId}
                onChange={(event) => setCustomerId(event.target.value)}
                required
              >
                {availableCustomers.map(option)}
              </select>
            </Field>
            <Field label="业务单元">
              <select
                value={businessUnitId}
                onChange={(event) => setBusinessUnitId(event.target.value)}
                required
              >
                {availableUnits.map(option)}
              </select>
            </Field>
            <Field label="订单日期">
              <input
                type="date"
                value={orderDate}
                onChange={(event) => setOrderDate(event.target.value)}
                required
              />
            </Field>
            <Field label="要求交付日">
              <input
                type="date"
                min={orderDate}
                value={requestedDeliveryDate}
                onChange={(event) =>
                  setRequestedDeliveryDate(event.target.value)
                }
              />
            </Field>
            <Field label="客户参考号">
              <input
                value={customerReference}
                maxLength={120}
                onChange={(event) => setCustomerReference(event.target.value)}
                placeholder="可选"
              />
            </Field>
          </div>

          <div className="entry-lines">
            <div className="entry-line-head">
              <span>商品</span>
              <span>仓库</span>
              <span>单位</span>
              <span>数量</span>
              <span>单价</span>
              <span>折扣</span>
              <span>税率 %</span>
              <span />
            </div>
            {lines.map((line, index) => (
              <div className="entry-line" key={line.key}>
                <label>
                  <span>商品 {index + 1}</span>
                  <select
                    aria-label={`第 ${index + 1} 行商品`}
                    value={line.skuId}
                    onChange={(event) =>
                      updateLine(line.key, "skuId", event.target.value)
                    }
                    required
                  >
                    {catalog.skus.map(option)}
                  </select>
                </label>
                <label>
                  <span>仓库</span>
                  <select
                    aria-label={`第 ${index + 1} 行仓库`}
                    value={line.warehouseId}
                    onChange={(event) =>
                      updateLine(line.key, "warehouseId", event.target.value)
                    }
                    required
                  >
                    {availableWarehouses.map(option)}
                  </select>
                </label>
                <label>
                  <span>单位</span>
                  <select
                    aria-label={`第 ${index + 1} 行单位`}
                    value={line.unitOfMeasureId}
                    onChange={(event) =>
                      updateLine(
                        line.key,
                        "unitOfMeasureId",
                        event.target.value,
                      )
                    }
                    required
                  >
                    {catalog.units.map(option)}
                  </select>
                </label>
                {(
                  [
                    "quantity",
                    "unitPrice",
                    "discountAmount",
                    "taxRate",
                  ] as const
                ).map((field) => (
                  <label key={field}>
                    <span>{lineLabel(field)}</span>
                    <input
                      aria-label={`第 ${index + 1} 行${lineLabel(field)}`}
                      type="number"
                      min="0"
                      step="0.000001"
                      value={line[field]}
                      onChange={(event) =>
                        updateLine(line.key, field, event.target.value)
                      }
                      required
                    />
                  </label>
                ))}
                <button
                  type="button"
                  className="line-remove secondary"
                  onClick={() =>
                    setLines((current) =>
                      current.length === 1
                        ? current
                        : current.filter((item) => item.key !== line.key),
                    )
                  }
                  disabled={lines.length === 1}
                  aria-label={`删除第 ${index + 1} 行`}
                >
                  ×
                </button>
              </div>
            ))}
          </div>

          <div className="entry-foot">
            <div>
              <button
                type="button"
                className="secondary"
                onClick={() =>
                  setLines((current) => [
                    ...current,
                    newLine(
                      catalog.skus[0]?.id,
                      availableWarehouses[0]?.id,
                      catalog.units[0]?.id,
                    ),
                  ])
                }
              >
                + 添加商品行
              </button>
              <label className="entry-note">
                <span>业务备注</span>
                <input
                  value={businessNote}
                  maxLength={500}
                  onChange={(event) => setBusinessNote(event.target.value)}
                  placeholder="可选"
                />
              </label>
            </div>
            <dl className="entry-total">
              <div>
                <dt>价税前</dt>
                <dd>¥ {formatAmount(totals.subtotal)}</dd>
              </div>
              <div>
                <dt>折扣</dt>
                <dd>− ¥ {formatAmount(totals.discount)}</dd>
              </div>
              <div>
                <dt>税额</dt>
                <dd>¥ {formatAmount(totals.tax)}</dd>
              </div>
              <div className="grand">
                <dt>订单合计</dt>
                <dd>CNY {formatAmount(totals.gross)}</dd>
              </div>
            </dl>
          </div>
          {notice && <p className="entry-notice">{notice}</p>}
          <button className="entry-save" type="submit" disabled={busy}>
            {busy ? "正在保存…" : "保存销售订单草稿"}
          </button>
        </form>
      )}
    </section>
  );
}

function Field({
  label,
  children,
}: React.PropsWithChildren<{ label: string }>) {
  const id = React.useId();
  const control = React.Children.only(children) as React.ReactElement<{
    id?: string;
  }>;
  return (
    <div className="entry-field">
      <label htmlFor={id}>{label}</label>
      {React.cloneElement(control, { id })}
    </div>
  );
}

function option(item: MasterDataRecord) {
  return (
    <option value={item.id} key={item.id}>
      {item.code} · {item.name}
    </option>
  );
}

async function loadMaster(resource: string) {
  const response = await request<MasterDataList>(
    `/api/v1/master-data/${resource}?limit=200`,
  );
  return response.items.filter((item) => item.status === "active");
}

function newLine(
  skuId = "",
  warehouseId = "",
  unitOfMeasureId = "",
): LineDraft {
  return {
    key: crypto.randomUUID(),
    skuId,
    warehouseId,
    unitOfMeasureId,
    quantity: "1",
    unitPrice: "0",
    discountAmount: "0",
    taxRate: "0",
  };
}

function lineLabel(
  field: "quantity" | "unitPrice" | "discountAmount" | "taxRate",
) {
  return {
    quantity: "数量",
    unitPrice: "单价",
    discountAmount: "折扣",
    taxRate: "税率 %",
  }[field];
}

function amount(value: string) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function today() {
  const value = new Date();
  return `${value.getFullYear()}-${String(value.getMonth() + 1).padStart(2, "0")}-${String(value.getDate()).padStart(2, "0")}`;
}
