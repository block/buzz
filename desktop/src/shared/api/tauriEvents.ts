import { isTauri } from "@tauri-apps/api/core";
import {
  listen,
  type EventCallback,
  type EventName,
  type Options,
  type UnlistenFn,
} from "@tauri-apps/api/event";

const noopUnlisten: UnlistenFn = () => {};

type TauriBridgeGlobal = typeof globalThis & {
  __TAURI_INTERNALS__?: {
    transformCallback?: unknown;
  };
};

function hasTauriCallbackBridge(): boolean {
  const internals = (globalThis as TauriBridgeGlobal).__TAURI_INTERNALS__;
  return typeof internals?.transformCallback === "function";
}

/**
 * Browser-safe wrapper for native Tauri event subscriptions.
 *
 * The desktop frontend can also be opened through Vite alone, where the Tauri
 * callback bridge does not exist. Guard before calling `listen()` so startup
 * hooks can mount without tripping over missing `transformCallback` internals.
 */
export async function listenTauriEvent<T>(
  event: EventName,
  handler: EventCallback<T>,
  options?: Options,
): Promise<UnlistenFn> {
  if (!isTauri() || !hasTauriCallbackBridge()) {
    return noopUnlisten;
  }

  return listen<T>(event, handler, options);
}
