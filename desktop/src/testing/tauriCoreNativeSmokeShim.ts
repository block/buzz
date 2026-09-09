/**
 * e2e-only replacement for `@tauri-apps/api/core`'s `invoke` (aliased in
 * `vite.config.ts`, `mode === "e2e"` only). Every other export passes
 * through unchanged. Needed because the installed Tauri runtime defines
 * `window.__TAURI_INTERNALS__` and its `.invoke` via `Object.defineProperty`
 * with no `writable`/`configurable`, so nothing can patch `invoke` at
 * runtime — this intercepts the exported function itself instead.
 *
 * Coverage: only `invoke(...)` calls made through this export are split by
 * `PLUGIN_SMOKE_PASSTHROUGH_COMMANDS` — that covers every `plugin_*` command
 * and Buzz's own app-level commands, which all import `invoke` from
 * `@tauri-apps/api/core`. It does NOT cover `checkPermissions`,
 * `requestPermissions`, or `Resource` methods re-exported below: those call
 * the real module's own internal `invoke` binding directly, not this
 * export, so any command they issue always reaches the real native bridge
 * regardless of the passthrough set. That's acceptable here — this smoke
 * flow doesn't mock permission checks or resource cleanup, and native
 * window/event SDK behavior being real during a native-smoke run is the
 * intended behavior, not a gap.
 */
export * from "../../node_modules/@tauri-apps/api/core.js";
import {
  invoke as realInvoke,
  type InvokeArgs,
  type InvokeOptions,
} from "../../node_modules/@tauri-apps/api/core.js";
import { PLUGIN_SMOKE_PASSTHROUGH_COMMANDS } from "@/features/plugins/browserChrome";

type MockInvokeWindow = Window & {
  __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
    command: string,
    payload: unknown,
  ) => Promise<unknown>;
};

/**
 * Passthrough commands reach the real native invoke. Every other command
 * goes to the e2e mock handler `testing/e2eBridge.ts` installs on
 * `window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__`. That handler may not be
 * installed yet for calls made before bootstrap finishes — falling back to
 * the real invoke in that case is strictly safer than throwing.
 */
export function invoke<T>(
  command: string,
  args?: InvokeArgs,
  options?: InvokeOptions,
): Promise<T> {
  if (PLUGIN_SMOKE_PASSTHROUGH_COMMANDS.has(command)) {
    return realInvoke<T>(command, args, options);
  }
  const mock = (window as MockInvokeWindow).__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
  if (!mock) {
    return realInvoke<T>(command, args, options);
  }
  return mock(command, args ?? null) as Promise<T>;
}
