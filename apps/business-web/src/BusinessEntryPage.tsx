import React from "react";
import { GoodsReceiptEntry } from "./GoodsReceiptEntry";
import { PurchaseOrderEntry } from "./PurchaseOrderEntry";
import { SalesOrderEntry } from "./SalesOrderEntry";
import { ShipmentEntry } from "./ShipmentEntry";
import {
  CustomerReceiptEntry,
  SupplierPaymentEntry,
} from "./SettlementForms";

export type BusinessEntryKind =
  | "sales-order"
  | "shipment"
  | "purchase-order"
  | "goods-receipt"
  | "customer-receipt"
  | "supplier-payment";

const entryCopy: Record<
  BusinessEntryKind,
  { eyebrow: string; title: string; caption: string }
> = {
  "sales-order": {
    eyebrow: "会话发起 / Sales order",
    title: "录入销售订单",
    caption: "核对会话中提到的客户、商品、数量与价格后保存草稿。",
  },
  shipment: {
    eyebrow: "会话发起 / Shipment",
    title: "录入销售出库",
    caption: "选择已确认订单并核对本次出库数量后保存草稿。",
  },
  "purchase-order": {
    eyebrow: "会话发起 / Purchase order",
    title: "录入采购订单",
    caption: "核对供应商、商品、数量与采购价格后保存草稿。",
  },
  "goods-receipt": {
    eyebrow: "会话发起 / Goods receipt",
    title: "录入采购收货",
    caption: "选择采购订单并核对实际到货数量后保存草稿。",
  },
  "customer-receipt": {
    eyebrow: "会话发起 / Customer receipt",
    title: "录入客户收款",
    caption: "核对客户、币种、金额与业务日期后保存草稿。",
  },
  "supplier-payment": {
    eyebrow: "会话发起 / Supplier payment",
    title: "录入供应商付款",
    caption: "核对供应商、币种、金额与业务日期后保存草稿。",
  },
};

export function BusinessEntryPage({ entry }: { entry: BusinessEntryKind }) {
  const [, setRevision] = React.useState(0);
  const onDone = () => setRevision((value) => value + 1);
  const content =
    entry === "sales-order" ? (
      <SalesOrderEntry onDone={onDone} />
    ) : entry === "shipment" ? (
      <ShipmentEntry onDone={onDone} />
    ) : entry === "purchase-order" ? (
      <PurchaseOrderEntry onDone={onDone} />
    ) : entry === "goods-receipt" ? (
      <GoodsReceiptEntry onDone={onDone} />
    ) : entry === "customer-receipt" ? (
      <CustomerReceiptEntry onDone={onDone} />
    ) : (
      <SupplierPaymentEntry onDone={onDone} />
    );
  const copy = entryCopy[entry];
  return (
    <section className="page">
      <div className="page-head">
        <div>
          <p>{copy.eyebrow}</p>
          <h1>{copy.title}</h1>
          <span>{copy.caption}</span>
        </div>
      </div>
      <div className="boundary-note">
        会话只负责发起录入。请在此核对字段并提交；业务权限、CSRF、幂等与审计规则保持不变。
      </div>
      {content}
    </section>
  );
}
