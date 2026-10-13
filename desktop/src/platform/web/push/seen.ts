// Dependency-free bridge between the SW's push arrivals and the in-app notifier / app badge (web only).
//
// iOS rule: Safari web push is userVisibleOnly — every push MUST show a notification, and pushes that don't can get the
// subscription revoked. So the service worker always shows the push notification, and when the app is open the in-app
// OS notification for the same event stands down instead (it waits briefly for the matching push, then only fires if
// the push never came — e.g. a channel message push doesn't cover).
const STATE_KEY = "buzz-web-push-state.v2";
/** Persisted "push is live on this device" flag, readable synchronously at startup (set after a lease is accepted). */
export const PUSH_ACTIVE_KEY = "buzz-web-push-active";
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
      (localStorage.getItem(PUSH_ACTIVE_KEY) === "1" ||
        !!localStorage.getItem(STATE_KEY))
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

export function setWebPushActive(on: boolean) {
  try {
    if (on) localStorage.setItem(PUSH_ACTIVE_KEY, "1");
    else localStorage.removeItem(PUSH_ACTIVE_KEY);
  } catch {
    /* storage denied */
  }
}

// ---- foreground time: events created before the app came to the foreground are catch-up, never "new" ----
let foregroundAtSecs = Math.floor(Date.now() / 1000);
if (typeof document !== "undefined") {
  const mark = () => {
    if (document.visibilityState === "visible")
      foregroundAtSecs = Math.floor(Date.now() / 1000);
  };
  document.addEventListener("visibilitychange", mark);
  window.addEventListener("pageshow", mark);
}
/** Unix seconds of the latest app load / return to foreground. */
export const foregroundAt = () => foregroundAtSecs;

// ---- event ids the SW already showed a push notification for (persisted in IDB "notified") ----
function readNotified(): Promise<string[]> {
  return new Promise((resolve) => {
    if (typeof indexedDB === "undefined") return resolve([]);
    const r = indexedDB.open("buzz-push", 1);
    r.onupgradeneeded = () => r.result.createObjectStore("kv");
    r.onerror = () => resolve([]);
    r.onsuccess = () => {
      try {
        const g = r.result
          .transaction("kv", "readonly")
          .objectStore("kv")
          .get("notified");
        g.onsuccess = () => resolve(Array.isArray(g.result) ? g.result : []);
        g.onerror = () => resolve([]);
      } catch {
        resolve([]);
      }
    };
  });
}
/** True if the SW already notified for this event (live message or persisted list). */
export async function pushAlreadyNotified(eventId: string): Promise<boolean> {
  if (arrived.has(eventId)) return true;
  return (await readNotified()).includes(eventId);
}

/**
 * Single gate for local OS notifications on web. Returns true when the local one must NOT fire:
 * - catch-up/backfill events (created before the app came to the foreground) never notify;
 * - with web push active on this device, message events never notify locally (the push is the one notification);
 * - an event the SW already showed a push for never notifies again.
 */
export async function suppressLocalNotification(
  target:
    | {
        eventId?: string | null;
        createdAt?: number | null;
      }
    | undefined,
): Promise<boolean> {
  if (!target) return false;
  if (
    typeof target.createdAt === "number" &&
    target.createdAt < foregroundAtSecs - 10
  )
    return true;
  if (!target.eventId) return false;
  if (webPushActive()) return true;
  return pushAlreadyNotified(target.eventId);
}

/** Close the push notifications for a conversation once it is read (channel id or `thread:<rootId>` read key). */
export async function closePushNotificationsFor(readKey: string) {
  if (typeof navigator === "undefined" || !navigator.serviceWorker) return;
  const reg = await navigator.serviceWorker
    .getRegistration()
    .catch(() => undefined);
  const list = (await reg?.getNotifications().catch(() => [])) ?? [];
  for (const n of list) {
    const t = (
      n.data as {
        target?: { channelId?: string; threadRootId?: string };
      } | null
    )?.target;
    if (!t) continue;
    if (
      t.channelId === readKey ||
      (t.threadRootId && `thread:${t.threadRootId}` === readKey)
    )
      n.close();
  }
}
