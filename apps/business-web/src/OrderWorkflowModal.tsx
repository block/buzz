import React from "react";
import { createPortal } from "react-dom";
import type { RegisterModalAction } from "./OrderWorkflowRegisters";

export type WorkflowModalState =
  | RegisterModalAction
  | { kind: "sales-create" }
  | { kind: "shipment-create" }
  | { kind: "purchase-create" }
  | { kind: "receipt-create" }
  | { kind: "customer-receipt-create" }
  | { kind: "supplier-payment-create" }
  | { kind: "sales-return-create" }
  | { kind: "purchase-return-create" };

export function WorkflowModal({
  state,
  onClose,
  children,
}: React.PropsWithChildren<{
  state: WorkflowModalState;
  onClose: () => void;
}>) {
  const panel = React.useRef<HTMLDivElement>(null);
  const trigger = React.useRef<HTMLElement | null>(null);
  React.useEffect(() => {
    trigger.current = document.activeElement as HTMLElement | null;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    panel.current
      ?.querySelector<HTMLElement>("button, input, select, textarea, a[href]")
      ?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
      if (event.key !== "Tab" || !panel.current) return;
      const items = [
        ...panel.current.querySelectorAll<HTMLElement>(
          "button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), a[href]",
        ),
      ];
      const first = items[0];
      const last = items.at(-1);
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last?.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first?.focus();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.body.style.overflow = previousOverflow;
      document.removeEventListener("keydown", onKey);
      trigger.current?.focus();
    };
  }, [onClose]);
  const domain =
    state.kind === "record-detail"
      ? state.domain
      : /purchase|supplier-payment|^receipt/.test(state.kind)
        ? "purchase"
        : "sales";
  return createPortal(
    <div className="workflow-modal-layer">
      <div className="workflow-modal-scrim" />
      <div
        className={`workflow-modal ${domain}`}
        role="dialog"
        aria-modal="true"
        aria-labelledby="workflow-modal-title"
        ref={panel}
      >
        <header>
          <div>
            <span>
              {state.kind === "record-detail"
                ? "业务记录 / Record detail"
                : "业务凭据 / Controlled action"}
            </span>
            <h2 id="workflow-modal-title">{modalTitle(state)}</h2>
          </div>
          <button
            type="button"
            className="modal-close"
            onClick={onClose}
            aria-label="关闭弹窗"
          >
            <CloseIcon />
          </button>
        </header>
        <div className="workflow-modal-body">{children}</div>
      </div>
    </div>,
    document.body,
  );
}

export function RecordDetail({
  state,
}: {
  state: Extract<WorkflowModalState, { kind: "record-detail" }>;
}) {
  return (
    <section className="record-detail" data-testid="workflow-record-detail">
      <div className="record-detail-intro">
        <span>业务记录 / Authoritative record</span>
        <p>{state.subtitle}</p>
      </div>
      <dl className="record-detail-grid">
        {state.fields.map((field) => (
          <div
            key={field.label}
            className={`record-detail-field ${field.format ?? "text"}`}
          >
            <dt>{field.label}</dt>
            <dd>
              {field.format === "id" ? <code>{field.value}</code> : field.value}
            </dd>
          </div>
        ))}
      </dl>
      <footer>
        <span>只读详情</span>
        <p>操作请返回记录行使用对应业务按钮，所有变更仍执行版本与权限校验。</p>
      </footer>
    </section>
  );
}

function modalTitle(state: WorkflowModalState) {
  if (state.kind === "record-detail") return state.title;
  if (state.kind === "customer-receipt-settle") {
    return `${state.receipt.receiptNumber} · 确认与核销`;
  }
  if (state.kind === "supplier-payment-settle") {
    return `${state.payment.supplierPaymentNumber} · 确认与核销`;
  }
  if (
    state.kind === "sales-return-confirm" ||
    state.kind === "purchase-return-confirm"
  ) {
    return `${state.item.returnNumber} · 确认退货影响`;
  }
  if (state.kind === "sales-return-inspect") {
    return `${state.item.returnNumber} · 退货质检处置`;
  }
  if (state.kind === "purchase-return-dispatch") {
    return `${state.item.returnNumber} · 登记退货发出`;
  }
  if (state.kind === "purchase-return-acknowledge") {
    return `${state.item.returnNumber} · 供应商签收`;
  }
  if ("number" in state) {
    return `${state.number} · ${state.kind.includes("confirm") ? "确认前检查" : "编辑草稿"}`;
  }
  if (state.kind === "sales-create") return "新增销售订单";
  if (state.kind === "shipment-create") return "新建销售出库单";
  if (state.kind === "purchase-create") return "新增采购订单";
  if (state.kind === "receipt-create") return "新建采购收货单";
  if (state.kind === "customer-receipt-create") return "登记客户收款";
  if (state.kind === "supplier-payment-create") return "登记供应商付款";
  if (state.kind === "sales-return-create") return "新增销售退货单";
  if (state.kind === "purchase-return-create") return "新增采购退货单";
  return state.title;
}

function CloseIcon() {
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
      <path d="m6 6 12 12M18 6 6 18" />
    </svg>
  );
}
