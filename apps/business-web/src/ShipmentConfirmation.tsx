import React from "react";
import { type ShipmentConfirmationPreview, request } from "./api";
import { formatMoney, formatQuantity } from "./formatters";

const READINESS = {
  ready: ["出库条件完整，可以确认", "确认后将冻结移动平均成本，并生成经营性应收。"],
  permission_required: ["当前角色不能确认出库", "检查结果仍可查看，请由具有出库确认权限的人员放行。"],
  shipment_not_draft: ["出库单已离开草稿状态", "只有草稿出库单需要执行确认前检查。"],
  order_on_hold: ["销售订单处于人工复核冻结", "解除订单冻结后重新执行确认前检查。"],
  order_not_fulfillable: ["销售订单当前不可履约", "订单必须处于已确认状态才可出库。"],
  missing_inventory_cost: ["移动平均成本缺失", "请先补齐该库存地点的成本事实。"],
  insufficient_inventory: ["库存或预占余额不足", "补充库存、恢复预占或调整出库数量后重试。"],
} as const;

export function ShipmentConfirmation({
  shipmentId,
  onDone,
}: {
  shipmentId: string;
  onDone: () => void;
}) {
  const [preview, setPreview] = React.useState<ShipmentConfirmationPreview>();
  const [loading, setLoading] = React.useState(true);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState("");

  React.useEffect(() => {
    let active = true;
    setLoading(true);
    setError("");
    request<ShipmentConfirmationPreview>(
      `/api/v1/shipments/${shipmentId}/confirmation-preview`,
    )
      .then((value) => active && setPreview(value))
      .catch((reason: Error) => active && setError(reason.message))
      .finally(() => active && setLoading(false));
    return () => {
      active = false;
    };
  }, [shipmentId]);

  if (loading) return <div className="confirmation-loading">正在核对出库事实…</div>;
  if (error || !preview) {
    return (
      <div className="error">
        <strong>无法完成出库确认前检查</strong>
        <span>{error || "出库放行证据不可用"}</span>
      </div>
    );
  }

  const [title, note] = READINESS[preview.readiness];
  return (
    <section className={`shipment-release ${preview.canConfirm ? "ready" : "blocked"}`}>
      <header className="release-head">
        <div>
          <p>出库放行单 / Dispatch release</p>
          <h2>{title}</h2>
          <span>{note}</span>
        </div>
        <div className="release-stamp">
          <small>{preview.shipmentNumber}</small>
          <strong>{preview.canConfirm ? "RELEASE" : "HOLD"}</strong>
        </div>
      </header>

      <dl className="release-context">
        <div><dt>销售订单</dt><dd>{preview.orderNumber}</dd></div>
        <div><dt>客户</dt><dd>{preview.customerCode} · {preview.customerName}</dd></div>
        <div><dt>出库仓</dt><dd>{preview.warehouseCode} · {preview.warehouseName}</dd></div>
        <div><dt>出库日期</dt><dd>{preview.shipmentDate}</dd></div>
      </dl>

      <div className="release-money" aria-label="出库金额影响">
        <Metric label="预计销售金额" value={formatMoney(preview.currency, preview.salesAmount)} />
        <Metric label="预计成本" value={preview.expectedCostAmount === null ? "成本缺失" : formatMoney(preview.currency, preview.expectedCostAmount)} warning={preview.expectedCostAmount === null} />
        <Metric label="预计经营应收" value={formatMoney(preview.currency, preview.expectedReceivableAmount)} />
        <Metric label="预计到期日" value={preview.expectedDueDate} />
      </div>

      <div className="release-lines">
        <div className="release-table release-table-head" aria-hidden="true">
          <span>商品</span><span>出库量</span><span>预占可用</span><span>现存</span><span>已预占</span><span>移动平均成本</span><span>预计成本</span><span>检查</span>
        </div>
        {preview.lines.map((line) => (
          <article className={`release-table ${line.ready ? "" : "blocked-line"}`} key={line.salesOrderLineId}>
            <div><strong>{line.skuCode}</strong><span>{line.skuName}</span></div>
            <b>{formatQuantity(line.quantity)}</b>
            <span>{formatQuantity(line.reservationOpenQuantity)}</span>
            <span>{formatQuantity(line.onHandQuantity)}</span>
            <span>{formatQuantity(line.reservedQuantity)}</span>
            <span>{line.averageUnitCost === null ? "—" : formatMoney(preview.currency, line.averageUnitCost)}</span>
            <strong>{line.expectedCostAmount === null ? "—" : formatMoney(preview.currency, line.expectedCostAmount)}</strong>
            <em>{line.ready ? "通过" : line.readiness === "missing_inventory_cost" ? "成本缺失" : "余额不足"}</em>
          </article>
        ))}
      </div>

      <footer className="confirmation-foot">
        <p>库存时点 {formatTime(preview.inventoryAsOf)} · 确认事务会重新锁定库存、预占和订单状态。</p>
        <button
          type="button"
          disabled={!preview.canConfirm || busy}
          onClick={async () => {
            setBusy(true);
            setError("");
            try {
              await request(`/api/v1/shipments/${shipmentId}/confirm`, {
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
          {busy ? "正在确认…" : "确认出库并生成应收"}
        </button>
      </footer>
      {error && <div className="confirmation-error">{error}</div>}
    </section>
  );
}

function Metric({ label, value, warning = false }: { label: string; value: string; warning?: boolean }) {
  return <div className={warning ? "warning" : ""}><dt>{label}</dt><dd>{value}</dd></div>;
}

function formatTime(value: string) {
  return new Intl.DateTimeFormat("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(new Date(value));
}
