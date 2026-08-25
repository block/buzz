import { isTauri } from "@tauri-apps/api/core";
import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";

import type { BusinessDockConfig } from "@/features/business-dock/businessDockConfig";

const EMBED_CODE_PATTERN = /^[A-Za-z0-9_-]{43}$/;

export type BusinessSsoMode = "web-native" | "desktop-embed-session";
export type EmbedSessionPhase =
  | "idle"
  | "authorizing"
  | "redeeming"
  | "ready"
  | "failed";

export function businessSsoMode(): BusinessSsoMode {
  return isTauri() ? "desktop-embed-session" : "web-native";
}

export function buildBusinessEmbedLoginUrl(
  config: BusinessDockConfig,
  targetUrl: string,
): string | null {
  try {
    const target = new URL(targetUrl, config.homeUrl);
    if (target.origin !== config.origin) return null;
    const login = new URL("/auth/embed-login", config.origin);
    login.searchParams.set("target", `${target.pathname}${target.search}`);
    return login.href;
  } catch {
    return null;
  }
}

export function businessEmbedCallbackUrl(
  channel = import.meta.env?.VITE_DESKTOP_BUILD_CHANNEL,
): string {
  const scheme = channel === "development" ? "buzz-dev" : "buzz";
  return `${scheme}://auth/business-bootstrap`;
}

export function parseBusinessEmbedCallback(
  value: string,
  expectedCallback = businessEmbedCallbackUrl(),
): string | null {
  try {
    const url = new URL(value);
    const expected = new URL(expectedCallback);
    if (
      url.protocol !== expected.protocol ||
      url.host !== expected.host ||
      url.pathname !== expected.pathname ||
      url.hash ||
      [...url.searchParams.keys()].some((key) => key !== "code")
    )
      return null;
    const code = url.searchParams.get("code");
    return code && EMBED_CODE_PATTERN.test(code) ? code : null;
  } catch {
    return null;
  }
}

export function buildBusinessEmbedBootstrapUrl(
  config: BusinessDockConfig,
  code: string,
): string | null {
  if (!EMBED_CODE_PATTERN.test(code)) return null;
  const target = new URL("/embed/bootstrap", config.origin);
  target.searchParams.set("code", code);
  return target.href;
}

export async function subscribeToBusinessEmbedCallbacks(
  onCode: (code: string) => void,
): Promise<() => void> {
  if (!isTauri()) return () => undefined;
  const consumed = new Set<string>();
  const consume = (value: string) => {
    const code = parseBusinessEmbedCallback(value);
    if (!code || consumed.has(code)) return;
    consumed.add(code);
    onCode(code);
  };
  for (const url of (await getCurrent()) ?? []) consume(url);
  return onOpenUrl((urls) => {
    for (const url of urls) consume(url);
  });
}

export function canAttemptBusinessRecovery(attempts: number): boolean {
  return Number.isInteger(attempts) && attempts < 1;
}
