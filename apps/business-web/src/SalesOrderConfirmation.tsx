import React from "react";
import { type SalesOrderConfirmationPreview, request } from "./api";
import { formatQuantity } from "./formatters";

const READINESS = {
  ready: {
    title: "库存充足，可以确认",
    note: "确认后将按下列数量全量预占库存。",
  },
  permission_required: {
    title: "当前角色不能确认订单",
    note: "库存核对结果仍可查看；请由具有销售订单确认权限的人员处理。",
  },
  insufficient_stock: {
    title: "库存不足，暂不能确认",
    note: "补充库存或调整订单行后重新核对。",
  },
  order_not_draft: {
    title: "订单已离开草稿状态",
    note: "只有草稿订单需要执行确认前库存核对。",
  },
} as const;

export function SalesOrderConfirmation({
  orderId,
  onDone,
}: {
  orderId: string;
  onDone: () => void;
}) {
  const [preview, setPreview] = React.useState<SalesOrderConfirmationPreview>();
  const [loading, setLoading] = React.useState(true);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState("");

  React.useEffect(() => {
    let active = true;
    setLoading(true);
    setError("");
    request<SalesOrderConfirmationPreview>(
      `/api/v1/sales-orders/${orderId}/confirmation-preview`,
    )
      .then((value) => active && setPreview(value))
      .catch((reason: Error) => active && setError(reason.message))
      .finally(() => active && setLoading(false));
    return () => {
      active = false;
    };
  }, [orderId]);

  if (loading) {
    return <div className="confirmation-loading">正在核对实时库存…</div>;
  }
  if (error || !preview) {
    return (
      <div className="error">
        <strong>无法完成确认前检查</strong>
        <span>{error || "订单库存核对结果不可用"}</span>
      </div>
    );
  }

  const message = READINESS[preview.readiness];
  return (
    <section
      className={`confirmation ${preview.canConfirm ? "ready" : "blocked"}`}
    >
      <header className="confirmation-head">
        <div>
          <p>确认前库存核对</p>
          <h2>{message.title}</h2>
          <span>{message.note}</span>
        </div>
        <div className="confirmation-decision">
          <small>{preview.orderNumber}</small>
          <strong>{preview.canConfirm ? "READY" : "BLOCKED"}</strong>
        </div>
      </header>

      <div
        className="confirmation-grid confirmation-grid-head"
        aria-hidden="true"
      >
        <span>商品 / 仓库</span>
        <span>需求</span>
        <span>现存</span>
        <span>已预占</span>
        <span>可用</span>
        <span>预计预占</span>
        <span>缺口</span>
      </div>
      {preview.lines.map((line) => {
        const shortage = Number(line.shortageQuantity) > 0;
        return (
          <article
            className={`confirmation-grid ${shortage ? "shortage" : ""}`}
            key={`${line.warehouseId}:${line.skuId}`}
          >
            <div>
              <strong>{line.skuCode}</strong>
              <span>{line.skuName}</span>
              <small>
                {line.warehouseCode} · {line.warehouseName}
              </small>
            </div>
            <b>{formatQuantity(line.requiredQuantity)}</b>
            <span>{formatQuantity(line.onHandQuantity)}</span>
            <span>{formatQuantity(line.reservedQuantity)}</span>
            <strong>{formatQuantity(line.availableQuantity)}</strong>
            <strong>{formatQuantity(line.expectedReservedQuantity)}</strong>
            <strong className="shortage-value">
              {formatQuantity(line.shortageQuantity)}
            </strong>
          </article>
        );
      })}

      <footer className="confirmation-foot">
        <p>
          库存时点 {formatTime(preview.inventoryAsOf)} ·
          确认事务会再次锁定并核对库存，避免并发超卖。
        </p>
        <button
          type="button"
          disabled={!preview.canConfirm || busy}
          onClick={async () => {
            setBusy(true);
            setError("");
            try {
              await request(`/api/v1/sales-orders/${orderId}/confirm`, {
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
          {busy ? "正在确认…" : "确认并预占库存"}
        </button>
      </footer>
      {error && <div className="confirmation-error">{error}</div>}
    </section>
  );
}

function formatTime(value: string) {
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(new Date(value));
}
