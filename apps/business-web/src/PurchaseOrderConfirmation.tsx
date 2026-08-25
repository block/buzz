import React from "react";
import { type PurchaseOrderConfirmationPreview, request } from "./api";
import { formatMoney, formatQuantity } from "./formatters";

const READINESS = {
  ready: ["采购承诺完整，可以确认", "供应商、交付仓库、商品行与金额已完成检查。"],
  permission_required: ["当前角色不能确认采购订单", "检查证据仍可查看，请由具有采购订单确认权限的人员处理。"],
  order_not_draft: ["采购订单已离开草稿状态", "只有草稿订单需要执行采购承诺确认前检查。"],
  supplier_inactive: ["供应商当前不可用于采购", "恢复供应商主数据或调整采购草稿后重新检查。"],
  line_incomplete: ["采购商品行尚未通过检查", "检查商品、计量单位与交付仓库是否仍然有效。"],
} as const;

export function PurchaseOrderConfirmation({ orderId, onDone }: { orderId: string; onDone: () => void }) {
  const [preview, setPreview] = React.useState<PurchaseOrderConfirmationPreview>();
  const [loading, setLoading] = React.useState(true);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState("");

  React.useEffect(() => {
    let active = true;
    request<PurchaseOrderConfirmationPreview>(`/api/v1/purchase-orders/${orderId}/confirmation-preview`)
      .then((value) => active && setPreview(value))
      .catch((reason: Error) => active && setError(reason.message))
      .finally(() => active && setLoading(false));
    return () => { active = false; };
  }, [orderId]);

  if (loading) return <div className="confirmation-loading">正在核对采购承诺与交付路径…</div>;
  if (error || !preview) return <div className="error"><strong>无法完成采购确认前检查</strong><span>{error || "采购承诺证据不可用"}</span></div>;

  const [title, note] = READINESS[preview.readiness];
  return (
    <section className={`purchase-review ${preview.canConfirm ? "ready" : "blocked"}`}>
      <header className="purchase-review-head">
        <div><p>采购承诺封面 / Commitment docket</p><h2>{title}</h2><span>{note}</span></div>
        <div className="purchase-review-stamp"><small>{preview.orderNumber}</small><strong>{preview.canConfirm ? "COMMIT" : "HOLD"}</strong></div>
      </header>

      <dl className="purchase-review-context">
        <div><dt>供应商</dt><dd>{preview.supplierCode} · {preview.supplierName}</dd></div>
        <div><dt>订单日期</dt><dd>{preview.orderDate}</dd></div>
        <div><dt>预计交付</dt><dd>{preview.expectedDeliveryDate ?? "未约定"}</dd></div>
        <div><dt>交付路径</dt><dd>{preview.warehouseCount} 个仓库 · {preview.paymentTermsDays} 天账期</dd></div>
      </dl>

      <div className="purchase-review-money" aria-label="采购订单金额">
        <Metric label="价税前" value={formatMoney(preview.currency, preview.subtotalAmount)} />
        <Metric label="折扣" value={`− ${formatMoney(preview.currency, preview.discountAmount)}`} />
        <Metric label="采购净额" value={formatMoney(preview.currency, preview.netAmount)} />
        <Metric label="税额" value={formatMoney(preview.currency, preview.taxAmount)} />
        <Metric label="价税合计" value={formatMoney(preview.currency, preview.grossAmount)} />
      </div>

      <div className="purchase-review-lines">
        <div className="purchase-review-line line-head" aria-hidden="true"><span>商品 / 交付仓库</span><span>数量</span><span>单价</span><span>折扣</span><span>净额</span><span>税额</span><span>价税合计</span><span>检查</span></div>
        {preview.lines.map((line) => (
          <article className={`purchase-review-line ${line.ready ? "" : "blocked-line"}`} key={line.lineNumber}>
            <div><strong>{line.skuCode} · {line.skuName}</strong><span>{line.warehouseCode} · {line.warehouseName}</span><small>{line.unitCode} · {line.unitName}</small></div>
            <strong>{formatQuantity(line.orderedQuantity)}</strong><span>{formatMoney(preview.currency, line.unitPrice)}</span><span>{formatMoney(preview.currency, line.discountAmount)}</span><strong>{formatMoney(preview.currency, line.netAmount)}</strong><span>{formatMoney(preview.currency, line.taxAmount)}<small>{percent(line.taxRate)}</small></span><strong>{formatMoney(preview.currency, line.grossAmount)}</strong><em>{line.ready ? "完整" : "主数据失效"}</em>
          </article>
        ))}
      </div>

      <footer className="confirmation-foot">
        <p>检查时点 {formatTime(preview.checkedAt)} · 确认仅形成采购承诺，不增加库存；确认事务会再次验证主数据与商品行。</p>
        <button type="button" disabled={!preview.canConfirm || busy} onClick={async () => {
          setBusy(true); setError("");
          try {
            await request(`/api/v1/purchase-orders/${orderId}/confirm`, { method: "POST", body: JSON.stringify({ expectedVersion: preview.version }) });
            onDone();
          } catch (reason) { setError((reason as Error).message); } finally { setBusy(false); }
        }}>{busy ? "正在确认…" : "确认采购承诺"}</button>
      </footer>
      {error && <div className="confirmation-error">{error}</div>}
    </section>
  );
}

function Metric({ label, value }: { label: string; value: string }) { return <div><dt>{label}</dt><dd>{value}</dd></div>; }
function percent(value: string) { return `${Number(value) * 100}%`; }
function formatTime(value: string) { return new Intl.DateTimeFormat("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(new Date(value)); }
