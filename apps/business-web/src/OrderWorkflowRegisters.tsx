import type React from "react";
import { formatAmount } from "./formatters";
import type {
  BusinessReturn,
  GoodsReceipt,
  Payable,
  PurchaseOrder,
  Receipt,
  Receivable,
  SalesOrder,
  Shipment,
  SupplierPayment,
} from "./api";
import {
  goodsReceiptDetail,
  payableDetail,
  paymentDetail,
  purchaseOrderDetail,
  receivableDetail,
  receiptDetail,
  returnReason,
  returnDetail,
  salesOrderDetail,
  shipmentDetail,
  statusLabel,
  type RecordDetailAction,
} from "./OrderWorkflowRecordDetails";

export type RegisterModalAction =
  | RecordDetailAction
  | { kind: "sales-confirm"; id: string; number: string }
  | { kind: "shipment-confirm"; id: string; number: string }
  | { kind: "purchase-edit"; id: string; number: string }
  | { kind: "purchase-confirm"; id: string; number: string }
  | { kind: "receipt-confirm"; id: string; number: string }
  | { kind: "customer-receipt-settle"; receipt: Receipt }
  | { kind: "supplier-payment-settle"; payment: SupplierPayment }
  | { kind: "sales-return-confirm"; item: BusinessReturn }
  | { kind: "purchase-return-confirm"; item: BusinessReturn }
  | { kind: "sales-return-inspect"; item: BusinessReturn }
  | { kind: "purchase-return-dispatch"; item: BusinessReturn }
  | { kind: "purchase-return-acknowledge"; item: BusinessReturn }
  | {
      kind: "command";
      title: string;
      description: string;
      path: string;
      body: Record<string, unknown>;
      confirmLabel: string;
      tone?: "danger" | "default";
    };

type OpenModal = (state: RegisterModalAction) => void;

export function SalesOrdersRegister({
  rows,
  onModal,
}: {
  rows: SalesOrder[];
  onModal: OpenModal;
}) {
  return (
    <Register
      columns={["销售订单", "订单金额", "订单状态", "履约状态", "下一步"]}
      empty="暂无销售订单。点击“新增销售订单”建立第一笔客户承诺。"
      count={rows.length}
    >
      {rows.map((row) => (
        <WorkflowRow key={row.id} onOpen={() => onModal(salesOrderDetail(row))}>
          <DocumentCell
            number={row.orderNumber}
            date={row.orderDate}
            id={row.customerId}
            onOpen={() => onModal(salesOrderDetail(row))}
          />
          <MoneyCell currency={row.currency} amount={row.grossAmount} />
          <StatusBadge
            value={
              row.holdStatus === "none" ? row.lifecycleStatus : row.holdStatus
            }
          />
          <StatusBadge value={row.fulfillmentStatus} />
          <div className="workflow-row-actions">
            {row.lifecycleStatus === "draft" && (
              <button
                type="button"
                onClick={() =>
                  onModal({
                    kind: "sales-confirm",
                    id: row.id,
                    number: row.orderNumber,
                  })
                }
              >
                确认订单
              </button>
            )}
            {row.lifecycleStatus !== "draft" &&
              row.holdStatus === "none" &&
              row.fulfillmentStatus !== "shipped" && (
                <button
                  type="button"
                  className="secondary"
                  onClick={() =>
                    onModal(
                      command(
                        "暂停履约",
                        "暂停后不可继续创建出库单，已产生的业务事实不会被改写。",
                        `/api/v1/sales-orders/${row.id}/manual-review-hold`,
                        {
                          expectedVersion: row.version,
                          reasonCode: "MANUAL_REVIEW",
                        },
                        "确认暂停",
                      ),
                    )
                  }
                >
                  暂停
                </button>
              )}
            {row.holdStatus !== "none" && (
              <button
                type="button"
                onClick={() =>
                  onModal(
                    command(
                      "恢复履约",
                      "恢复后订单可继续按库存预占创建出库单。",
                      `/api/v1/sales-orders/${row.id}/release-manual-review-hold`,
                      {
                        expectedVersion: row.version,
                        reasonCode: "REVIEW_COMPLETE",
                      },
                      "确认恢复",
                    ),
                  )
                }
              >
                恢复
              </button>
            )}
            {row.lifecycleStatus !== "draft" &&
              row.fulfillmentStatus !== "shipped" && (
                <button
                  type="button"
                  className="secondary danger"
                  onClick={() =>
                    onModal(
                      command(
                        "取消订单剩余量",
                        "仅取消尚未出库的剩余数量；已确认出库及其应收事实保持不变。",
                        `/api/v1/sales-orders/${row.id}/cancel-remaining`,
                        { expectedVersion: row.version },
                        "确认取消剩余",
                        "danger",
                      ),
                    )
                  }
                >
                  取消剩余
                </button>
              )}
            <button
              type="button"
              className="secondary"
              onClick={() => onModal(salesOrderDetail(row))}
              aria-label={`查看销售订单 ${row.orderNumber}`}
            >
              查看详情
            </button>
          </div>
        </WorkflowRow>
      ))}
    </Register>
  );
}

export function ShipmentsRegister({
  rows,
  onModal,
}: {
  rows: Shipment[];
  onModal: OpenModal;
}) {
  return (
    <Register
      columns={["销售出库单", "关联订单", "出库日期", "状态", "下一步"]}
      empty="暂无出库单。已确认且有可用预占的订单可以创建出库草稿。"
      count={rows.length}
    >
      {rows.map((row) => (
        <WorkflowRow key={row.id} onOpen={() => onModal(shipmentDetail(row))}>
          <DocumentCell
            number={row.shipmentNumber}
            date={row.updatedAt}
            id={row.warehouseId}
            onOpen={() => onModal(shipmentDetail(row))}
          />
          <code>{compactId(row.salesOrderId)}</code>
          <span>{row.shipmentDate}</span>
          <StatusBadge value={row.status} />
          <div className="workflow-row-actions">
            {row.status === "draft" && (
              <button
                type="button"
                onClick={() =>
                  onModal({
                    kind: "shipment-confirm",
                    id: row.id,
                    number: row.shipmentNumber,
                  })
                }
              >
                确认出库
              </button>
            )}
            {row.status === "confirmed" && (
              <button
                type="button"
                className="secondary danger"
                onClick={() =>
                  onModal(
                    command(
                      "冲销销售出库",
                      "冲销会写入反向库存事实，并要求关联应收仍满足冲销条件。",
                      `/api/v1/shipments/${row.id}/reverse`,
                      {
                        expectedVersion: row.version,
                        reasonCode: "SHIPMENT_CORRECTION",
                      },
                      "确认冲销",
                      "danger",
                    ),
                  )
                }
              >
                冲销
              </button>
            )}
            <button
              type="button"
              className="secondary"
              onClick={() => onModal(shipmentDetail(row))}
            >
              查看详情
            </button>
          </div>
        </WorkflowRow>
      ))}
    </Register>
  );
}

export function ReceivablesRegister({
  rows,
  onModal,
}: {
  rows: Receivable[];
  onModal: OpenModal;
}) {
  return (
    <Register
      columns={["经营应收", "关联销售", "原始金额", "未收金额", "状态"]}
      empty="暂无经营应收。确认出库后，系统会在同一事务中生成应收。"
      count={rows.length}
    >
      {rows.map((row) => (
        <WorkflowRow key={row.id} onOpen={() => onModal(receivableDetail(row))}>
          <DocumentCell
            number={row.receivableNumber}
            date={`到期 ${row.dueDate}`}
            id={row.customerId}
            onOpen={() => onModal(receivableDetail(row))}
          />
          <code>{compactId(row.salesOrderId)}</code>
          <MoneyCell currency={row.currency} amount={row.originalAmount} />
          <MoneyCell
            currency={row.currency}
            amount={row.openAmount}
            emphasis={row.isOverdue}
          />
          <StatusBadge
            value={row.isOverdue ? `overdue_${row.overdueDays}` : row.status}
          />
        </WorkflowRow>
      ))}
    </Register>
  );
}

export function ReceiptsRegister({
  rows,
  onModal,
  onCreate,
}: {
  rows: Receipt[];
  onModal: OpenModal;
  onCreate: () => void;
}) {
  return (
    <Register
      columns={["客户收款", "收款日期", "收款金额", "未核销", "状态"]}
      empty="暂无客户收款。登记收款后可按同一主体、客户和币种核销经营应收。"
      count={rows.length}
      action={
        <button type="button" onClick={onCreate}>
          + 登记客户收款
        </button>
      }
    >
      {rows.map((row) => (
        <WorkflowRow key={row.id} onOpen={() => onModal(receiptDetail(row))}>
          <DocumentCell
            number={row.receiptNumber}
            date={row.updatedAt}
            id={row.customerId}
            onOpen={() => onModal(receiptDetail(row))}
          />
          <span>{row.receiptDate}</span>
          <MoneyCell currency={row.currency} amount={row.amount} />
          <MoneyCell currency={row.currency} amount={row.unappliedAmount} />
          <div className="workflow-row-actions">
            <StatusBadge value={row.status} />
            <button
              type="button"
              onClick={() =>
                onModal({ kind: "customer-receipt-settle", receipt: row })
              }
            >
              {row.status === "draft"
                ? "确认收款"
                : ["fully_allocated", "reversed"].includes(row.status)
                  ? "查看"
                  : "核销"}
            </button>
          </div>
        </WorkflowRow>
      ))}
    </Register>
  );
}

export function PurchaseOrdersRegister({
  rows,
  onModal,
}: {
  rows: PurchaseOrder[];
  onModal: OpenModal;
}) {
  return (
    <Register
      columns={["采购订单", "订单金额", "承诺状态", "到货状态", "下一步"]}
      empty="暂无采购订单。点击“新增采购订单”建立第一笔供应承诺。"
      count={rows.length}
    >
      {rows.map((row) => (
        <WorkflowRow
          key={row.id}
          onOpen={() => onModal(purchaseOrderDetail(row))}
        >
          <DocumentCell
            number={row.purchaseOrderNumber}
            date={row.orderDate}
            id={row.supplierId}
            onOpen={() => onModal(purchaseOrderDetail(row))}
          />
          <MoneyCell currency={row.currency} amount={row.grossAmount} />
          <StatusBadge value={row.lifecycleStatus} />
          <StatusBadge value={row.receivingStatus} />
          <div className="workflow-row-actions">
            {row.lifecycleStatus === "draft" && (
              <>
                <button
                  type="button"
                  className="secondary"
                  onClick={() =>
                    onModal({
                      kind: "purchase-edit",
                      id: row.id,
                      number: row.purchaseOrderNumber,
                    })
                  }
                >
                  编辑
                </button>
                <button
                  type="button"
                  onClick={() =>
                    onModal({
                      kind: "purchase-confirm",
                      id: row.id,
                      number: row.purchaseOrderNumber,
                    })
                  }
                >
                  确认订单
                </button>
              </>
            )}
            {row.lifecycleStatus !== "draft" &&
              row.receivingStatus !== "fully_received" && (
                <button
                  type="button"
                  className="secondary danger"
                  onClick={() =>
                    onModal(
                      command(
                        "取消采购剩余量",
                        "仅取消尚未到货的剩余数量；已确认入库及其库存、应付事实保持不变。",
                        `/api/v1/purchase-orders/${row.id}/cancel-remaining`,
                        { expectedVersion: row.version },
                        "确认取消剩余",
                        "danger",
                      ),
                    )
                  }
                >
                  取消剩余
                </button>
              )}
            <button
              type="button"
              className="secondary"
              onClick={() => onModal(purchaseOrderDetail(row))}
            >
              查看详情
            </button>
          </div>
        </WorkflowRow>
      ))}
    </Register>
  );
}

export function GoodsReceiptsRegister({
  rows,
  onModal,
}: {
  rows: GoodsReceipt[];
  onModal: OpenModal;
}) {
  return (
    <Register
      columns={["采购收货单", "关联采购", "暂估成本", "状态", "下一步"]}
      empty="暂无采购收货单。已确认且仍有可收数量的采购订单可以登记到货。"
      count={rows.length}
    >
      {rows.map((row) => (
        <WorkflowRow
          key={row.id}
          onOpen={() => onModal(goodsReceiptDetail(row))}
        >
          <DocumentCell
            number={row.goodsReceiptNumber}
            date={row.receiptDate}
            id={row.supplierId}
            onOpen={() => onModal(goodsReceiptDetail(row))}
          />
          <code>{compactId(row.purchaseOrderId)}</code>
          <MoneyCell currency={row.currency} amount={row.inventoryCostAmount} />
          <StatusBadge value={row.status} />
          <div className="workflow-row-actions">
            {row.status === "draft" && (
              <button
                type="button"
                onClick={() =>
                  onModal({
                    kind: "receipt-confirm",
                    id: row.id,
                    number: row.goodsReceiptNumber,
                  })
                }
              >
                确认入库
              </button>
            )}
            {row.status === "confirmed" && (
              <button
                type="button"
                className="secondary danger"
                onClick={() =>
                  onModal(
                    command(
                      "冲销采购入库",
                      "冲销会写入反向库存与应付事实，不会覆盖原始收货凭据。",
                      `/api/v1/goods-receipts/${row.id}/reverse`,
                      {
                        expectedVersion: row.version,
                        reasonCode: "RECEIPT_CORRECTION",
                      },
                      "确认冲销",
                      "danger",
                    ),
                  )
                }
              >
                冲销
              </button>
            )}
            <button
              type="button"
              className="secondary"
              onClick={() => onModal(goodsReceiptDetail(row))}
            >
              查看详情
            </button>
          </div>
        </WorkflowRow>
      ))}
    </Register>
  );
}

export function PayablesRegister({
  rows,
  onModal,
}: {
  rows: Payable[];
  onModal: OpenModal;
}) {
  return (
    <Register
      columns={["经营应付", "关联采购", "原始金额", "未付金额", "状态"]}
      empty="暂无经营应付。确认采购入库后，系统会在同一事务中生成应付。"
      count={rows.length}
    >
      {rows.map((row) => (
        <WorkflowRow key={row.id} onOpen={() => onModal(payableDetail(row))}>
          <DocumentCell
            number={row.payableNumber}
            date={`到期 ${row.dueDate}`}
            id={row.supplierId}
            onOpen={() => onModal(payableDetail(row))}
          />
          <code>{compactId(row.purchaseOrderId)}</code>
          <MoneyCell currency={row.currency} amount={row.originalAmount} />
          <MoneyCell
            currency={row.currency}
            amount={row.openAmount}
            emphasis={row.isOverdue}
          />
          <StatusBadge
            value={row.isOverdue ? `overdue_${row.overdueDays}` : row.status}
          />
        </WorkflowRow>
      ))}
    </Register>
  );
}

export function PaymentsRegister({
  rows,
  onModal,
  onCreate,
}: {
  rows: SupplierPayment[];
  onModal: OpenModal;
  onCreate: () => void;
}) {
  return (
    <Register
      columns={["供应商付款", "付款日期", "付款金额", "未核销", "状态"]}
      empty="暂无供应商付款。登记付款后可按同一主体、供应商和币种核销经营应付。"
      count={rows.length}
      action={
        <button type="button" onClick={onCreate}>
          + 登记供应商付款
        </button>
      }
    >
      {rows.map((row) => (
        <WorkflowRow key={row.id} onOpen={() => onModal(paymentDetail(row))}>
          <DocumentCell
            number={row.supplierPaymentNumber}
            date={row.updatedAt}
            id={row.supplierId}
            onOpen={() => onModal(paymentDetail(row))}
          />
          <span>{row.paymentDate}</span>
          <MoneyCell currency={row.currency} amount={row.amount} />
          <MoneyCell currency={row.currency} amount={row.unappliedAmount} />
          <div className="workflow-row-actions">
            <StatusBadge value={row.status} />
            <button
              type="button"
              onClick={() =>
                onModal({ kind: "supplier-payment-settle", payment: row })
              }
            >
              {row.status === "draft"
                ? "确认付款"
                : ["fully_allocated", "reversed"].includes(row.status)
                  ? "查看"
                  : "核销"}
            </button>
          </div>
        </WorkflowRow>
      ))}
    </Register>
  );
}

export function ReturnsRegister({
  rows,
  side,
  onModal,
  onCreate,
}: {
  rows: BusinessReturn[];
  side: "sales" | "purchase";
  onModal: OpenModal;
  onCreate: () => void;
}) {
  const sales = side === "sales";
  return (
    <Register
      columns={[
        sales ? "销售退货单" : "采购退货单",
        "来源凭据",
        "退货金额",
        "退货原因",
        "状态 / 下一步",
      ]}
      empty={`暂无${sales ? "销售" : "采购"}退货。只有已确认且仍有可退数量的${sales ? "出库" : "收货"}凭据可以发起退货。`}
      count={rows.length}
      action={
        <button type="button" onClick={onCreate}>
          + 新增{sales ? "销售" : "采购"}退货
        </button>
      }
    >
      {rows.map((row) => (
        <WorkflowRow
          key={row.id}
          onOpen={() => onModal(returnDetail(row, side))}
        >
          <DocumentCell
            number={row.returnNumber}
            date={row.returnDate}
            id={row.partnerId}
            onOpen={() => onModal(returnDetail(row, side))}
          />
          <code>{compactId(row.sourceId)}</code>
          <MoneyCell currency={row.currency} amount={row.amount} />
          <span>{returnReason(row.reasonCode)}</span>
          <div className="workflow-row-actions">
            <StatusBadge value={row.status} />
            {row.status === "confirmed" && (
              <StatusBadge value={row.workflowStatus} />
            )}
            {row.status === "draft" && (
              <>
                <button
                  type="button"
                  className="secondary"
                  onClick={() =>
                    onModal(
                      command(
                        `作废${sales ? "销售" : "采购"}退货草稿`,
                        "作废后释放已占用的可退数量；草稿及操作轨迹仍会保留。",
                        `/api/v1/${sales ? "sales-returns" : "purchase-returns"}/${row.id}/cancel`,
                        { expectedVersion: row.version },
                        "确认作废",
                        "danger",
                      ),
                    )
                  }
                >
                  作废
                </button>
                <button
                  type="button"
                  onClick={() =>
                    onModal({
                      kind: sales
                        ? "sales-return-confirm"
                        : "purchase-return-confirm",
                      item: row,
                    })
                  }
                >
                  确认退货
                </button>
              </>
            )}
            {sales &&
              row.status === "confirmed" &&
              row.workflowStatus === "pending" && (
                <button
                  type="button"
                  onClick={() =>
                    onModal({ kind: "sales-return-inspect", item: row })
                  }
                >
                  质检处置
                </button>
              )}
            {!sales &&
              row.status === "confirmed" &&
              row.workflowStatus === "not_dispatched" && (
                <button
                  type="button"
                  onClick={() =>
                    onModal({ kind: "purchase-return-dispatch", item: row })
                  }
                >
                  登记发出
                </button>
              )}
            {!sales &&
              row.status === "confirmed" &&
              row.workflowStatus === "dispatched" && (
                <button
                  type="button"
                  onClick={() =>
                    onModal({
                      kind: "purchase-return-acknowledge",
                      item: row,
                    })
                  }
                >
                  供应商签收
                </button>
              )}
          </div>
        </WorkflowRow>
      ))}
    </Register>
  );
}

function Register({
  columns,
  empty,
  count,
  action,
  children,
}: React.PropsWithChildren<{
  columns: string[];
  empty: string;
  count: number;
  action?: React.ReactNode;
}>) {
  return (
    <section className="workflow-table">
      <header>
        <div>
          <strong>{columns[0]}</strong>
          <span>{count} 条业务记录</span>
        </div>
        {action}
      </header>
      <div className="workflow-columns" aria-hidden="true">
        {columns.map((column) => (
          <span key={column}>{column}</span>
        ))}
      </div>
      {count > 0 ? (
        children
      ) : (
        <div className="workflow-empty">
          <DocumentIcon />
          <strong>还没有业务凭据</strong>
          <span>{empty}</span>
        </div>
      )}
    </section>
  );
}

function WorkflowRow({
  onOpen,
  children,
}: React.PropsWithChildren<{ onOpen: () => void }>) {
  return (
    <article
      className="workflow-row workflow-row-clickable"
      onClick={(event) => {
        const target = event.target;
        if (
          target instanceof Element &&
          target.closest("button, a, input, select, textarea")
        ) {
          return;
        }
        onOpen();
      }}
    >
      {children}
    </article>
  );
}

function DocumentCell({
  number,
  date,
  id,
  onOpen,
}: {
  number: string;
  date: string;
  id: string;
  onOpen: () => void;
}) {
  return (
    <button
      type="button"
      className="document-cell workflow-record-trigger"
      onClick={onOpen}
      aria-label={`查看 ${number} 详情`}
    >
      <strong>{number}</strong>
      <span>{date}</span>
      <code>{compactId(id)}</code>
    </button>
  );
}

function MoneyCell({
  currency,
  amount,
  emphasis = false,
}: {
  currency: string;
  amount: string;
  emphasis?: boolean;
}) {
  return (
    <div className={`money-cell ${emphasis ? "attention" : ""}`}>
      <small>{currency}</small>
      <strong>
        {formatAmount(amount)}
      </strong>
    </div>
  );
}

function StatusBadge({ value }: { value: string }) {
  return (
    <span className={`workflow-status ${statusTone(value)}`}>
      <i />
      {statusLabel(value)}
    </span>
  );
}

function command(
  title: string,
  description: string,
  path: string,
  body: Record<string, unknown>,
  confirmLabel: string,
  tone: "danger" | "default" = "default",
): RegisterModalAction {
  return {
    kind: "command",
    title,
    description,
    path,
    body,
    confirmLabel,
    tone,
  };
}

function compactId(value: string) {
  return value.length > 14 ? `${value.slice(0, 8)}…${value.slice(-4)}` : value;
}

function statusTone(value: string) {
  if (/overdue|blocked|reversed|cancel/.test(value)) return "bad";
  if (/draft|partial|hold|open|pending|not_dispatched/.test(value))
    return "warn";
  if (
    /confirmed|shipped|received|settled|allocated|ready|completed|acknowledged|dispatched/.test(
      value,
    )
  )
    return "good";
  return "neutral";
}

function DocumentIcon() {
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
      <path d="M6 3h8l4 4v14H6zM14 3v5h5M9 13h6M9 17h4" />
    </svg>
  );
}
