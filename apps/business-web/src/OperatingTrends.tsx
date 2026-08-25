import React from "react";
import {
  type OperatingSubscription,
  type OperatingSubscriptionList,
  type OperatingTrendSeries,
  request,
} from "./api";

type Cadence = "daily" | "weekly";

export function OperatingTrendsView() {
  const [cadence, setCadence] = React.useState<Cadence>("daily");
  const [series, setSeries] = React.useState<OperatingTrendSeries | null>(null);
  const [subscriptions, setSubscriptions] =
    React.useState<OperatingSubscriptionList | null>(null);
  const [busy, setBusy] = React.useState(false);
  const [notice, setNotice] = React.useState<string | null>(null);

  const load = React.useCallback(async () => {
    const [nextSeries, nextSubscriptions] = await Promise.all([
      request<OperatingTrendSeries>(
        `/api/v1/operations/trends?cadence=${cadence}&currency=CNY&limit=14`,
      ),
      request<OperatingSubscriptionList>("/api/v1/operations/subscriptions"),
    ]);
    setSeries(nextSeries);
    setSubscriptions(nextSubscriptions);
  }, [cadence]);

  React.useEffect(() => {
    load().catch((error: Error) => setNotice(error.message));
  }, [load]);

  async function freeze() {
    setBusy(true);
    setNotice(null);
    try {
      const result = await request<{ created: boolean }>(
        "/api/v1/operations/snapshots",
        {
          method: "POST",
          body: JSON.stringify({
            cadence,
            currency: "CNY",
            periodStart: completedPeriodStart(cadence),
            utcOffsetMinutes: -new Date().getTimezoneOffset(),
          }),
        },
      );
      setNotice(
        result.created
          ? "已冻结上一完整周期。"
          : "该周期已冻结，历史未被改写。",
      );
      await load();
    } catch (error) {
      setNotice((error as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function subscribe() {
    setBusy(true);
    setNotice(null);
    try {
      await request("/api/v1/operations/subscriptions", {
        method: "POST",
        body: JSON.stringify({
          cadence,
          currency: "CNY",
          utcOffsetMinutes: -new Date().getTimezoneOffset(),
          deliveryHour: 8,
        }),
      });
      setNotice(
        `已启用${cadence === "daily" ? "每日" : "每周"} 08:00 站内快照。`,
      );
      await load();
    } catch (error) {
      setNotice((error as Error).message);
    } finally {
      setBusy(false);
    }
  }

  async function toggle(subscription: OperatingSubscription) {
    setBusy(true);
    setNotice(null);
    try {
      await request(
        `/api/v1/operations/subscriptions/${subscription.id}/commands`,
        {
          method: "POST",
          body: JSON.stringify({
            action: subscription.status === "active" ? "pause" : "resume",
            expectedVersion: subscription.version,
          }),
        },
      );
      setNotice(
        subscription.status === "active" ? "订阅已暂停。" : "订阅已恢复。",
      );
      await load();
    } catch (error) {
      setNotice((error as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const activeSubscription = subscriptions?.items.find(
    (item) => item.cadence === cadence && item.currency === "CNY",
  );

  return (
    <main className="page trend-page">
      <header className="page-head">
        <div>
          <span className="eyebrow">经营刻度尺 / Operating cadence</span>
          <h1>每天看变化，每周看方向</h1>
          <p>
            冻结完整周期的经营事实，比较销售、采购、利润与异常
            SLA；所有数值均为经营管理口径。
          </p>
        </div>
        <div className="trend-actions">
          <div className="cadence-switch">
            {(["daily", "weekly"] as const).map((value) => (
              <button
                type="button"
                className={cadence === value ? "active" : "secondary"}
                onClick={() => setCadence(value)}
                key={value}
              >
                {value === "daily" ? "日报" : "周报"}
              </button>
            ))}
          </div>
          <button type="button" onClick={freeze} disabled={busy}>
            {busy ? "处理中…" : "冻结上一周期"}
          </button>
        </div>
      </header>

      {notice && <p className="incident-notice">{notice}</p>}

      <section className="schedule-strip" aria-label="站内订阅计划">
        <div>
          <span>BUSINESS DOCK DELIVERY</span>
          <strong>
            {activeSubscription
              ? `${activeSubscription.status === "active" ? "运行中" : "已暂停"} · 下次 ${formatInstant(activeSubscription.nextRunAt)}`
              : "尚未设置站内快照"}
          </strong>
          <small>固定按当前时区 08:00 生成上一完整周期，不发送外部邮件。</small>
        </div>
        {activeSubscription ? (
          <button
            type="button"
            className="secondary"
            onClick={() => toggle(activeSubscription)}
            disabled={busy}
          >
            {activeSubscription.status === "active" ? "暂停订阅" : "恢复订阅"}
          </button>
        ) : (
          <button type="button" onClick={subscribe} disabled={busy}>
            订阅 {cadence === "daily" ? "日报" : "周报"}
          </button>
        )}
      </section>

      {!series ? (
        <div className="empty">正在读取经营刻度…</div>
      ) : series.items.length === 0 ? (
        <div className="empty trend-empty">
          <span>—</span>
          <p>
            还没有{cadence === "daily" ? "日报" : "周报"}
            基线。先冻结上一完整周期。
          </p>
        </div>
      ) : (
        <section className="trend-ruler" aria-label="经营趋势快照">
          <div className="ruler-legend">
            <span>周期</span>
            <span>销售额</span>
            <span>出库收入</span>
            <span>采购额</span>
            <span>经营利润</span>
            <span>SLA 超时</span>
          </div>
          {series.items.map((item, index) => (
            <article className="ruler-row" key={item.id}>
              <div className="ruler-date">
                <i>{String(series.items.length - index).padStart(2, "0")}</i>
                <strong>{item.periodStart}</strong>
                <small>{cadence === "daily" ? "DAY" : "WEEK"}</small>
              </div>
              <TrendValue
                value={`¥ ${compactMoney(item.metrics.salesOrderAmount)}`}
                change={item.change?.salesOrderAmount}
              />
              <TrendValue
                value={`¥ ${compactMoney(item.metrics.shippedRevenue)}`}
                change={item.change?.shippedRevenue}
              />
              <TrendValue
                value={`¥ ${compactMoney(item.metrics.purchaseOrderAmount)}`}
                change={item.change?.purchaseOrderAmount}
              />
              <TrendValue
                value={`¥ ${compactMoney(item.metrics.managementOperatingProfit)}`}
                change={item.change?.managementOperatingProfit}
              />
              <TrendValue
                value={`${item.metrics.slaBreached}`}
                change={item.change?.slaBreached}
                risk={item.metrics.slaBreached > 0}
              />
              <footer>
                <span className={`quality-mark ${item.dataQualityStatus}`}>
                  {item.dataQualityStatus}
                </span>
                <span>平均解决 {item.metrics.averageResolutionHours}h</span>
                <code>{item.sourceHash.slice(0, 10)}</code>
              </footer>
            </article>
          ))}
        </section>
      )}
      <p className="report-warning">
        库存价值与缺货数为快照生成时点值；趋势快照不可变，不是法定财务报表。
      </p>
    </main>
  );
}

function TrendValue({
  value,
  change,
  risk = false,
}: {
  value: string;
  change?: string | null;
  risk?: boolean;
}) {
  const numeric = change ? Number(change) : null;
  return (
    <div className={`trend-value ${risk ? "risk" : ""}`}>
      <strong>{value}</strong>
      <small className={numeric !== null && numeric < 0 ? "down" : ""}>
        {numeric === null ? "首个基线" : `${numeric > 0 ? "+" : ""}${change}%`}
      </small>
    </div>
  );
}

function completedPeriodStart(cadence: Cadence) {
  const date = new Date();
  date.setHours(12, 0, 0, 0);
  if (cadence === "daily") date.setDate(date.getDate() - 1);
  else {
    const mondayDistance = (date.getDay() + 6) % 7;
    date.setDate(date.getDate() - mondayDistance - 7);
  }
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

function compactMoney(value: string) {
  return new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 0 }).format(
    Number(value),
  );
}

function formatInstant(value: string) {
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}
