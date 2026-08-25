import React from "react";
import { createPortal } from "react-dom";
import type {
  Envelope,
  InventoryAgingItem,
  InventoryCountDetail,
  InventoryCountOption,
  InventoryCountSummary,
  InventoryTurnover,
} from "./api";
import { request } from "./api";
import { fixedDecimal, formatMoney, formatQuantity } from "./formatters";
import "./inventory-count.css";

type Modal =
  | { kind: "create" }
  | { kind: "enter"; item: InventoryCountSummary }
  | { kind: "post"; item: InventoryCountSummary }
  | { kind: "cancel"; item: InventoryCountSummary };

export function InventoryCountPanel({ onChanged }: { onChanged: () => void }) {
  const [revision, setRevision] = React.useState(0);
  const [counts, setCounts] = React.useState<InventoryCountSummary[]>([]);
  const [aging, setAging] = React.useState<InventoryAgingItem[]>([]);
  const [turnover, setTurnover] = React.useState<InventoryTurnover | null>(
    null,
  );
  const [modal, setModal] = React.useState<Modal | null>(null);
  const [notice, setNotice] = React.useState("");

  // biome-ignore lint/correctness/useExhaustiveDependencies: revision explicitly reloads server state after a command.
  React.useEffect(() => {
    Promise.all([
      request<Envelope<InventoryCountSummary>>(
        "/api/v1/inventory-counts?limit=100",
      ),
      request<{ items: InventoryAgingItem[] }>(
        "/api/v1/inventory-aging?thresholdDays=90&limit=100",
      ),
      request<InventoryTurnover>(
        `/api/v1/inventory-turnover?period=${month()}&currency=CNY`,
      ),
    ])
      .then(([countResult, agingResult, turnoverResult]) => {
        setCounts(countResult.items);
        setAging(agingResult.items);
        setTurnover(turnoverResult);
        setNotice("");
      })
      .catch((error: Error) => setNotice(error.message));
  }, [revision]);

  const refresh = () => {
    setModal(null);
    setRevision((value) => value + 1);
    onChanged();
  };
  const frozen = counts.filter((item) =>
    ["counting", "counted"].includes(item.status),
  ).length;
  const sluggishValues = aging.reduce((totals, item) => {
    const currency = item.currency ?? "未标币种";
    totals.set(
      currency,
      (totals.get(currency) ?? 0) + number(item.inventoryValue),
    );
    return totals;
  }, new Map<string, number>());
  const sluggishEntry = [...sluggishValues.entries()][0];
  return (
    <section className="inventory-control">
      <header>
        <div>
          <span>Inventory control</span>
          <strong>盘点与库存健康</strong>
        </div>
        <button type="button" onClick={() => setModal({ kind: "create" })}>
          新建盘点任务
        </button>
      </header>
      <div className="inventory-health">
        <Metric
          label="冻结中的盘点"
          value={`${frozen} 个`}
          note="按仓库 / SKU 冻结"
        />
        <Metric
          label="本月库存周转"
          value={turnover?.turnoverRate ? `${turnover.turnoverRate} 次` : "—"}
          note={
            turnover?.turnoverDays
              ? `${turnover.turnoverDays} 天`
              : "暂无出库成本"
          }
        />
        <Metric
          label="90 天以上商品"
          value={`${aging.length} 个`}
          note="按最后出库日"
        />
        <Metric
          label="呆滞库存价值"
          value={
            sluggishValues.size <= 1 && sluggishEntry
              ? formatMoney(sluggishEntry[0], sluggishEntry[1])
              : sluggishValues.size > 1
                ? `${sluggishValues.size} 个币种`
                : formatMoney("CNY", 0)
          }
          note={sluggishValues.size > 1 ? "分币种查看明细" : "经营管理口径"}
        />
      </div>
      {notice && <p className="inventory-count-notice">{notice}</p>}
      <div className="inventory-count-table">
        <div className="inventory-count-head">
          <span>盘点任务</span>
          <span>盘点日</span>
          <span>范围</span>
          <span>差异</span>
          <span>状态 / 下一步</span>
        </div>
        {counts.map((item) => (
          <article key={item.id}>
            <div>
              <strong>{item.countNumber}</strong>
              <code>{short(item.warehouseId)}</code>
            </div>
            <span>{item.countDate}</span>
            <span>{item.lineCount} 个 SKU</span>
            <span>
              {item.varianceLineCount} 行 ·{" "}
              {formatMoney(item.currency, item.varianceValue)}
            </span>
            <div className="inventory-count-actions">
              <Status value={item.status} />
              {item.status === "counting" && (
                <button
                  type="button"
                  onClick={() => setModal({ kind: "enter", item })}
                >
                  录入实盘
                </button>
              )}
              {item.status === "counted" && (
                <button
                  type="button"
                  onClick={() => setModal({ kind: "post", item })}
                >
                  确认差异
                </button>
              )}
              {["counting", "counted"].includes(item.status) && (
                <button
                  className="secondary"
                  type="button"
                  onClick={() => setModal({ kind: "cancel", item })}
                >
                  取消
                </button>
              )}
            </div>
          </article>
        ))}
        {counts.length === 0 && (
          <p className="inventory-count-empty">
            暂无盘点任务。新建后，所选库存范围立即冻结。
          </p>
        )}
      </div>
      {aging.length > 0 && (
        <details className="aging-register">
          <summary>查看 90 天以上呆滞库存 · {aging.length} 项</summary>
          {aging.map((item) => (
            <div key={`${item.warehouseId}-${item.skuId}`}>
              <strong>
                {item.skuCode} · {item.skuName}
              </strong>
              <span>{item.daysWithoutIssue} 天未出库</span>
              <span>在手 {formatQuantity(item.onHandQuantity)}</span>
              <span>
                {formatMoney(item.currency ?? "CNY", item.inventoryValue)}
              </span>
            </div>
          ))}
        </details>
      )}
      {modal &&
        createPortal(
          <div className="inventory-count-scrim" role="presentation">
            <section
              className="inventory-count-modal"
              role="dialog"
              aria-modal="true"
              aria-label={modalTitle(modal)}
            >
              <header>
                <strong>{modalTitle(modal)}</strong>
                <button
                  type="button"
                  aria-label="关闭"
                  onClick={() => setModal(null)}
                >
                  ×
                </button>
              </header>
              {modal.kind === "create" && <CreateCountForm onDone={refresh} />}
              {modal.kind === "enter" && (
                <CountEntryForm item={modal.item} onDone={refresh} />
              )}
              {(modal.kind === "post" || modal.kind === "cancel") && (
                <CountCommand
                  item={modal.item}
                  action={modal.kind}
                  onDone={refresh}
                />
              )}
            </section>
          </div>,
          document.body,
        )}
    </section>
  );
}

function CreateCountForm({ onDone }: { onDone: () => void }) {
  const [options, setOptions] = React.useState<InventoryCountOption[]>([]);
  const [warehouse, setWarehouse] = React.useState("");
  const [selected, setSelected] = React.useState<Set<string>>(new Set());
  const [date, setDate] = React.useState(today());
  const [note, setNote] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [notice, setNotice] = React.useState("");
  React.useEffect(() => {
    request<{ items: InventoryCountOption[] }>(
      "/api/v1/inventory-counts/options",
    )
      .then((result) => setOptions(result.items))
      .catch((error: Error) => setNotice(error.message));
  }, []);
  const warehouses = uniqueWarehouses(options);
  const lines = options.filter((item) => item.warehouseId === warehouse);
  return (
    <form
      className="count-form"
      onSubmit={async (event) => {
        event.preventDefault();
        const source = lines.find((line) => selected.has(line.skuId));
        if (!source || selected.size === 0) {
          setNotice("请选择仓库和至少一个 SKU。");
          return;
        }
        setBusy(true);
        setNotice("");
        try {
          await request("/api/v1/inventory-counts", {
            method: "POST",
            body: JSON.stringify({
              legalEntityId: source.legalEntityId,
              warehouseId: source.warehouseId,
              countDate: date,
              currency: source.currency,
              businessNote: note.trim() || undefined,
              skuIds: [...selected],
            }),
          });
          onDone();
        } catch (error) {
          setNotice((error as Error).message);
        } finally {
          setBusy(false);
        }
      }}
    >
      <div className="count-callout">
        <strong>创建即冻结库存范围</strong>
        <span>盘点完成或取消前，选中 SKU 不可预占、出入库或退货处置。</span>
      </div>
      <label>
        <span>盘点仓库</span>
        <select
          value={warehouse}
          onChange={(event) => {
            setWarehouse(event.target.value);
            setSelected(new Set());
          }}
          required
        >
          <option value="">请选择</option>
          {warehouses.map((item) => (
            <option value={item.id} key={item.id}>
              {item.label}
            </option>
          ))}
        </select>
      </label>
      <label>
        <span>盘点日期</span>
        <input
          type="date"
          value={date}
          onChange={(event) => setDate(event.target.value)}
          required
        />
      </label>
      <div className="count-sku-list">
        <header>
          <span>SKU 范围</span>
          <button
            type="button"
            className="secondary"
            onClick={() =>
              setSelected(new Set(lines.map((line) => line.skuId)))
            }
          >
            全选
          </button>
        </header>
        {lines.map((line) => (
          <label key={line.skuId}>
            <input
              type="checkbox"
              checked={selected.has(line.skuId)}
              onChange={(event) =>
                setSelected((current) => {
                  const next = new Set(current);
                  if (event.target.checked) next.add(line.skuId);
                  else next.delete(line.skuId);
                  return next;
                })
              }
            />
            <span>
              <strong>{line.skuCode}</strong>
              {line.skuName}
            </span>
            <small>
              在手 {formatQuantity(line.onHandQuantity)} · 可用{" "}
              {formatQuantity(
                number(line.onHandQuantity) -
                  number(line.reservedQuantity) -
                  number(line.quarantinedQuantity),
              )}
            </small>
          </label>
        ))}
      </div>
      <label>
        <span>盘点说明</span>
        <textarea
          value={note}
          maxLength={1000}
          onChange={(event) => setNote(event.target.value)}
        />
      </label>
      {notice && <p className="inventory-count-notice">{notice}</p>}
      <button type="submit" disabled={busy || selected.size === 0}>
        {busy ? "正在冻结…" : `创建并冻结 ${selected.size} 个 SKU`}
      </button>
    </form>
  );
}

function CountEntryForm({
  item,
  onDone,
}: {
  item: InventoryCountSummary;
  onDone: () => void;
}) {
  const [detail, setDetail] = React.useState<InventoryCountDetail | null>(null);
  const [actual, setActual] = React.useState<Record<string, string>>({});
  const [cost, setCost] = React.useState<Record<string, string>>({});
  const [notice, setNotice] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  React.useEffect(() => {
    request<InventoryCountDetail>(`/api/v1/inventory-counts/${item.id}`)
      .then((result) => {
        setDetail(result);
        setActual(
          Object.fromEntries(
            result.lines.map((line) => [
              line.id,
              fixedDecimal(line.snapshotOnHandQuantity),
            ]),
          ),
        );
      })
      .catch((error: Error) => setNotice(error.message));
  }, [item.id]);
  return (
    <form
      className="count-form"
      onSubmit={async (event) => {
        event.preventDefault();
        if (!detail) return;
        setBusy(true);
        setNotice("");
        try {
          await request(`/api/v1/inventory-counts/${item.id}/submit`, {
            method: "POST",
            body: JSON.stringify({
              expectedVersion: detail.version,
              lines: detail.lines.map((line) => ({
                countLineId: line.id,
                actualOnHandQuantity: actual[line.id],
                surplusUnitCost: cost[line.id] || undefined,
              })),
            }),
          });
          onDone();
        } catch (error) {
          setNotice((error as Error).message);
        } finally {
          setBusy(false);
        }
      }}
    >
      <div className="count-callout">
        <strong>录入实盘数量</strong>
        <span>
          实盘数不得低于已预占与退货隔离数量之和；盘盈且无成本时补录单位成本。
        </span>
      </div>
      <div className="count-entry-lines">
        {detail?.lines.map((line) => {
          const variance =
            number(actual[line.id]) - number(line.snapshotOnHandQuantity);
          return (
            <div key={line.id}>
              <span>
                <strong>{line.skuCode}</strong>
                <small>{line.skuName}</small>
              </span>
              <span>账面 {formatQuantity(line.snapshotOnHandQuantity)}</span>
              <label>
                <span>实盘</span>
                <input
                  inputMode="decimal"
                  value={actual[line.id] ?? ""}
                  onChange={(event) =>
                    setActual((current) => ({
                      ...current,
                      [line.id]: event.target.value,
                    }))
                  }
                  required
                />
              </label>
              <strong
                className={
                  variance === 0 ? "" : variance > 0 ? "positive" : "negative"
                }
              >
                {variance > 0 ? "+" : ""}
                {formatQuantity(variance)}
              </strong>
              {variance > 0 && !line.snapshotAverageUnitCost ? (
                <label>
                  <span>盘盈单位成本</span>
                  <input
                    inputMode="decimal"
                    value={cost[line.id] ?? ""}
                    onChange={(event) =>
                      setCost((current) => ({
                        ...current,
                        [line.id]: event.target.value,
                      }))
                    }
                    required
                  />
                </label>
              ) : (
                <span />
              )}
            </div>
          );
        })}
      </div>
      {notice && <p className="inventory-count-notice">{notice}</p>}
      <button type="submit" disabled={!detail || busy}>
        {busy ? "正在保存…" : "完成实盘录入"}
      </button>
    </form>
  );
}

function CountCommand({
  item,
  action,
  onDone,
}: {
  item: InventoryCountSummary;
  action: "post" | "cancel";
  onDone: () => void;
}) {
  const [busy, setBusy] = React.useState(false);
  const [notice, setNotice] = React.useState("");
  const post = action === "post";
  return (
    <div className="count-command">
      <div className={`count-callout ${post ? "" : "danger"}`}>
        <strong>{post ? "确认差异并调整库存" : "取消盘点并解冻"}</strong>
        <span>
          {post
            ? "系统将按移动平均成本生成盘盈盘亏流水；确认后不可编辑。"
            : "不会生成库存调整，已录入的盘点轨迹仍会保留。"}
        </span>
      </div>
      {notice && <p className="inventory-count-notice">{notice}</p>}
      <button
        type="button"
        className={post ? "" : "danger"}
        disabled={busy}
        onClick={async () => {
          setBusy(true);
          setNotice("");
          try {
            await request(`/api/v1/inventory-counts/${item.id}/${action}`, {
              method: "POST",
              body: JSON.stringify({ expectedVersion: item.version }),
            });
            onDone();
          } catch (error) {
            setNotice((error as Error).message);
          } finally {
            setBusy(false);
          }
        }}
      >
        {busy ? "正在处理…" : post ? "确认并写入库存流水" : "确认取消盘点"}
      </button>
    </div>
  );
}

function Metric({
  label,
  value,
  note,
}: {
  label: string;
  value: string;
  note: string;
}) {
  return (
    <div>
      <span>{label}</span>
      <strong>{value}</strong>
      <small>{note}</small>
    </div>
  );
}
function Status({ value }: { value: string }) {
  return (
    <span className={`count-status ${value}`}>
      {(
        {
          counting: "盘点中",
          counted: "待确认",
          posted: "已调整",
          cancelled: "已取消",
        } as Record<string, string>
      )[value] ?? value}
    </span>
  );
}
function modalTitle(modal: Modal) {
  if (modal.kind === "create") return "新建库存盘点";
  if (modal.kind === "enter") return `${modal.item.countNumber} · 实盘录入`;
  return `${modal.item.countNumber} · ${modal.kind === "post" ? "差异确认" : "取消盘点"}`;
}
function uniqueWarehouses(items: InventoryCountOption[]) {
  return [
    ...new Map(
      items.map((item) => [
        item.warehouseId,
        {
          id: item.warehouseId,
          label: `${item.warehouseCode} · ${item.warehouseName}`,
        },
      ]),
    ).values(),
  ];
}
function short(value: string) {
  return value.length > 12 ? `${value.slice(0, 7)}…` : value;
}
function number(value: string | undefined) {
  const result = Number(value);
  return Number.isFinite(result) ? result : 0;
}
function today() {
  return new Date().toISOString().slice(0, 10);
}
function month() {
  return new Date().toISOString().slice(0, 7);
}
