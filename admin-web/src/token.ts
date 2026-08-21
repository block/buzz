import { useSyncExternalStore } from "react";

/// Session-scoped so the credential dies with the browser tab. It is never
/// written to localStorage or a cookie, and never placed in a URL.
const STORAGE_KEY = "buzz-admin-token";

const listeners = new Set<() => void>();
let token = read();

function read(): string | null {
  try {
    return sessionStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function publish(next: string | null) {
  if (token === next) return;
  token = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getToken() {
  return token;
}

export function setToken(next: string) {
  try {
    sessionStorage.setItem(STORAGE_KEY, next);
  } catch {
    // The token still works for this page load if storage is blocked.
  }
  publish(next);
}

/// Idempotent so concurrent 401s from parallel requests re-prompt once.
export function clearToken() {
  try {
    sessionStorage.removeItem(STORAGE_KEY);
  } catch {
    // Nothing was stored; the in-memory token is authoritative.
  }
  publish(null);
}

export function useToken() {
  return useSyncExternalStore(subscribe, getToken, getToken);
}
