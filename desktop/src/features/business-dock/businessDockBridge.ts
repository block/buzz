import type { BusinessDockConfig } from "@/features/business-dock/businessDockConfig";
import {
  type BusinessResource,
  resolveBusinessResource,
} from "@/features/business-dock/businessResourceResolver";

export type BusinessBridgeV1Message = {
  payload?: unknown;
  requestId?: string;
  type: string;
  version: 1;
};

export type BusinessBridgeEnvelope<T = unknown, V extends 2 | 3 = 2> = {
  version: V;
  type: string;
  requestId: string;
  sessionNonce: string;
  payload?: T;
};

export type BusinessActionPayload = {
  action: string;
  message: string;
  traceId?: string;
  resource?: { type: BusinessResource["type"]; id?: string };
};

type InboundV2Payload<T> = Omit<BusinessBridgeEnvelope<T>, "payload"> & {
  payload: T;
};

type InboundV3Payload<T> = Omit<BusinessBridgeEnvelope<T, 3>, "payload"> & {
  payload: T;
};

export type BusinessAuthStatusPayload =
  | { authenticated: false }
  | {
      authenticated: true;
      user: { subject: string; displayName: string };
    };

export type InboundBusinessBridgeMessage =
  | { version: 1; type: "BUSINESS_READY"; requestId?: string }
  | {
      version: 1;
      type: "TITLE_CHANGED";
      requestId?: string;
      payload: { title: string };
    }
  | {
      version: 1;
      type: "ROUTE_CHANGED";
      requestId?: string;
      payload: { url: string };
    }
  | (BusinessBridgeEnvelope<undefined> & { type: "BUSINESS_READY" })
  | (InboundV2Payload<{ title: string }> & { type: "TITLE_CHANGED" })
  | (InboundV2Payload<{ url: string }> & { type: "ROUTE_CHANGED" })
  | (InboundV2Payload<{ resource: BusinessResource }> & {
      type: "RESOURCE_CHANGED";
    })
  | (InboundV2Payload<BusinessActionPayload> & {
      type: "ACTION_COMPLETED" | "ACTION_FAILED";
    })
  | (InboundV2Payload<{ resource?: BusinessResource; traceId?: string }> & {
      type: "DATA_CHANGED";
    })
  | (InboundV2Payload<{ dirty: boolean }> & { type: "DIRTY_STATE_CHANGED" })
  | (InboundV3Payload<BusinessAuthStatusPayload> & { type: "AUTH_STATUS" })
  | (InboundV3Payload<{ reason?: string }> & {
      type: "AUTH_REQUIRED" | "SESSION_EXPIRED";
    });

export type BusinessHostBridgeType =
  | "HOST_INIT"
  | "SET_THEME"
  | "REFRESH"
  | "NAVIGATE"
  | "SET_CONTEXT"
  | "REQUEST_CURRENT_RESOURCE"
  | "CHECK_AUTH"
  | "LOGOUT";

const MAX_REQUEST_ID = 128;
const MAX_NONCE = 256;
const MAX_BRIDGE_TEXT = 500;
const ACTION_RESOURCE_TYPES = new Set<BusinessResource["type"]>([
  "sales_order",
  "shipment",
  "purchase_order",
  "customer",
  "supplier",
  "inventory",
  "receivable",
  "payable",
  "invoice",
  "payment",
  "customer_receipt",
  "management_report",
  "anomaly",
  "action_proposal",
  "work_item",
  "approval_draft",
  "generic",
]);
const ALLOWED_ACTION_EVENTS = new Set([
  "work_item_created",
  "work_item_status_changed",
  "approval_draft_created",
  "approval_draft_updated",
  "finding_acknowledged",
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasOnlyKeys(value: Record<string, unknown>, keys: string[]): boolean {
  const allowed = new Set(keys);
  return Object.keys(value).every((key) => allowed.has(key));
}

function parseV3AuthPayload(
  type: string,
  value: unknown,
): BusinessAuthStatusPayload | { reason?: string } | null {
  if (!isRecord(value)) return null;
  if (type === "AUTH_STATUS") {
    if (value.authenticated === false && hasOnlyKeys(value, ["authenticated"]))
      return { authenticated: false };
    if (
      value.authenticated === true &&
      hasOnlyKeys(value, ["authenticated", "user"]) &&
      isRecord(value.user) &&
      hasOnlyKeys(value.user, ["subject", "displayName"]) &&
      isBoundedText(value.user.subject, 256) &&
      isBoundedText(value.user.displayName, 180)
    ) {
      return {
        authenticated: true,
        user: {
          subject: value.user.subject,
          displayName: value.user.displayName,
        },
      };
    }
    return null;
  }
  if (!hasOnlyKeys(value, ["reason"])) return null;
  if (value.reason !== undefined && !isBoundedText(value.reason, 240))
    return null;
  return typeof value.reason === "string" ? { reason: value.reason } : {};
}

function isBoundedText(value: unknown, max = MAX_BRIDGE_TEXT): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= max;
}

function parseActionPayload(value: unknown): BusinessActionPayload | null {
  if (
    !isRecord(value) ||
    !isBoundedText(value.action, 100) ||
    !ALLOWED_ACTION_EVENTS.has(value.action) ||
    !isBoundedText(value.message)
  )
    return null;
  let resource: BusinessActionPayload["resource"];
  if (value.resource !== undefined) {
    if (
      !isRecord(value.resource) ||
      !ACTION_RESOURCE_TYPES.has(
        value.resource.type as BusinessResource["type"],
      ) ||
      (value.resource.id !== undefined &&
        !isBoundedText(value.resource.id, 128))
    )
      return null;
    resource = {
      type: value.resource.type as BusinessResource["type"],
      ...(typeof value.resource.id === "string"
        ? { id: value.resource.id }
        : {}),
    };
  }
  if (value.traceId !== undefined && !isBoundedText(value.traceId, 128))
    return null;
  return {
    action: value.action,
    message: value.message,
    ...(typeof value.traceId === "string" ? { traceId: value.traceId } : {}),
    ...(resource ? { resource } : {}),
  };
}

function parseV1(
  value: Record<string, unknown>,
): InboundBusinessBridgeMessage | null {
  const requestId =
    typeof value.requestId === "string" ? value.requestId : undefined;
  if (value.type === "BUSINESS_READY")
    return { version: 1, type: value.type, requestId };
  if (!isRecord(value.payload)) return null;
  if (
    value.type === "TITLE_CHANGED" &&
    isBoundedText(value.payload.title, 180)
  ) {
    return {
      version: 1,
      type: value.type,
      requestId,
      payload: { title: value.payload.title },
    };
  }
  if (
    value.type === "ROUTE_CHANGED" &&
    isBoundedText(value.payload.url, 2048)
  ) {
    return {
      version: 1,
      type: value.type,
      requestId,
      payload: { url: value.payload.url },
    };
  }
  return null;
}

export function parseInboundBusinessBridgeMessage(
  value: unknown,
  config?: BusinessDockConfig,
  expectedNonce?: string,
): InboundBusinessBridgeMessage | null {
  if (!isRecord(value) || typeof value.type !== "string") return null;
  if (value.version === 1) return parseV1(value);
  if (
    value.version === 3 &&
    isBoundedText(value.requestId, MAX_REQUEST_ID) &&
    isBoundedText(value.sessionNonce, MAX_NONCE) &&
    expectedNonce &&
    value.sessionNonce === expectedNonce
  ) {
    const envelope = {
      version: 3 as const,
      requestId: value.requestId,
      sessionNonce: value.sessionNonce,
    };
    if (
      value.type !== "AUTH_STATUS" &&
      value.type !== "AUTH_REQUIRED" &&
      value.type !== "SESSION_EXPIRED"
    )
      return null;
    const payload = parseV3AuthPayload(value.type, value.payload);
    return payload
      ? ({
          ...envelope,
          type: value.type,
          payload,
        } as InboundBusinessBridgeMessage)
      : null;
  }
  if (
    value.version !== 2 ||
    !isBoundedText(value.requestId, MAX_REQUEST_ID) ||
    !isBoundedText(value.sessionNonce, MAX_NONCE) ||
    !expectedNonce ||
    value.sessionNonce !== expectedNonce
  )
    return null;
  const envelope = {
    version: 2 as const,
    requestId: value.requestId,
    sessionNonce: value.sessionNonce,
  };
  if (value.type === "BUSINESS_READY") return { ...envelope, type: value.type };
  if (!isRecord(value.payload)) return null;
  if (
    value.type === "TITLE_CHANGED" &&
    isBoundedText(value.payload.title, 180)
  ) {
    return {
      ...envelope,
      type: value.type,
      payload: { title: value.payload.title },
    };
  }
  if (
    value.type === "ROUTE_CHANGED" &&
    isBoundedText(value.payload.url, 2048)
  ) {
    return {
      ...envelope,
      type: value.type,
      payload: { url: value.payload.url },
    };
  }
  if (
    value.type === "DIRTY_STATE_CHANGED" &&
    typeof value.payload.dirty === "boolean"
  ) {
    return {
      ...envelope,
      type: value.type,
      payload: { dirty: value.payload.dirty },
    };
  }
  if (value.type === "RESOURCE_CHANGED") {
    if (!config) return null;
    const resource = resolveBusinessResource(value.payload.resource, config);
    return resource
      ? { ...envelope, type: value.type, payload: { resource } }
      : null;
  }
  if (value.type === "ACTION_COMPLETED" || value.type === "ACTION_FAILED") {
    const payload = parseActionPayload(value.payload);
    return payload ? { ...envelope, type: value.type, payload } : null;
  }
  if (value.type === "DATA_CHANGED") {
    if (
      value.payload.traceId !== undefined &&
      !isBoundedText(value.payload.traceId, 128)
    )
      return null;
    let resource: BusinessResource | undefined;
    if (value.payload.resource !== undefined) {
      if (!config) return null;
      resource =
        resolveBusinessResource(value.payload.resource, config) ?? undefined;
      if (!resource) return null;
    }
    return {
      ...envelope,
      type: value.type,
      payload: {
        ...(resource ? { resource } : {}),
        ...(typeof value.payload.traceId === "string"
          ? { traceId: value.payload.traceId }
          : {}),
      },
    };
  }
  return null;
}

export function readBusinessBridgeEvent(
  event: MessageEvent,
  expectedSource: Window | null,
  config: BusinessDockConfig,
  expectedNonce?: string,
): InboundBusinessBridgeMessage | null {
  if (event.origin !== config.origin || event.source !== expectedSource)
    return null;
  const message = parseInboundBusinessBridgeMessage(
    event.data,
    config,
    expectedNonce,
  );
  if (message?.type !== "ROUTE_CHANGED") return message;
  const resource = resolveBusinessResource(message.payload.url, config);
  if (!resource) return null;
  return {
    ...message,
    payload: { url: new URL(resource.path, `${config.origin}/`).href },
  };
}

export function createBusinessBridgeMessage(
  type: "HOST_INIT" | "SET_THEME" | "REFRESH" | "NAVIGATE",
  payload?: unknown,
): BusinessBridgeV1Message {
  return { version: 1, type, payload };
}

export function createBusinessBridgeV2Message<T>(
  type: BusinessHostBridgeType,
  sessionNonce: string,
  payload?: T,
  requestId = createBusinessRequestId(),
): BusinessBridgeEnvelope<T> {
  return { version: 2, type, requestId, sessionNonce, payload };
}

export function createBusinessBridgeV3Message<T>(
  type: BusinessHostBridgeType,
  sessionNonce: string,
  payload?: T,
  requestId = createBusinessRequestId(),
): BusinessBridgeEnvelope<T, 3> {
  return { version: 3, type, requestId, sessionNonce, payload };
}

export function createBusinessRequestId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `req-${Date.now().toString(36)}`;
}

export function createBusinessSessionNonce(): string {
  const bytes = new Uint8Array(24);
  if (globalThis.crypto?.getRandomValues) {
    globalThis.crypto.getRandomValues(bytes);
    return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
      "",
    );
  }
  return `${Date.now().toString(36)}-${createBusinessRequestId()}`;
}
