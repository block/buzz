import type { BusinessDockConfig } from "@/features/business-dock/businessDockConfig";

export type BusinessResourceType =
  | "agent_query"
  | "sales_order_entry"
  | "shipment_entry"
  | "purchase_order_entry"
  | "goods_receipt_entry"
  | "customer_receipt_entry"
  | "supplier_payment_entry"
  | "sales_order"
  | "shipment"
  | "purchase_order"
  | "goods_receipt"
  | "customer"
  | "supplier"
  | "inventory"
  | "receivable"
  | "payable"
  | "invoice"
  | "payment"
  | "supplier_payment"
  | "customer_receipt"
  | "order_profit"
  | "profitability"
  | "profit_adjustment"
  | "management_report"
  | "operations_dashboard"
  | "data_quality"
  | "operating_incidents"
  | "operating_trends"
  | "anomaly"
  | "action_proposal"
  | "work_item"
  | "approval_draft"
  | "generic";

export type BusinessResource = {
  version: 1;
  type: BusinessResourceType;
  id?: string;
  path: string;
  title?: string;
  legalEntityId?: string;
  period?: string;
  metadata?: Record<string, string>;
};

type RouteDefinition = {
  type: Exclude<BusinessResourceType, "generic">;
  deepLink: string;
  prefix: string;
  entity: boolean;
  label: string;
};

const ROUTES: readonly RouteDefinition[] = [
  {
    type: "agent_query",
    deepLink: "agent-query",
    prefix: "/embed/agent-queries/",
    entity: true,
    label: "查询记录",
  },
  {
    type: "operations_dashboard",
    deepLink: "operations-dashboard",
    prefix: "/embed/operations-dashboard",
    entity: false,
    label: "经营驾驶舱",
  },
  {
    type: "sales_order_entry",
    deepLink: "sales-order-entry",
    prefix: "/embed/entries/sales-order",
    entity: false,
    label: "录入销售订单",
  },
  {
    type: "shipment_entry",
    deepLink: "shipment-entry",
    prefix: "/embed/entries/shipment",
    entity: false,
    label: "录入销售出库",
  },
  {
    type: "purchase_order_entry",
    deepLink: "purchase-order-entry",
    prefix: "/embed/entries/purchase-order",
    entity: false,
    label: "录入采购订单",
  },
  {
    type: "goods_receipt_entry",
    deepLink: "goods-receipt-entry",
    prefix: "/embed/entries/goods-receipt",
    entity: false,
    label: "录入采购收货",
  },
  {
    type: "customer_receipt_entry",
    deepLink: "customer-receipt-entry",
    prefix: "/embed/entries/customer-receipt",
    entity: false,
    label: "录入客户收款",
  },
  {
    type: "supplier_payment_entry",
    deepLink: "supplier-payment-entry",
    prefix: "/embed/entries/supplier-payment",
    entity: false,
    label: "录入供应商付款",
  },
  {
    type: "data_quality",
    deepLink: "data-quality",
    prefix: "/embed/data-quality",
    entity: false,
    label: "数据质量",
  },
  {
    type: "operating_incidents",
    deepLink: "operating-incidents",
    prefix: "/embed/operating-incidents",
    entity: false,
    label: "经营异常处置",
  },
  {
    type: "operating_trends",
    deepLink: "operating-trends",
    prefix: "/embed/operating-trends",
    entity: false,
    label: "经营日报与趋势",
  },
  {
    type: "order_profit",
    deepLink: "order-profit",
    prefix: "/embed/order-profits/",
    entity: true,
    label: "订单真实利润",
  },
  {
    type: "profitability",
    deepLink: "profitability",
    prefix: "/embed/profitability/",
    entity: false,
    label: "盈利分析",
  },
  {
    type: "profit_adjustment",
    deepLink: "profit-adjustment",
    prefix: "/embed/profit-adjustments/",
    entity: true,
    label: "经营费用调整",
  },
  {
    type: "anomaly",
    deepLink: "anomaly",
    prefix: "/embed/anomalies/",
    entity: true,
    label: "经营异常",
  },
  {
    type: "action_proposal",
    deepLink: "action-proposal",
    prefix: "/embed/action-proposals/",
    entity: true,
    label: "处置建议",
  },
  {
    type: "work_item",
    deepLink: "work-item",
    prefix: "/embed/work-items/",
    entity: true,
    label: "人工待办",
  },
  {
    type: "approval_draft",
    deepLink: "approval-draft",
    prefix: "/embed/approval-drafts/",
    entity: true,
    label: "审批草稿",
  },
  {
    type: "sales_order",
    deepLink: "sales-order",
    prefix: "/embed/sales-orders/",
    entity: true,
    label: "销售订单",
  },
  {
    type: "shipment",
    deepLink: "shipment",
    prefix: "/embed/shipments/",
    entity: true,
    label: "销售出库",
  },
  {
    type: "purchase_order",
    deepLink: "purchase-order",
    prefix: "/embed/purchase-orders/",
    entity: true,
    label: "采购订单",
  },
  {
    type: "goods_receipt",
    deepLink: "goods-receipt",
    prefix: "/embed/goods-receipts/",
    entity: true,
    label: "采购收货",
  },
  {
    type: "customer",
    deepLink: "customer",
    prefix: "/embed/customers/",
    entity: true,
    label: "客户",
  },
  {
    type: "supplier",
    deepLink: "supplier",
    prefix: "/embed/suppliers/",
    entity: true,
    label: "供应商",
  },
  {
    type: "inventory",
    deepLink: "inventory",
    prefix: "/embed/inventory/",
    entity: false,
    label: "库存",
  },
  {
    type: "receivable",
    deepLink: "receivable",
    prefix: "/embed/receivables/",
    entity: false,
    label: "应收",
  },
  {
    type: "payable",
    deepLink: "payable",
    prefix: "/embed/payables/supplier/",
    entity: false,
    label: "应付",
  },
  {
    type: "supplier_payment",
    deepLink: "supplier-payment",
    prefix: "/embed/supplier-payments/",
    entity: true,
    label: "供应商付款",
  },
  {
    type: "invoice",
    deepLink: "invoice",
    prefix: "/embed/invoices/",
    entity: true,
    label: "发票",
  },
  {
    type: "payment",
    deepLink: "payment",
    prefix: "/embed/payments/",
    entity: true,
    label: "付款",
  },
  {
    type: "customer_receipt",
    deepLink: "customer-receipt",
    prefix: "/embed/customer-receipts/",
    entity: true,
    label: "客户收款",
  },
  {
    type: "management_report",
    deepLink: "management-report",
    prefix: "/embed/management-reports/",
    entity: false,
    label: "管理报表",
  },
] as const;

const ROUTE_BY_TYPE = new Map(ROUTES.map((route) => [route.type, route]));
const ROUTE_BY_DEEP_LINK = new Map(
  ROUTES.map((route) => [route.deepLink, route]),
);
const SINGLETON_DEEP_LINKS = new Set([
  "operations-dashboard",
  "data-quality",
  "operating-incidents",
  "operating-trends",
  "sales-order-entry",
  "shipment-entry",
  "purchase-order-entry",
  "goods-receipt-entry",
  "customer-receipt-entry",
  "supplier-payment-entry",
]);
const ACCOUNT_DEEP_LINKS = new Map([
  ["customer/receivables", ROUTE_BY_TYPE.get("receivable")],
  ["supplier/payables", ROUTE_BY_TYPE.get("payable")],
] as const);
const RESOURCE_TYPES = new Set<BusinessResourceType>([
  ...ROUTES.map((route) => route.type),
  "generic",
]);
const FORBIDDEN_METADATA_KEY =
  /(?:access[_-]?token|cookie|password|secret|credential|bank|invoice.*detail|voucher)/i;
const SAFE_SEGMENT = /^[A-Za-z0-9][A-Za-z0-9._:@-]{0,127}$/;
const SAFE_PERIOD = /^\d{4}-(?:0[1-9]|1[0-2])$/;
const PROFIT_DIMENSIONS = new Set(["customer", "sku", "brand", "salesperson"]);
const MAX_TITLE_LENGTH = 180;
const MAX_METADATA_ENTRIES = 20;
const MAX_METADATA_VALUE_LENGTH = 500;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function safeText(value: unknown, maxLength: number): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  return trimmed && trimmed.length <= maxLength ? trimmed : undefined;
}

function decodePath(path: string): string | null {
  try {
    return decodeURIComponent(path);
  } catch {
    return null;
  }
}

function containsPathTraversal(value: string): boolean {
  const decoded = decodePath(value);
  return (
    !decoded ||
    decoded.includes("\\") ||
    decoded
      .split(/[/?#]/)
      .some((segment) => segment === "." || segment === "..")
  );
}

export function normalizeBusinessPath(value: unknown): string | null {
  if (
    typeof value !== "string" ||
    !value.startsWith("/") ||
    value.length > 1024
  ) {
    return null;
  }
  if (
    value.startsWith("//") ||
    /[\\?#]/.test(value) ||
    [...value].some((character) => character.charCodeAt(0) < 32)
  )
    return null;
  const decoded = decodePath(value);
  if (!decoded || decoded.includes("\\")) return null;
  const segments = decoded.split("/");
  if (segments.some((segment) => segment === "." || segment === ".."))
    return null;
  try {
    const url = new URL(value, "https://business.invalid");
    return url.origin === "https://business.invalid" && url.pathname === value
      ? url.pathname
      : null;
  } catch {
    return null;
  }
}

function normalizeMetadata(value: unknown): Record<string, string> | undefined {
  if (value === undefined) return undefined;
  if (!isRecord(value)) return undefined;
  const entries = Object.entries(value);
  if (entries.length > MAX_METADATA_ENTRIES) return undefined;
  const metadata: Record<string, string> = {};
  for (const [key, item] of entries) {
    if (
      !SAFE_SEGMENT.test(key) ||
      FORBIDDEN_METADATA_KEY.test(key) ||
      typeof item !== "string" ||
      item.length > MAX_METADATA_VALUE_LENGTH
    ) {
      return undefined;
    }
    metadata[key] = item;
  }
  return metadata;
}

function resourceFromPath(path: string): BusinessResource {
  const profitability = path.match(
    /^\/embed\/profitability\/(customer|sku|brand|salesperson)\/([^/]+)\/period\/(\d{4}-(?:0[1-9]|1[0-2]))$/,
  );
  if (profitability && SAFE_SEGMENT.test(profitability[2])) {
    return {
      version: 1,
      type: "profitability",
      id: profitability[2],
      period: profitability[3],
      metadata: { dimension: profitability[1] },
      path,
    };
  }
  if (path.startsWith("/embed/reports/")) {
    const id = path.slice("/embed/reports/".length).split("/")[0];
    return {
      version: 1,
      type: "management_report",
      ...(SAFE_SEGMENT.test(id) ? { id } : {}),
      path,
    };
  }
  for (const route of ROUTES) {
    if (SINGLETON_DEEP_LINKS.has(route.deepLink)) {
      if (path === route.prefix) {
        return { version: 1, type: route.type, path };
      }
      continue;
    }
    if (!path.startsWith(route.prefix)) continue;
    const tail = path.slice(route.prefix.length);
    const firstSegment = tail.split("/")[0];
    const id =
      firstSegment && SAFE_SEGMENT.test(firstSegment)
        ? firstSegment
        : undefined;
    return { version: 1, type: route.type, ...(id ? { id } : {}), path };
  }
  return { version: 1, type: "generic", path };
}

export function parseBusinessUrl(
  value: string,
  config: BusinessDockConfig,
): BusinessResource | null {
  const input = value.trim();
  if (containsPathTraversal(input)) return null;
  if (input.startsWith("biz://")) {
    let url: URL;
    try {
      url = new URL(input);
    } catch {
      return null;
    }
    if (
      url.protocol !== "biz:" ||
      url.username ||
      url.password ||
      url.search ||
      url.hash
    ) {
      return null;
    }
    const rawSegments = url.pathname.split("/").filter(Boolean);
    if (url.hostname === "profitability") {
      if (rawSegments.length !== 3) return null;
      const [dimension, rawId, period] = rawSegments;
      const id = decodePath(rawId);
      if (
        !PROFIT_DIMENSIONS.has(dimension) ||
        !id ||
        !SAFE_SEGMENT.test(id) ||
        !SAFE_PERIOD.test(period)
      )
        return null;
      return {
        version: 1,
        type: "profitability",
        id,
        period,
        metadata: { dimension },
        path: `/embed/profitability/${dimension}/${encodeURIComponent(id)}/period/${period}`,
      };
    }
    if (SINGLETON_DEEP_LINKS.has(url.hostname)) {
      if (rawSegments.length !== 0) return null;
      const route = ROUTE_BY_DEEP_LINK.get(url.hostname);
      return route
        ? { version: 1, type: route.type, path: route.prefix }
        : null;
    }
    const accountRoute =
      rawSegments.length === 2
        ? ACCOUNT_DEEP_LINKS.get(
            `${url.hostname}/${rawSegments[1]}` as
              | "customer/receivables"
              | "supplier/payables",
          )
        : undefined;
    const route = accountRoute ?? ROUTE_BY_DEEP_LINK.get(url.hostname);
    if (!route || rawSegments.length !== (accountRoute ? 2 : 1)) return null;
    const decodedId = decodePath(rawSegments[0]);
    if (!decodedId || !SAFE_SEGMENT.test(decodedId)) return null;
    const id = decodedId;
    return {
      version: 1,
      type: route.type,
      id,
      path: `${route.prefix}${encodeURIComponent(id)}`,
    };
  }

  let url: URL;
  try {
    url = new URL(input, config.homeUrl);
  } catch {
    return null;
  }
  if (
    !new Set(["http:", "https:"]).has(url.protocol) ||
    url.origin !== config.origin ||
    url.username ||
    url.password ||
    url.search ||
    url.hash
  ) {
    return null;
  }
  const path = normalizeBusinessPath(url.pathname);
  return path ? resourceFromPath(path) : null;
}

export function isBusinessResource(value: unknown): value is BusinessResource {
  if (
    !isRecord(value) ||
    value.version !== 1 ||
    !RESOURCE_TYPES.has(value.type as BusinessResourceType)
  ) {
    return false;
  }
  const path = normalizeBusinessPath(value.path);
  if (!path) return false;
  if (
    value.id !== undefined &&
    (typeof value.id !== "string" || !SAFE_SEGMENT.test(value.id))
  )
    return false;
  if (value.title !== undefined && !safeText(value.title, MAX_TITLE_LENGTH))
    return false;
  if (
    value.legalEntityId !== undefined &&
    (typeof value.legalEntityId !== "string" ||
      !SAFE_SEGMENT.test(value.legalEntityId))
  )
    return false;
  if (
    value.period !== undefined &&
    (typeof value.period !== "string" || !SAFE_PERIOD.test(value.period))
  )
    return false;
  if (
    value.metadata !== undefined &&
    normalizeMetadata(value.metadata) === undefined
  )
    return false;
  const route =
    value.type === "generic"
      ? undefined
      : ROUTE_BY_TYPE.get(
          value.type as Exclude<BusinessResourceType, "generic">,
        );
  if (
    route &&
    (SINGLETON_DEEP_LINKS.has(route.deepLink)
      ? path !== route.prefix
      : !path.startsWith(route.prefix))
  )
    return false;
  return true;
}

export function resolveBusinessResource(
  input: unknown,
  config: BusinessDockConfig,
): BusinessResource | null {
  if (typeof input === "string") return parseBusinessUrl(input, config);
  if (!isBusinessResource(input)) return null;
  const metadata = normalizeMetadata(input.metadata);
  return {
    version: 1,
    type: input.type,
    ...(input.id ? { id: input.id } : {}),
    path: normalizeBusinessPath(input.path) as string,
    ...(input.title ? { title: input.title.trim() } : {}),
    ...(input.legalEntityId ? { legalEntityId: input.legalEntityId } : {}),
    ...(input.period ? { period: input.period } : {}),
    ...(metadata ? { metadata } : {}),
  };
}

export function buildBusinessUrl(
  resource: BusinessResource,
  config: BusinessDockConfig,
): string | null {
  const normalized = resolveBusinessResource(resource, config);
  if (!normalized) return null;
  const url = new URL(normalized.path, `${config.origin}/`);
  return url.origin === config.origin ? url.href : null;
}

export function buildBusinessReference(
  resource: BusinessResource,
): string | null {
  if (!isBusinessResource(resource)) return null;
  const route =
    resource.type === "generic" ? undefined : ROUTE_BY_TYPE.get(resource.type);
  if (
    route &&
    resource.id &&
    resource.path === `${route.prefix}${encodeURIComponent(resource.id)}`
  ) {
    if (resource.type === "receivable") {
      return `biz://customer/${encodeURIComponent(resource.id)}/receivables`;
    }
    if (resource.type === "payable") {
      return `biz://supplier/${encodeURIComponent(resource.id)}/payables`;
    }
    return `biz://${route.deepLink}/${encodeURIComponent(resource.id)}`;
  }
  if (
    route &&
    SINGLETON_DEEP_LINKS.has(route.deepLink) &&
    !resource.id &&
    resource.path === route.prefix
  ) {
    return `biz://${route.deepLink}`;
  }
  if (resource.type === "profitability" && resource.id && resource.period) {
    const dimension = resource.metadata?.dimension;
    if (
      dimension &&
      PROFIT_DIMENSIONS.has(dimension) &&
      resource.path ===
        `/embed/profitability/${dimension}/${encodeURIComponent(resource.id)}/period/${resource.period}`
    ) {
      return `biz://profitability/${dimension}/${encodeURIComponent(resource.id)}/${resource.period}`;
    }
  }
  return null;
}

export function formatBusinessResourceLabel(
  resource: BusinessResource,
): string {
  const label =
    resource.type === "generic"
      ? "业务页面"
      : (ROUTE_BY_TYPE.get(resource.type)?.label ?? "业务页面");
  return resource.id ? `${label} · ${resource.id}` : resource.title || label;
}

export function isBusinessDeepLinkCandidate(value: string): boolean {
  if (!value.startsWith("biz://") || value.length > 256) return false;
  try {
    const url = new URL(value);
    const segments = url.pathname.split("/").filter(Boolean);
    if (url.hostname === "profitability") {
      return Boolean(
        segments.length === 3 &&
          PROFIT_DIMENSIONS.has(segments[0]) &&
          SAFE_SEGMENT.test(segments[1]) &&
          SAFE_PERIOD.test(segments[2]) &&
          !url.username &&
          !url.password &&
          !url.search &&
          !url.hash,
      );
    }
    if (SINGLETON_DEEP_LINKS.has(url.hostname)) {
      return Boolean(
        segments.length === 0 &&
          !url.username &&
          !url.password &&
          !url.search &&
          !url.hash,
      );
    }
    const accountRoute =
      segments.length === 2
        ? ACCOUNT_DEEP_LINKS.get(
            `${url.hostname}/${segments[1]}` as
              | "customer/receivables"
              | "supplier/payables",
          )
        : undefined;
    const route = accountRoute ?? ROUTE_BY_DEEP_LINK.get(url.hostname);
    const id =
      segments.length === (accountRoute ? 2 : 1)
        ? decodePath(segments[0])
        : null;
    return Boolean(
      route &&
        id &&
        SAFE_SEGMENT.test(id) &&
        !url.username &&
        !url.password &&
        !url.search &&
        !url.hash,
    );
  } catch {
    return false;
  }
}
