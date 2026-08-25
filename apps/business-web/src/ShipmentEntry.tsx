import React from "react";
import {
  type ShipmentDraftOptionLine,
  type ShipmentDraftOptions,
  request,
} from "./api";
import { formatQuantity } from "./formatters";

type ShipmentGroup = {
  key: string;
  orderId: string;
  orderNumber: string;
  customerCode: string;
  customerName: string;
  currency: string;
  warehouseId: string;
  warehouseCode: string;
  warehouseName: string;
  lines: ShipmentDraftOptionLine[];
};

export function ShipmentEntry({ onDone }: { onDone: () => void }) {
  const [options, setOptions] = React.useState<ShipmentDraftOptions>();
  const [selectedKey, setSelectedKey] = React.useState("");
  const [shipmentDate, setShipmentDate] = React.useState(today());
  const [quantities, setQuantities] = React.useState<Record<string, string>>(
    {},
  );
  const [loading, setLoading] = React.useState(true);
  const [busy, setBusy] = React.useState(false);
  const [notice, setNotice] = React.useState("");

  React.useEffect(() => {
    let active = true;
    request<ShipmentDraftOptions>("/api/v1/shipments/draft-options?limit=500")
      .then((value) => {
        if (!active) return;
        setOptions(value);
        const first = groupOptions(value.items)[0];
        if (first) {
          setSelectedKey(first.key);
          setQuantities(defaultQuantities(first.lines));
        }
      })
      .catch((error: Error) => active && setNotice(error.message))
      .finally(() => active && setLoading(false));
    return () => {
      active = false;
    };
  }, []);

  const groups = groupOptions(options?.items ?? []);
  const selected = groups.find((group) => group.key === selectedKey);
  const totalQuantity = selected?.lines.reduce(
    (sum, line) => sum + quantity(quantities[line.salesOrderLineId]),
    0,
  );

  function changeSelection(key: string) {
    setSelectedKey(key);
    const group = groups.find((item) => item.key === key);
    setQuantities(defaultQuantities(group?.lines ?? []));
    setNotice("");
  }

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setNotice("");
    if (!selected || !options?.canCreate) return;
    const lines = selected.lines
      .map((line) => ({
        salesOrderLineId: line.salesOrderLineId,
        quantity: quantities[line.salesOrderLineId] ?? "0",
        maximum: Number(line.shippableQuantity),
      }))
      .filter((line) => quantity(line.quantity) > 0);
    if (lines.length === 0) {
      setNotice("至少填写一项本次发货数量。");
      return;
    }
    if (
      lines.some(
        (line) =>
          !Number.isFinite(quantity(line.quantity)) ||
          quantity(line.quantity) > line.maximum,
      )
    ) {
      setNotice("本次发货数量不能超过可发数量。");
      return;
    }
    setBusy(true);
    try {
      const result = await request<{ number: string }>("/api/v1/shipments", {
        method: "POST",
        body: JSON.stringify({
          salesOrderId: selected.orderId,
          warehouseId: selected.warehouseId,
          shipmentDate,
          lines: lines.map(({ salesOrderLineId, quantity: lineQuantity }) => ({
            salesOrderLineId,
            quantity: lineQuantity,
          })),
        }),
      });
      setNotice(`出库单 ${result.number} 已保存为草稿。`);
      setOptions((current) =>
        current
          ? {
              ...current,
              items: current.items
                .map((item) => {
                  const shipped = lines.find(
                    (line) => line.salesOrderLineId === item.salesOrderLineId,
                  );
                  if (!shipped) return item;
                  const allocated = quantity(shipped.quantity);
                  return {
                    ...item,
                    draftAllocatedQuantity: String(
                      quantity(item.draftAllocatedQuantity) + allocated,
                    ),
                    shippableQuantity: String(
                      Math.max(0, quantity(item.shippableQuantity) - allocated),
                    ),
                  };
                })
                .filter((item) => quantity(item.shippableQuantity) > 0),
            }
          : current,
      );
      setQuantities({});
      onDone();
    } catch (error) {
      setNotice((error as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section
      className="sales-entry shipment-entry"
      aria-labelledby="shipment-entry-title"
    >
      <header>
        <div>
          <span>PICK &amp; SHIP</span>
          <h2 id="shipment-entry-title">创建销售出库单</h2>
          <p>按仓库读取已预占订单；保存草稿不会扣减库存。</p>
        </div>
        <strong>拣货单</strong>
      </header>
      {loading ? (
        <p className="entry-loading">正在读取已确认订单与剩余预占…</p>
      ) : (
        <form onSubmit={submit}>
          <div className="shipment-fields">
            <label>
              <span>订单 / 仓库</span>
              <select
                value={selectedKey}
                onChange={(event) => changeSelection(event.target.value)}
                disabled={groups.length === 0}
              >
                {groups.map((group) => (
                  <option value={group.key} key={group.key}>
                    {group.orderNumber} · {group.customerName} ·{" "}
                    {group.warehouseCode}
                  </option>
                ))}
              </select>
            </label>
            <label>
              <span>出库日期</span>
              <input
                type="date"
                value={shipmentDate}
                onChange={(event) => setShipmentDate(event.target.value)}
                required
              />
            </label>
          </div>

          {!options?.canCreate && (
            <div className="shipment-gate">
              当前角色没有创建销售出库单的权限，可查看已有出库记录。
            </div>
          )}
          {options?.canCreate && groups.length === 0 && (
            <div className="shipment-gate">
              当前没有可发货订单。订单需已确认、未被
              Hold，并且仍有未占用的预留数量。
            </div>
          )}

          {selected && (
            <>
              <div className="pick-ticket">
                <div className="pick-ticket-meta">
                  <div>
                    <span>客户</span>
                    <strong>{selected.customerName}</strong>
                    <small>{selected.customerCode}</small>
                  </div>
                  <div>
                    <span>发货仓库</span>
                    <strong>{selected.warehouseName}</strong>
                    <small>{selected.warehouseCode}</small>
                  </div>
                  <div>
                    <span>销售订单</span>
                    <strong>{selected.orderNumber}</strong>
                    <small>{selected.currency}</small>
                  </div>
                </div>
                <div className="pick-line pick-line-head" aria-hidden="true">
                  <span>行 / 商品</span>
                  <span>订购</span>
                  <span>已发</span>
                  <span>预留余额</span>
                  <span>草稿占用</span>
                  <span>本次可发</span>
                  <span>本次发货</span>
                </div>
                {selected.lines.map((line) => (
                  <div className="pick-line" key={line.salesOrderLineId}>
                    <div>
                      <b>{String(line.lineNumber).padStart(2, "0")}</b>
                      <strong>{line.skuCode}</strong>
                      <small>{line.skuName}</small>
                    </div>
                    <span>{formatQuantity(line.orderedQuantity)}</span>
                    <span>{formatQuantity(line.shippedQuantity)}</span>
                    <span>{formatQuantity(line.reservationOpenQuantity)}</span>
                    <span>{formatQuantity(line.draftAllocatedQuantity)}</span>
                    <strong>{formatQuantity(line.shippableQuantity)}</strong>
                    <label>
                      <span>本次发货数量</span>
                      <input
                        aria-label={`${line.skuCode} 本次发货数量`}
                        type="number"
                        min="0"
                        max={line.shippableQuantity}
                        step="0.000001"
                        value={quantities[line.salesOrderLineId] ?? "0"}
                        onChange={(event) =>
                          setQuantities((current) => ({
                            ...current,
                            [line.salesOrderLineId]: event.target.value,
                          }))
                        }
                      />
                    </label>
                  </div>
                ))}
              </div>
              <div className="pick-total">
                <span>本次发货合计</span>
                <strong>{formatQuantity(totalQuantity ?? 0)}</strong>
              </div>
            </>
          )}

          {notice && <p className="entry-notice">{notice}</p>}
          <button
            className="entry-save"
            type="submit"
            disabled={!options?.canCreate || !selected || busy}
          >
            {busy ? "正在保存…" : "保存销售出库草稿"}
          </button>
        </form>
      )}
    </section>
  );
}

function groupOptions(lines: ShipmentDraftOptionLine[]) {
  const groups = new Map<string, ShipmentGroup>();
  for (const line of lines) {
    const key = `${line.orderId}:${line.warehouseId}`;
    const group = groups.get(key) ?? {
      key,
      orderId: line.orderId,
      orderNumber: line.orderNumber,
      customerCode: line.customerCode,
      customerName: line.customerName,
      currency: line.currency,
      warehouseId: line.warehouseId,
      warehouseCode: line.warehouseCode,
      warehouseName: line.warehouseName,
      lines: [],
    };
    group.lines.push(line);
    groups.set(key, group);
  }
  return [...groups.values()];
}

function defaultQuantities(lines: ShipmentDraftOptionLine[]) {
  return Object.fromEntries(
    lines.map((line) => [line.salesOrderLineId, line.shippableQuantity]),
  );
}

function quantity(value = "0") {
  return Number(value || 0);
}

function today() {
  return new Date().toISOString().slice(0, 10);
}
