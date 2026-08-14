import * as React from "react";

/**
 * App-global registry of volatile, reload-destroyed work.
 *
 * The idle auto-reload backstop (`useIdleAutoReload`) navigates the webview
 * (`location.reload()`), discarding the renderer heap. Any work that lives only
 * in renderer memory — a hook-local queue of local files, unsent composer text a
 * draft cannot serialize, an open native picker whose result has not returned —
 * would be lost if the reload fired mid-flight. Each such source registers here;
 * {@link isAnyVolatileWorkPending} folds them into the single blocker the policy
 * reads, so a new volatile-work class is guarded by registering it, not by
 * threading another gate through the reload predicate.
 *
 * Two registration shapes, because volatile work comes in two lifetimes:
 *
 *  - {@link registerVolatileWorkPredicate} — a module-level store (e.g. the
 *    background upload store) whose pending state is read on demand. Registered
 *    once at import; never released.
 *  - {@link acquireVolatileWork} / {@link useRegisterVolatileWork} — a bounded
 *    operation or a mounted component's live state. Acquiring returns a release
 *    handle; the work counts as pending until it is released. A leaked handle
 *    blocks the backstop forever and nothing surfaces it, so release is wired to
 *    both unmount and the `active` flag flipping false (see the hook).
 */

/**
 * Live acquire handles, keyed so repeated acquisition from one owner (a
 * component key, an operation) collapses to a single hold. StrictMode double-
 * invokes effects (acquire → release → acquire); a keyed map is idempotent
 * under that replay where a bare counter would drift.
 */
const holders = new Set<string>();

/** On-demand predicates contributed by module-level stores. */
const predicates = new Set<() => boolean>();

/**
 * Register a predicate read at reload-check time. Returns an unregister fn for
 * symmetry and tests; module-level stores register once and never unregister.
 */
export function registerVolatileWorkPredicate(
  predicate: () => boolean,
): () => void {
  predicates.add(predicate);
  return () => {
    predicates.delete(predicate);
  };
}

/**
 * Mark a bounded volatile operation as in progress under `key`, returning a
 * release handle. Repeated acquisition under the same key is idempotent (the
 * key is either held or not); the returned release clears that key. Callers own
 * exactly one release call per acquire — a native-picker wrapper releases in a
 * `finally`, a component effect releases on cleanup.
 */
export function acquireVolatileWork(key: string): () => void {
  holders.add(key);
  return () => {
    holders.delete(key);
  };
}

/** Monotonic suffix so overlapping {@link withVolatileWork} calls hold distinct
 * keys (e.g. the composer paperclip and a custom-emoji upload running at once). */
let workSeq = 0;

/**
 * Run an async operation while holding a volatile-work key, releasing it once
 * the operation settles — resolve OR reject. The single seam for bounded
 * reload-destroyed work (native pickers, IPC uploads): the `finally` cannot
 * leak the hold, and a per-call key keeps concurrent operations independent.
 */
export async function withVolatileWork<T>(
  label: string,
  run: () => Promise<T>,
): Promise<T> {
  const release = acquireVolatileWork(`${label}-${workSeq++}`);
  try {
    return await run();
  } finally {
    release();
  }
}

/**
 * True while any registered source has volatile work outstanding — a held
 * acquire key or a predicate that reports pending. Read imperatively by the
 * idle backstop at check time; holds no subscription.
 */
export function isAnyVolatileWorkPending(): boolean {
  if (holders.size > 0) return true;
  for (const predicate of predicates) {
    if (predicate()) return true;
  }
  return false;
}

/**
 * Register a mounted component's live volatile state with the registry.
 *
 * While `active` is true the component holds a unique per-mount key (from
 * {@link React.useId}, so two composers never collide); when `active` flips
 * false the hold releases, and unmount releases it too. Both paths matter: a
 * component that clears its work but stays mounted must stop blocking, and a
 * component torn down mid-work (the forum view closing with unsent text) must
 * not leak its key.
 *
 * StrictMode double-invokes this effect (acquire → release → acquire); because
 * the key is stable across the replay and the registry is keyed, the final
 * state is a single hold, matching production's single invoke.
 */
export function useRegisterVolatileWork(active: boolean): void {
  const key = React.useId();
  React.useEffect(() => {
    if (!active) return;
    const release = acquireVolatileWork(key);
    return release;
  }, [active, key]);
}

/** Clear all registrations. Tests only — production never resets. */
export function _resetVolatileWorkForTests(): void {
  holders.clear();
  predicates.clear();
}
