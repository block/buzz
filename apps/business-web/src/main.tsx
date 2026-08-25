import React from "react";
import { createRoot } from "react-dom/client";
import {
  type AgentQueryRun,
  type AgentQueryRunList,
  type Envelope,
  type GoodsReceipt,
  type DataQuality,
  type ManagementReport,
  type ManagementSnapshot,
  type OrderProfit,
  type OperationsDashboard,
  type OperatingAlert,
  type OperatingIncident,
  type OperatingIncidentQueue,
  type Payable,
  type ProfitAdjustment,
  type ProfitabilityRow,
  type PurchaseOrder,
  type Receipt,
  type ReadDiagnostics,
  type Receivable,
  type ReportRun,
  type SalesOrder,
  type Shipment,
  type SupplierPayment,
  request,
} from "./api";
import { connectBusinessDockAuthBridge } from "./businessDockBridge";
import {
  BusinessEntryPage,
  type BusinessEntryKind,
} from "./BusinessEntryPage";
import { formatAmount, formatMoney } from "./formatters";
import { InventoryLedger } from "./InventoryLedger";
import { CoreMasterDataCenter } from "./CoreMasterDataCenter";
import { ProductMasterDataCenter } from "./ProductMasterDataCenter";
import { NumberingRulesCenter } from "./NumberingRulesCenter";
import { PAGE_ZOOM_STEPS, usePageZoom } from "./pageZoom";
import { OperatingTrendsView } from "./OperatingTrends";
import { GoodsReceiptConfirmation } from "./GoodsReceiptConfirmation";
import { GoodsReceiptEntry } from "./GoodsReceiptEntry";
import { PurchaseOrderEntry } from "./PurchaseOrderEntry";
import { PurchaseOrderConfirmation } from "./PurchaseOrderConfirmation";
import { SalesOrderEntry } from "./SalesOrderEntry";
import { SalesOrderConfirmation } from "./SalesOrderConfirmation";
import { ShipmentEntry } from "./ShipmentEntry";
import { ShipmentConfirmation } from "./ShipmentConfirmation";
import {
  PurchaseOrderWorkflowPage,
  SalesOrderWorkflowPage,
} from "./OrderWorkflowPages";
import "./styles.css";

type Section =
  | "agentQuery"
  | "dashboard"
  | "quality"
  | "incidents"
  | "trends"
  | "coreData"
  | "productData"
  | "numbering"
  | "sales"
  | "shipments"
  | "inventory"
  | "receivables"
  | "receipts"
  | "purchasing"
  | "goodsReceipts"
  | "payables"
  | "supplierPayments"
  | "profits"
  | "profitability"
  | "adjustments"
  | "reports";
type LoadState<T> = { data: T; error: string | null; loading: boolean };

type NavItem = { id: Section; label: string; index: string };

const NAV_GROUPS: Array<{
  id: string;
  label: string;
  index: string;
  items: NavItem[];
}> = [
  {
    id: "control",
    label: "经营控制",
    index: "01",
    items: [
      { id: "dashboard", label: "经营驾驶舱", index: "OPS" },
      { id: "quality", label: "数据质量", index: "DQ" },
      { id: "incidents", label: "异常处置", index: "INC" },
      { id: "trends", label: "日报与趋势", index: "TRD" },
    ],
  },
  {
    id: "master-data",
    label: "基础资料",
    index: "02",
    items: [
      { id: "coreData", label: "核心数据", index: "MDM" },
      { id: "productData", label: "商品数据", index: "PDM" },
      { id: "numbering", label: "编码规则", index: "NUM" },
    ],
  },
  {
    id: "workflows",
    label: "业务闭环",
    index: "03",
    items: [
      { id: "sales", label: "销售订单闭环", index: "O2C" },
      { id: "inventory", label: "库存台账", index: "INV" },
      { id: "purchasing", label: "采购订单闭环", index: "P2P" },
    ],
  },
  {
    id: "analysis",
    label: "经营分析",
    index: "04",
    items: [
      { id: "profits", label: "订单真实利润", index: "P&L" },
      { id: "profitability", label: "多维盈利分析", index: "DIM" },
      { id: "adjustments", label: "经营费用归集", index: "ADJ" },
      { id: "reports", label: "管理利润报表", index: "RPT" },
    ],
  },
];
const NAV = NAV_GROUPS.flatMap((group) => group.items);
const NAVIGATION_COLLAPSED_STORAGE_KEY =
  "bizfin.business.navigationCollapsed";

function savedNavigationCollapsed() {
  try {
    return (
      window.localStorage.getItem(NAVIGATION_COLLAPSED_STORAGE_KEY) === "true"
    );
  } catch {
    return false;
  }
}

const WORKFLOW_NAV_ALIASES: Partial<Record<Section, Section>> = {
  shipments: "sales",
  receivables: "sales",
  receipts: "sales",
  goodsReceipts: "purchasing",
  payables: "purchasing",
  supplierPayments: "purchasing",
};

function route(): {
  section: Section;
  id?: string;
  entry?: BusinessEntryKind;
  embed: boolean;
} {
  const path = window.location.pathname;
  const embed = path.startsWith("/embed/");
  const clean = path.replace(/^\/embed/, "");
  const entry = clean.match(
    /^\/entries\/(sales-order|shipment|purchase-order|goods-receipt|customer-receipt|supplier-payment)$/,
  )?.[1] as BusinessEntryKind | undefined;
  if (entry) {
    const section: Section = [
      "sales-order",
      "shipment",
      "customer-receipt",
    ].includes(entry)
      ? "sales"
      : "purchasing";
    return { section, entry, embed };
  }
  if (clean === "/operations-dashboard") return { section: "dashboard", embed };
  if (clean === "/data-quality") return { section: "quality", embed };
  if (clean === "/operating-incidents") return { section: "incidents", embed };
  if (clean === "/operating-trends") return { section: "trends", embed };
  const agentQuery = clean.match(/^\/agent-queries\/([^/]+)$/);
  if (agentQuery)
    return {
      section: "agentQuery",
      id: decodeURIComponent(agentQuery[1]),
      embed,
    };
  if (clean === "/core-data") return { section: "coreData", embed };
  if (clean === "/product-data") return { section: "productData", embed };
  const patterns: Array<[Section, RegExp]> = [
    ["sales", /^\/(?:sales-orders|sales\/orders)\/([^/]+)$/],
    ["shipments", /^\/shipments\/([^/]+)$/],
    ["inventory", /^\/inventory\/([^/]+)$/],
    ["receivables", /^\/receivables\/(?:customer\/)?([^/]+)$/],
    ["receipts", /^\/customer-receipts\/([^/]+)$/],
    ["purchasing", /^\/purchase-orders\/([^/]+)$/],
    ["goodsReceipts", /^\/goods-receipts\/([^/]+)$/],
    ["payables", /^\/payables\/supplier\/([^/]+)$/],
    ["supplierPayments", /^\/supplier-payments\/([^/]+)$/],
    ["profits", /^\/order-profits\/([^/]+)$/],
    ["adjustments", /^\/profit-adjustments\/([^/]+)$/],
    ["reports", /^\/management-reports\/([^/]+)$/],
    [
      "profitability",
      /^\/profitability\/(?:customer|sku|brand|salesperson)\/([^/]+)\/period\/\d{4}-\d{2}$/,
    ],
  ];
  for (const [section, pattern] of patterns) {
    const match = clean.match(pattern);
    if (match) return { section, id: decodeURIComponent(match[1]), embed };
  }
  const fromHash = window.location.hash.slice(1) as Section;
  return {
    section: WORKFLOW_NAV_ALIASES[fromHash]
      ? WORKFLOW_NAV_ALIASES[fromHash]
      : NAV.some((item) => item.id === fromHash)
        ? fromHash
        : "dashboard",
    embed,
  };
}

function useLoad<T>(
  loader: () => Promise<T>,
  deps: React.DependencyList,
): LoadState<T | null> {
  const [state, setState] = React.useState<LoadState<T | null>>({
    data: null,
    error: null,
    loading: true,
  });
  React.useEffect(() => {
    let active = true;
    setState((current) => ({ ...current, loading: true, error: null }));
    loader()
      .then((data) => active && setState({ data, error: null, loading: false }))
      .catch(
        (error: Error) =>
          active &&
          setState({ data: null, error: error.message, loading: false }),
      );
    return () => {
      active = false;
    };
    // biome-ignore lint/correctness/useExhaustiveDependencies: callers provide the explicit reload key list.
  }, deps);
  return state;
}

function App() {
  const [current, setCurrent] = React.useState(route);
  const [navigationCollapsed, setNavigationCollapsed] = React.useState(
    savedNavigationCollapsed,
  );
  const pageZoom = usePageZoom();
  const activeNavigation =
    WORKFLOW_NAV_ALIASES[current.section] ?? current.section;
  React.useEffect(() => {
    const update = () => setCurrent(route());
    window.addEventListener("hashchange", update);
    window.addEventListener("popstate", update);
    return () => {
      window.removeEventListener("hashchange", update);
      window.removeEventListener("popstate", update);
    };
  }, []);
  React.useEffect(() => {
    try {
      window.localStorage.setItem(
        NAVIGATION_COLLAPSED_STORAGE_KEY,
        String(navigationCollapsed),
      );
    } catch {
      // Storage can be unavailable in privacy-restricted embedded webviews.
    }
  }, [navigationCollapsed]);
  React.useEffect(() => {
    if (window.parent === window) return;
    let parentOrigin: string;
    try {
      parentOrigin = new URL(document.referrer).origin;
      if (!/^https?:$/.test(new URL(parentOrigin).protocol)) return;
    } catch {
      return;
    }
    window.parent.postMessage(
      { version: 1, type: "BUSINESS_READY" },
      parentOrigin,
    );
    window.parent.postMessage(
      {
        version: 1,
        type: "ROUTE_CHANGED",
        payload: { url: window.location.href },
      },
      parentOrigin,
    );
  }, []);
  return (
    <div
      className={`${current.embed ? "app embed" : "app"}${navigationCollapsed ? " navigation-collapsed" : ""}`}
    >
      {!current.embed && !navigationCollapsed && (
        <aside className="rail" id="business-navigation">
          <div className="brand">
            <div>
              <strong>企业工作台</strong>
              <small>Business Core · S1</small>
            </div>
            <button
              type="button"
              className="rail-collapse"
              aria-label="隐藏导航栏"
              aria-controls="business-navigation"
              aria-expanded="true"
              title="隐藏导航栏"
              onClick={() => setNavigationCollapsed(true)}
            >
              <span aria-hidden="true">‹</span>
            </button>
          </div>
          <nav aria-label="业务导航">
            {NAV_GROUPS.map((group) => (
              <section className="rail-group" key={group.id}>
                <header className="rail-group-head">
                  <span>{group.index}</span>
                  <strong>{group.label}</strong>
                </header>
                <div className="rail-group-items">
                  {group.items.map((item) => (
                    <a
                      key={item.id}
                      className={item.id === activeNavigation ? "active" : ""}
                      href={`#${item.id}`}
                    >
                      <span className="rail-item-index">{item.index}</span>
                      <span className="rail-item-label">{item.label}</span>
                    </a>
                  ))}
                </div>
              </section>
            ))}
          </nav>
          <div className="rail-foot">
            <i /> 真实数据 · Staging
          </div>
        </aside>
      )}
      <main>
        <header className="topline">
          <div className="topline-context">
            {!current.embed && navigationCollapsed && (
              <button
                type="button"
                className="rail-reveal"
                aria-label="显示导航栏"
                aria-controls="business-navigation"
                aria-expanded="false"
                title="显示导航栏"
                onClick={() => setNavigationCollapsed(false)}
              >
                <span aria-hidden="true">›</span>
                显示导航
              </button>
            )}
            <span>
              {current.embed
                ? "嵌入业务视图"
                : "销售、采购与真实利润经营闭环"}
            </span>
          </div>
          <div className="topline-tools">
            <div className="page-zoom" role="group" aria-label="页面缩放">
              <button
                type="button"
                aria-label="缩小页面"
                title="缩小页面（⌘/Ctrl -）"
                disabled={pageZoom.zoom === PAGE_ZOOM_STEPS[0]}
                onClick={pageZoom.zoomOut}
              >
                −
              </button>
              <button
                type="button"
                className="page-zoom-value"
                aria-label={`当前缩放 ${Math.round(pageZoom.zoom * 100)}%，点击恢复 100%`}
                title="恢复 100%（⌘/Ctrl 0）"
                onClick={pageZoom.resetZoom}
              >
                {Math.round(pageZoom.zoom * 100)}%
              </button>
              <button
                type="button"
                aria-label="放大页面"
                title="放大页面（⌘/Ctrl +）"
                disabled={pageZoom.zoom === PAGE_ZOOM_STEPS.at(-1)}
                onClick={pageZoom.zoomIn}
              >
                ＋
              </button>
            </div>
            <time>
              {new Intl.DateTimeFormat("zh-CN", { dateStyle: "long" }).format(
                new Date(),
              )}
            </time>
          </div>
        </header>
        <SectionView
          section={current.section}
          id={current.id}
          entry={current.entry}
        />
      </main>
    </div>
  );
}

function SectionView({
  section,
  id,
  entry,
}: {
  section: Section;
  id?: string;
  entry?: BusinessEntryKind;
}) {
  if (entry) return <BusinessEntryPage entry={entry} />;
  if (section === "agentQuery") return <AgentQueryReceipt traceId={id} />;
  if (section === "dashboard") return <OperationsDashboardView />;
  if (section === "quality") return <DataQualityView />;
  if (section === "incidents") return <OperatingIncidentsView />;
  if (section === "trends") return <OperatingTrendsView />;
  if (section === "coreData") return <CoreMasterDataCenter />;
  if (section === "productData") return <ProductMasterDataCenter />;
  if (section === "numbering") return <NumberingRulesCenter />;
  if (section === "profits") return <OrderProfits id={id} />;
  if (section === "profitability") return <Profitability />;
  if (section === "adjustments") return <ProfitAdjustments id={id} />;
  if (section === "reports") return <ManagementReports id={id} />;
  if (section === "sales") return <SalesOrderWorkflowPage id={id} />;
  if (section === "inventory") return <InventoryLedger skuId={id} />;
  if (section === "receivables") return <Receivables customerId={id} />;
  if (section === "receipts") return <ReceiptView id={id} />;
  if (section === "purchasing") return <PurchaseOrderWorkflowPage id={id} />;
  if (section === "goodsReceipts") return <GoodsReceipts id={id} />;
  if (section === "payables") return <Payables supplierId={id} />;
  if (section === "supplierPayments") return <SupplierPayments id={id} />;
  return <Shipments id={id} />;
}

const QUERY_STATUS_LABELS: Record<AgentQueryRun["status"], string> = {
  running: "处理中",
  query_complete: "查询完成",
  complete: "已回传",
  failed: "失败",
};

const QUERY_STAGE_LABELS: Record<string, string> = {
  AGENT_DELEGATION_AUTHORIZED: "已核验用户授权",
  AGENT_DELEGATION_ISSUED: "已签发单次查询授权",
  AGENT_DELEGATION_CONSUMED: "已校验查询范围",
  AGENT_TURN_AUTHORIZED: "已授权本次 Agent 回合",
  BUSINESS_MCP_TOOL_CALLED: "已调用业务只读工具",
  BUSINESS_MCP_TOOL_SUCCEEDED: "业务查询成功",
  BUSINESS_READ_PARTIAL_RESULT: "业务查询部分完成",
  BUSINESS_MCP_TOOL_FAILED: "业务查询失败",
  AGENT_BUSINESS_RESPONSE_EMITTED: "结果已回传 Buzz",
  AGENT_BUSINESS_RESPONSE_FAILED: "结果回传失败",
  AGENT_DELEGATION_REVOKED: "查询授权已撤销",
};

function AgentQueryReceipt({ traceId }: { traceId?: string }) {
  const state = useLoad(
    () =>
      traceId
        ? request<AgentQueryRun>(`/api/v1/agent-query-runs/${traceId}`)
        : Promise.reject(new Error("查询追踪号缺失")),
    [traceId],
  );
  return (
    <Page
      eyebrow="Agent 查询回执 / Audited receipt"
      title="业务查询记录"
      caption="仅展示当前账号自己的最小化审计信息；原始查询结果、令牌和授权凭据不会写入此记录。"
    >
      <State state={state}>
        {state.data && (
          <div className={`agent-query-receipt ${state.data.status}`}>
            <div className="agent-query-summary">
              <div>
                <span>状态</span>
                <strong>{QUERY_STATUS_LABELS[state.data.status]}</strong>
              </div>
              <div>
                <span>查询工具</span>
                <strong>{state.data.toolName ?? "尚未调用"}</strong>
              </div>
              <div>
                <span>结果数</span>
                <strong>{state.data.resultCount}</strong>
              </div>
              <div>
                <span>耗时</span>
                <strong>
                  {state.data.durationMs === null
                    ? "—"
                    : formatDuration(state.data.durationMs)}
                </strong>
              </div>
            </div>
            <dl className="agent-query-identifiers">
              <div>
                <dt>Trace ID</dt>
                <dd><code>{state.data.traceId}</code></dd>
              </div>
              <div>
                <dt>发起消息</dt>
                <dd title={state.data.sourceBuzzEventId ?? undefined}>
                  {state.data.sourceBuzzEventId
                    ? short(state.data.sourceBuzzEventId)
                    : "—"}
                </dd>
              </div>
              <div>
                <dt>回传消息</dt>
                <dd title={state.data.responseBuzzEventId ?? undefined}>
                  {state.data.responseBuzzEventId
                    ? short(state.data.responseBuzzEventId)
                    : "—"}
                </dd>
              </div>
              <div>
                <dt>完成时间</dt>
                <dd>{formatInstant(state.data.completedAt)}</dd>
              </div>
            </dl>
            <section className="agent-query-timeline" aria-label="查询审计步骤">
              <h2>处理轨迹</h2>
              {state.data.stages.map((stage, index) => (
                <article
                  className={stage.result}
                  key={`${stage.eventType}-${stage.occurredAt}-${index}`}
                >
                  <i aria-hidden="true" />
                  <div>
                    <strong>
                      {QUERY_STAGE_LABELS[stage.eventType] ?? stage.eventType}
                    </strong>
                    <span>{formatInstant(stage.occurredAt)}</span>
                  </div>
                  <Status value={stage.result} />
                </article>
              ))}
            </section>
          </div>
        )}
      </State>
    </Page>
  );
}

function OperationsDashboardView() {
  const period = currentPeriod();
  const state = useLoad(
    () =>
      request<OperationsDashboard>(
        `/api/v1/operations/dashboard?managementPeriod=${period}&currency=CNY`,
      ),
    [period],
  );
  return (
    <Page
      eyebrow="经营驾驶舱 / Operating truth"
      title="核心业务一屏掌握"
      caption="销售、采购、库存和真实利润来自同一组权威业务事实；金额严格限定单一币种。"
    >
      <RecentAgentQueries />
      <State state={state}>
        {state.data && (
          <>
            <OperationsReadout
              status={state.data.reportHealth.status}
              updatedAt={state.data.reportHealth.updatedAt}
              freshnessAgeSeconds={state.data.reportHealth.freshnessAgeSeconds}
              diagnostics={state.data.diagnostics}
              run={state.data.run}
            />
            <AlertList alerts={state.data.reportHealth.alerts} />
            <div className="kpi-grid">
              <Kpi
                label="销售订单额"
                value={formatMoney(
                  state.data.currency,
                  state.data.sales.orderAmount,
                )}
                note={`${state.data.sales.orderCount} 单 · 履约 ${percentage(state.data.sales.fulfillmentRate)}`}
                href="#sales"
              />
              <Kpi
                label="采购订单额"
                value={formatMoney(
                  state.data.currency,
                  state.data.purchasing.purchaseOrderAmount,
                )}
                note={`${state.data.purchasing.purchaseOrderCount} 单 · 到货 ${percentage(state.data.purchasing.receiptRate)}`}
                href="#purchasing"
              />
              <Kpi
                label="库存价值"
                value={formatMoney(
                  state.data.currency,
                  state.data.inventory.inventoryValue,
                )}
                note={`${state.data.inventory.skuLocationCount} 库位商品 · 缺货 ${state.data.inventory.stockoutCount}`}
                href="#inventory"
              />
              <Kpi
                label="管理经营利润"
                value={formatMoney(
                  state.data.currency,
                  state.data.profit.managementOperatingProfit,
                )}
                note={`利润率 ${percentage(state.data.profit.managementOperatingMarginRate)}`}
                href="#profits"
              />
            </div>
            <div className="dashboard-panels">
              <article>
                <h2>销售履约</h2>
                <dl>
                  <Metric
                    label="已承诺订单"
                    value={state.data.sales.committedOrderCount}
                  />
                  <Metric
                    label="已完成出库"
                    value={state.data.sales.shippedOrderCount}
                  />
                  <Metric
                    label="人工复核"
                    value={state.data.sales.manualHoldCount}
                  />
                  <Metric
                    label="出库收入"
                    value={formatMoney(
                      state.data.currency,
                      state.data.sales.shippedRevenue,
                    )}
                  />
                </dl>
                <a href="#sales">进入销售订单闭环 →</a>
              </article>
              <article>
                <h2>采购到货</h2>
                <dl>
                  <Metric
                    label="已到齐订单"
                    value={state.data.purchasing.receivedOrderCount}
                  />
                  <Metric
                    label="采购行数"
                    value={state.data.purchasing.lineCount}
                  />
                  <Metric
                    label="已完成行数"
                    value={state.data.purchasing.receivedLineCount}
                  />
                  <Metric
                    label="到货行完成率"
                    value={percentage(state.data.purchasing.receiptRate)}
                  />
                </dl>
                <a href="#purchasing">进入采购订单闭环 →</a>
              </article>
              <article>
                <h2>库存结构</h2>
                <dl>
                  <Metric
                    label="有货库位商品"
                    value={state.data.inventory.stockedLocationCount}
                  />
                  <Metric
                    label="有预留库位商品"
                    value={state.data.inventory.reservedLocationCount}
                  />
                  <Metric
                    label="缺货组合"
                    value={state.data.inventory.stockoutCount}
                  />
                  <Metric label="成本口径" value="移动平均" />
                </dl>
                <a href="#inventory">下钻库存台账 →</a>
              </article>
              <article>
                <h2>真实利润</h2>
                <dl>
                  <Metric
                    label="净收入"
                    value={formatMoney(
                      state.data.currency,
                      state.data.profit.netRevenue,
                    )}
                  />
                  <Metric
                    label="产品成本"
                    value={formatMoney(
                      state.data.currency,
                      state.data.profit.productCost,
                    )}
                  />
                  <Metric
                    label="毛利"
                    value={formatMoney(
                      state.data.currency,
                      state.data.profit.grossProfit,
                    )}
                  />
                  <Metric
                    label="事实水位"
                    value={state.data.profit.sourceWatermark}
                  />
                </dl>
                <a href="#profitability">下钻多维盈利 →</a>
              </article>
            </div>
            <p className="report-warning">{state.data.warnings.join(" · ")}</p>
          </>
        )}
      </State>
    </Page>
  );
}

function RecentAgentQueries() {
  const state = useLoad(
    () => request<AgentQueryRunList>("/api/v1/agent-query-runs"),
    [],
  );
  if (state.loading) return null;
  if (state.error || !state.data || state.data.items.length === 0) return null;
  return (
    <section className="recent-agent-queries" aria-label="最近 Agent 查询">
      <header>
        <div>
          <span>Agent audit</span>
          <h2>最近 Agent 查询</h2>
        </div>
        <small>仅当前账号可见</small>
      </header>
      <div>
        {state.data.items.slice(0, 5).map((run) => (
          <a
            href={`/embed/agent-queries/${run.traceId}`}
            key={run.traceId}
          >
            <span>{QUERY_STATUS_LABELS[run.status]}</span>
            <strong>{run.toolName ?? "等待调用"}</strong>
            <small>
              {run.resultCount} 条结果 · {formatInstant(run.completedAt)}
            </small>
            <code>{short(run.traceId)}</code>
          </a>
        ))}
      </div>
    </section>
  );
}

function DataQualityView() {
  const state = useLoad(
    () => request<DataQuality>("/api/v1/operations/data-quality"),
    [],
  );
  return (
    <Page
      eyebrow="稳定性控制台 / Reconciliation"
      title="业务数据质量中心"
      caption="统一核对销售、采购、库存和利润投影；只展示证据与安全恢复指引，不直接改写权威事实。"
    >
      <State state={state}>
        {state.data && (
          <>
            <OperationsReadout
              status={state.data.status}
              updatedAt={state.data.projection.updatedAt}
              freshnessAgeSeconds={state.data.projection.freshnessAgeSeconds}
              diagnostics={state.data.diagnostics}
              run={state.data.run}
            />
            <AlertList alerts={state.data.alerts} />
            <div className={`quality-hero ${state.data.status}`}>
              <div>
                <span>整体状态</span>
                <strong>{state.data.status}</strong>
              </div>
              <div>
                <span>对账差异</span>
                <strong>{state.data.differenceCount}</strong>
              </div>
              <div>
                <span>投影积压</span>
                <strong>{state.data.projection.pendingEvents}</strong>
              </div>
              <div>
                <span>失败待处理</span>
                <strong>{state.data.projection.pendingFailures}</strong>
              </div>
            </div>
            <div className="quality-list">
              {state.data.checks.map((item) => (
                <article key={item.domain}>
                  <div>
                    <strong>{qualityLabel(item.domain)}</strong>
                    <small>权威事实与业务投影逐项核对</small>
                  </div>
                  <Status value={item.status} />
                  <b>{item.differenceCount}</b>
                  <a href={item.evidencePath}>查看证据</a>
                </article>
              ))}
            </div>
            <div className="projection-note">
              <strong>利润投影器</strong>
              <span>
                Worker{" "}
                {state.data.projection.workerEnabled ? "已启用" : "未启用"} ·{" "}
                {state.data.projection.fresh ? "水位新鲜" : "水位过期"} · 新鲜度{" "}
                {formatAge(state.data.projection.freshnessAgeSeconds)} ·
                最后事实序号 {state.data.projection.lastFactSequence ?? "尚无"}
              </span>
              <small>
                恢复原则：先检查证据，再运行范围内对账或幂等投影重放；禁止直接更新事实表。
              </small>
            </div>
          </>
        )}
      </State>
    </Page>
  );
}

function OperatingIncidentsView() {
  const [revision, setRevision] = React.useState(0);
  const [busy, setBusy] = React.useState<string | null>(null);
  const [notice, setNotice] = React.useState<string | null>(null);
  const state = useLoad(
    () => request<OperatingIncidentQueue>("/api/v1/operations/incidents"),
    [revision],
  );
  const items = state.data?.items ?? [];
  const unresolved = items.filter((item) => item.reviewStatus !== "resolved");
  const overdue = unresolved.filter((item) => item.overdue).length;
  const critical = unresolved.filter(
    (item) => item.severity === "critical" && item.conditionStatus === "active",
  ).length;

  async function scan() {
    setBusy("scan");
    setNotice(null);
    try {
      const result = await request<{
        createdCount: number;
        reopenedCount: number;
        clearedCount: number;
      }>("/api/v1/operations/incidents/scan", {
        method: "POST",
        body: "{}",
      });
      setNotice(
        `扫描完成：新增 ${result.createdCount} · 重开 ${result.reopenedCount} · 条件清除 ${result.clearedCount}`,
      );
      setRevision((value) => value + 1);
    } catch (error) {
      setNotice((error as Error).message);
    } finally {
      setBusy(null);
    }
  }

  async function command(
    incident: OperatingIncident,
    action: "claim" | "acknowledge" | "start" | "resolve" | "set_due",
  ) {
    const key = `${incident.id}:${action}`;
    setBusy(key);
    setNotice(null);
    try {
      await request(`/api/v1/operations/incidents/${incident.id}/commands`, {
        method: "POST",
        body: JSON.stringify({
          action,
          expectedVersion: incident.version,
          dueAt:
            action === "set_due"
              ? new Date(Date.now() + 24 * 60 * 60 * 1000).toISOString()
              : undefined,
        }),
      });
      setNotice(`${incident.alertCode}：${incidentActionLabel(action)}完成`);
      setRevision((value) => value + 1);
    } catch (error) {
      setNotice((error as Error).message);
    } finally {
      setBusy(null);
    }
  }

  return (
    <Page
      eyebrow="经营事件簿 / Incident docket"
      title="异常必须有人接住"
      caption="将数据质量异常变成有负责人、有时限、有处置轨迹的经营事件；底层条件未清除时不能标记解决。"
      actions={
        <button type="button" onClick={scan} disabled={busy !== null}>
          {busy === "scan" ? "扫描中…" : "扫描当前异常"}
        </button>
      }
    >
      <State state={state}>
        {state.data && (
          <>
            <section className="incident-totals" aria-label="异常处置摘要">
              <Metric label="待处置" value={unresolved.length} />
              <Metric label="关键且生效" value={critical} />
              <Metric label="已超时" value={overdue} />
              <Metric
                label="已解决"
                value={
                  items.filter((item) => item.reviewStatus === "resolved")
                    .length
                }
              />
            </section>
            {notice && <p className="incident-notice">{notice}</p>}
            {items.length === 0 ? (
              <div className="empty incident-empty">
                <span>✓</span>
                <p>事件簿为空。运行一次扫描，记录当前数据质量状态。</p>
              </div>
            ) : (
              <div className="incident-docket">
                {items.map((item) => (
                  <article
                    key={item.id}
                    className={`incident-card ${item.severity} ${item.reviewStatus}`}
                  >
                    <div className="incident-clock">
                      <span>{item.severity === "critical" ? "P0" : "P1"}</span>
                      <strong>
                        {item.overdue ? "OVERDUE" : dueDistance(item.dueAt)}
                      </strong>
                      <small>{formatInstant(item.dueAt)}</small>
                    </div>
                    <div className="incident-body">
                      <header>
                        <div>
                          <code>{item.alertCode}</code>
                          <h2>{item.message}</h2>
                        </div>
                        <div className="incident-statuses">
                          <Status value={item.conditionStatus} />
                          <Status value={item.reviewStatus} />
                        </div>
                      </header>
                      <dl className="incident-facts">
                        <Metric
                          label="负责人"
                          value={item.assigneeName ?? "未认领"}
                        />
                        <Metric label="发生次数" value={item.occurrenceCount} />
                        <Metric
                          label="最后发现"
                          value={formatInstant(item.lastSeenAt)}
                        />
                        <Metric
                          label="追踪号"
                          value={`${item.lastTraceId.slice(0, 8)}…${item.lastTraceId.slice(-4)}`}
                        />
                      </dl>
                      <div className="incident-actions">
                        {!item.assigneeUserId && (
                          <button
                            type="button"
                            className="mini"
                            disabled={busy !== null}
                            onClick={() => command(item, "claim")}
                          >
                            归我处理
                          </button>
                        )}
                        {item.reviewStatus === "open" && (
                          <button
                            type="button"
                            className="mini"
                            disabled={busy !== null}
                            onClick={() => command(item, "acknowledge")}
                          >
                            确认异常
                          </button>
                        )}
                        {item.reviewStatus !== "in_progress" &&
                          item.reviewStatus !== "resolved" && (
                            <button
                              type="button"
                              className="mini"
                              disabled={busy !== null}
                              onClick={() => command(item, "start")}
                            >
                              开始处理
                            </button>
                          )}
                        {item.reviewStatus !== "resolved" && (
                          <button
                            type="button"
                            className="mini secondary"
                            disabled={busy !== null}
                            onClick={() => command(item, "set_due")}
                          >
                            重设 24h
                          </button>
                        )}
                        {item.conditionStatus === "cleared" &&
                          item.reviewStatus !== "resolved" && (
                            <button
                              type="button"
                              className="mini resolve"
                              disabled={busy !== null}
                              onClick={() => command(item, "resolve")}
                            >
                              标记解决
                            </button>
                          )}
                        <a href={item.evidencePath}>查看证据</a>
                      </div>
                      <details className="incident-history">
                        <summary>处置轨迹 · {item.events.length}</summary>
                        <ol>
                          {item.events.map((event) => (
                            <li key={event.id}>
                              <span>{incidentEventLabel(event.eventType)}</span>
                              <strong>{event.actorName}</strong>
                              <time>{formatInstant(event.occurredAt)}</time>
                            </li>
                          ))}
                        </ol>
                      </details>
                    </div>
                  </article>
                ))}
              </div>
            )}
          </>
        )}
      </State>
    </Page>
  );
}

function incidentActionLabel(action: string) {
  return (
    {
      claim: "认领",
      acknowledge: "确认",
      start: "开始处理",
      resolve: "解决",
      set_due: "处理时限更新",
    }[action] ?? action
  );
}

function incidentEventLabel(event: string) {
  return (
    {
      detected: "检测到异常",
      condition_cleared: "底层条件已清除",
      reopened: "异常再次出现",
      claimed: "认领负责人",
      acknowledged: "确认异常",
      started: "开始处理",
      due_changed: "更新处理时限",
      resolved: "完成解决",
    }[event] ?? event
  );
}

function dueDistance(value: string) {
  const hours = Math.max(
    0,
    Math.ceil((new Date(value).getTime() - Date.now()) / (60 * 60 * 1000)),
  );
  return hours < 1 ? "< 1H" : `${hours}H LEFT`;
}

function OperationsReadout({
  status,
  updatedAt,
  freshnessAgeSeconds,
  diagnostics,
  run,
}: {
  status: "complete" | "partial" | "blocked";
  updatedAt: string | null;
  freshnessAgeSeconds: number | null;
  diagnostics: ReadDiagnostics;
  run: ReportRun;
}) {
  return (
    <section
      className={`operations-readout ${status}`}
      aria-label="经营报表运行状态"
    >
      <div className="readout-state">
        <span>经营状态</span>
        <strong>{status}</strong>
        <small>
          {status === "complete"
            ? "权威事实可用于经营判断"
            : "请先查看异常与证据"}
        </small>
      </div>
      <dl>
        <div>
          <dt>数据新鲜度</dt>
          <dd>{formatAge(freshnessAgeSeconds)}</dd>
          <small>{updatedAt ? formatInstant(updatedAt) : "尚无投影水位"}</small>
        </div>
        <div>
          <dt>本次读取</dt>
          <dd>{formatDuration(run.durationMs)}</dd>
          <small>目标 ≤ {formatDuration(run.targetMs)}</small>
        </div>
        <div>
          <dt>最慢阶段</dt>
          <dd>
            {diagnostics.slowestStage
              ? stageLabel(diagnostics.slowestStage)
              : "—"}
          </dd>
          <small>
            {diagnostics.status === "healthy" ? "运行正常" : "超过耗时目标"}
          </small>
        </div>
        <div>
          <dt>追踪号</dt>
          <dd>
            <code title={run.traceId}>{short(run.traceId)}</code>
          </dd>
          <small>{formatInstant(run.completedAt)}</small>
        </div>
      </dl>
      <a href="#quality">查看稳定性证据 →</a>
    </section>
  );
}

function AlertList({ alerts }: { alerts: OperatingAlert[] }) {
  if (alerts.length === 0)
    return (
      <div className="alert-clear">
        <span>NO ACTIVE ALERTS</span>
        <strong>当前没有经营报表异常</strong>
      </div>
    );
  return (
    <section className="operating-alerts" aria-label="经营报表异常">
      {alerts.map((item) => (
        <article
          className={item.severity}
          key={`${item.code}-${item.evidencePath}-${item.message}`}
        >
          <span>{item.severity}</span>
          <div>
            <strong>{item.message}</strong>
            <code>{item.code}</code>
          </div>
          <a href={item.evidencePath}>查看证据 →</a>
        </article>
      ))}
    </section>
  );
}

function Kpi({
  label,
  value,
  note,
  href,
}: {
  label: string;
  value: string;
  note: string;
  href: string;
}) {
  return (
    <a className="kpi" href={href}>
      <span>{label}</span>
      <strong>{value}</strong>
      <small>{note}</small>
    </a>
  );
}
function Metric({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}
function percentage(value: string | null) {
  return value === null ? "—" : `${(Number(value) * 100).toFixed(1)}%`;
}
function qualityLabel(value: string) {
  return (
    (
      {
        inventory: "库存台账",
        receivables: "经营应收",
        payables: "经营应付",
        profitFacts: "利润事实",
      } as Record<string, string>
    )[value] ?? value
  );
}
function stageLabel(value: string) {
  return (
    (
      {
        authorization: "权限范围",
        salesOrders: "销售订单",
        shipments: "销售出库",
        purchaseOrders: "采购订单",
        inventoryBalances: "库存余额",
        profitFacts: "利润事实",
        projectionHealth: "投影状态",
        inventoryReconciliation: "库存对账",
        receivablesReconciliation: "应收对账",
        payablesReconciliation: "应付对账",
        profitReconciliation: "利润对账",
        projectionFailures: "投影失败",
        projectionWatermark: "投影水位",
      } as Record<string, string>
    )[value] ?? value
  );
}
function formatDuration(value: number) {
  return value < 1
    ? `${Math.round(value * 1000)}µs`
    : `${value.toFixed(value >= 100 ? 0 : 1)}ms`;
}
function formatAge(value: number | null) {
  if (value === null) return "未知";
  if (value < 60) return `${value} 秒`;
  if (value < 3600) return `${Math.floor(value / 60)} 分钟`;
  return `${Math.floor(value / 3600)} 小时`;
}
function formatInstant(value: string) {
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(new Date(value));
}

function Page({
  eyebrow,
  title,
  caption,
  actions,
  children,
}: React.PropsWithChildren<{
  eyebrow: string;
  title: string;
  caption: string;
  actions?: React.ReactNode;
}>) {
  return (
    <section className="page">
      <div className="page-head">
        <div>
          <p>{eyebrow}</p>
          <h1>{title}</h1>
          <span>{caption}</span>
        </div>
        {actions}
      </div>
      {children}
    </section>
  );
}

function Sales({ id }: { id?: string }) {
  const [revision, setRevision] = React.useState(0);
  const state = useLoad(
    () => request<Envelope<SalesOrder>>("/api/v1/sales-orders?limit=200"),
    [revision],
  );
  const rows =
    state.data?.items.filter(
      (item) => !id || item.id === id || item.orderNumber === id,
    ) ?? [];
  return (
    <Page
      eyebrow="订单控制台 / Sales order"
      title={id ? "订单详情" : "销售订单"}
      caption="确认即全量预占；已确认订单只能通过显式业务命令变更。"
    >
      {id ? (
        <>
          <SalesOrderConfirmation
            key={`${rows[0]?.id ?? id}:${revision}`}
            orderId={rows[0]?.id ?? id}
            onDone={() => setRevision((value) => value + 1)}
          />
          <CommandConsole
            title="高级草稿编辑"
            defaultPath={`/api/v1/sales-orders/${id}`}
            defaultMethod="PUT"
            sample={{
              expectedVersion: rows[0]?.version ?? 1,
              customerId: "",
              businessUnitId: "",
              currency: "CNY",
              orderDate: today(),
              lines: [],
            }}
            onDone={() => setRevision((value) => value + 1)}
          />
        </>
      ) : (
        <SalesOrderEntry onDone={() => setRevision((value) => value + 1)} />
      )}
      <State state={state}>
        {
          <OrderTable
            rows={rows}
            onDone={() => setRevision((value) => value + 1)}
          />
        }
      </State>
    </Page>
  );
}

function OrderTable({
  rows,
  onDone,
}: {
  rows: SalesOrder[];
  onDone: () => void;
}) {
  return (
    <div className="sheet">
      <div className="sheet-rule">
        <span>单号</span>
        <span>客户</span>
        <span>履约</span>
        <span>金额</span>
        <span>版本 / 命令</span>
      </div>
      {rows.map((row) => (
        <article className="ledger-row" key={row.id}>
          <div>
            <a href={`/sales/orders/${row.id}`}>{row.orderNumber}</a>
            <small>{row.orderDate}</small>
          </div>
          <code>{short(row.customerId)}</code>
          <Status
            value={
              row.holdStatus === "none" ? row.fulfillmentStatus : row.holdStatus
            }
          />
          <strong>
            {formatMoney(row.currency, row.grossAmount)}
          </strong>
          <div className="row-actions">
            <em>v{row.version}</em>
            {row.lifecycleStatus === "draft" && (
              <a className="mini action-link" href={`/sales/orders/${row.id}`}>
                核对库存
              </a>
            )}
            {row.lifecycleStatus === "confirmed" &&
              row.holdStatus === "none" && (
                <CommandButton
                  label="Hold"
                  path={`/api/v1/sales-orders/${row.id}/manual-review-hold`}
                  body={{
                    expectedVersion: row.version,
                    reasonCode: "MANUAL_REVIEW",
                  }}
                  onDone={onDone}
                />
              )}
            {row.holdStatus !== "none" && (
              <CommandButton
                label="Release"
                path={`/api/v1/sales-orders/${row.id}/release-manual-review-hold`}
                body={{
                  expectedVersion: row.version,
                  reasonCode: "REVIEW_COMPLETE",
                }}
                onDone={onDone}
              />
            )}
            <CommandButton
              label="取消剩余"
              path={`/api/v1/sales-orders/${row.id}/cancel-remaining`}
              body={{ expectedVersion: row.version }}
              onDone={onDone}
            />
          </div>
        </article>
      ))}
      {rows.length === 0 && (
        <Empty text="当前范围内没有销售订单。新建草稿后，可在库存充足时确认。" />
      )}
    </div>
  );
}

function Receivables({ customerId }: { customerId?: string }) {
  const query = customerId
    ? `?customerId=${encodeURIComponent(customerId)}`
    : "?limit=200";
  const state = useLoad(
    () => request<Envelope<Receivable>>(`/api/v1/trade-receivables${query}`),
    [customerId],
  );
  return (
    <Page
      eyebrow="经营口径 / Trade receivable"
      title="经营性应收"
      caption="出库确认时确认收入；它不是会计总账，也不替代发票。"
    >
      <State state={state}>
        <div className="sheet">
          {state.data?.items.map((row) => (
            <article className="receivable-row" key={row.id}>
              <div>
                <a href={`/receivables/customer/${row.customerId}`}>
                  {row.receivableNumber}
                </a>
                <small>到期 {row.dueDate}</small>
              </div>
              <div className="amount">
                <small>未核销</small>
                <strong>
                  {formatMoney(row.currency, row.openAmount)}
                </strong>
              </div>
              <Status
                value={
                  row.isOverdue ? `逾期 ${row.overdueDays} 天` : row.status
                }
              />
            </article>
          ))}
          {state.data?.items.length === 0 && (
            <Empty text="当前客户范围内没有经营性应收。应收会在出库确认时自动生成。" />
          )}
        </div>
      </State>
    </Page>
  );
}

function ReceiptView({ id }: { id?: string }) {
  const [revision, setRevision] = React.useState(0);
  const state = useLoad<{ item: Receipt } | Envelope<Receipt>>(
    () =>
      id
        ? request<{ item: Receipt }>(
            `/api/v1/customer-receipts/${encodeURIComponent(id)}`,
          )
        : request<Envelope<Receipt>>("/api/v1/customer-receipts?limit=200"),
    [id, revision],
  );
  const receipt = state.data && "item" in state.data ? state.data.item : null;
  return (
    <Page
      eyebrow="收款登记 / Settlement"
      title={receipt?.receiptNumber ?? "收款与核销"}
      caption="收款先确认，再按同一主体、客户和币种核销应收。"
    >
      <CommandConsole
        title="收款 / 核销 / 冲销命令"
        defaultPath={
          receipt
            ? `/api/v1/customer-receipts/${receipt.id}/allocations`
            : "/api/v1/customer-receipts"
        }
        defaultMethod="POST"
        sample={
          receipt
            ? {
                expectedReceiptVersion: receipt.version,
                allocations: [{ receivableId: "", amount: "0" }],
              }
            : {
                legalEntityId: "",
                customerId: "",
                currency: "CNY",
                receiptDate: today(),
                amount: "0",
                paymentMethod: "bank_transfer",
              }
        }
        onDone={() => setRevision((value) => value + 1)}
      />
      <State state={state}>
        {receipt ? (
          <div className="receipt-slip">
            <div>
              <span>收款金额</span>
              <strong>
                {formatMoney(receipt.currency, receipt.amount)}
              </strong>
            </div>
            <dl>
              <div>
                <dt>已核销</dt>
                <dd>{formatAmount(receipt.allocatedAmount)}</dd>
              </div>
              <div>
                <dt>未核销</dt>
                <dd>{formatAmount(receipt.unappliedAmount)}</dd>
              </div>
              <div>
                <dt>状态</dt>
                <dd>
                  <Status value={receipt.status} />
                </dd>
              </div>
            </dl>
            <footer>
              {receipt.receiptDate} · v{receipt.version}
            </footer>
            <div className="row-actions">
              {receipt.status === "draft" && (
                <CommandButton
                  label="确认"
                  path={`/api/v1/customer-receipts/${receipt.id}/confirm`}
                  body={{ expectedVersion: receipt.version }}
                  onDone={() => setRevision((value) => value + 1)}
                />
              )}
              <CommandButton
                label="冲销收款"
                path={`/api/v1/customer-receipts/${receipt.id}/reverse`}
                body={{
                  expectedVersion: receipt.version,
                  reasonCode: "RECEIPT_CORRECTION",
                }}
                onDone={() => setRevision((value) => value + 1)}
              />
            </div>
          </div>
        ) : (
          <div className="compact-list">
            {state.data &&
              "items" in state.data &&
              state.data.items.map((item) => (
                <article key={item.id}>
                  <a href={`/customer-receipts/${item.id}`}>
                    {item.receiptNumber}
                  </a>
                  <strong>
                    {formatMoney(item.currency, item.amount)}
                  </strong>
                  <Status value={item.status} />
                  <span>未核销 {formatAmount(item.unappliedAmount)}</span>
                </article>
              ))}
          </div>
        )}
      </State>
    </Page>
  );
}

function Shipments({ id }: { id?: string }) {
  const [revision, setRevision] = React.useState(0);
  const query = id ? `?shipmentId=${encodeURIComponent(id)}` : "?limit=200";
  const state = useLoad(
    () => request<Envelope<Shipment>>(`/api/v1/shipments${query}`),
    [id, revision],
  );
  return (
    <Page
      eyebrow="履约凭据 / Shipment"
      title={id ? "出库单详情" : "销售出库"}
      caption="每次确认都冻结成本快照，并在同一事务生成库存流水与经营性应收。"
    >
      {id ? (
        <ShipmentConfirmation
          key={`${id}:${revision}`}
          shipmentId={id}
          onDone={() => setRevision((value) => value + 1)}
        />
      ) : (
        <ShipmentEntry onDone={() => setRevision((value) => value + 1)} />
      )}
      <div className="process">
        <span>预占</span>
        <b>→</b>
        <span>分批出库</span>
        <b>→</b>
        <span>成本快照</span>
        <b>→</b>
        <span>应收</span>
      </div>
      <State state={state}>
        <div className="compact-list">
          {state.data?.items.map((item) => (
            <article key={item.id}>
              <a href={`/shipments/${item.id}`}>{item.shipmentNumber}</a>
              <Status value={item.status} />
              <span>{item.shipmentDate}</span>
              <div className="row-actions">
                <em>v{item.version}</em>
                {item.status === "draft" && (
                  <a className="action-link" href={`/shipments/${item.id}`}>
                    确认前检查
                  </a>
                )}
                {item.status === "confirmed" && (
                  <CommandButton
                    label="冲销"
                    path={`/api/v1/shipments/${item.id}/reverse`}
                    body={{
                      expectedVersion: item.version,
                      reasonCode: "SHIPMENT_CORRECTION",
                    }}
                    onDone={() => setRevision((value) => value + 1)}
                  />
                )}
              </div>
            </article>
          ))}
        </div>
      </State>
    </Page>
  );
}

function PurchaseOrders({ id }: { id?: string }) {
  const [revision, setRevision] = React.useState(0);
  const state = useLoad(
    () => request<Envelope<PurchaseOrder>>("/api/v1/purchase-orders?limit=200"),
    [revision],
  );
  const rows =
    state.data?.items.filter(
      (item) => !id || item.id === id || item.purchaseOrderNumber === id,
    ) ?? [];
  return (
    <Page
      eyebrow="采购承诺 / Purchase order"
      title={id ? "采购订单详情" : "采购订单"}
      caption="确认形成采购承诺；收货后按采购净额更新暂估库存成本。"
    >
      {id && (
        <PurchaseOrderConfirmation
          key={`${id}:${revision}`}
          orderId={id}
          onDone={() => setRevision((value) => value + 1)}
        />
      )}
      {(!id || rows[0]?.lifecycleStatus === "draft") && (
        <PurchaseOrderEntry
          orderId={id}
          onDone={() => setRevision((value) => value + 1)}
        />
      )}
      <State state={state}>
        <div className="sheet">
          {rows.map((row) => (
            <article className="ledger-row" key={row.id}>
              <div>
                <a href={`/purchase-orders/${row.id}`}>
                  {row.purchaseOrderNumber}
                </a>
                <small>{row.orderDate}</small>
              </div>
              <code>{short(row.supplierId)}</code>
              <Status value={row.receivingStatus} />
              <strong>
                {formatMoney(row.currency, row.grossAmount)}
              </strong>
              <div className="row-actions">
                <em>v{row.version}</em>
                {row.lifecycleStatus === "draft" && (
                  <a
                    className="action-link"
                    href={`/purchase-orders/${row.id}`}
                  >
                    确认前检查
                  </a>
                )}
                <CommandButton
                  label="取消剩余"
                  path={`/api/v1/purchase-orders/${row.id}/cancel-remaining`}
                  body={{ expectedVersion: row.version }}
                  onDone={() => setRevision((value) => value + 1)}
                />
              </div>
            </article>
          ))}
          {rows.length === 0 && <Empty text="当前范围内没有采购订单。" />}
        </div>
      </State>
    </Page>
  );
}

function GoodsReceipts({ id }: { id?: string }) {
  const [revision, setRevision] = React.useState(0);
  const state = useLoad(
    () => request<Envelope<GoodsReceipt>>("/api/v1/goods-receipts?limit=200"),
    [revision],
  );
  const rows =
    state.data?.items.filter(
      (item) => !id || item.id === id || item.goodsReceiptNumber === id,
    ) ?? [];
  return (
    <Page
      eyebrow="采购到货 / Goods receipt"
      title={id ? "收货单详情" : "采购收货"}
      caption="确认在单一事务中增加库存数量与价值，并创建经营性应付。"
    >
      {id ? (
        <GoodsReceiptConfirmation
          key={`${id}:${revision}`}
          receiptId={id}
          onDone={() => setRevision((value) => value + 1)}
        />
      ) : (
        <GoodsReceiptEntry onDone={() => setRevision((value) => value + 1)} />
      )}
      <State state={state}>
        <div className="compact-list">
          {rows.map((item) => (
            <article key={item.id}>
              <a href={`/goods-receipts/${item.id}`}>
                {item.goodsReceiptNumber}
              </a>
              <Status value={item.status} />
              <span>
                暂估成本 {formatMoney(item.currency, item.inventoryCostAmount)}
              </span>
              <div className="row-actions">
                <em>v{item.version}</em>
                {item.status === "draft" && (
                  <a
                    className="action-link"
                    href={`/goods-receipts/${item.id}`}
                  >
                    确认前检查
                  </a>
                )}
                {item.status === "confirmed" && (
                  <CommandButton
                    label="纠错冲销"
                    path={`/api/v1/goods-receipts/${item.id}/reverse`}
                    body={{
                      expectedVersion: item.version,
                      reasonCode: "RECEIPT_CORRECTION",
                    }}
                    onDone={() => setRevision((value) => value + 1)}
                  />
                )}
              </div>
            </article>
          ))}
          {rows.length === 0 && <Empty text="当前范围内没有采购收货单。" />}
        </div>
      </State>
    </Page>
  );
}

function Payables({ supplierId }: { supplierId?: string }) {
  const query = supplierId
    ? `?supplierId=${encodeURIComponent(supplierId)}`
    : "?limit=200";
  const state = useLoad(
    () => request<Envelope<Payable>>(`/api/v1/trade-payables${query}`),
    [supplierId],
  );
  return (
    <Page
      eyebrow="经营口径 / Trade payable"
      title="经营性应付"
      caption="收货确认时产生；不是供应商发票、会计子账或总账。"
    >
      <State state={state}>
        <div className="sheet">
          {state.data?.items.map((row) => (
            <article className="receivable-row" key={row.id}>
              <div>
                <code>{row.payableNumber}</code>
                <small>到期 {row.dueDate}</small>
              </div>
              <div className="amount">
                <small>未核销</small>
                <strong>
                  {formatMoney(row.currency, row.openAmount)}
                </strong>
              </div>
              <Status
                value={
                  row.isOverdue ? `逾期 ${row.overdueDays} 天` : row.status
                }
              />
            </article>
          ))}
          {state.data?.items.length === 0 && (
            <Empty text="当前供应商范围内没有经营性应付。" />
          )}
        </div>
      </State>
    </Page>
  );
}

function SupplierPayments({ id }: { id?: string }) {
  const [revision, setRevision] = React.useState(0);
  const state = useLoad(
    () =>
      request<Envelope<SupplierPayment>>("/api/v1/supplier-payments?limit=200"),
    [revision],
  );
  const rows =
    state.data?.items.filter(
      (item) => !id || item.id === id || item.supplierPaymentNumber === id,
    ) ?? [];
  const payment = rows[0];
  return (
    <Page
      eyebrow="付款登记 / Supplier settlement"
      title={id ? "供应商付款详情" : "供应商付款"}
      caption="付款先确认，再按同一主体、供应商和币种核销应付。"
    >
      <CommandConsole
        title={payment ? "核销经营性应付" : "登记供应商付款"}
        defaultPath={
          payment
            ? `/api/v1/supplier-payments/${payment.id}/allocations`
            : "/api/v1/supplier-payments"
        }
        defaultMethod="POST"
        sample={
          payment
            ? {
                expectedPaymentVersion: payment.version,
                allocations: [{ payableId: "", amount: "0" }],
              }
            : {
                legalEntityId: "",
                supplierId: "",
                currency: "CNY",
                paymentDate: today(),
                amount: "0",
                paymentMethod: "bank_transfer",
              }
        }
        onDone={() => setRevision((value) => value + 1)}
      />
      <State state={state}>
        <div className="compact-list">
          {rows.map((item) => (
            <article key={item.id}>
              <a href={`/supplier-payments/${item.id}`}>
                {item.supplierPaymentNumber}
              </a>
              <strong>
                {formatMoney(item.currency, item.amount)}
              </strong>
              <Status value={item.status} />
              <span>未核销 {formatAmount(item.unappliedAmount)}</span>
              <div className="row-actions">
                <em>v{item.version}</em>
                {item.status === "draft" && (
                  <CommandButton
                    label="确认"
                    path={`/api/v1/supplier-payments/${item.id}/confirm`}
                    body={{ expectedVersion: item.version }}
                    onDone={() => setRevision((value) => value + 1)}
                  />
                )}
                {item.status !== "draft" && item.status !== "reversed" && (
                  <CommandButton
                    label="冲销付款"
                    path={`/api/v1/supplier-payments/${item.id}/reverse`}
                    body={{
                      expectedVersion: item.version,
                      reasonCode: "PAYMENT_CORRECTION",
                    }}
                    onDone={() => setRevision((value) => value + 1)}
                  />
                )}
              </div>
            </article>
          ))}
        </div>
      </State>
    </Page>
  );
}

function OrderProfits({ id }: { id?: string }) {
  const query = new URLSearchParams({ limit: "200" });
  if (id) query.set("orderId", id);
  const state = useLoad(
    () => request<Envelope<OrderProfit>>(`/api/v1/order-profits?${query}`),
    [id],
  );
  return (
    <Page
      eyebrow="经营利润 / Order truth"
      title={id ? "订单利润详情" : "订单真实利润"}
      caption="收入与产品成本来自已确认出库，经营费用来自已过账管理调整；该口径不是法定利润。"
    >
      <State state={state}>
        <div className="sheet">
          {state.data?.items.map((row) => (
            <article className="ledger-row" key={row.salesOrderId}>
              <div>
                <a href={`/order-profits/${row.salesOrderId}`}>
                  {short(row.salesOrderId)}
                </a>
                <small>{row.dataAsOf}</small>
              </div>
              <strong>
                {formatMoney(row.currency, row.netRevenue)}
              </strong>
              <span>毛利 {formatAmount(row.grossProfit)}</span>
              <span>贡献利润 {formatAmount(row.contributionProfit)}</span>
              <div>
                <strong>
                  经营利润 {formatAmount(row.managementOperatingProfit)}
                </strong>
                <Status value={row.dataQualityStatus} />
              </div>
            </article>
          ))}
          {state.data?.items.length === 0 && (
            <Empty text="尚无已投影的订单利润。确认出库后由利润投影器生成事实。" />
          )}
        </div>
      </State>
    </Page>
  );
}

function Profitability() {
  const [dimension, setDimension] = React.useState("customer");
  const period = currentPeriod();
  const state = useLoad(
    () =>
      request<Envelope<ProfitabilityRow>>(
        `/api/v1/profitability?managementPeriod=${period}&currency=CNY&dimensionOne=${dimension}&limit=200`,
      ),
    [dimension, period],
  );
  return (
    <Page
      eyebrow="盈利分析 / Dimensions"
      title="多维盈利分析"
      caption="按客户、商品、品类、品牌、业务员、组织和仓库复用同一不可变利润事实。"
      actions={
        <select
          value={dimension}
          onChange={(event) => setDimension(event.target.value)}
        >
          {[
            "customer",
            "sku",
            "product_category",
            "brand",
            "salesperson",
            "business_unit",
            "legal_entity",
            "warehouse",
          ].map((value) => (
            <option key={value} value={value}>
              {value}
            </option>
          ))}
        </select>
      }
    >
      <State state={state}>
        <div className="sheet">
          {state.data?.items.map((row) => (
            <article
              className="ledger-row"
              key={`${row.dimensionOne}:${row.dimensionOneId ?? "unassigned"}:${row.dimensionTwo ?? ""}:${row.dimensionTwoId ?? ""}`}
            >
              <code>
                {row.dimensionOneId ? short(row.dimensionOneId) : "未归属"}
              </code>
              <strong>
                {formatMoney(row.currency, row.netRevenue)}
              </strong>
              <span>毛利 {formatAmount(row.grossProfit)}</span>
              <span>贡献利润 {formatAmount(row.contributionProfit)}</span>
              <div>
                <strong>
                  经营利润 {formatAmount(row.managementOperatingProfit)}
                </strong>
                <Status value={row.dataQualityStatus} />
              </div>
            </article>
          ))}
        </div>
      </State>
    </Page>
  );
}

function ProfitAdjustments({ id }: { id?: string }) {
  const [revision, setRevision] = React.useState(0);
  const state = useLoad(
    () =>
      request<Envelope<ProfitAdjustment>>(
        "/api/v1/profit-adjustments?limit=200",
      ),
    [revision],
  );
  const rows =
    state.data?.items.filter(
      (item) => !id || item.id === id || item.adjustmentNumber === id,
    ) ?? [];
  return (
    <Page
      eyebrow="管理调整 / Allocation"
      title={id ? "经营费用批次" : "经营费用归集"}
      caption="草稿先预览；过账必须匹配预览哈希和事实水位，失效后明确返回 STALE_PREVIEW。"
    >
      <CommandConsole
        title="创建经营调整草稿"
        defaultPath="/api/v1/profit-adjustments"
        defaultMethod="POST"
        sample={{
          legalEntityId: "",
          currency: "CNY",
          managementPeriod: currentPeriod(),
          lines: [
            {
              metricType: "outbound_freight",
              amount: "0.00",
              businessDate: today(),
              allocationBasis: "direct",
              directSalesOrderId: "",
              reasonCode: "OPERATING_COST",
            },
          ],
        }}
        onDone={() => setRevision((value) => value + 1)}
      />
      <State state={state}>
        <div className="compact-list">
          {rows.map((item) => (
            <article key={item.id}>
              <a href={`/profit-adjustments/${item.id}`}>
                {item.adjustmentNumber}
              </a>
              <Status value={item.status} />
              <span>
                {item.managementPeriod} · {item.currency}
              </span>
              <div className="row-actions">
                <em>v{item.version}</em>
                {item.status === "draft" && (
                  <CommandButton
                    label="预览"
                    path={`/api/v1/profit-adjustments/${item.id}/preview`}
                    body={{ expectedVersion: item.version }}
                    onDone={() => setRevision((value) => value + 1)}
                  />
                )}
                {item.status === "posted" && (
                  <CommandButton
                    label="冲销"
                    path={`/api/v1/profit-adjustments/${item.id}/reverse`}
                    body={{ expectedVersion: item.version }}
                    onDone={() => setRevision((value) => value + 1)}
                  />
                )}
              </div>
            </article>
          ))}
        </div>
      </State>
    </Page>
  );
}

function ManagementReports({ id }: { id?: string }) {
  const [revision, setRevision] = React.useState(0);
  const period = currentPeriod();
  const state = useLoad(
    async () => ({
      report: await request<ManagementReport>(
        `/api/v1/management-profit-report?managementPeriod=${period}&currency=CNY`,
      ),
      snapshots: id
        ? await request<Envelope<ManagementSnapshot>>(
            `/api/v1/management-report-snapshots/${encodeURIComponent(id)}`,
          )
        : await request<Envelope<ManagementSnapshot>>(
            "/api/v1/management-report-snapshots?limit=100",
          ),
    }),
    [id, revision, period],
  );
  return (
    <Page
      eyebrow="管理报表 / Immutable snapshot"
      title="管理利润报表"
      caption="当前报表随事实更新；正式留痕使用不可变快照、来源水位与内容哈希。"
    >
      <CommandConsole
        title="生成管理报表快照"
        defaultPath="/api/v1/management-report-snapshots"
        defaultMethod="POST"
        sample={{
          reportType: "management_profit_statement",
          managementPeriod: period,
          currency: "CNY",
          legalEntityIds: [],
        }}
        onDone={() => setRevision((value) => value + 1)}
      />
      <State state={state}>
        {state.data && (
          <>
            <div className="balance-callout">
              <span>{period} 管理经营利润</span>
              <strong>
                {formatMoney(
                  state.data.report.currency,
                  state.data.report.rows.managementOperatingProfit ?? "0",
                )}
              </strong>
              <small>
                未分配费用{" "}
                {formatAmount(state.data.report.unallocatedOperatingExpense)} ·
                水位 {state.data.report.sourceWatermark}
              </small>
            </div>
            <div className="compact-list">
              {state.data.snapshots.items.map((item) => (
                <article key={item.id}>
                  <a href={`/management-reports/${item.id}`}>
                    {item.snapshotNumber}
                  </a>
                  <Status value={item.status} />
                  <span>
                    {item.managementPeriod} · 水位 {item.sourceWatermark}
                  </span>
                  <code>{short(item.sourceHash)}</code>
                </article>
              ))}
            </div>
          </>
        )}
      </State>
    </Page>
  );
}

function CommandButton({
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
      className="mini"
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
      {busy ? "…" : label}
    </button>
  );
}

function CommandConsole({
  title,
  defaultPath,
  defaultMethod,
  sample,
  onDone,
}: {
  title: string;
  defaultPath: string;
  defaultMethod: "POST" | "PUT";
  sample: unknown;
  onDone: () => void;
}) {
  const sampleText = JSON.stringify(sample, null, 2);
  const [open, setOpen] = React.useState(false);
  const [path, setPath] = React.useState(defaultPath);
  const [method, setMethod] = React.useState(defaultMethod);
  const [body, setBody] = React.useState(sampleText);
  const [result, setResult] = React.useState("");
  React.useEffect(() => {
    setPath(defaultPath);
    setMethod(defaultMethod);
    setBody(sampleText);
  }, [defaultPath, defaultMethod, sampleText]);
  return (
    <details
      className="command"
      open={open}
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary>
        {title}
        <span>BusinessSession 安全命令</span>
      </summary>
      <form
        onSubmit={async (event) => {
          event.preventDefault();
          try {
            const output = await request<unknown>(path, {
              method,
              body: JSON.stringify(JSON.parse(body)),
            });
            setResult(JSON.stringify(output, null, 2));
            onDone();
          } catch (error) {
            setResult((error as Error).message);
          }
        }}
      >
        <div className="command-route">
          <select
            value={method}
            onChange={(event) =>
              setMethod(event.target.value as "POST" | "PUT")
            }
          >
            <option>POST</option>
            <option>PUT</option>
          </select>
          <input
            aria-label="API path"
            value={path}
            onChange={(event) => setPath(event.target.value)}
          />
        </div>
        <textarea
          aria-label="JSON request"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          value={body}
          onChange={(event) => setBody(event.target.value)}
        />
        <button type="submit">校验并执行</button>
        {result && <pre>{result}</pre>}
      </form>
    </details>
  );
}

function State<T>({
  state,
  children,
}: React.PropsWithChildren<{ state: LoadState<T | null> }>) {
  if (state.loading) return <div className="loading">正在核对权威账簿…</div>;
  if (state.error)
    return (
      <div className="error">
        <strong>无法读取业务数据</strong>
        <span>{state.error}</span>
      </div>
    );
  return children;
}
function Status({ value }: { value: string }) {
  return (
    <span
      className={`status ${value.includes("hold") || value.includes("逾期") ? "warn" : ""}`}
    >
      {value.replaceAll("_", " ")}
    </span>
  );
}
function Empty({ text }: { text: string }) {
  return (
    <div className="empty">
      <span>空</span>
      <p>{text}</p>
    </div>
  );
}
function short(value: string) {
  return value.length > 16 ? `${value.slice(0, 8)}…${value.slice(-4)}` : value;
}
function today() {
  return new Date().toISOString().slice(0, 10);
}
function currentPeriod() {
  return new Date().toISOString().slice(0, 7);
}

connectBusinessDockAuthBridge();
const root = document.getElementById("root");
if (!root) throw new Error("Business Web root element is missing");
createRoot(root).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
