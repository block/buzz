const hostOrigin = window.location.origin;
const currentUrl = document.querySelector("#current-url");
const currentResource = document.querySelector("#current-resource");
const theme = document.querySelector("#theme");
const bridge = document.querySelector("#bridge");
const refreshCount = document.querySelector("#refresh-count");
const authStatus = document.querySelector("#auth-status");
const lifecycleKind = document.querySelector("#lifecycle-kind");
const lifecycleTitle = document.querySelector("#lifecycle-title");
const lifecycleSummary = document.querySelector("#lifecycle-summary");
const conditionStatus = document.querySelector("#condition-status");
const reviewStatus = document.querySelector("#review-status");
const draftOnlyWarning = document.querySelector("#draft-only-warning");
let refreshes = 0;
let requestSequence = 0;
let sessionNonce = null;
let authSessionNonce = null;
let authenticated = true;
let resource = null;

const resourceLabels = {
  sales_order: "销售订单",
  purchase_order: "采购订单",
  customer: "客户",
  supplier: "供应商",
  invoice: "发票",
  payment: "付款",
  anomaly: "经营异常",
  action_proposal: "处置建议",
  work_item: "人工待办",
  approval_draft: "审批草稿",
};

const lifecycleCopy = {
  anomaly: [
    "Finding lifecycle",
    "低毛利销售订单",
    "确定性规则发现；等待人工确认处置方式。",
  ],
  action_proposal: [
    "System suggestion",
    "复核订单定价",
    "由 trade-action-v1.0 白名单生成，尚未创建待办。",
  ],
  work_item_preview: [
    "Human confirmation preview",
    "待办创建预览",
    "核对负责人、截止时间、异常快照和预览哈希后再确认。",
  ],
  work_item: [
    "Confirmed internal work item",
    "人工待办 WI-001",
    "仅追踪内部复核过程；源异常变化不会自动完成此待办。",
  ],
  approval_draft: [
    "Approval draft",
    "客户信用复核审批草稿 AD-001",
    "仅供准备和复核，不进入正式审批流。",
  ],
};

function renderLifecycle(kind) {
  const copy = lifecycleCopy[kind] ?? [
    "Finding lifecycle",
    "经营异常处置工作台",
    "仅创建内部处置记录；不修改任何权威业务数据。",
  ];
  lifecycleKind.textContent = copy[0];
  lifecycleTitle.textContent = copy[1];
  lifecycleSummary.textContent = copy[2];
  conditionStatus.textContent =
    kind === "work_item" ? "source active" : "active";
  reviewStatus.textContent =
    kind === "work_item" ? "in_progress" : "unreviewed";
  draftOnlyWarning.hidden = kind !== "approval_draft";
}

function send(type, payload) {
  if (!sessionNonce) return;
  requestSequence += 1;
  window.parent.postMessage(
    {
      version: 2,
      type,
      requestId: `mock-${requestSequence}`,
      sessionNonce,
      ...(payload === undefined ? {} : { payload }),
    },
    hostOrigin,
  );
}

function sendAuth(type, payload) {
  if (!authSessionNonce) return;
  requestSequence += 1;
  window.parent.postMessage(
    {
      version: 3,
      type,
      requestId: `mock-auth-${requestSequence}`,
      sessionNonce: authSessionNonce,
      payload,
    },
    hostOrigin,
  );
}

function resourceUrl(value) {
  return new URL(value.path, `${window.location.origin}/`).href;
}

function displayResource(value) {
  resource = value;
  currentResource.textContent = value
    ? `${value.type}${value.id ? ` · ${value.id}` : ""}`
    : "none";
  currentUrl.textContent = value ? resourceUrl(value) : window.location.href;
  renderLifecycle(value?.type);
}

function announceResource(value) {
  displayResource(value);
  send("RESOURCE_CHANGED", { resource: value });
  send("ROUTE_CHANGED", { url: resourceUrl(value) });
  send("TITLE_CHANGED", {
    title:
      value.title ??
      `${resourceLabels[value.type] ?? "Business"} ${value.id ?? ""}`.trim(),
  });
}

window.addEventListener("message", (event) => {
  if (event.origin !== hostOrigin || event.source !== window.parent) return;
  const message = event.data;
  if (message?.version === 3 && typeof message.sessionNonce === "string") {
    if (message.type === "HOST_INIT") authSessionNonce = message.sessionNonce;
    if (message.sessionNonce !== authSessionNonce) return;
    if (message.type === "CHECK_AUTH") {
      if (authenticated) {
        sendAuth("AUTH_STATUS", {
          authenticated: true,
          user: {
            subject: "mock-business-user",
            displayName: "POC User",
          },
        });
      } else {
        sendAuth("AUTH_REQUIRED", {
          reason: "Mock Business session is absent.",
        });
      }
    } else if (message.type === "LOGOUT") {
      authenticated = false;
      authStatus.textContent = "signed out";
      sendAuth("AUTH_STATUS", { authenticated: false });
    }
    return;
  }
  if (message?.version !== 2 || typeof message.sessionNonce !== "string")
    return;
  if (message.type === "HOST_INIT") {
    sessionNonce = message.sessionNonce;
    bridge.textContent = "connected-v2";
    send("BUSINESS_READY");
    send("TITLE_CHANGED", { title: "Pacioli Business" });
    return;
  }
  if (message.sessionNonce !== sessionNonce) return;
  if (message.type === "SET_THEME") {
    theme.textContent = message.payload?.theme ?? "unknown";
    document.documentElement.style.colorScheme = theme.textContent;
  } else if (message.type === "REFRESH") {
    refreshes += 1;
    refreshCount.textContent = String(refreshes);
  } else if (message.type === "NAVIGATE" && message.payload?.resource) {
    announceResource(message.payload.resource);
  } else if (message.type === "REQUEST_CURRENT_RESOURCE" && resource) {
    send("RESOURCE_CHANGED", { resource });
  }
});

document.querySelector("#open-order").addEventListener("click", () => {
  announceResource({
    version: 1,
    type: "sales_order",
    id: "SO-1042",
    path: "/embed/sales-orders/SO-1042",
    title: "销售订单 SO-1042",
  });
});
document.querySelector("#open-anomaly").addEventListener("click", () => {
  announceResource({
    version: 1,
    type: "anomaly",
    id: "FIND-001",
    path: "/embed/anomalies/FIND-001",
    title: "经营异常 FIND-001",
  });
});
document.querySelector("#open-proposal").addEventListener("click", () => {
  announceResource({
    version: 1,
    type: "action_proposal",
    id: "AP-001",
    path: "/embed/action-proposals/AP-001",
    title: "处置建议 AP-001",
  });
});
document.querySelector("#prepare-work-item").addEventListener("click", () => {
  renderLifecycle("work_item_preview");
});
document.querySelector("#confirm-work-item").addEventListener("click", () => {
  const item = {
    version: 1,
    type: "work_item",
    id: "WI-001",
    path: "/embed/work-items/WI-001",
    title: "人工待办 WI-001",
  };
  announceResource(item);
  send("ACTION_COMPLETED", {
    action: "work_item_created",
    resource: { type: item.type, id: item.id },
    message: "待办 WI-001 已由当前用户确认创建",
    traceId: "trace-work-item-created",
  });
});
document.querySelector("#open-work-item").addEventListener("click", () => {
  announceResource({
    version: 1,
    type: "work_item",
    id: "WI-001",
    path: "/embed/work-items/WI-001",
    title: "人工待办 WI-001",
  });
});
document.querySelector("#open-approval-draft").addEventListener("click", () => {
  announceResource({
    version: 1,
    type: "approval_draft",
    id: "AD-001",
    path: "/embed/approval-drafts/AD-001",
    title: "审批草稿 AD-001",
  });
});
document.querySelector("#change-resource").addEventListener("click", () => {
  announceResource({
    version: 1,
    type: "customer",
    id: "CUST-2048",
    path: "/embed/customers/CUST-2048",
    title: "客户 CUST-2048",
  });
});
document.querySelector("#mark-dirty").addEventListener("click", () => {
  send("DIRTY_STATE_CHANGED", { dirty: true });
});
document.querySelector("#mark-clean").addEventListener("click", () => {
  send("DIRTY_STATE_CHANGED", { dirty: false });
});
document.querySelector("#action-success").addEventListener("click", () => {
  send("ACTION_COMPLETED", {
    action: "finding_acknowledged",
    resource: resource ? { type: resource.type, id: resource.id } : undefined,
    message: "经营异常已确认收到",
    traceId: "trace-success",
  });
});
document.querySelector("#action-failed").addEventListener("click", () => {
  send("ACTION_FAILED", {
    action: "work_item_status_changed",
    resource: resource ? { type: resource.type, id: resource.id } : undefined,
    message: "待办状态更新失败",
    traceId: "trace-failed",
    stack: "sensitive stack must not be displayed",
  });
});
document.querySelector("#data-changed").addEventListener("click", () => {
  send("DATA_CHANGED", {
    ...(resource ? { resource } : {}),
    traceId: "trace-data",
  });
});
document.querySelector("#auth-required").addEventListener("click", () => {
  authenticated = false;
  authStatus.textContent = "required";
  sendAuth("AUTH_REQUIRED", { reason: "Mock Business session is absent." });
});
document.querySelector("#auth-success").addEventListener("click", () => {
  authenticated = true;
  authStatus.textContent = "authenticated";
  sendAuth("AUTH_STATUS", {
    authenticated: true,
    user: {
      subject: "mock-business-user",
      displayName: "POC User",
    },
  });
});
document.querySelector("#auth-expired").addEventListener("click", () => {
  authenticated = false;
  authStatus.textContent = "expired";
  sendAuth("SESSION_EXPIRED", { reason: "Mock session expired." });
});

displayResource(null);
