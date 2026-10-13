// Dependency-free bridge between the SW's push arrivals and the in-app notifier / app badge (web only).
//
// iOS rule: Safari web push is userVisibleOnly — every push MUST show a notification, and pushes that don't can get the
// subscription revoked. So the service worker always shows the push notification, and when the app is open the in-app
// OS notification for the same event stands down instead (it waits briefly for the matching push, then only fires if
// the push never came — e.g. a channel message push doesn't cover).
const STATE_KEY = "buzz-web-push-state.v2";
const arrived = new Map<string, number>();
const waiters = new Map<string, Array<() => void>>();
let pushCount = 0;
let lastPushAt = 0;

export function notePushArrived(eventId: string | null | undefined) {
  pushCount += 1;
  lastPushAt = Date.now();
  if (!eventId) return;
  const now = Date.now();
  arrived.set(eventId, now);
  for (const [k, t] of arrived) if (now - t > 5 * 60_000) arrived.delete(k);
  for (const f of waiters.get(eventId) ?? []) f();
  waiters.delete(eventId);
}
export const pushStats = () => ({ pushCount, lastPushAt });

/** This device has a live web push registration (so pushes will cover DMs/mentions/replies/approvals). */
export function webPushActive(): boolean {
  try {
    return (
      typeof Notification !== "undefined" &&
      Notification.permission === "granted" &&
      !!localStorage.getItem(STATE_KEY)
    );
  } catch {
    return false;
  }
}

/** Resolves true if a push for `eventId` arrives (or already arrived) within `ms`. */
export function waitForPush(eventId: string, ms: number): Promise<boolean> {
  if (arrived.has(eventId)) return Promise.resolve(true);
  return new Promise((resolve) => {
    const done = () => {
      clearTimeout(t);
      resolve(true);
    };
    const t = setTimeout(() => {
      const list = waiters.get(eventId)?.filter((f) => f !== done);
      if (list?.length) waiters.set(eventId, list);
      else waiters.delete(eventId);
      resolve(false);
    }, ms);
    waiters.set(eventId, [...(waiters.get(eventId) ?? []), done]);
  });
}

/** App badge on the Home Screen icon: real unread total while the app runs; the SW increments it per push. */
export function syncWebAppBadge(count: number) {
  const nav = navigator as Navigator & {
    setAppBadge?: (n?: number) => Promise<void>;
    clearAppBadge?: () => Promise<void>;
  };
  const n = Math.max(0, Math.floor(count));
  if (n > 0) void nav.setAppBadge?.(n).catch(() => undefined);
  else void nav.clearAppBadge?.().catch(() => undefined);
  if (typeof indexedDB === "undefined") return;
  const r = indexedDB.open("buzz-push", 1);
  r.onupgradeneeded = () => r.result.createObjectStore("kv");
  r.onsuccess = () => {
    try {
      r.result.transaction("kv", "readwrite").objectStore("kv").put(n, "badge");
    } catch {
      /* ignore */
    }
  };
}
