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
import { formatMoney } from "./formatters.ts";

export type RecordDetailAction = {
  kind: "record-detail";
  domain: "sales" | "purchase";
  title: string;
  subtitle: string;
  fields: Array<{
    label: string;
    value: string;
    format?: "text" | "money" | "status" | "id";
  }>;
};

export function salesOrderDetail(row: SalesOrder): RecordDetailAction {
  return recordDetail(
    "sales",
    `销售订单 · ${row.orderNumber}`,
    "客户承诺与履约状态",
    [
      detailField("订单编号", row.orderNumber),
      detailField(
        "含税总额",
        formatMoney(row.currency, row.grossAmount),
        "money",
      ),
      detailField("订单状态", statusLabel(row.lifecycleStatus), "status"),
      detailField("履约状态", statusLabel(row.fulfillmentStatus), "status"),
      detailField("冻结状态", statusLabel(row.holdStatus), "status"),
      detailField("订单日期", row.orderDate),
      detailField("客户 ID", row.customerId, "id"),
      detailField("经营主体 ID", row.legalEntityId, "id"),
      detailField("记录版本", String(row.version)),
      detailField("最近更新", formatDateTime(row.updatedAt)),
    ],
  );
}

export function shipmentDetail(row: Shipment): RecordDetailAction {
  return recordDetail(
    "sales",
    `销售出库单 · ${row.shipmentNumber}`,
    "库存出库与履约凭据",
    [
      detailField("出库单号", row.shipmentNumber),
      detailField("状态", statusLabel(row.status), "status"),
      detailField("出库日期", row.shipmentDate),
      detailField("确认时间", formatDateTime(row.confirmedAt)),
      detailField("销售订单 ID", row.salesOrderId, "id"),
      detailField("仓库 ID", row.warehouseId, "id"),
      detailField("记录版本", String(row.version)),
      detailField("最近更新", formatDateTime(row.updatedAt)),
    ],
  );
}

export function receivableDetail(row: Receivable): RecordDetailAction {
  return recordDetail(
    "sales",
    `经营应收 · ${row.receivableNumber}`,
    "销售履约形成的经营债权",
    [
      detailField("应收单号", row.receivableNumber),
      detailField("状态", statusLabel(row.status), "status"),
      detailField(
        "原始金额",
        formatMoney(row.currency, row.originalAmount),
        "money",
      ),
      detailField(
        "已收金额",
        formatMoney(row.currency, row.settledAmount),
        "money",
      ),
      detailField(
        "未收金额",
        formatMoney(row.currency, row.openAmount),
        "money",
      ),
      detailField("到期日期", row.dueDate),
      detailField(
        "逾期情况",
        row.isOverdue ? `逾期 ${row.overdueDays} 天` : "未逾期",
        "status",
      ),
      detailField("销售订单 ID", row.salesOrderId, "id"),
      detailField("出库单 ID", row.shipmentId, "id"),
      detailField("客户 ID", row.customerId, "id"),
      detailField("经营主体 ID", row.legalEntityId, "id"),
      detailField("记录版本", String(row.version)),
      detailField("最近更新", formatDateTime(row.updatedAt)),
    ],
  );
}

export function receiptDetail(row: Receipt): RecordDetailAction {
  return recordDetail(
    "sales",
    `客户收款 · ${row.receiptNumber}`,
    "收款确认与应收核销进度",
    [
      detailField("收款单号", row.receiptNumber),
      detailField("状态", statusLabel(row.status), "status"),
      detailField("收款金额", formatMoney(row.currency, row.amount), "money"),
      detailField(
        "已核销",
        formatMoney(row.currency, row.allocatedAmount),
        "money",
      ),
      detailField(
        "未核销",
        formatMoney(row.currency, row.unappliedAmount),
        "money",
      ),
      detailField("收款日期", row.receiptDate),
      detailField("客户 ID", row.customerId, "id"),
      detailField("经营主体 ID", row.legalEntityId, "id"),
      detailField("记录版本", String(row.version)),
      detailField("最近更新", formatDateTime(row.updatedAt)),
    ],
  );
}

export function purchaseOrderDetail(row: PurchaseOrder): RecordDetailAction {
  return recordDetail(
    "purchase",
    `采购订单 · ${row.purchaseOrderNumber}`,
    "供应承诺与到货状态",
    [
      detailField("采购单号", row.purchaseOrderNumber),
      detailField(
        "含税总额",
        formatMoney(row.currency, row.grossAmount),
        "money",
      ),
      detailField("订单状态", statusLabel(row.lifecycleStatus), "status"),
      detailField("到货状态", statusLabel(row.receivingStatus), "status"),
      detailField("订单日期", row.orderDate),
      detailField("供应商 ID", row.supplierId, "id"),
      detailField("经营主体 ID", row.legalEntityId, "id"),
      detailField("记录版本", String(row.version)),
      detailField("最近更新", formatDateTime(row.updatedAt)),
    ],
  );
}

export function goodsReceiptDetail(row: GoodsReceipt): RecordDetailAction {
  return recordDetail(
    "purchase",
    `采购收货单 · ${row.goodsReceiptNumber}`,
    "实际到货、库存与暂估成本凭据",
    [
      detailField("收货单号", row.goodsReceiptNumber),
      detailField("状态", statusLabel(row.status), "status"),
      detailField(
        "收货金额",
        formatMoney(row.currency, row.grossAmount),
        "money",
      ),
      detailField(
        "暂估成本",
        formatMoney(row.currency, row.inventoryCostAmount),
        "money",
      ),
      detailField("收货日期", row.receiptDate),
      detailField("采购订单 ID", row.purchaseOrderId, "id"),
      detailField("供应商 ID", row.supplierId, "id"),
      detailField("仓库 ID", row.warehouseId, "id"),
      detailField("经营主体 ID", row.legalEntityId, "id"),
      detailField("记录版本", String(row.version)),
      detailField("最近更新", formatDateTime(row.updatedAt)),
    ],
  );
}

export function payableDetail(row: Payable): RecordDetailAction {
  return recordDetail(
    "purchase",
    `经营应付 · ${row.payableNumber}`,
    "采购到货形成的经营债务",
    [
      detailField("应付单号", row.payableNumber),
      detailField("状态", statusLabel(row.status), "status"),
      detailField(
        "原始金额",
        formatMoney(row.currency, row.originalAmount),
        "money",
      ),
      detailField(
        "已付金额",
        formatMoney(row.currency, row.settledAmount),
        "money",
      ),
      detailField(
        "未付金额",
        formatMoney(row.currency, row.openAmount),
        "money",
      ),
      detailField("到期日期", row.dueDate),
      detailField(
        "逾期情况",
        row.isOverdue ? `逾期 ${row.overdueDays} 天` : "未逾期",
        "status",
      ),
      detailField("采购订单 ID", row.purchaseOrderId, "id"),
      detailField("收货单 ID", row.goodsReceiptId, "id"),
      detailField("供应商 ID", row.supplierId, "id"),
      detailField("经营主体 ID", row.legalEntityId, "id"),
      detailField("记录版本", String(row.version)),
      detailField("最近更新", formatDateTime(row.updatedAt)),
    ],
  );
}

export function paymentDetail(row: SupplierPayment): RecordDetailAction {
  return recordDetail(
    "purchase",
    `供应商付款 · ${row.supplierPaymentNumber}`,
    "付款确认与应付核销进度",
    [
      detailField("付款单号", row.supplierPaymentNumber),
      detailField("状态", statusLabel(row.status), "status"),
      detailField("付款金额", formatMoney(row.currency, row.amount), "money"),
      detailField(
        "已核销",
        formatMoney(row.currency, row.allocatedAmount),
        "money",
      ),
      detailField(
        "未核销",
        formatMoney(row.currency, row.unappliedAmount),
        "money",
      ),
      detailField("付款日期", row.paymentDate),
      detailField("供应商 ID", row.supplierId, "id"),
      detailField("经营主体 ID", row.legalEntityId, "id"),
      detailField("记录版本", String(row.version)),
      detailField("最近更新", formatDateTime(row.updatedAt)),
    ],
  );
}

export function returnDetail(
  row: BusinessReturn,
  side: "sales" | "purchase",
): RecordDetailAction {
  const sales = side === "sales";
  return recordDetail(
    side,
    `${sales ? "销售" : "采购"}退货单 · ${row.returnNumber}`,
    "来源凭据、金额与处置进度",
    [
      detailField("退货单号", row.returnNumber),
      detailField("状态", statusLabel(row.status), "status"),
      detailField("处置进度", statusLabel(row.workflowStatus), "status"),
      detailField("退货金额", formatMoney(row.currency, row.amount), "money"),
      detailField("退货日期", row.returnDate),
      detailField("退货原因", returnReason(row.reasonCode)),
      detailField("来源凭据 ID", row.sourceId, "id"),
      detailField("关联订单 ID", row.orderId, "id"),
      detailField(sales ? "客户 ID" : "供应商 ID", row.partnerId, "id"),
      detailField("仓库 ID", row.warehouseId, "id"),
      detailField("记录版本", String(row.version)),
      detailField("最近更新", formatDateTime(row.updatedAt)),
    ],
  );
}

export function statusLabel(value: string) {
  if (value === "none") return "正常";
  if (value.startsWith("overdue_")) return `逾期 ${value.split("_")[1]} 天`;
  return (
    {
      draft: "草稿",
      confirmed: "已确认",
      partially_shipped: "部分出库",
      shipped: "已出库",
      manual_review_hold: "人工暂停",
      not_started: "未开始",
      partially_received: "部分到货",
      fully_received: "已到齐",
      open: "未结",
      partially_settled: "部分结清",
      settled: "已结清",
      reversed: "已冲销",
      partially_allocated: "部分核销",
      fully_allocated: "已核销",
      not_required: "无需后续处置",
      pending: "待质检",
      completed: "质检完成",
      not_dispatched: "待发出",
      dispatched: "已发出",
      supplier_acknowledged: "供应商已签收",
    }[value] ?? value.replaceAll("_", " ")
  );
}

export function returnReason(value: string) {
  return (
    {
      QUALITY_ISSUE: "质量问题",
      WRONG_ITEM: "错发 / 错收",
      DAMAGED: "运输破损",
      COMMERCIAL_AGREEMENT: "商业协商",
      OTHER: "其他",
    }[value] ?? value
  );
}

function recordDetail(
  domain: "sales" | "purchase",
  title: string,
  subtitle: string,
  fields: RecordDetailAction["fields"],
): RecordDetailAction {
  return { kind: "record-detail", domain, title, subtitle, fields };
}

function detailField(
  label: string,
  value: string,
  format: "text" | "money" | "status" | "id" = "text",
) {
  return { label, value, format };
}

function formatDateTime(value: string | null) {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat("zh-CN", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}
