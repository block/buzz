import type { RemoteHostConnection } from "./types";

const STORAGE_KEY = "buzz.remote-agents.host.v1";

/**
 * v1: localStorage for connection metadata.
 * Codex gate: do not log the token; migrate to OS keyring in a follow-up.
 */
export function loadRemoteHostConnection(): RemoteHostConnection | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as RemoteHostConnection;
    if (!parsed?.baseUrl || !parsed?.token) return null;
    return {
      label: parsed.label || "home",
      baseUrl: parsed.baseUrl.replace(/\/$/, ""),
      token: parsed.token,
      defaultRoom: parsed.defaultRoom || "",
    };
  } catch {
    return null;
  }
}

export function saveRemoteHostConnection(conn: RemoteHostConnection): void {
  const safe: RemoteHostConnection = {
    label: conn.label.trim() || "home",
    baseUrl: conn.baseUrl.trim().replace(/\/$/, ""),
    token: conn.token.trim(),
    defaultRoom: (conn.defaultRoom || "").trim(),
  };
  localStorage.setItem(STORAGE_KEY, JSON.stringify(safe));
}

export function clearRemoteHostConnection(): void {
  localStorage.removeItem(STORAGE_KEY);
}
