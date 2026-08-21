// Temporary renderer profiling harness (never merges) — pure resolver decision
// for routing every `@tauri-apps/api` core import through tauriCoreProxy.ts.
//
// A bare-specifier alias only catches `import ... from "@tauri-apps/api/core"`.
// Tauri's own submodules (`event`, `app`, `window`, `webview`, …) reach core
// via the RELATIVE import `import { invoke } from "./core.js"`, which an alias
// never sees — so their `plugin:event|*`, `plugin:window|*`, … traffic would
// bypass the wrapper and go unrecorded. A Vite `resolveId` hook can see the
// relative form (via the importer) and redirect it too, making the module seam
// canonical for the resolved core module regardless of how it is imported.
//
// This module holds only the pure decision so it is unit-testable without a
// Vite/Node type dependency (importing `vite` here would pull Node's global
// typings into the DOM-typed src program). The thin `Plugin` wrapper that calls
// it lives in vite.config.ts.

/** Specifier the proxy uses to reach the real core implementation. */
export const REAL_CORE_SPECIFIER = "@tauri-core-impl";

/**
 * Decide where a module specifier should resolve so that every Tauri core
 * import lands on the profiling proxy, while the proxy's own escape import
 * reaches the real core without looping.
 *
 * Returns `proxyId` to redirect through the proxy, `realCoreId` for the escape
 * hatch, or `null` to fall through to Vite's normal resolution.
 */
export function resolveTauriCore(
  source: string,
  importer: string | undefined,
  proxyId: string,
  realCoreId: string,
): string | null {
  // Escape hatch: the proxy asks for the real implementation. Checked first so
  // the proxy's own re-export never loops back into itself.
  if (source === REAL_CORE_SPECIFIER) return realCoreId;
  // Any other import originating in the proxy passes through untouched.
  if (importer === proxyId) return null;
  // Bare specifier — app code and external plugins (`plugin-opener`, …).
  if (source === "@tauri-apps/api/core") return proxyId;
  // Relative import from inside Tauri's own API package (`event`, `app`,
  // `window`, `webview`, …), which resolve core as `./core.js`.
  if (source === "./core.js" && importer?.includes("/@tauri-apps/api/")) {
    return proxyId;
  }
  return null;
}
