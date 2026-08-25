import React from "react";
import { createPortal } from "react-dom";
import type {
  PurchaseDelivery,
  PurchaseDeliveryResponse,
  SupplierDeliveryPerformance,
  SupplierPerformanceResponse,
} from "./api";
import { request } from "./api";
import { formatQuantity } from "./formatters";
import "./purchase-delivery.css";

export function PurchaseDeliveryPanel({
  onChanged,
}: {
  onChanged: () => void;
}) {
  const [revision, setRevision] = React.useState(0);
  const [deliveries, setDeliveries] = React.useState<PurchaseDelivery[]>([]);
  const [performance, setPerformance] = React.useState<
    SupplierDeliveryPerformance[]
  >([]);
  const [canManage, setCanManage] = React.useState(false);
  const [period, setPeriod] = React.useState("90");
  const [selected, setSelected] = React.useState<PurchaseDelivery | null>(null);
  const [notice, setNotice] = React.useState("");
  const [loading, setLoading] = React.useState(true);

  // biome-ignore lint/correctness/useExhaustiveDependencies: revision is an explicit command-completion reload token.
  React.useEffect(() => {
    setLoading(true);
    Promise.all([
      request<PurchaseDeliveryResponse>(
        "/api/v1/purchase-deliveries?limit=500",
      ),
      request<SupplierPerformanceResponse>(
        `/api/v1/supplier-delivery-performance?days=${period}`,
      ),
    ])
      .then(([deliveryResult, performanceResult]) => {
        setDeliveries(deliveryResult.items);
        setCanManage(deliveryResult.canManageCommitments);
        setPerformance(performanceResult.items);
        setNotice("");
      })
      .catch((error: Error) => setNotice(error.message))
      .finally(() => setLoading(false));
  }, [revision, period]);

  const open = deliveries.filter(
    (item) =>
      Number(item.openQuantity) > 0 && item.lifecycleStatus === "confirmed",
  );
  const overdue = open.filter((item) => item.deliveryStatus === "overdue");
  const dueSoon = open.filter((item) =>
    ["due_today", "due_soon"].includes(item.deliveryStatus),
  );
  const unscheduled = open.filter(
    (item) => item.deliveryStatus === "unscheduled",
  );
  const partial = open.filter((item) => Number(item.receivedQuantity) > 0);

  const done = () => {
    setSelected(null);
    setRevision((value) => value + 1);
    onChanged();
  };

  return (
    <section className="delivery-control" aria-busy={loading}>
      <header className="delivery-titlebar">
        <div>
          <span>Supplier delivery control</span>
          <strong>采购交期与到货履约</strong>
          <p>供应商承诺、分批到货和退货质量事实实时归集。</p>
        </div>
        <label>
          <span>表现周期</span>
          <select
            value={period}
            onChange={(event) => setPeriod(event.target.value)}
          >
            <option value="30">近 30 天</option>
            <option value="90">近 90 天</option>
            <option value="180">近 180 天</option>
            <option value="365">近 365 天</option>
          </select>
        </label>
      </header>

      <div className="delivery-metrics">
        <Metric
          label="逾期未齐"
          value={overdue.length}
          tone="danger"
          note="需要推进供应商"
        />
        <Metric
          label="三日内到期"
          value={dueSoon.length}
          tone="warning"
          note="含今日到期"
        />
        <Metric label="部分到货" value={partial.length} note="仍有未交数量" />
        <Metric
          label="未取得承诺"
          value={unscheduled.length}
          note="需补录交期"
        />
      </div>

      {notice && <div className="delivery-notice">{notice}</div>}

      <div className="delivery-register">
        <div className="delivery-head" aria-hidden="true">
          <span>采购订单 / 供应商</span>
          <span>交期轨道</span>
          <span>数量履约</span>
          <span>状态</span>
          <span>下一步</span>
        </div>
        {deliveries.map((item) => (
          <article key={item.purchaseOrderId}>
            <div className="delivery-document">
              <strong>{item.purchaseOrderNumber}</strong>
              <span>
                {item.supplierCode} · {item.supplierName}
              </span>
              <small>下单 {item.orderDate}</small>
            </div>
            <DeliveryTrack item={item} />
            <QuantityProgress item={item} />
            <div className="delivery-state">
              <Status value={item.deliveryStatus} />
              <small>{variance(item)}</small>
            </div>
            <div className="delivery-actions">
              {canManage &&
                item.lifecycleStatus === "confirmed" &&
                Number(item.openQuantity) > 0 && (
                  <button type="button" onClick={() => setSelected(item)}>
                    {item.commitmentRevision > 0 ? "更新承诺" : "记录承诺"}
                  </button>
                )}
              <a href={`/purchase-orders/${item.purchaseOrderId}`}>查看订单</a>
            </div>
          </article>
        ))}
        {!loading && deliveries.length === 0 && (
          <p className="delivery-empty">
            暂无采购履约记录。确认采购订单后将在这里出现。
          </p>
        )}
      </div>

      <details className="supplier-scorecard" open>
        <summary>供应商履约表现 · {period} 天</summary>
        <div className="scorecard-head" aria-hidden="true">
          <span>供应商</span>
          <span>准时完成</span>
          <span>数量履约</span>
          <span>质量接受</span>
          <span>逾期</span>
        </div>
        {performance.map((item) => (
          <article key={item.supplierId}>
            <div>
              <strong>{item.supplierCode}</strong>
              <span>{item.supplierName}</span>
            </div>
            <Rate
              value={item.onTimeRate}
              detail={`${item.onTimeOrderCount}/${item.completedOrderCount} 单`}
            />
            <Rate
              value={item.fulfillmentRate}
              detail={`${formatQuantity(item.receivedQuantity)}/${formatQuantity(item.orderedQuantity)}`}
            />
            <Rate
              value={item.qualityAcceptanceRate}
              detail={`退货 ${formatQuantity(item.returnedQuantity)}`}
            />
            <b className={item.overdueOrderCount > 0 ? "scorecard-alert" : ""}>
              {item.overdueOrderCount}
            </b>
          </article>
        ))}
        {!loading && performance.length === 0 && (
          <p className="delivery-empty">当前周期暂无可评价的采购订单。</p>
        )}
      </details>

      {selected &&
        createPortal(
          <CommitmentModal
            item={selected}
            onClose={() => setSelected(null)}
            onDone={done}
          />,
          document.body,
        )}
    </section>
  );
}

function CommitmentModal({
  item,
  onClose,
  onDone,
}: {
  item: PurchaseDelivery;
  onClose: () => void;
  onDone: () => void;
}) {
  const [date, setDate] = React.useState(
    item.promisedDeliveryDate ?? item.expectedDeliveryDate ?? "",
  );
  const [note, setNote] = React.useState(item.commitmentNote ?? "");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState("");

  const save = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError("");
    try {
      await request(
        `/api/v1/purchase-orders/${item.purchaseOrderId}/delivery-commitments`,
        {
          method: "POST",
          body: JSON.stringify({
            promisedDeliveryDate: date,
            expectedRevision: item.commitmentRevision,
            commitmentNote: note.trim() || undefined,
          }),
        },
      );
      onDone();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "交期承诺保存失败");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="delivery-scrim">
      <section
        className="delivery-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="delivery-modal-title"
      >
        <header>
          <div>
            <span>Delivery commitment</span>
            <h2 id="delivery-modal-title">
              {item.commitmentRevision ? "更新供应商承诺" : "记录供应商承诺"}
            </h2>
          </div>
          <button type="button" aria-label="关闭" onClick={onClose}>
            ×
          </button>
        </header>
        <form onSubmit={save}>
          <div className="delivery-modal-context">
            <strong>{item.purchaseOrderNumber}</strong>
            <span>
              {item.supplierCode} · {item.supplierName}
            </span>
            <small>
              订单计划 {item.expectedDeliveryDate ?? "未设置"} · 未交{" "}
              {formatQuantity(item.openQuantity)}
            </small>
          </div>
          {item.commitmentRevision > 0 && (
            <p className="delivery-history-note">
              本次保存将形成第 {item.commitmentRevision + 1} 版承诺；第{" "}
              {item.commitmentRevision} 版继续保留在审计历史中。
            </p>
          )}
          <label>
            <span>供应商承诺到货日</span>
            <input
              type="date"
              min={item.orderDate}
              value={date}
              onChange={(event) => setDate(event.target.value)}
              required
            />
          </label>
          <label>
            <span>承诺依据与跟进备注</span>
            <textarea
              value={note}
              maxLength={1000}
              onChange={(event) => setNote(event.target.value)}
              placeholder="例如：供应商邮件确认，首批 25 日到仓"
            />
          </label>
          {error && <p className="delivery-form-error">{error}</p>}
          <footer>
            <button type="button" className="secondary" onClick={onClose}>
              取消
            </button>
            <button type="submit" disabled={busy || !date}>
              {busy ? "正在保存…" : "保存交期承诺"}
            </button>
          </footer>
        </form>
      </section>
    </div>
  );
}

function DeliveryTrack({ item }: { item: PurchaseDelivery }) {
  const promised = item.promisedDeliveryDate;
  return (
    <div className={`delivery-track ${item.deliveryStatus}`}>
      <div>
        <i className="order-dot" />
        <span />
        <i className="promise-dot" />
      </div>
      <small>
        <b>{promised ?? "未承诺"}</b>
        <em>
          {item.commitmentSource === "supplier_commitment"
            ? `供应商 v${item.commitmentRevision}`
            : "订单计划"}
        </em>
      </small>
    </div>
  );
}

function QuantityProgress({ item }: { item: PurchaseDelivery }) {
  const ordered = Number(item.orderedQuantity);
  const received = Number(item.receivedQuantity);
  const width = ordered > 0 ? Math.min(100, (received / ordered) * 100) : 0;
  return (
    <div className="delivery-quantity">
      <div>
        <i style={{ width: `${width}%` }} />
      </div>
      <small>
        已到 {formatQuantity(item.receivedQuantity)} / 订购 {formatQuantity(item.orderedQuantity)}
      </small>
      <span>{item.receiptCount} 批</span>
    </div>
  );
}

function Metric({
  label,
  value,
  note,
  tone = "default",
}: {
  label: string;
  value: number;
  note: string;
  tone?: "default" | "warning" | "danger";
}) {
  return (
    <div className={tone}>
      <span>{label}</span>
      <strong>{value}</strong>
      <small>{note}</small>
    </div>
  );
}

function Status({ value }: { value: PurchaseDelivery["deliveryStatus"] }) {
  const labels: Record<PurchaseDelivery["deliveryStatus"], string> = {
    cancelled: "已取消",
    unscheduled: "未排期",
    completed_on_time: "准时到齐",
    completed_late: "逾期到齐",
    overdue: "已逾期",
    due_today: "今日到期",
    due_soon: "即将到期",
    on_track: "按期推进",
  };
  return <b className={`delivery-status ${value}`}>{labels[value]}</b>;
}

function Rate({ value, detail }: { value: string | null; detail: string }) {
  return (
    <div className="score-rate">
      <strong>
        {value === null ? "—" : `${Math.round(Number(value) * 100)}%`}
      </strong>
      <span>{detail}</span>
    </div>
  );
}

function variance(item: PurchaseDelivery) {
  if (item.deliveryVarianceDays === null) return "尚无日期基线";
  if (item.deliveryStatus === "completed_on_time")
    return item.deliveryVarianceDays === 0
      ? "按承诺日到齐"
      : `提前 ${Math.abs(item.deliveryVarianceDays)} 天`;
  if (item.deliveryVarianceDays > 0)
    return `超期 ${item.deliveryVarianceDays} 天`;
  if (item.deliveryVarianceDays === 0) return "今天到期";
  return `距交期 ${Math.abs(item.deliveryVarianceDays)} 天`;
}
