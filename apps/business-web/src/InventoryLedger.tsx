import React from "react";
import {
  type Envelope,
  type InventoryBalance,
  type InventoryMovement,
  type InventoryOpening,
  request,
} from "./api";
import { InventoryCountPanel } from "./InventoryCountPanel";
import { ReplenishmentPanel } from "./ReplenishmentPanel";
import {
  formatAmount,
  formatQuantity,
  formatSignedQuantity,
} from "./formatters";
import "./inventory-ledger.css";

type InventoryData = {
  balances: Envelope<InventoryBalance>;
  movements: Envelope<InventoryMovement>;
  openings: Envelope<InventoryOpening>;
};

type LedgerView = "balances" | "movements" | "operations" | "replenishment";
type StockFilter = "all" | "available" | "locked" | "zero";

const VIEWS: Array<{ id: LedgerView; label: string; note: string }> = [
  { id: "balances", label: "库存余额", note: "在手与可用" },
  { id: "movements", label: "库存流水", note: "不可变记录" },
  { id: "operations", label: "期初与盘点", note: "库存校准" },
  { id: "replenishment", label: "补货建议", note: "采购触发" },
];

export function InventoryLedger({ skuId }: { skuId?: string }) {
  const [revision, setRevision] = React.useState(0);
  const [view, setView] = React.useState<LedgerView>("balances");
  const [keyword, setKeyword] = React.useState(skuId ?? "");
  const [warehouse, setWarehouse] = React.useState("all");
  const [stockFilter, setStockFilter] = React.useState<StockFilter>("all");
  const state = useInventoryData(skuId, revision);
  const balances = state.data?.balances.items ?? [];
  const movements = state.data?.movements.items ?? [];
  const openings = state.data?.openings.items ?? [];
  const normalizedKeyword = keyword.trim().toLowerCase();
  const visibleBalances = balances.filter((item) => {
    const matchesKeyword =
      !normalizedKeyword ||
      item.skuId.toLowerCase().includes(normalizedKeyword) ||
      item.warehouseId.toLowerCase().includes(normalizedKeyword);
    const matchesWarehouse =
      warehouse === "all" || item.warehouseId === warehouse;
    const available = Number(item.availableQuantity);
    const locked =
      Number(item.reservedQuantity) + Number(item.quarantinedQuantity);
    const matchesStock =
      stockFilter === "all" ||
      (stockFilter === "available" && available > 0) ||
      (stockFilter === "locked" && locked > 0) ||
      (stockFilter === "zero" && available <= 0);
    return matchesKeyword && matchesWarehouse && matchesStock;
  });
  const visibleMovements = movements.filter(
    (item) =>
      (warehouse === "all" || item.warehouseId === warehouse) &&
      (!normalizedKeyword ||
        item.skuId.toLowerCase().includes(normalizedKeyword) ||
        item.movementType.toLowerCase().includes(normalizedKeyword)),
  );
  const totals = balances.reduce(
    (result, item) => ({
      onHand: result.onHand + Number(item.onHandQuantity),
      reserved: result.reserved + Number(item.reservedQuantity),
      quarantined: result.quarantined + Number(item.quarantinedQuantity),
      available: result.available + Number(item.availableQuantity),
      value: result.value + Number(item.inventoryValue),
    }),
    { onHand: 0, reserved: 0, quarantined: 0, available: 0, value: 0 },
  );
  const warehouses = Array.from(
    new Set(balances.map((item) => item.warehouseId)),
  );
  const refresh = () => setRevision((value) => value + 1);

  return (
    <section className="page inventory-ledger-page">
      <div className="page-head inventory-ledger-head">
        <div>
          <p>仓位控制台 / Inventory control</p>
          <h1>库存台账</h1>
          <span>
            从库存公式进入每个仓库与商品的余额、锁定量和不可变流水；隔离库存完成质检前不可销售。
          </span>
        </div>
        <button type="button" onClick={refresh}>
          刷新台账
        </button>
      </div>

      <InventoryEquation totals={totals} />

      <nav className="inventory-ledger-tabs">
        {VIEWS.map((item) => (
          <button
            type="button"
            className={view === item.id ? "active" : ""}
            aria-pressed={view === item.id}
            onClick={() => setView(item.id)}
            key={item.id}
          >
            <b>{item.label}</b>
            <span>{item.note}</span>
          </button>
        ))}
      </nav>

      {state.error && (
        <div className="inventory-ledger-error">
          <strong>无法读取库存台账</strong>
          <span>{state.error}</span>
        </div>
      )}
      {state.loading && !state.data && (
        <div className="inventory-ledger-loading">正在核对库存权威账簿…</div>
      )}

      {state.data && view === "balances" && (
        <>
          <InventoryFilters
            keyword={keyword}
            warehouse={warehouse}
            stockFilter={stockFilter}
            warehouses={warehouses}
            resultCount={visibleBalances.length}
            onKeyword={setKeyword}
            onWarehouse={setWarehouse}
            onStockFilter={setStockFilter}
          />
          <BalanceTable items={visibleBalances} />
        </>
      )}

      {state.data && view === "movements" && (
        <>
          <InventoryFilters
            keyword={keyword}
            warehouse={warehouse}
            stockFilter="all"
            warehouses={warehouses}
            resultCount={visibleMovements.length}
            onKeyword={setKeyword}
            onWarehouse={setWarehouse}
            onStockFilter={setStockFilter}
            movements
          />
          <MovementTable items={visibleMovements} />
        </>
      )}

      {state.data && view === "operations" && (
        <div className="inventory-operations">
          <OpeningConsole onDone={refresh} />
          <OpeningRegister items={openings} onDone={refresh} />
          <InventoryCountPanel onChanged={refresh} />
        </div>
      )}

      {state.data && view === "replenishment" && (
        <ReplenishmentPanel onChanged={refresh} />
      )}
    </section>
  );
}

function InventoryEquation({
  totals,
}: {
  totals: {
    onHand: number;
    reserved: number;
    quarantined: number;
    available: number;
    value: number;
  };
}) {
  return (
    <section className="inventory-equation">
      <div className="inventory-equation-item on-hand">
        <span>在手库存</span>
        <strong>{formatQuantity(totals.onHand)}</strong>
        <small>全部仓库账面数量</small>
      </div>
      <i>−</i>
      <div className="inventory-equation-item reserved">
        <span>销售预占</span>
        <strong>{formatQuantity(totals.reserved)}</strong>
        <small>已确认订单锁定</small>
      </div>
      <i>−</i>
      <div className="inventory-equation-item quarantined">
        <span>退货隔离</span>
        <strong>{formatQuantity(totals.quarantined)}</strong>
        <small>等待质检处置</small>
      </div>
      <i>=</i>
      <div className="inventory-equation-item available">
        <span>可用库存</span>
        <strong>{formatQuantity(totals.available)}</strong>
        <small>当前可承诺数量</small>
      </div>
      <div className="inventory-equation-value">
        <span>库存账面值</span>
        <strong>{formatAmount(totals.value)}</strong>
        <small>按移动平均成本汇总</small>
      </div>
    </section>
  );
}

function InventoryFilters({
  keyword,
  warehouse,
  stockFilter,
  warehouses,
  resultCount,
  onKeyword,
  onWarehouse,
  onStockFilter,
  movements = false,
}: {
  keyword: string;
  warehouse: string;
  stockFilter: StockFilter;
  warehouses: string[];
  resultCount: number;
  onKeyword: (value: string) => void;
  onWarehouse: (value: string) => void;
  onStockFilter: (value: StockFilter) => void;
  movements?: boolean;
}) {
  return (
    <div className="inventory-ledger-filters">
      <label className="inventory-search">
        <span>{movements ? "搜索商品或流水类型" : "搜索商品或仓库"}</span>
        <input
          value={keyword}
          placeholder={
            movements ? "输入 SKU 或流水类型" : "输入 SKU 或仓库编号"
          }
          onChange={(event) => onKeyword(event.target.value)}
        />
      </label>
      <label>
        <span>仓库范围</span>
        <select
          value={warehouse}
          onChange={(event) => onWarehouse(event.target.value)}
        >
          <option value="all">全部仓库</option>
          {warehouses.map((id) => (
            <option value={id} key={id}>
              {shortId(id)}
            </option>
          ))}
        </select>
      </label>
      {!movements && (
        <label>
          <span>库存状态</span>
          <select
            value={stockFilter}
            onChange={(event) =>
              onStockFilter(event.target.value as StockFilter)
            }
          >
            <option value="all">全部状态</option>
            <option value="available">有可用库存</option>
            <option value="locked">存在锁定量</option>
            <option value="zero">可用量为零</option>
          </select>
        </label>
      )}
      <div className="inventory-result-count">
        <b>{resultCount}</b>
        <span>{movements ? "条流水" : "个仓库商品组合"}</span>
      </div>
    </div>
  );
}

function BalanceTable({ items }: { items: InventoryBalance[] }) {
  if (items.length === 0)
    return (
      <InventoryEmpty text="当前筛选下没有库存余额。可清除筛选，或在“期初与盘点”中建立库存期初。" />
    );
  return (
    <div className="inventory-table-wrap">
      <table className="inventory-table balance-table">
        <thead>
          <tr>
            <th>商品 / 仓库</th>
            <th>在手</th>
            <th>预占</th>
            <th>隔离</th>
            <th>可用</th>
            <th>移动均价</th>
            <th>库存价值</th>
            <th>更新时间</th>
          </tr>
        </thead>
        <tbody>
          {items.map((item) => {
            const locked =
              Number(item.reservedQuantity) + Number(item.quarantinedQuantity);
            return (
              <tr
                className={Number(item.availableQuantity) <= 0 ? "zero" : ""}
                key={`${item.warehouseId}:${item.skuId}`}
              >
                <td data-label="商品 / 仓库">
                  <code title={item.skuId}>{shortId(item.skuId)}</code>
                  <small title={item.warehouseId}>
                    仓库 {shortId(item.warehouseId)} · v{item.version}
                  </small>
                </td>
                <td data-label="在手">{formatQuantity(item.onHandQuantity)}</td>
                <td data-label="预占" className={locked > 0 ? "locked" : ""}>
                  {formatQuantity(item.reservedQuantity)}
                </td>
                <td
                  data-label="隔离"
                  className={
                    Number(item.quarantinedQuantity) > 0 ? "quarantine" : ""
                  }
                >
                  {formatQuantity(item.quarantinedQuantity)}
                </td>
                <td data-label="可用" className="available">
                  <strong>{formatQuantity(item.availableQuantity)}</strong>
                </td>
                <td data-label="移动均价">
                  {item.averageUnitCost === null
                    ? "—"
                    : formatAmount(item.averageUnitCost)}
                </td>
                <td data-label="库存价值">{formatAmount(item.inventoryValue)}</td>
                <td data-label="更新时间">{formatInstant(item.updatedAt)}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function MovementTable({ items }: { items: InventoryMovement[] }) {
  if (items.length === 0)
    return <InventoryEmpty text="当前筛选下没有库存流水。" />;
  return (
    <div className="inventory-table-wrap">
      <table className="inventory-table movement-table">
        <thead>
          <tr>
            <th>业务日期</th>
            <th>流水类型</th>
            <th>商品 / 仓库</th>
            <th>数量</th>
            <th>单位成本</th>
            <th>成本金额</th>
            <th>过账时间</th>
          </tr>
        </thead>
        <tbody>
          {items.map((item) => (
            <tr key={item.id}>
              <td data-label="业务日期">{item.businessDate}</td>
              <td data-label="流水类型">
                <span
                  className={`movement-kind ${movementDirection(item.quantity)}`}
                >
                  {movementLabel(item.movementType)}
                </span>
              </td>
              <td data-label="商品 / 仓库">
                <code title={item.skuId}>{shortId(item.skuId)}</code>
                <small title={item.warehouseId}>
                  仓库 {shortId(item.warehouseId)}
                </small>
              </td>
              <td
                data-label="数量"
                className={movementDirection(item.quantity)}
              >
                <strong>{formatSignedQuantity(item.quantity)}</strong>
              </td>
              <td data-label="单位成本">{formatAmount(item.unitCost)}</td>
              <td data-label="成本金额">{formatAmount(item.totalCost)}</td>
              <td data-label="过账时间">{formatInstant(item.postedAt)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function OpeningRegister({
  items,
  onDone,
}: {
  items: InventoryOpening[];
  onDone: () => void;
}) {
  return (
    <section className="opening-register">
      <header>
        <div>
          <h2>期初批次</h2>
          <p>期初过账后进入库存权威流水；修正必须显式冲销。</p>
        </div>
        <span>{items.length} 个批次</span>
      </header>
      {items.length === 0 ? (
        <InventoryEmpty text="尚无库存期初批次。" />
      ) : (
        <div className="opening-list">
          {items.map((item) => (
            <article key={item.id}>
              <div>
                <code>{item.batchNumber}</code>
                <small>
                  {item.businessDate} · {item.currency} · v{item.version}
                </small>
              </div>
              <span className={`inventory-state ${item.status}`}>
                {statusLabel(item.status)}
              </span>
              <div className="opening-actions">
                {item.status === "draft" && (
                  <InventoryCommandButton
                    label="过账"
                    path={`/api/v1/inventory-openings/${item.id}/post`}
                    body={{ expectedVersion: item.version }}
                    onDone={onDone}
                  />
                )}
                {item.status === "posted" && (
                  <InventoryCommandButton
                    label="冲销"
                    path={`/api/v1/inventory-openings/${item.id}/reverse`}
                    body={{
                      expectedVersion: item.version,
                      reasonCode: "OPENING_CORRECTION",
                    }}
                    onDone={onDone}
                  />
                )}
              </div>
            </article>
          ))}
        </div>
      )}
    </section>
  );
}

function OpeningConsole({ onDone }: { onDone: () => void }) {
  const sample = JSON.stringify(
    {
      legalEntityId: "",
      businessDate: new Date().toISOString().slice(0, 10),
      currency: "CNY",
      lines: [{ warehouseId: "", skuId: "", quantity: "1", unitCost: "0" }],
    },
    null,
    2,
  );
  const [body, setBody] = React.useState(sample);
  const [result, setResult] = React.useState("");
  return (
    <details className="inventory-opening-console">
      <summary>
        <div>
          <b>建立库存期初</b>
          <span>高级安全命令</span>
        </div>
        <small>展开录入</small>
      </summary>
      <form
        onSubmit={async (event) => {
          event.preventDefault();
          try {
            const output = await request("/api/v1/inventory-openings", {
              method: "POST",
              body: JSON.stringify(JSON.parse(body)),
            });
            setResult(JSON.stringify(output, null, 2));
            onDone();
          } catch (error) {
            setResult((error as Error).message);
          }
        }}
      >
        <textarea
          aria-label="库存期初 JSON"
          value={body}
          spellCheck={false}
          onChange={(event) => setBody(event.target.value)}
        />
        <button type="submit">校验并建立草稿</button>
        {result && <pre>{result}</pre>}
      </form>
    </details>
  );
}

function InventoryCommandButton({
  label,
  path,
  body,
  onDone,
}: {
  label: string;
  path: string;
  body: unknown;
  onDone: () => void;
}) {
  const [busy, setBusy] = React.useState(false);
  return (
    <button
      type="button"
      disabled={busy}
      onClick={async () => {
        setBusy(true);
        try {
          await request(path, { method: "POST", body: JSON.stringify(body) });
          onDone();
        } catch (error) {
          alert((error as Error).message);
        } finally {
          setBusy(false);
        }
      }}
    >
      {busy ? "处理中…" : label}
    </button>
  );
}

function InventoryEmpty({ text }: { text: string }) {
  return (
    <div className="inventory-empty">
      <span>空</span>
      <p>{text}</p>
    </div>
  );
}

function useInventoryData(skuId: string | undefined, revision: number) {
  const [state, setState] = React.useState<{
    data: InventoryData | null;
    error: string | null;
    loading: boolean;
  }>({ data: null, error: null, loading: true });
  React.useEffect(() => {
    void revision;
    let active = true;
    const query = skuId ? `?skuId=${encodeURIComponent(skuId)}` : "?limit=200";
    setState((current) => ({ ...current, error: null, loading: true }));
    Promise.all([
      request<Envelope<InventoryBalance>>(`/api/v1/inventory-balances${query}`),
      request<Envelope<InventoryMovement>>(
        `/api/v1/inventory-movements${query}`,
      ),
      request<Envelope<InventoryOpening>>(
        "/api/v1/inventory-openings?limit=100",
      ),
    ])
      .then(([balances, movements, openings]) => {
        if (active)
          setState({
            data: { balances, movements, openings },
            error: null,
            loading: false,
          });
      })
      .catch((error: Error) => {
        if (active)
          setState({ data: null, error: error.message, loading: false });
      });
    return () => {
      active = false;
    };
  }, [skuId, revision]);
  return state;
}

function movementLabel(value: string) {
  return (
    (
      {
        inventory_opening: "库存期初",
        inventory_opening_reversal: "期初冲销",
        purchase_receipt: "采购入库",
        purchase_receipt_reversal: "入库冲销",
        sales_shipment: "销售出库",
        sales_shipment_reversal: "出库冲销",
        sales_return: "销售退货",
        purchase_return: "采购退货",
        inventory_count_gain: "盘盈",
        inventory_count_loss: "盘亏",
      } as Record<string, string>
    )[value] ?? value.replaceAll("_", " ")
  );
}

function movementDirection(quantity: string) {
  return Number(quantity) < 0 ? "outbound" : "inbound";
}

function statusLabel(value: string) {
  return (
    (
      { draft: "草稿", posted: "已过账", reversed: "已冲销" } as Record<
        string,
        string
      >
    )[value] ?? value
  );
}

function shortId(value: string) {
  return value.length > 16 ? `${value.slice(0, 8)}…${value.slice(-4)}` : value;
}

function formatInstant(value: string) {
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).format(new Date(value));
}
