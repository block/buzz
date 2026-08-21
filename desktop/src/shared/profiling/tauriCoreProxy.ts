// Temporary renderer profiling harness (never merges) — @tauri-apps/api/core
// module proxy.
//
// Vite aliases `@tauri-apps/api/core` to this module (see vite.config.ts), so
// every importer — all raw `@tauri-apps/api/core` consumers, `invokeTauri()`,
// and every bundled Tauri plugin — resolves here. We re-export the real core
// surface unchanged (`export *`, which carries values and types) and override
// only `invoke`; an explicit named export shadows the star re-export of the
// same name.
//
// The real module is imported through the `@tauri-core-impl` alias (pointing at
// the actual core module) so this proxy's own import does not re-enter the
// alias. The wrapper reports to the harness's observer once registered and is a
// transparent pass-through until then. Crucially it never writes to
// `window.__TAURI_INTERNALS__`, which native Tauri defines as a
// non-configurable value property — the only interception seam that survives
// the production runtime, since the real core dereferences that property per
// call.

import { invoke as realInvoke } from "@tauri-core-impl";

import { getInvokeObserver, wrapInvoke } from "@/shared/profiling/ipc";

export * from "@tauri-core-impl";

export const invoke = wrapInvoke(
  realInvoke as (
    cmd: string,
    args?: unknown,
    opts?: unknown,
  ) => Promise<unknown>,
  getInvokeObserver,
) as typeof realInvoke;
