import { listen as realListen } from "@tauri-apps/api/event";

type ListenFn = <T>(
  eventName: string,
  handler: (event: { payload: T }) => void,
) => Promise<() => void>;

/**
 * Registers a one-shot event listener and exposes `ready` (resolves once the
 * native `listen` registration itself completes) separately from `result`
 * (resolves once a matching event arrives). Callers must await `ready`
 * before triggering whatever action might produce the event — otherwise a
 * fast native response racing the async `listen()` registration is lost.
 *
 * If the registration itself fails, both `ready` and `result` reject with
 * the same cause and the timeout is cleared — a caller that only awaits
 * `ready` before acting must see the failure rather than hang forever.
 *
 * `listenFn` defaults to the real Tauri `listen` — tests inject a fake to
 * exercise the registration-failure path deterministically.
 */
export function subscribeOnce<T>(
  eventName: string,
  matches: (payload: T) => boolean,
  timeoutMs = 30_000,
  listenFn: ListenFn = realListen,
): { ready: Promise<void>; result: Promise<T> } {
  let unlisten: (() => void) | null = null;
  let markReady = () => {};
  let markReadyFailed = (_cause: unknown) => {};
  const ready = new Promise<void>((resolve, reject) => {
    markReady = resolve;
    markReadyFailed = reject;
  });
  const result = new Promise<T>((resolve, reject) => {
    // `listenFn`'s callback can fire — and settle `result` — before its own
    // registration promise resolves (Tauri wires the callback synchronously;
    // the promise only tracks the native IPC round-trip). `settled` catches
    // that race so the late-arriving `unlisten` is disposed immediately
    // instead of being stored where nothing will ever call it again.
    let settled = false;
    const timeout = window.setTimeout(() => {
      settled = true;
      unlisten?.();
      reject(new Error(`timed out waiting for "${eventName}"`));
    }, timeoutMs);
    listenFn<T>(eventName, (event) => {
      if (!matches(event.payload)) return;
      settled = true;
      window.clearTimeout(timeout);
      unlisten?.();
      resolve(event.payload);
    }).then(
      (fn) => {
        if (settled) {
          fn();
        } else {
          unlisten = fn;
        }
        markReady();
      },
      (cause) => {
        window.clearTimeout(timeout);
        reject(cause);
        markReadyFailed(cause);
      },
    );
  });
  // `ready`'s rejection is what a caller following the documented
  // ready-then-act order actually sees — it can abandon `result` without
  // ever awaiting it, which would otherwise surface as an unhandled
  // rejection here. `result` is still fully usable by any caller that does
  // await it; a promise can carry more than one reaction.
  result.catch(() => {});
  return { ready, result };
}
