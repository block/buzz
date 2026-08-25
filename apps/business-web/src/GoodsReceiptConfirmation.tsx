import React from "react";
import { type GoodsReceiptConfirmationPreview, request } from "./api";
import { formatMoney, formatQuantity } from "./formatters";

const READINESS = {
  ready: ["入库条件完整，可以确认", "确认后将更新库存数量、价值和移动平均成本。"],
  permission_required: ["当前角色不能确认入库", "检查结果仍可查看，请由具有采购入库确认权限的人员处理。"],
  receipt_not_draft: ["收货单已离开草稿状态", "只有草稿收货单需要执行确认前检查。"],
  order_not_open: ["采购订单当前不可收货", "采购订单必须保持已确认且未完成状态。"],
  over_receipt: ["到货数量超过采购剩余", "刷新采购订单余量或调整收货草稿后重新检查。"],
} as const;

export function GoodsReceiptConfirmation({
  receiptId,
  onDone,
}: {
  receiptId: string;
  onDone: () => void;
}) {
  const [preview, setPreview] = React.useState<GoodsReceiptConfirmationPreview>();
  const [loading, setLoading] = React.useState(true);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState("");

  React.useEffect(() => {
    let active = true;
    setLoading(true);
    setError("");
    request<GoodsReceiptConfirmationPreview>(`/api/v1/goods-receipts/${receiptId}/confirmation-preview`)
      .then((value) => active && setPreview(value))
      .catch((reason: Error) => active && setError(reason.message))
      .finally(() => active && setLoading(false));
    return () => {
      active = false;
    };
  }, [receiptId]);

  if (loading) return <div className="confirmation-loading">正在计算入库成本影响…</div>;
  if (error || !preview) {
    return <div className="error"><strong>无法完成入库确认前检查</strong><span>{error || "收货与库存证据不可用"}</span></div>;
  }

  const [title, note] = READINESS[preview.readiness];
  return (
    <section className={`receipt-inspection ${preview.canConfirm ? "ready" : "blocked"}`}>
      <header className="release-head">
        <div>
          <p>入库检验放行 / Receiving release</p>
          <h2>{title}</h2>
          <span>{note}</span>
        </div>
        <div className="release-stamp receipt-stamp">
          <small>{preview.receiptNumber}</small>
          <strong>{preview.canConfirm ? "ACCEPT" : "HOLD"}</strong>
        </div>
      </header>

      <dl className="release-context">
        <div><dt>采购订单</dt><dd>{preview.orderNumber}</dd></div>
        <div><dt>供应商</dt><dd>{preview.supplierCode} · {preview.supplierName}</dd></div>
        <div><dt>收货仓库</dt><dd>{preview.warehouseCode} · {preview.warehouseName}</dd></div>
        <div><dt>实际收货日</dt><dd>{preview.receiptDate}</dd></div>
      </dl>

      <div className="release-money receipt-impact" aria-label="入库经营影响">
        <Metric label="预计暂估库存成本" value={formatMoney(preview.currency, preview.expectedInventoryCost)} />
        <Metric label="预计税额" value={formatMoney(preview.currency, preview.expectedTaxAmount)} />
        <Metric label="预计经营应付" value={formatMoney(preview.currency, preview.expectedPayableAmount)} />
        <Metric label="预计到期日" value={preview.expectedDueDate} />
      </div>

      <div className="cost-track-list">
        <div className="cost-track cost-track-head" aria-hidden="true">
          <span>商品 / 到货</span><span>当前库存</span><span>当前均价</span><i>→</i><span>入库后库存</span><span>入库后均价</span><span>暂估成本</span><span>检查</span>
        </div>
        {preview.lines.map((line) => (
          <article className={`cost-track ${line.ready ? "" : "blocked-line"}`} key={line.purchaseOrderLineId}>
            <div><strong>{line.skuCode}</strong><span>{line.skuName}</span><small>到货 {formatQuantity(line.receivedQuantity)} / 剩余 {formatQuantity(line.orderRemainingQuantity)}</small></div>
            <div><strong>{formatQuantity(line.currentOnHandQuantity)}</strong><small>{formatMoney(preview.currency, line.currentInventoryValue)}</small></div>
            <strong>{line.currentAverageUnitCost === null ? "尚无成本" : formatMoney(preview.currency, line.currentAverageUnitCost)}</strong>
            <i>→</i>
            <div><strong>{formatQuantity(line.projectedOnHandQuantity)}</strong><small>{formatMoney(preview.currency, line.projectedInventoryValue)}</small></div>
            <strong>{formatMoney(preview.currency, line.projectedAverageUnitCost)}</strong>
            <div><strong>{formatMoney(preview.currency, line.provisionalInventoryCost)}</strong><small>单价 {formatMoney(preview.currency, line.provisionalUnitCost)}</small></div>
            <em>{line.ready ? "通过" : line.readiness === "over_receipt" ? "超收" : "订单关闭"}</em>
          </article>
        ))}
      </div>

      <footer className="confirmation-foot">
        <p>库存时点 {formatTime(preview.inventoryAsOf)} · 确认事务会重新锁定采购行与库存余额，预览不会替代最终校验。</p>
        <button
          type="button"
          disabled={!preview.canConfirm || busy}
          onClick={async () => {
            setBusy(true);
            setError("");
            try {
              await request(`/api/v1/goods-receipts/${receiptId}/confirm`, {
                method: "POST",
                body: JSON.stringify({ expectedVersion: preview.version }),
              });
              onDone();
            } catch (reason) {
              setError((reason as Error).message);
            } finally {
              setBusy(false);
            }
          }}
        >
          {busy ? "正在确认…" : "确认入库并更新移动平均"}
        </button>
      </footer>
      {error && <div className="confirmation-error">{error}</div>}
    </section>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return <div><dt>{label}</dt><dd>{value}</dd></div>;
}

function formatTime(value: string) {
  return new Intl.DateTimeFormat("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(new Date(value));
}
