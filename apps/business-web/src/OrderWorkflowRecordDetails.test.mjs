import assert from "node:assert/strict";
import test from "node:test";
import {
  salesOrderDetail,
  statusLabel,
} from "./OrderWorkflowRecordDetails.ts";

test("builds a readable sales-order detail record", () => {
  const detail = salesOrderDetail({
    id: "order-1",
    orderNumber: "SO-2026-001",
    legalEntityId: "entity-1",
    customerId: "customer-1",
    currency: "CNY",
    lifecycleStatus: "confirmed",
    holdStatus: "none",
    fulfillmentStatus: "partially_shipped",
    grossAmount: "1280.5",
    orderDate: "2026-08-22",
    updatedAt: "2026-08-22T08:30:00Z",
    version: 3,
  });

  assert.equal(detail.kind, "record-detail");
  assert.equal(detail.domain, "sales");
  assert.equal(detail.title, "销售订单 · SO-2026-001");
  assert.deepEqual(
    detail.fields.slice(0, 5).map(({ label, value }) => [label, value]),
    [
      ["订单编号", "SO-2026-001"],
      ["含税总额", "CNY 1,280.50"],
      ["订单状态", "已确认"],
      ["履约状态", "部分出库"],
      ["冻结状态", "正常"],
    ],
  );
});

test("keeps status labels consistent between rows and details", () => {
  assert.equal(statusLabel("fully_allocated"), "已核销");
  assert.equal(statusLabel("supplier_acknowledged"), "供应商已签收");
});
