import React from "react";
import { createPortal } from "react-dom";
import type {
  Envelope,
  PurchaseOrder,
  PurchaseRequisitionSummary,
  ReplenishmentOptions,
  ReplenishmentSuggestion,
} from "./api";
import { request } from "./api";
import { fixedDecimal, formatMoney, formatQuantity } from "./formatters";
import "./replenishment.css";

type Modal =
  | { kind: "policy"; suggestion?: ReplenishmentSuggestion }
  | { kind: "requisition"; items: ReplenishmentSuggestion[] }
  | { kind: "convert"; item: PurchaseRequisitionSummary }
  | {
      kind: "command";
      action: "confirm" | "cancel";
      item: PurchaseRequisitionSummary;
    };

export function ReplenishmentPanel({ onChanged }: { onChanged: () => void }) {
  const [revision, setRevision] = React.useState(0);
  const [suggestions, setSuggestions] = React.useState<
    ReplenishmentSuggestion[]
  >([]);
  const [requisitions, setRequisitions] = React.useState<
    PurchaseRequisitionSummary[]
  >([]);
  const [selected, setSelected] = React.useState<Set<string>>(new Set());
  const [modal, setModal] = React.useState<Modal | null>(null);
  const [notice, setNotice] = React.useState("");

  // biome-ignore lint/correctness/useExhaustiveDependencies: revision is an explicit command-completion reload token.
  React.useEffect(() => {
    Promise.all([
      request<Envelope<ReplenishmentSuggestion>>(
        "/api/v1/replenishment-suggestions?limit=500",
      ),
      request<Envelope<PurchaseRequisitionSummary>>(
        "/api/v1/purchase-requisitions?limit=200",
      ),
    ])
      .then(([suggestionResult, requisitionResult]) => {
        setSuggestions(suggestionResult.items);
        setRequisitions(requisitionResult.items);
        setNotice("");
      })
      .catch((error: Error) => setNotice(error.message));
  }, [revision]);

  const refresh = () => {
    setModal(null);
    setSelected(new Set());
    setRevision((value) => value + 1);
    onChanged();
  };
  const critical = suggestions.filter(
    (item) => item.riskState === "critical",
  ).length;
  const actionable = suggestions.filter(
    (item) => number(item.suggestedQuantity) > 0,
  );
  const protectedCount = suggestions.filter((item) =>
    ["inbound_covered", "requisition_open"].includes(item.riskState),
  ).length;
  const selectedItems = suggestions.filter((item) => selected.has(item.id));
  const anchor = selectedItems[0];

  return (
    <section className="replenishment-panel">
      <header className="replenishment-titlebar">
        <div>
          <span>Replenishment control</span>
          <strong>补货与缺货预警</strong>
        </div>
        <div>
          <button type="button" onClick={() => setModal({ kind: "policy" })}>
            设置库存策略
          </button>
          <button
            type="button"
            disabled={selectedItems.length === 0}
            onClick={() =>
              setModal({ kind: "requisition", items: selectedItems })
            }
          >
            生成采购需求 · {selectedItems.length}
          </button>
        </div>
      </header>
      <div className="replenishment-metrics">
        <Metric
          label="安全库存告急"
          value={`${critical} 项`}
          note="可用量 ≤ 安全库存"
        />
        <Metric
          label="建议补货"
          value={`${actionable.length} 项`}
          note="扣除在途与未结需求"
        />
        <Metric
          label="已被供应覆盖"
          value={`${protectedCount} 项`}
          note="在途订单或需求单"
        />
        <Metric
          label="待确认需求"
          value={`${requisitions.filter((item) => item.status === "draft").length} 单`}
          note="确认后进入采购计划"
        />
      </div>
      {notice && <p className="replenishment-notice">{notice}</p>}
      <div className="replenishment-grid">
        <div className="replenishment-head">
          <span>选择</span>
          <span>商品 / 仓库</span>
          <span>库存水位</span>
          <span>供应覆盖</span>
          <span>建议 / 状态</span>
          <span>策略</span>
        </div>
        {suggestions.map((item) => {
          const eligible = number(item.suggestedQuantity) > 0;
          const compatible =
            !anchor ||
            selected.has(item.id) ||
            (anchor.warehouseId === item.warehouseId &&
              anchor.preferredSupplierId === item.preferredSupplierId &&
              anchor.legalEntityId === item.legalEntityId);
          return (
            <article key={item.id}>
              <input
                type="checkbox"
                aria-label={`选择 ${item.skuCode}`}
                checked={selected.has(item.id)}
                disabled={!eligible || !compatible}
                onChange={(event) =>
                  setSelected((current) => {
                    const next = new Set(current);
                    if (event.target.checked) next.add(item.id);
                    else next.delete(item.id);
                    return next;
                  })
                }
              />
              <div>
                <strong>
                  {item.skuCode} · {item.skuName}
                </strong>
                <small>
                  {item.warehouseCode} · {item.warehouseName}
                </small>
              </div>
              <StockLevel item={item} />
              <div className="supply-cover">
                <span>在途 {formatQuantity(item.inboundQuantity)}</span>
                <span>需求 {formatQuantity(item.openRequisitionQuantity)}</span>
                <small>预计 {formatQuantity(item.projectedQuantity)}</small>
              </div>
              <div className="replenishment-action-state">
                <Risk value={item.riskState} />
                <strong>
                  {eligible
                    ? `建议 ${formatQuantity(item.suggestedQuantity)}`
                    : "无需新增"}
                </strong>
                <small>
                  {item.supplierCode} · {item.suggestedRequiredDate}
                </small>
              </div>
              <button
                type="button"
                className="secondary"
                onClick={() => setModal({ kind: "policy", suggestion: item })}
              >
                调整
              </button>
            </article>
          );
        })}
        {suggestions.length === 0 && (
          <p className="replenishment-empty">
            尚未设置安全库存策略。先选择一个仓库商品建立库存水位线。
          </p>
        )}
      </div>
      {requisitions.length > 0 && (
        <details className="requisition-register" open>
          <summary>采购需求单 · {requisitions.length}</summary>
          {requisitions.map((item) => (
            <div key={item.id}>
              <strong>{item.requisitionNumber}</strong>
              <span>
                {item.lineCount} 行 · {formatQuantity(item.totalQuantity)} 件
              </span>
              <span>要求 {item.requiredDate}</span>
              <Risk value={item.status} />
              {item.status === "draft" && (
                <span className="requisition-actions">
                  <button
                    type="button"
                    onClick={() =>
                      setModal({ kind: "command", action: "confirm", item })
                    }
                  >
                    确认需求
                  </button>
                  <button
                    type="button"
                    className="secondary"
                    onClick={() =>
                      setModal({ kind: "command", action: "cancel", item })
                    }
                  >
                    取消
                  </button>
                </span>
              )}
              {item.status === "confirmed" && (
                <span className="requisition-actions">
                  <button
                    type="button"
                    onClick={() => setModal({ kind: "convert", item })}
                  >
                    关联采购订单
                  </button>
                  <button
                    type="button"
                    className="secondary"
                    onClick={() =>
                      setModal({ kind: "command", action: "cancel", item })
                    }
                  >
                    取消
                  </button>
                </span>
              )}
            </div>
          ))}
        </details>
      )}
      {modal &&
        createPortal(
          <div className="replenishment-scrim" role="presentation">
            <section
              className="replenishment-modal"
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
              {modal.kind === "policy" && (
                <PolicyForm suggestion={modal.suggestion} onDone={refresh} />
              )}
              {modal.kind === "requisition" && (
                <RequisitionForm items={modal.items} onDone={refresh} />
              )}
              {modal.kind === "command" && (
                <RequisitionCommand
                  item={modal.item}
                  action={modal.action}
                  onDone={refresh}
                />
              )}
              {modal.kind === "convert" && (
                <ConvertForm item={modal.item} onDone={refresh} />
              )}
            </section>
          </div>,
          document.body,
        )}
    </section>
  );
}

function PolicyForm({
  suggestion,
  onDone,
}: {
  suggestion?: ReplenishmentSuggestion;
  onDone: () => void;
}) {
  const [options, setOptions] = React.useState<ReplenishmentOptions | null>(
    null,
  );
  const [inventoryKey, setInventoryKey] = React.useState(
    suggestion ? `${suggestion.warehouseId}:${suggestion.skuId}` : "",
  );
  const [supplier, setSupplier] = React.useState(
    suggestion?.preferredSupplierId ?? "",
  );
  const [safety, setSafety] = React.useState(
    fixedDecimal(suggestion?.safetyStock ?? "5"),
  );
  const [reorder, setReorder] = React.useState(
    fixedDecimal(suggestion?.reorderPoint ?? "10"),
  );
  const [target, setTarget] = React.useState(
    fixedDecimal(suggestion?.targetStock ?? "30"),
  );
  const [minimum, setMinimum] = React.useState(
    fixedDecimal(suggestion?.minimumOrderQuantity ?? "1"),
  );
  const [multiple, setMultiple] = React.useState(
    fixedDecimal(suggestion?.orderMultiple ?? "1"),
  );
  const [leadTime, setLeadTime] = React.useState(
    String(suggestion?.leadTimeDays ?? 7),
  );
  const [busy, setBusy] = React.useState(false);
  const [notice, setNotice] = React.useState("");
  React.useEffect(() => {
    request<ReplenishmentOptions>("/api/v1/replenishment-options")
      .then(setOptions)
      .catch((error: Error) => setNotice(error.message));
  }, []);
  const inventory = options?.inventory.find(
    (item) => `${item.warehouseId}:${item.skuId}` === inventoryKey,
  );
  return (
    <form
      className="replenishment-form"
      onSubmit={async (event) => {
        event.preventDefault();
        if (!inventory) return;
        setBusy(true);
        setNotice("");
        try {
          await request("/api/v1/replenishment-policies", {
            method: "POST",
            body: JSON.stringify({
              legalEntityId: inventory.legalEntityId,
              warehouseId: inventory.warehouseId,
              skuId: inventory.skuId,
              preferredSupplierId: supplier,
              unitOfMeasureId: inventory.unitOfMeasureId,
              safetyStock: safety,
              reorderPoint: reorder,
              targetStock: target,
              minimumOrderQuantity: minimum,
              orderMultiple: multiple,
              leadTimeDays: Number(leadTime),
              status: "active",
              expectedVersion: suggestion?.version,
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
      <p className="replenishment-callout">
        <strong>库存水位策略</strong>
        可用量触及订货点时，系统按目标库存、最小起订量和包装倍数计算建议。
      </p>
      <label>
        <span>仓库商品</span>
        <select
          required
          value={inventoryKey}
          disabled={Boolean(suggestion)}
          onChange={(event) => setInventoryKey(event.target.value)}
        >
          <option value="">请选择</option>
          {options?.inventory.map((item) => (
            <option
              key={`${item.warehouseId}:${item.skuId}`}
              value={`${item.warehouseId}:${item.skuId}`}
            >
              {item.warehouseCode} · {item.skuCode} · 可用{" "}
              {formatQuantity(item.availableQuantity)}
            </option>
          ))}
        </select>
      </label>
      <label>
        <span>首选供应商</span>
        <select
          required
          value={supplier}
          onChange={(event) => setSupplier(event.target.value)}
        >
          <option value="">请选择</option>
          {options?.suppliers.map((item) => (
            <option key={item.id} value={item.id}>
              {item.code} · {item.name}
            </option>
          ))}
        </select>
      </label>
      <div className="replenishment-form-grid">
        <NumberField label="安全库存" value={safety} onChange={setSafety} />
        <NumberField label="订货点" value={reorder} onChange={setReorder} />
        <NumberField label="目标库存" value={target} onChange={setTarget} />
        <NumberField label="最小起订量" value={minimum} onChange={setMinimum} />
        <NumberField label="订货倍数" value={multiple} onChange={setMultiple} />
        <NumberField
          label="交期（天）"
          value={leadTime}
          onChange={setLeadTime}
        />
      </div>
      {notice && <p className="replenishment-notice">{notice}</p>}
      <button type="submit" disabled={!inventory || !supplier || busy}>
        {busy ? "正在保存…" : suggestion ? "保存策略" : "建立库存策略"}
      </button>
    </form>
  );
}

function RequisitionForm({
  items,
  onDone,
}: {
  items: ReplenishmentSuggestion[];
  onDone: () => void;
}) {
  const [note, setNote] = React.useState("系统补货建议转采购需求");
  const [busy, setBusy] = React.useState(false);
  const [notice, setNotice] = React.useState("");
  return (
    <form
      className="replenishment-form"
      onSubmit={async (event) => {
        event.preventDefault();
        setBusy(true);
        setNotice("");
        try {
          await request("/api/v1/purchase-requisitions", {
            method: "POST",
            body: JSON.stringify({
              policyIds: items.map((item) => item.id),
              requestDate: today(),
              businessNote: note.trim() || undefined,
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
      <p className="replenishment-callout">
        <strong>
          {items[0]?.supplierCode} · {items[0]?.warehouseCode}
        </strong>
        {items.length} 个商品将按当前实时建议量形成同一张采购需求单。
      </p>
      <div className="requisition-preview">
        {items.map((item) => (
          <div key={item.id}>
            <strong>{item.skuCode}</strong>
            <span>可用 {formatQuantity(item.availableQuantity)}</span>
            <span>建议 {formatQuantity(item.suggestedQuantity)}</span>
          </div>
        ))}
      </div>
      <label>
        <span>需求说明</span>
        <textarea
          value={note}
          maxLength={1000}
          onChange={(event) => setNote(event.target.value)}
        />
      </label>
      {notice && <p className="replenishment-notice">{notice}</p>}
      <button type="submit" disabled={busy}>
        {busy ? "正在生成…" : "生成采购需求单"}
      </button>
    </form>
  );
}

function RequisitionCommand({
  item,
  action,
  onDone,
}: {
  item: PurchaseRequisitionSummary;
  action: "confirm" | "cancel";
  onDone: () => void;
}) {
  const [busy, setBusy] = React.useState(false);
  const [notice, setNotice] = React.useState("");
  const confirm = action === "confirm";
  return (
    <div className="replenishment-command">
      <p className={`replenishment-callout ${confirm ? "" : "danger"}`}>
        <strong>{confirm ? "确认采购需求" : "取消采购需求"}</strong>
        {confirm
          ? "确认后建议量继续计入补货覆盖，供采购计划转单使用。"
          : "取消后释放需求覆盖，系统将重新计算缺货和建议量。"}
      </p>
      {notice && <p className="replenishment-notice">{notice}</p>}
      <button
        type="button"
        className={confirm ? "" : "danger"}
        disabled={busy}
        onClick={async () => {
          setBusy(true);
          setNotice("");
          try {
            await request(
              `/api/v1/purchase-requisitions/${item.id}/${action}`,
              {
                method: "POST",
                body: JSON.stringify({ expectedVersion: item.version }),
              },
            );
            onDone();
          } catch (error) {
            setNotice((error as Error).message);
          } finally {
            setBusy(false);
          }
        }}
      >
        {busy ? "正在处理…" : confirm ? "确认进入采购计划" : "确认取消"}
      </button>
    </div>
  );
}

function ConvertForm({
  item,
  onDone,
}: {
  item: PurchaseRequisitionSummary;
  onDone: () => void;
}) {
  const [orders, setOrders] = React.useState<PurchaseOrder[]>([]);
  const [orderId, setOrderId] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [notice, setNotice] = React.useState("");
  React.useEffect(() => {
    request<Envelope<PurchaseOrder>>("/api/v1/purchase-orders?limit=200")
      .then((result) =>
        setOrders(
          result.items.filter(
            (order) =>
              order.legalEntityId === item.legalEntityId &&
              order.supplierId === item.supplierId &&
              ["draft", "confirmed"].includes(order.lifecycleStatus),
          ),
        ),
      )
      .catch((error: Error) => setNotice(error.message));
  }, [item.legalEntityId, item.supplierId]);
  return (
    <form
      className="replenishment-form"
      onSubmit={async (event) => {
        event.preventDefault();
        setBusy(true);
        setNotice("");
        try {
          await request(`/api/v1/purchase-requisitions/${item.id}/convert`, {
            method: "POST",
            body: JSON.stringify({
              expectedVersion: item.version,
              purchaseOrderId: orderId,
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
      <p className="replenishment-callout">
        <strong>采购需求闭环</strong>
        采购订单必须属于同一供应商，并完整覆盖需求单的商品、仓库与数量。
      </p>
      <label>
        <span>采购订单</span>
        <select
          required
          value={orderId}
          onChange={(event) => setOrderId(event.target.value)}
        >
          <option value="">请选择</option>
          {orders.map((order) => (
            <option key={order.id} value={order.id}>
              {order.purchaseOrderNumber} · {order.lifecycleStatus} ·{" "}
              {formatMoney(order.currency, order.grossAmount)}
            </option>
          ))}
        </select>
      </label>
      {orders.length === 0 && (
        <p className="replenishment-notice">
          暂无匹配采购订单，请先在采购订单页面创建对应商品与数量。
        </p>
      )}
      {notice && <p className="replenishment-notice">{notice}</p>}
      <button type="submit" disabled={!orderId || busy}>
        {busy ? "正在关联…" : "确认关联并关闭需求"}
      </button>
    </form>
  );
}

function StockLevel({ item }: { item: ReplenishmentSuggestion }) {
  const maximum = Math.max(number(item.targetStock), 1);
  const available = percent(number(item.availableQuantity) / maximum);
  const inbound = percent(number(item.inboundQuantity) / maximum);
  const planned = percent(number(item.openRequisitionQuantity) / maximum);
  return (
    <div className="stock-level">
      <div>
        <i className="available" style={{ width: `${available}%` }} />
        <i
          className="inbound"
          style={{ left: `${available}%`, width: `${inbound}%` }}
        />
        <i
          className="planned"
          style={{ left: `${available + inbound}%`, width: `${planned}%` }}
        />
        <b
          className="safety"
          style={{ left: `${percent(number(item.safetyStock) / maximum)}%` }}
        />
        <b
          className="reorder"
          style={{ left: `${percent(number(item.reorderPoint) / maximum)}%` }}
        />
      </div>
      <small>
        可用 {formatQuantity(item.availableQuantity)} / 目标{" "}
        {formatQuantity(item.targetStock)}
      </small>
    </div>
  );
}

function NumberField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <label>
      <span>{label}</span>
      <input
        inputMode="decimal"
        value={value}
        required
        onChange={(event) => onChange(event.target.value)}
      />
    </label>
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
function Risk({ value }: { value: string }) {
  const labels: Record<string, string> = {
    critical: "缺货风险",
    warning: "接近订货点",
    inbound_covered: "在途覆盖",
    requisition_open: "需求已建",
    healthy: "库存健康",
    paused: "策略暂停",
    draft: "待确认",
    confirmed: "已确认",
    converted: "已转采购",
    cancelled: "已取消",
  };
  return (
    <span className={`replenishment-risk ${value}`}>
      {labels[value] ?? value}
    </span>
  );
}
function modalTitle(modal: Modal) {
  if (modal.kind === "policy")
    return modal.suggestion ? "调整补货策略" : "建立补货策略";
  if (modal.kind === "requisition") return "生成采购需求单";
  if (modal.kind === "convert")
    return `${modal.item.requisitionNumber} · 关联采购订单`;
  return `${modal.item.requisitionNumber} · ${modal.action === "confirm" ? "确认" : "取消"}`;
}
function number(value: string) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : 0;
}
function percent(value: number) {
  return Math.max(0, Math.min(value * 100, 100));
}
function today() {
  return new Date().toISOString().slice(0, 10);
}
