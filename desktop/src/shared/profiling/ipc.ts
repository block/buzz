// Temporary renderer profiling harness (never merges) — IPC invoke probe core.
//
// Pure, Tauri-free logic so it is unit-testable without harness.ts's
// module-load Tauri/store imports (matching drift.ts). The module seam that
// actually wraps `@tauri-apps/api/core`'s `invoke` lives in tauriCoreProxy.ts;
// this file only decides what to record.

/** A command outstanding >10s is logged with its age — the deadlock detector. */
export const PENDING_INVOKE_MS = 10_000;
/** Watchdog scan cadence. */
export const PENDING_SCAN_MS = 5_000;

export type InvokeFn = (
  cmd: string,
  args?: unknown,
  opts?: unknown,
) => Promise<unknown>;

export type PendingInvoke = { cmd: string; startedAt: number };

/**
 * Observes one invoke: `begin(cmd)` marks it outstanding and returns a `settle`
 * that records duration/outcome and clears it.
 */
export type InvokeObserver = { begin: (cmd: string) => (ok: boolean) => void };

/**
 * Build an observer that tracks each call in `pending` (for the watchdog) and
 * records an `ipc` result on settle.
 */
export function createInvokeObserver(
  pending: Map<number, PendingInvoke>,
  record: (cmd: string, dur: number, ok: boolean) => void,
  now: () => number = () => performance.now(),
): InvokeObserver {
  let seq = 0;
  return {
    begin(cmd) {
      const id = seq++;
      const startedAt = now();
      pending.set(id, { cmd, startedAt });
      return (ok) => {
        pending.delete(id);
        record(cmd, now() - startedAt, ok);
      };
    },
  };
}

// Module-level observer registry. The core proxy wraps `invoke` at module load
// (before the harness starts), but only reports once the harness registers its
// observer. Until then the wrapper is a transparent pass-through.
let activeObserver: InvokeObserver | null = null;

/** Register the observer that live invoke calls report to. */
export function setInvokeObserver(observer: InvokeObserver | null): void {
  activeObserver = observer;
}

/** The currently registered observer, or `null` when the harness is off. */
export function getInvokeObserver(): InvokeObserver | null {
  return activeObserver;
}

/**
 * Wrap a real `invoke` so each call routes through the current observer (or
 * passes straight through when none is registered). The wrapper never touches
 * `window.__TAURI_INTERNALS__`: native Tauri defines `invoke` as a
 * non-configurable, non-writable value property, so redefining it throws and
 * would abort the whole harness. Intercepting at the module layer instead keeps
 * the probe passive against native, terminal-accessor, and mockIPC shapes
 * alike — the real core module dereferences the window property per call, so a
 * module wrapper sees every call without owning the property.
 */
export function wrapInvoke(
  real: InvokeFn,
  getObserver: () => InvokeObserver | null,
): InvokeFn {
  return (cmd, args, opts) => {
    const observer = getObserver();
    if (!observer) {
      return real(cmd, args, opts);
    }
    const settle = observer.begin(cmd);
    return real(cmd, args, opts).then(
      (value) => {
        settle(true);
        return value;
      },
      (error) => {
        settle(false);
        throw error;
      },
    );
  };
}
