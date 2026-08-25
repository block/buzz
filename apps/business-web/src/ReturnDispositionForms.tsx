import React from "react";
import type { BusinessReturn, ReturnAnalytics, ReturnInspection } from "./api";
import { request } from "./api";
import { formatMoney, formatQuantity } from "./formatters";
import "./return-disposition.css";

export function SalesReturnInspection({
  item,
  onDone,
}: {
  item: BusinessReturn;
  onDone: () => void;
}) {
  const [inspection, setInspection] = React.useState<ReturnInspection | null>(
    null,
  );
  const [accepted, setAccepted] = React.useState<Record<string, string>>({});
  const [scrap, setScrap] = React.useState<Record<string, string>>({});
  const [date, setDate] = React.useState(today());
  const [note, setNote] = React.useState("");
  const [notice, setNotice] = React.useState("");
  const [busy, setBusy] = React.useState(false);

  React.useEffect(() => {
    request<ReturnInspection>(`/api/v1/sales-returns/${item.id}/inspection`)
      .then((result) => {
        setInspection(result);
        setAccepted(
          Object.fromEntries(
            result.lines.map((line) => [line.returnLineId, line.quantity]),
          ),
        );
        setScrap(
          Object.fromEntries(
            result.lines.map((line) => [line.returnLineId, "0"]),
          ),
        );
      })
      .catch((error: Error) => setNotice(error.message));
  }, [item.id]);

  function updateDisposition(lineId: string, quantity: string, value: string) {
    const scrapValue = Math.max(0, Math.min(number(value), number(quantity)));
    setScrap((current) => ({ ...current, [lineId]: trim(scrapValue) }));
    setAccepted((current) => ({
      ...current,
      [lineId]: trim(number(quantity) - scrapValue),
    }));
  }

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!inspection) return;
    setBusy(true);
    setNotice("");
    try {
      await request(`/api/v1/sales-returns/${item.id}/inspection`, {
        method: "POST",
        body: JSON.stringify({
          expectedVersion: inspection.version,
          inspectionDate: date,
          inspectionNote: note.trim() || undefined,
          lines: inspection.lines.map((line) => ({
            returnLineId: line.returnLineId,
            acceptedQuantity: accepted[line.returnLineId] ?? "0",
            scrapQuantity: scrap[line.returnLineId] ?? "0",
          })),
        }),
      });
      onDone();
    } catch (error) {
      setNotice((error as Error).message);
    } finally {
      setBusy(false);
    }
  }

  if (!inspection && !notice)
    return <p className="disposition-loading">正在读取隔离库存…</p>;
  return (
    <form className="disposition-form" onSubmit={submit}>
      <div className="disposition-callout">
        <strong>隔离库存必须一次完成处置</strong>
        <span>
          合格数量释放为可售库存；报废数量扣减库存价值并计入退货损失。
        </span>
      </div>
      <label>
        <span>质检日期</span>
        <input
          type="date"
          value={date}
          min={item.returnDate}
          onChange={(event) => setDate(event.target.value)}
          required
        />
      </label>
      {inspection?.lines.map((line) => (
        <div className="inspection-line" key={line.returnLineId}>
          <div>
            <strong>{line.skuCode}</strong>
            <span>{line.skuName}</span>
          </div>
          <span>退回 {formatQuantity(line.quantity)}</span>
          <label>
            <span>合格</span>
            <input value={accepted[line.returnLineId] ?? "0"} readOnly />
          </label>
          <label>
            <span>报废</span>
            <input
              inputMode="decimal"
              value={scrap[line.returnLineId] ?? "0"}
              onChange={(event) =>
                updateDisposition(
                  line.returnLineId,
                  line.quantity,
                  event.target.value,
                )
              }
            />
          </label>
        </div>
      ))}
      <label>
        <span>质检说明</span>
        <textarea
          value={note}
          maxLength={1000}
          onChange={(event) => setNote(event.target.value)}
          placeholder="选填：外观、包装、功能检查结果"
        />
      </label>
      {notice && <p className="form-notice error">{notice}</p>}
      <button type="submit" disabled={!inspection || busy}>
        {busy ? "正在写入处置事实…" : "确认质检与库存处置"}
      </button>
    </form>
  );
}

export function PurchaseReturnDispatch({
  item,
  onDone,
}: {
  item: BusinessReturn;
  onDone: () => void;
}) {
  return (
    <PurchaseLogisticsForm
      title="登记退货发出"
      description="记录承运商与运单，形成供应商退货在途证据。"
      button="确认发出"
      dateLabel="发出日期"
      onSubmit={(date, _note, carrier, tracking) =>
        request(`/api/v1/purchase-returns/${item.id}/dispatch`, {
          method: "POST",
          body: JSON.stringify({
            expectedVersion: item.version,
            dispatchDate: date,
            carrier,
            trackingNumber: tracking,
          }),
        })
      }
      onDone={onDone}
      requireTracking
    />
  );
}

export function PurchaseReturnAcknowledgment({
  item,
  onDone,
}: {
  item: BusinessReturn;
  onDone: () => void;
}) {
  return (
    <PurchaseLogisticsForm
      title="登记供应商签收"
      description="仅记录供应商对实物退货的签收，不产生银行或总账事实。"
      button="确认供应商已签收"
      dateLabel="签收日期"
      onSubmit={(date, note) =>
        request(`/api/v1/purchase-returns/${item.id}/supplier-acknowledge`, {
          method: "POST",
          body: JSON.stringify({
            expectedVersion: item.version,
            acknowledgedDate: date,
            acknowledgmentNote: note.trim() || undefined,
          }),
        })
      }
      onDone={onDone}
    />
  );
}

function PurchaseLogisticsForm({
  title,
  description,
  button,
  dateLabel,
  onSubmit,
  onDone,
  requireTracking = false,
}: {
  title: string;
  description: string;
  button: string;
  dateLabel: string;
  onSubmit: (
    date: string,
    note: string,
    carrier: string,
    tracking: string,
  ) => Promise<unknown>;
  onDone: () => void;
  requireTracking?: boolean;
}) {
  const [date, setDate] = React.useState(today());
  const [note, setNote] = React.useState("");
  const [carrier, setCarrier] = React.useState("");
  const [tracking, setTracking] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [notice, setNotice] = React.useState("");
  return (
    <form
      className="disposition-form logistics"
      onSubmit={async (event) => {
        event.preventDefault();
        setBusy(true);
        setNotice("");
        try {
          await onSubmit(date, note, carrier, tracking);
          onDone();
        } catch (error) {
          setNotice((error as Error).message);
        } finally {
          setBusy(false);
        }
      }}
    >
      <div className="disposition-callout">
        <strong>{title}</strong>
        <span>{description}</span>
      </div>
      <label>
        <span>{dateLabel}</span>
        <input
          type="date"
          value={date}
          onChange={(event) => setDate(event.target.value)}
          required
        />
      </label>
      {requireTracking && (
        <div className="logistics-fields">
          <label>
            <span>承运商</span>
            <input
              value={carrier}
              maxLength={120}
              onChange={(event) => setCarrier(event.target.value)}
              required
            />
          </label>
          <label>
            <span>运单号</span>
            <input
              value={tracking}
              maxLength={120}
              onChange={(event) => setTracking(event.target.value)}
              required
            />
          </label>
        </div>
      )}
      {!requireTracking && (
        <label>
          <span>签收备注</span>
          <textarea
            value={note}
            maxLength={1000}
            onChange={(event) => setNote(event.target.value)}
          />
        </label>
      )}
      {notice && <p className="form-notice error">{notice}</p>}
      <button type="submit" disabled={busy}>
        {busy ? "正在写入业务事实…" : button}
      </button>
    </form>
  );
}

export function ReturnAnalyticsPanel({ side }: { side: "sales" | "purchase" }) {
  const [data, setData] = React.useState<ReturnAnalytics | null>(null);
  React.useEffect(() => {
    request<ReturnAnalytics>(
      `/api/v1/return-analytics?period=${month()}&currency=CNY`,
    )
      .then(setData)
      .catch(() => setData(null));
  }, []);
  const items = data?.items ?? [];
  const sales = side === "sales";
  const amount = total(
    items,
    sales ? "salesReturnAmount" : "purchaseReturnAmount",
  );
  const base = total(
    items,
    sales ? "shippedSalesAmount" : "receivedPurchaseAmount",
  );
  const loss = total(items, "returnLossAmount");
  const scrapCost = total(items, "scrapCostAmount");
  return (
    <section className="return-analytics" aria-label="本月退货经营指标">
      <div>
        <span>本月退货率</span>
        <strong>
          {base === 0 ? "—" : `${((amount / base) * 100).toFixed(2)}%`}
        </strong>
        <small>{sales ? "按销售出库金额" : "按采购收货金额"}</small>
      </div>
      <div>
        <span>本月退货金额</span>
        <strong>{formatMoney("CNY", amount)}</strong>
        <small>{items.length} 个经营主体</small>
      </div>
      {sales && (
        <>
          <div>
            <span>退货毛利影响</span>
            <strong>{formatMoney("CNY", loss)}</strong>
            <small>收入冲减－成本转回＋报废</small>
          </div>
          <div>
            <span>报废成本</span>
            <strong>{formatMoney("CNY", scrapCost)}</strong>
            <small>已完成质检处置</small>
          </div>
        </>
      )}
    </section>
  );
}

function total(
  items: ReturnAnalytics["items"],
  key:
    | "salesReturnAmount"
    | "purchaseReturnAmount"
    | "shippedSalesAmount"
    | "receivedPurchaseAmount"
    | "returnLossAmount"
    | "scrapCostAmount",
) {
  return items.reduce((sum, item) => sum + Number(item[key]), 0);
}
function number(value: string) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : 0;
}
function trim(value: number) {
  return String(Number(value.toFixed(6)));
}
function today() {
  return new Date().toISOString().slice(0, 10);
}
function month() {
  return new Date().toISOString().slice(0, 7);
}
