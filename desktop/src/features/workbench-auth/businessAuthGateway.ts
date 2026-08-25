import { isTauri } from "@tauri-apps/api/core";

import { signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";

export type EnterpriseUserSummary = {
  id: string;
  email?: string | null;
  displayName: string;
  status: "active" | "disabled";
};

export type BuzzIdentityBinding = {
  id: string;
  buzzPubkey: string;
  deviceId: string;
  deviceName: string;
  devicePlatform: "macos" | "windows" | "linux" | "web";
  status: "active" | "revoked";
  boundAt: string;
  lastSeenAt: string;
  revokedAt?: string | null;
  version: number;
};

export type GatewayMe = {
  user: EnterpriseUserSummary;
  workbenchSessionId: string;
  bindings: BuzzIdentityBinding[];
};

export type WorkbenchAuthState =
  | { status: "initializing" }
  | { status: "unauthenticated" }
  | { status: "authenticating" }
  | {
      status: "authenticated";
      user: EnterpriseUserSummary;
      binding: BuzzIdentityBinding;
      workbenchSessionId: string;
    }
  | {
      status: "binding_required";
      user: EnterpriseUserSummary;
      workbenchSessionId: string;
    }
  | {
      status: "device_revoked";
      user: EnterpriseUserSummary;
      workbenchSessionId: string;
    }
  | { status: "error"; error: string };

const DEVICE_ID_KEY = "bizfin.workbench.device-id.v1";

export function getBusinessAuthGatewayUrl(): string | null {
  const value = import.meta.env.VITE_BUSINESS_AUTH_GATEWAY_URL?.trim();
  if (!value) return null;
  try {
    const url = new URL(value);
    if (!["https:", "http:"].includes(url.protocol) || url.pathname !== "/")
      return null;
    return url.origin;
  } catch {
    return null;
  }
}

export function getOrCreateDeviceId(storage: Storage): string {
  const current = storage.getItem(DEVICE_ID_KEY);
  if (current && /^[0-9a-f-]{36}$/.test(current)) return current;
  const next = crypto.randomUUID();
  storage.setItem(DEVICE_ID_KEY, next);
  return next;
}

async function gatewayFetch<T>(
  gateway: string,
  token: string,
  path: string,
  init: RequestInit = {},
): Promise<T> {
  const response = await fetch(new URL(path, gateway), {
    ...init,
    cache: "no-store",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
      "X-Trace-Id": crypto.randomUUID(),
      ...init.headers,
    },
  });
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as {
      error?: string;
    } | null;
    throw new Error(
      body?.error ?? `Gateway request failed (${response.status})`,
    );
  }
  return (response.status === 204 ? undefined : await response.json()) as T;
}

export async function readGatewayState(
  gateway: string,
  token: string,
): Promise<WorkbenchAuthState> {
  const me = await gatewayFetch<GatewayMe>(gateway, token, "/api/me");
  if (!isTauri()) {
    return {
      status: "binding_required",
      user: me.user,
      workbenchSessionId: me.workbenchSessionId,
    };
  }
  const identity = await getIdentity();
  const deviceId = getOrCreateDeviceId(window.localStorage);
  const active = me.bindings.find(
    (binding) =>
      binding.buzzPubkey === identity.pubkey &&
      binding.deviceId === deviceId &&
      binding.status === "active",
  );
  if (active)
    return {
      status: "authenticated",
      user: me.user,
      binding: active,
      workbenchSessionId: me.workbenchSessionId,
    };
  const revoked = me.bindings.some(
    (binding) =>
      binding.buzzPubkey === identity.pubkey &&
      binding.deviceId === deviceId &&
      binding.status === "revoked",
  );
  return {
    status: revoked ? "device_revoked" : "binding_required",
    user: me.user,
    workbenchSessionId: me.workbenchSessionId,
  };
}

export async function bindCurrentDevice(
  gateway: string,
  token: string,
): Promise<WorkbenchAuthState> {
  if (!isTauri()) throw new Error("Device binding requires the Desktop app.");
  const identity = await getIdentity();
  const deviceId = getOrCreateDeviceId(window.localStorage);
  const platform = navigator.userAgent.includes("Windows")
    ? "windows"
    : navigator.userAgent.includes("Linux")
      ? "linux"
      : "macos";
  const challenge = await gatewayFetch<{
    id: string;
    payload: string;
  }>(gateway, token, "/api/identity-bindings/challenges", {
    method: "POST",
    body: JSON.stringify({
      pubkey: identity.pubkey,
      deviceId,
      deviceName: navigator.platform || "Business Dock device",
      devicePlatform: platform,
    }),
  });
  const signedEvent = await signRelayEvent({
    kind: 24243,
    content: challenge.payload,
    tags: [["aud", "bizfin-workbench-device-binding"]],
  });
  await gatewayFetch(gateway, token, "/api/identity-bindings/verify", {
    method: "POST",
    body: JSON.stringify({ challengeId: challenge.id, signedEvent }),
  });
  return readGatewayState(gateway, token);
}

export async function issueEmbedSession(
  gateway: string,
  token: string,
  target: { type: string; id: string; path: string },
): Promise<{ id: string; embedUrl: string; traceId: string }> {
  const identity = await getIdentity();
  return gatewayFetch(gateway, token, "/api/embed-sessions", {
    method: "POST",
    body: JSON.stringify({
      target,
      pubkey: identity.pubkey,
      deviceId: getOrCreateDeviceId(window.localStorage),
    }),
  });
}

export async function logoutWorkbenchSession(
  gateway: string,
  token: string,
  global: boolean,
): Promise<{ logoutUrl?: string } | undefined> {
  return gatewayFetch(
    gateway,
    token,
    global ? "/api/logout/global" : "/api/logout/workbench",
    { method: "POST", body: "{}" },
  );
}
