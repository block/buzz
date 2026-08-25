import React from "react";
import {
  type BusinessReturn,
  type Envelope,
  type GoodsReceipt,
  type Payable,
  type PurchaseOrder,
  type Receipt,
  type Receivable,
  type SalesOrder,
  type Shipment,
  type SupplierPayment,
  request,
} from "./api";
import { formatAmount } from "./formatters";
import { GoodsReceiptConfirmation } from "./GoodsReceiptConfirmation";
import { GoodsReceiptEntry } from "./GoodsReceiptEntry";
import {
  GoodsReceiptsRegister,
  PayablesRegister,
  PaymentsRegister,
  PurchaseOrdersRegister,
  ReceiptsRegister,
  ReceivablesRegister,
  ReturnsRegister,
  SalesOrdersRegister,
  ShipmentsRegister,
} from "./OrderWorkflowRegisters";
import {
  RecordDetail,
  WorkflowModal,
  type WorkflowModalState,
} from "./OrderWorkflowModal";
import { PurchaseOrderConfirmation } from "./PurchaseOrderConfirmation";
import { PurchaseOrderEntry } from "./PurchaseOrderEntry";
import { PurchaseDeliveryPanel } from "./PurchaseDeliveryPanel";
import { SalesOrderConfirmation } from "./SalesOrderConfirmation";
import { SalesOrderEntry } from "./SalesOrderEntry";
import { PurchaseReturnEntry, SalesReturnEntry } from "./ReturnEntry";
import {
  PurchaseReturnAcknowledgment,
  PurchaseReturnDispatch,
  ReturnAnalyticsPanel,
  SalesReturnInspection,
} from "./ReturnDispositionForms";
import { ShipmentConfirmation } from "./ShipmentConfirmation";
import { ShipmentEntry } from "./ShipmentEntry";
import {
  CustomerReceiptEntry,
  CustomerReceiptSettlement,
  SupplierPaymentEntry,
  SupplierPaymentSettlement,
} from "./SettlementForms";
import "./order-workflows.css";
import "./order-workflow-detail.css";

type SalesTab =
  | "orders"
  | "fulfillment"
  | "receivables"
  | "settlement"
  | "returns";
type PurchaseTab =
  | "orders"
  | "delivery"
  | "receiving"
  | "payables"
  | "settlement"
  | "returns";
type ModalState = WorkflowModalState;

type SalesWorkflowData = {
  orders: SalesOrder[];
  shipments: Shipment[];
  receivables: Receivable[];
  receipts: Receipt[];
  returns: BusinessReturn[];
  errors: Partial<Record<SalesTab, string>>;
};

type PurchaseWorkflowData = {
  orders: PurchaseOrder[];
  receipts: GoodsReceipt[];
  payables: Payable[];
  payments: SupplierPayment[];
  returns: BusinessReturn[];
  errors: Partial<Record<PurchaseTab, string>>;
};

const salesStages: Array<{ id: SalesTab; code: string; label: string }> = [
  { id: "orders", code: "01", label: "销售订单" },
  { id: "fulfillment", code: "02", label: "出库履约" },
  { id: "receivables", code: "03", label: "经营应收" },
  { id: "settlement", code: "04", label: "收款核销" },
  { id: "returns", code: "05", label: "销售退货" },
];

const purchaseStages: Array<{ id: PurchaseTab; code: string; label: string }> =
  [
    { id: "orders", code: "01", label: "采购订单" },
    { id: "delivery", code: "02", label: "交期履约" },
    { id: "receiving", code: "03", label: "到货入库" },
    { id: "payables", code: "04", label: "经营应付" },
    { id: "settlement", code: "05", label: "付款核销" },
    { id: "returns", code: "06", label: "采购退货" },
  ];

export function SalesOrderWorkflowPage({ id }: { id?: string }) {
  const [tab, setTab] = React.useState<SalesTab>("orders");
  const [revision, setRevision] = React.useState(0);
  const [modal, setModal] = React.useState<ModalState | null>(null);
  const [query, setQuery] = React.useState(id ?? "");
  const state = useWorkflowData<SalesWorkflowData>(async () => {
    const [orders, shipments, receivables, receipts, returns] =
      await Promise.all([
        loadWorkflowStage<SalesOrder>("/api/v1/sales-orders?limit=200"),
        loadWorkflowStage<Shipment>("/api/v1/shipments?limit=200"),
        loadWorkflowStage<Receivable>("/api/v1/trade-receivables?limit=200"),
        loadWorkflowStage<Receipt>("/api/v1/customer-receipts?limit=200"),
        loadWorkflowStage<BusinessReturn>("/api/v1/sales-returns?limit=200"),
      ]);
    return {
      orders: orders.items,
      shipments: shipments.items,
      receivables: receivables.items,
      receipts: receipts.items,
      returns: returns.items,
      errors: compactErrors<SalesTab>({
        orders: orders.error,
        fulfillment: shipments.error,
        receivables: receivables.error,
        settlement: receipts.error,
        returns: returns.error,
      }),
    };
  }, [revision]);
  const data = state.data;
  const stageError = data?.errors[tab] ?? null;
  const search = query.trim().toLowerCase();
  const orders = filterRows(data?.orders ?? [], search, (item) => [
    item.orderNumber,
    item.customerId,
    item.lifecycleStatus,
  ]);
  const shipments = filterRows(data?.shipments ?? [], search, (item) => [
    item.shipmentNumber,
    item.salesOrderId,
    item.status,
  ]);
  const receivables = filterRows(data?.receivables ?? [], search, (item) => [
    item.receivableNumber,
    item.customerId,
    item.salesOrderId,
    item.status,
  ]);
  const receipts = filterRows(data?.receipts ?? [], search, (item) => [
    item.receiptNumber,
    item.customerId,
    item.status,
  ]);
  const returns = filterRows(data?.returns ?? [], search, (item) => [
    item.returnNumber,
    item.sourceId,
    item.partnerId,
    item.reasonCode,
    item.status,
  ]);
  const openReceivable = (data?.receivables ?? []).reduce(
    (total, item) => total + Number(item.openAmount),
    0,
  );
  const completedOrders = (data?.orders ?? []).filter(
    (item) => item.fulfillmentStatus === "shipped",
  ).length;
  const refresh = () => setRevision((value) => value + 1);
  const done = () => {
    setModal(null);
    refresh();
  };

  return (
    <WorkflowPage
      domain="sales"
      eyebrow="销售闭环 / Order to cash"
      title={id ? "销售订单全链路" : "销售订单闭环"}
      caption="从客户承诺、库存预占、分批出库到经营应收与收款核销，始终沿同一销售订单追溯。"
      primaryAction={
        <button
          type="button"
          onClick={() => setModal({ kind: "sales-create" })}
        >
          <PlusIcon /> 新增销售订单
        </button>
      }
      secondaryAction={
        <button
          type="button"
          className="secondary"
          onClick={() => setModal({ kind: "shipment-create" })}
        >
          <TruckIcon /> 新建出库单
        </button>
      }
    >
      <WorkflowRail
        active={tab}
        stages={salesStages}
        onSelect={(value) => setTab(value as SalesTab)}
        metrics={[
          workflowMetric(data?.errors.orders, `${data?.orders.length ?? 0} 单`),
          "实时",
          workflowMetric(
            data?.errors.fulfillment,
            `${data?.shipments.length ?? 0} 次`,
          ),
          workflowMetric(
            data?.errors.receivables,
            `待收 ${money(openReceivable)}`,
          ),
          workflowMetric(
            data?.errors.settlement,
            `${data?.receipts.length ?? 0} 笔`,
          ),
          workflowMetric(
            data?.errors.returns,
            `${data?.returns.length ?? 0} 笔`,
          ),
        ]}
      />
      <WorkflowPulse
        items={[
          {
            label: "订单总额",
            value: workflowValue(
              data?.errors.orders,
              money(sum(data?.orders, "grossAmount")),
            ),
            note: workflowNote(
              data?.errors.orders,
              `${data?.orders.length ?? 0} 张订单`,
            ),
          },
          {
            label: "已履约订单",
            value: workflowValue(data?.errors.orders, String(completedOrders)),
            note: workflowNote(
              data?.errors.orders,
              `履约率 ${ratio(completedOrders, data?.orders.length ?? 0)}`,
            ),
          },
          {
            label: "经营应收余额",
            value: workflowValue(
              data?.errors.receivables,
              money(openReceivable),
            ),
            note: workflowNote(
              data?.errors.receivables,
              `${(data?.receivables ?? []).filter((item) => item.status !== "settled").length} 笔未结`,
            ),
          },
        ]}
      />
      <WorkflowToolbar
        query={query}
        onQuery={setQuery}
        placeholder="搜索订单号、客户、出库单或应收单…"
        meta={
          state.loading ? "正在同步业务事实…" : `数据已同步 · v${revision + 1}`
        }
      />
      {state.error || stageError ? (
        <WorkflowError
          error={state.error ?? stageError ?? "业务数据加载失败"}
          onRetry={refresh}
        />
      ) : (
        <div className="workflow-register" aria-busy={state.loading}>
          {tab === "orders" && (
            <SalesOrdersRegister rows={orders} onModal={setModal} />
          )}
          {tab === "fulfillment" && (
            <ShipmentsRegister rows={shipments} onModal={setModal} />
          )}
          {tab === "receivables" && (
            <ReceivablesRegister rows={receivables} onModal={setModal} />
          )}
          {tab === "settlement" && (
            <ReceiptsRegister
              rows={receipts}
              onModal={setModal}
              onCreate={() => setModal({ kind: "customer-receipt-create" })}
            />
          )}
          {tab === "returns" && (
            <>
              <ReturnAnalyticsPanel side="sales" />
              <ReturnsRegister
                rows={returns}
                side="sales"
                onModal={setModal}
                onCreate={() => setModal({ kind: "sales-return-create" })}
              />
            </>
          )}
        </div>
      )}
      {modal && (
        <WorkflowModal state={modal} onClose={() => setModal(null)}>
          {modal.kind === "sales-create" && <SalesOrderEntry onDone={done} />}
          {modal.kind === "sales-confirm" && (
            <SalesOrderConfirmation orderId={modal.id} onDone={done} />
          )}
          {modal.kind === "shipment-create" && <ShipmentEntry onDone={done} />}
          {modal.kind === "shipment-confirm" && (
            <ShipmentConfirmation shipmentId={modal.id} onDone={done} />
          )}
          {modal.kind === "customer-receipt-create" && (
            <CustomerReceiptEntry onDone={done} />
          )}
          {modal.kind === "customer-receipt-settle" && (
            <CustomerReceiptSettlement
              receipt={modal.receipt}
              receivables={data?.receivables ?? []}
              onDone={done}
            />
          )}
          {modal.kind === "sales-return-create" && (
            <SalesReturnEntry onDone={done} />
          )}
          {modal.kind === "sales-return-confirm" && (
            <CommandConfirmation
              state={returnConfirmation(modal.item, "sales")}
              onCancel={() => setModal(null)}
              onDone={done}
            />
          )}
          {modal.kind === "sales-return-inspect" && (
            <SalesReturnInspection item={modal.item} onDone={done} />
          )}
          {modal.kind === "record-detail" && <RecordDetail state={modal} />}
          {modal.kind === "command" && (
            <CommandConfirmation
              state={modal}
              onCancel={() => setModal(null)}
              onDone={done}
            />
          )}
        </WorkflowModal>
      )}
    </WorkflowPage>
  );
}

export function PurchaseOrderWorkflowPage({ id }: { id?: string }) {
  const [tab, setTab] = React.useState<PurchaseTab>("orders");
  const [revision, setRevision] = React.useState(0);
  const [modal, setModal] = React.useState<ModalState | null>(null);
  const [query, setQuery] = React.useState(id ?? "");
  const state = useWorkflowData<PurchaseWorkflowData>(async () => {
    const [orders, receipts, payables, payments, returns] = await Promise.all([
      loadWorkflowStage<PurchaseOrder>("/api/v1/purchase-orders?limit=200"),
      loadWorkflowStage<GoodsReceipt>("/api/v1/goods-receipts?limit=200"),
      loadWorkflowStage<Payable>("/api/v1/trade-payables?limit=200"),
      loadWorkflowStage<SupplierPayment>("/api/v1/supplier-payments?limit=200"),
      loadWorkflowStage<BusinessReturn>("/api/v1/purchase-returns?limit=200"),
    ]);
    return {
      orders: orders.items,
      receipts: receipts.items,
      payables: payables.items,
      payments: payments.items,
      returns: returns.items,
      errors: compactErrors<PurchaseTab>({
        orders: orders.error,
        delivery: null,
        receiving: receipts.error,
        payables: payables.error,
        settlement: payments.error,
        returns: returns.error,
      }),
    };
  }, [revision]);
  const data = state.data;
  const stageError = data?.errors[tab] ?? null;
  const search = query.trim().toLowerCase();
  const orders = filterRows(data?.orders ?? [], search, (item) => [
    item.purchaseOrderNumber,
    item.supplierId,
    item.lifecycleStatus,
    item.receivingStatus,
  ]);
  const receipts = filterRows(data?.receipts ?? [], search, (item) => [
    item.goodsReceiptNumber,
    item.purchaseOrderId,
    item.supplierId,
    item.status,
  ]);
  const payables = filterRows(data?.payables ?? [], search, (item) => [
    item.payableNumber,
    item.purchaseOrderId,
    item.supplierId,
    item.status,
  ]);
  const payments = filterRows(data?.payments ?? [], search, (item) => [
    item.supplierPaymentNumber,
    item.supplierId,
    item.status,
  ]);
  const returns = filterRows(data?.returns ?? [], search, (item) => [
    item.returnNumber,
    item.sourceId,
    item.partnerId,
    item.reasonCode,
    item.status,
  ]);
  const openPayable = (data?.payables ?? []).reduce(
    (total, item) => total + Number(item.openAmount),
    0,
  );
  const receivedOrders = (data?.orders ?? []).filter(
    (item) => item.receivingStatus === "fully_received",
  ).length;
  const refresh = () => setRevision((value) => value + 1);
  const done = () => {
    setModal(null);
    refresh();
  };

  return (
    <WorkflowPage
      domain="purchase"
      eyebrow="采购闭环 / Procure to pay"
      title={id ? "采购订单全链路" : "采购订单闭环"}
      caption="从采购承诺、实际到货、移动平均成本到经营应付与付款核销，所有变化保留来源凭据。"
      primaryAction={
        <button
          type="button"
          onClick={() => setModal({ kind: "purchase-create" })}
        >
          <PlusIcon /> 新增采购订单
        </button>
      }
      secondaryAction={
        <button
          type="button"
          className="secondary"
          onClick={() => setModal({ kind: "receipt-create" })}
        >
          <ReceiveIcon /> 新建收货单
        </button>
      }
    >
      <WorkflowRail
        active={tab}
        stages={purchaseStages}
        onSelect={(value) => setTab(value as PurchaseTab)}
        metrics={[
          workflowMetric(data?.errors.orders, `${data?.orders.length ?? 0} 单`),
          workflowMetric(
            data?.errors.receiving,
            `${data?.receipts.length ?? 0} 次`,
          ),
          workflowMetric(data?.errors.payables, `待付 ${money(openPayable)}`),
          workflowMetric(
            data?.errors.settlement,
            `${data?.payments.length ?? 0} 笔`,
          ),
          workflowMetric(
            data?.errors.returns,
            `${data?.returns.length ?? 0} 笔`,
          ),
        ]}
      />
      <WorkflowPulse
        items={[
          {
            label: "采购总额",
            value: workflowValue(
              data?.errors.orders,
              money(sum(data?.orders, "grossAmount")),
            ),
            note: workflowNote(
              data?.errors.orders,
              `${data?.orders.length ?? 0} 张订单`,
            ),
          },
          {
            label: "已到齐订单",
            value: workflowValue(data?.errors.orders, String(receivedOrders)),
            note: workflowNote(
              data?.errors.orders,
              `到货率 ${ratio(receivedOrders, data?.orders.length ?? 0)}`,
            ),
          },
          {
            label: "经营应付余额",
            value: workflowValue(data?.errors.payables, money(openPayable)),
            note: workflowNote(
              data?.errors.payables,
              `${(data?.payables ?? []).filter((item) => item.status !== "settled").length} 笔未结`,
            ),
          },
        ]}
      />
      <WorkflowToolbar
        query={query}
        onQuery={setQuery}
        placeholder="搜索采购单、供应商、收货单或应付单…"
        meta={
          state.loading ? "正在同步业务事实…" : `数据已同步 · v${revision + 1}`
        }
      />
      {state.error || stageError ? (
        <WorkflowError
          error={state.error ?? stageError ?? "业务数据加载失败"}
          onRetry={refresh}
        />
      ) : (
        <div className="workflow-register" aria-busy={state.loading}>
          {tab === "orders" && (
            <PurchaseOrdersRegister rows={orders} onModal={setModal} />
          )}
          {tab === "delivery" && <PurchaseDeliveryPanel onChanged={refresh} />}
          {tab === "receiving" && (
            <GoodsReceiptsRegister rows={receipts} onModal={setModal} />
          )}
          {tab === "payables" && (
            <PayablesRegister rows={payables} onModal={setModal} />
          )}
          {tab === "settlement" && (
            <PaymentsRegister
              rows={payments}
              onModal={setModal}
              onCreate={() => setModal({ kind: "supplier-payment-create" })}
            />
          )}
          {tab === "returns" && (
            <>
              <ReturnAnalyticsPanel side="purchase" />
              <ReturnsRegister
                rows={returns}
                side="purchase"
                onModal={setModal}
                onCreate={() => setModal({ kind: "purchase-return-create" })}
              />
            </>
          )}
        </div>
      )}
      {modal && (
        <WorkflowModal state={modal} onClose={() => setModal(null)}>
          {modal.kind === "purchase-create" && (
            <PurchaseOrderEntry onDone={done} />
          )}
          {modal.kind === "purchase-edit" && (
            <PurchaseOrderEntry orderId={modal.id} onDone={done} />
          )}
          {modal.kind === "purchase-confirm" && (
            <PurchaseOrderConfirmation orderId={modal.id} onDone={done} />
          )}
          {modal.kind === "receipt-create" && (
            <GoodsReceiptEntry onDone={done} />
          )}
          {modal.kind === "receipt-confirm" && (
            <GoodsReceiptConfirmation receiptId={modal.id} onDone={done} />
          )}
          {modal.kind === "supplier-payment-create" && (
            <SupplierPaymentEntry onDone={done} />
          )}
          {modal.kind === "supplier-payment-settle" && (
            <SupplierPaymentSettlement
              payment={modal.payment}
              payables={data?.payables ?? []}
              onDone={done}
            />
          )}
          {modal.kind === "purchase-return-create" && (
            <PurchaseReturnEntry onDone={done} />
          )}
          {modal.kind === "purchase-return-confirm" && (
            <CommandConfirmation
              state={returnConfirmation(modal.item, "purchase")}
              onCancel={() => setModal(null)}
              onDone={done}
            />
          )}
          {modal.kind === "purchase-return-dispatch" && (
            <PurchaseReturnDispatch item={modal.item} onDone={done} />
          )}
          {modal.kind === "purchase-return-acknowledge" && (
            <PurchaseReturnAcknowledgment item={modal.item} onDone={done} />
          )}
          {modal.kind === "record-detail" && <RecordDetail state={modal} />}
          {modal.kind === "command" && (
            <CommandConfirmation
              state={modal}
              onCancel={() => setModal(null)}
              onDone={done}
            />
          )}
        </WorkflowModal>
      )}
    </WorkflowPage>
  );
}

function WorkflowPage({
  domain,
  eyebrow,
  title,
  caption,
  primaryAction,
  secondaryAction,
  children,
}: React.PropsWithChildren<{
  domain: "sales" | "purchase";
  eyebrow: string;
  title: string;
  caption: string;
  primaryAction: React.ReactNode;
  secondaryAction: React.ReactNode;
}>) {
  return (
    <section className={`page order-workflow ${domain}`}>
      <div className="page-head workflow-head">
        <div>
          <p>{eyebrow}</p>
          <h1>{title}</h1>
          <span>{caption}</span>
        </div>
        <div className="workflow-head-actions">
          {secondaryAction}
          {primaryAction}
        </div>
      </div>
      {children}
    </section>
  );
}

function WorkflowRail({
  active,
  stages,
  metrics,
  onSelect,
}: {
  active: string;
  stages: Array<{ id: string; code: string; label: string }>;
  metrics: string[];
  onSelect: (id: string) => void;
}) {
  return (
    <nav className="workflow-rail" aria-label="订单闭环阶段">
      {stages.map((stage, index) => (
        <React.Fragment key={stage.id}>
          <button
            type="button"
            className={active === stage.id ? "active" : ""}
            aria-current={active === stage.id ? "step" : undefined}
            onClick={() => onSelect(stage.id)}
          >
            <span>{stage.code}</span>
            <strong>{stage.label}</strong>
            <small>{metrics[index]}</small>
          </button>
          {index < stages.length - 1 && <i aria-hidden="true" />}
        </React.Fragment>
      ))}
    </nav>
  );
}

function WorkflowPulse({
  items,
}: {
  items: Array<{ label: string; value: string; note: string }>;
}) {
  return (
    <div className="workflow-pulse">
      {items.map((item) => (
        <div key={item.label}>
          <span>{item.label}</span>
          <strong>{item.value}</strong>
          <small>{item.note}</small>
        </div>
      ))}
      <div className="workflow-rule-note">
        <span>闭环规则</span>
        <strong>先确认，再形成业务事实</strong>
        <small>草稿不会改变库存、应收或应付</small>
      </div>
    </div>
  );
}

function WorkflowToolbar({
  query,
  onQuery,
  placeholder,
  meta,
}: {
  query: string;
  onQuery: (value: string) => void;
  placeholder: string;
  meta: string;
}) {
  return (
    <div className="workflow-toolbar">
      <label>
        <SearchIcon />
        <span className="sr-only">搜索业务单据</span>
        <input
          type="search"
          value={query}
          placeholder={placeholder}
          onChange={(event) => onQuery(event.target.value)}
        />
      </label>
      <small>
        <i /> {meta}
      </small>
    </div>
  );
}

function CommandConfirmation({
  state,
  onCancel,
  onDone,
}: {
  state: Extract<ModalState, { kind: "command" }>;
  onCancel: () => void;
  onDone: () => void;
}) {
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState("");
  return (
    <section
      className={`command-confirmation ${state.tone === "danger" ? "danger" : ""}`}
    >
      <div className="command-symbol">
        <ShieldIcon />
      </div>
      <h3>{state.title}</h3>
      <p>{state.description}</p>
      <dl>
        <div>
          <dt>控制方式</dt>
          <dd>版本校验 + 幂等命令</dd>
        </div>
        <div>
          <dt>记录方式</dt>
          <dd>审计日志与业务事实同步写入</dd>
        </div>
      </dl>
      {error && (
        <div className="workflow-inline-error" role="alert">
          {error}
        </div>
      )}
      <footer>
        <button type="button" className="secondary" onClick={onCancel}>
          取消
        </button>
        <button
          type="button"
          className={state.tone === "danger" ? "danger" : ""}
          disabled={busy}
          onClick={async () => {
            setBusy(true);
            setError("");
            try {
              await request(state.path, {
                method: "POST",
                body: JSON.stringify(state.body),
              });
              onDone();
            } catch (reason) {
              setError((reason as Error).message);
            } finally {
              setBusy(false);
            }
          }}
        >
          {busy ? "正在执行…" : state.confirmLabel}
        </button>
      </footer>
    </section>
  );
}

function WorkflowError({
  error,
  onRetry,
}: {
  error: string;
  onRetry: () => void;
}) {
  return (
    <div className="workflow-error" role="alert">
      <strong>业务数据暂不可用</strong>
      <span>{error}</span>
      <button type="button" className="secondary" onClick={onRetry}>
        重新加载
      </button>
    </div>
  );
}

function useWorkflowData<T>(
  loader: () => Promise<T>,
  deps: React.DependencyList,
) {
  const [state, setState] = React.useState<{
    data: T | null;
    loading: boolean;
    error: string | null;
  }>({ data: null, loading: true, error: null });
  React.useEffect(() => {
    let active = true;
    setState((current) => ({ ...current, loading: true, error: null }));
    loader()
      .then((data) => active && setState({ data, loading: false, error: null }))
      .catch(
        (error: Error) =>
          active &&
          setState({ data: null, loading: false, error: error.message }),
      );
    return () => {
      active = false;
    };
    // biome-ignore lint/correctness/useExhaustiveDependencies: caller owns the explicit reload keys.
  }, deps);
  return state;
}

async function loadWorkflowStage<T>(path: string): Promise<{
  items: T[];
  error: string | null;
}> {
  try {
    const response = await request<Envelope<T>>(path);
    return { items: response.items, error: null };
  } catch (reason) {
    return {
      items: [],
      error:
        reason instanceof Error ? reason.message : "业务数据加载失败，请重试",
    };
  }
}

function compactErrors<T extends string>(
  errors: Record<T, string | null>,
): Partial<Record<T, string>> {
  return Object.fromEntries(
    Object.entries(errors).filter((entry): entry is [string, string] =>
      Boolean(entry[1]),
    ),
  ) as Partial<Record<T, string>>;
}

function workflowMetric(error: string | undefined, value: string) {
  return error ? "暂不可用" : value;
}

function workflowValue(error: string | undefined, value: string) {
  return error ? "—" : value;
}

function workflowNote(error: string | undefined, value: string) {
  return error ? "当前账号无权读取" : value;
}

function filterRows<T>(
  rows: T[],
  search: string,
  terms: (row: T) => Array<string | null | undefined>,
) {
  if (!search) return rows;
  return rows.filter((row) =>
    terms(row).some((term) => term?.toLowerCase().includes(search)),
  );
}

function sum<T>(rows: T[] | undefined, key: keyof T) {
  return (rows ?? []).reduce((total, item) => total + Number(item[key]), 0);
}

function money(value: number) {
  return `¥ ${formatAmount(value)}`;
}

function ratio(value: number, total: number) {
  return total === 0 ? "—" : `${Math.round((value / total) * 100)}%`;
}

function returnConfirmation(
  item: BusinessReturn,
  side: "sales" | "purchase",
): Extract<ModalState, { kind: "command" }> {
  const sales = side === "sales";
  return {
    kind: "command",
    title: `确认${sales ? "销售" : "采购"}退货`,
    description: sales
      ? "确认后商品按原出库冻结成本入库，并冲减对应未结经营应收与订单利润事实。"
      : "确认后商品按当前移动平均成本出库，并按原收货价税金额冲减对应未结经营应付。",
    path: `/api/v1/${sales ? "sales-returns" : "purchase-returns"}/${item.id}/confirm`,
    body: { expectedVersion: item.version },
    confirmLabel: "确认退货并写入业务事实",
  };
}

function Icon({ children }: React.PropsWithChildren) {
  return (
    <svg
      viewBox="0 0 24 24"
      aria-hidden="true"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {children}
    </svg>
  );
}

function PlusIcon() {
  return (
    <Icon>
      <path d="M12 5v14M5 12h14" />
    </Icon>
  );
}

function SearchIcon() {
  return (
    <Icon>
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-4-4" />
    </Icon>
  );
}

function TruckIcon() {
  return (
    <Icon>
      <path d="M3 6h11v10H3zM14 10h4l3 3v3h-7z" />
      <circle cx="7" cy="18" r="2" />
      <circle cx="18" cy="18" r="2" />
    </Icon>
  );
}

function ReceiveIcon() {
  return (
    <Icon>
      <path d="M4 4h16v5H4zM6 9v11h12V9M9 13h6M12 10v7" />
    </Icon>
  );
}

function ShieldIcon() {
  return (
    <Icon>
      <path d="M12 3 5 6v5c0 4.6 2.8 8.1 7 10 4.2-1.9 7-5.4 7-10V6z" />
      <path d="m9 12 2 2 4-4" />
    </Icon>
  );
}
