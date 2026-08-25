import React from "react";
import {
  type GoodsReceiptDraftOptionLine,
  type GoodsReceiptDraftOptions,
  request,
} from "./api";
import { formatQuantity } from "./formatters";

type ReceiptGroup = {
  key: string;
  orderId: string;
  orderNumber: string;
  supplierCode: string;
  supplierName: string;
  currency: string;
  warehouseId: string;
  warehouseCode: string;
  warehouseName: string;
  lines: GoodsReceiptDraftOptionLine[];
};

export function GoodsReceiptEntry({ onDone }: { onDone: () => void }) {
  const [options, setOptions] = React.useState<GoodsReceiptDraftOptions>();
  const [selectedKey, setSelectedKey] = React.useState("");
  const [receiptDate, setReceiptDate] = React.useState(today());
  const [quantities, setQuantities] = React.useState<Record<string, string>>({});
  const [loading, setLoading] = React.useState(true);
  const [busy, setBusy] = React.useState(false);
  const [notice, setNotice] = React.useState("");

  React.useEffect(() => {
    let active = true;
    request<GoodsReceiptDraftOptions>("/api/v1/goods-receipts/draft-options?limit=500")
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
    (sum, line) => sum + quantity(quantities[line.purchaseOrderLineId]),
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
        purchaseOrderLineId: line.purchaseOrderLineId,
        quantity: quantities[line.purchaseOrderLineId] ?? "0",
        maximum: quantity(line.receivableQuantity),
      }))
      .filter((line) => quantity(line.quantity) > 0);
    if (lines.length === 0) {
      setNotice("至少填写一项本次到货数量。");
      return;
    }
    if (lines.some((line) => !Number.isFinite(quantity(line.quantity)) || quantity(line.quantity) > line.maximum)) {
      setNotice("本次到货数量不能超过当前可收数量。");
      return;
    }
    setBusy(true);
    try {
      const result = await request<{ number: string }>("/api/v1/goods-receipts", {
        method: "POST",
        body: JSON.stringify({
          purchaseOrderId: selected.orderId,
          warehouseId: selected.warehouseId,
          receiptDate,
          lines: lines.map(({ purchaseOrderLineId, quantity: received }) => ({
            purchaseOrderLineId,
            quantity: received,
          })),
        }),
      });
      setNotice(`收货单 ${result.number} 已保存为草稿。`);
      setOptions((current) =>
        current
          ? {
              ...current,
              items: current.items
                .map((item) => {
                  const received = lines.find((line) => line.purchaseOrderLineId === item.purchaseOrderLineId);
                  if (!received) return item;
                  const allocated = quantity(received.quantity);
                  return {
                    ...item,
                    draftAllocatedQuantity: String(quantity(item.draftAllocatedQuantity) + allocated),
                    receivableQuantity: String(Math.max(0, quantity(item.receivableQuantity) - allocated)),
                  };
                })
                .filter((item) => quantity(item.receivableQuantity) > 0),
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
    <section className="sales-entry receipt-entry" aria-labelledby="receipt-entry-title">
      <header>
        <div>
          <span>RECEIVING INSPECTION</span>
          <h2 id="receipt-entry-title">创建采购收货单</h2>
          <p>按采购承诺登记实际到货；保存草稿不会增加库存。</p>
        </div>
        <strong>待验收入库</strong>
      </header>
      {loading ? (
        <p className="entry-loading">正在读取已确认采购订单与剩余到货量…</p>
      ) : (
        <form onSubmit={submit}>
          <div className="shipment-fields">
            <label>
              <span>采购订单 / 收货仓库</span>
              <select value={selectedKey} onChange={(event) => changeSelection(event.target.value)} disabled={groups.length === 0}>
                {groups.map((group) => (
                  <option value={group.key} key={group.key}>{group.orderNumber} · {group.supplierName} · {group.warehouseCode}</option>
                ))}
              </select>
            </label>
            <label>
              <span>实际收货日期</span>
              <input type="date" value={receiptDate} onChange={(event) => setReceiptDate(event.target.value)} required />
            </label>
          </div>

          {options && !options.canCreate && <div className="shipment-gate">当前角色没有创建采购收货单的权限，可查看已有收货记录。</div>}
          {options?.canCreate && groups.length === 0 && <div className="shipment-gate">当前没有可收货采购订单。采购订单需已确认，并且仍有未被其他草稿占用的剩余数量。</div>}

          {selected && (
            <>
              <div className="receiving-ticket">
                <div className="pick-ticket-meta receipt-ticket-meta">
                  <div><span>供应商</span><strong>{selected.supplierName}</strong><small>{selected.supplierCode}</small></div>
                  <div><span>收货仓库</span><strong>{selected.warehouseName}</strong><small>{selected.warehouseCode}</small></div>
                  <div><span>采购订单</span><strong>{selected.orderNumber}</strong><small>{selected.currency}</small></div>
                </div>
                <div className="receipt-line receipt-line-head" aria-hidden="true">
                  <span>行 / 商品</span><span>订购</span><span>已收</span><span>已取消</span><span>草稿占用</span><span>本次可收</span><span>本次到货</span>
                </div>
                {selected.lines.map((line) => (
                  <div className="receipt-line" key={line.purchaseOrderLineId}>
                    <div><b>{String(line.lineNumber).padStart(2, "0")}</b><strong>{line.skuCode}</strong><small>{line.skuName} · {line.unitCode}</small></div>
                    <span>{formatQuantity(line.orderedQuantity)}</span>
                    <span>{formatQuantity(line.receivedQuantity)}</span>
                    <span>{formatQuantity(line.cancelledQuantity)}</span>
                    <span>{formatQuantity(line.draftAllocatedQuantity)}</span>
                    <strong>{formatQuantity(line.receivableQuantity)}</strong>
                    <label><span>本次到货数量</span><input aria-label={`${line.skuCode} 本次到货数量`} type="number" min="0" max={line.receivableQuantity} step="0.000001" value={quantities[line.purchaseOrderLineId] ?? "0"} onChange={(event) => setQuantities((current) => ({ ...current, [line.purchaseOrderLineId]: event.target.value }))} /></label>
                  </div>
                ))}
              </div>
              <div className="receipt-total"><span>本次到货合计</span><strong>{formatQuantity(totalQuantity ?? 0)}</strong><small>确认前将计算暂估成本与移动平均影响</small></div>
            </>
          )}

          {notice && <p className="entry-notice">{notice}</p>}
          <button className="entry-save" type="submit" disabled={!options?.canCreate || !selected || busy}>{busy ? "正在保存…" : "保存采购收货草稿"}</button>
        </form>
      )}
    </section>
  );
}

function groupOptions(lines: GoodsReceiptDraftOptionLine[]) {
  const groups = new Map<string, ReceiptGroup>();
  for (const line of lines) {
    const key = `${line.orderId}:${line.warehouseId}`;
    const group = groups.get(key) ?? {
      key,
      orderId: line.orderId,
      orderNumber: line.orderNumber,
      supplierCode: line.supplierCode,
      supplierName: line.supplierName,
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

function defaultQuantities(lines: GoodsReceiptDraftOptionLine[]) {
  return Object.fromEntries(lines.map((line) => [line.purchaseOrderLineId, line.receivableQuantity]));
}

function quantity(value = "0") {
  return Number(value || 0);
}

function today() {
  return new Date().toISOString().slice(0, 10);
}
