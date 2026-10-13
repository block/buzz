// Pure capability checks (unit-tested): what the current browser can do for push.
export type PushEnv = {
  hasSw: boolean;
  hasPush: boolean;
  hasNotification: boolean;
  ios: boolean;
  standalone: boolean;
  permission: NotificationPermission | "unsupported";
};

export type PushUiState =
  | "unsupported" // no Push API at all (old iOS, iOS Safari tab without install)
  | "ios-install" // iOS Safari tab: must Add to Home Screen first
  | "needs-tap" // permission not asked yet; a user gesture is required (iOS, Firefox, Safari)
  | "denied" // user blocked notifications
  | "ready"; // permission granted; subscribe silently

export function classifyPushEnv(env: PushEnv): PushUiState {
  if (env.ios && !env.standalone) return "ios-install";
  if (
    !env.hasSw ||
    !env.hasPush ||
    !env.hasNotification ||
    env.permission === "unsupported"
  )
    return "unsupported";
  if (env.permission === "denied") return "denied";
  if (env.permission === "granted") return "ready";
  return "needs-tap";
}

const GRANTED_KEY = "buzz-web-push-granted.v1";

/** Remember the OS answer from requestPermission(): iOS standalone can keep `Notification.permission` at "default" for a
 *  while after the user tapped Allow, which kept the "Turn on notifications" banner up and skipped the subscribe. */
export function markPermissionGranted(): void {
  try {
    localStorage.setItem(GRANTED_KEY, "1");
  } catch {
    /* storage denied */
  }
}

/** The live permission, except a stale "default" after a remembered grant reads as "granted". "denied" always wins
 *  (and forgets the grant), so turning notifications off in iOS Settings shows the blocked hint again. */
export function effectivePermission(live: NotificationPermission): NotificationPermission {
  try {
    if (live === "granted") localStorage.setItem(GRANTED_KEY, "1");
    else if (live === "denied") localStorage.removeItem(GRANTED_KEY);
    else if (localStorage.getItem(GRANTED_KEY) === "1") return "granted";
  } catch {
    /* storage denied or not in a browser */
  }
  return live;
}

export function readPushEnv(): PushEnv {
  const nav = navigator as Navigator & { standalone?: boolean };
  const ua = nav.userAgent || "";
  const ios =
    /iPad|iPhone|iPod/.test(ua) ||
    (ua.includes("Macintosh") && (nav.maxTouchPoints ?? 0) > 1);
  const standalone =
    nav.standalone === true ||
    window.matchMedia?.("(display-mode: standalone)").matches === true;
  const hasNotification = typeof Notification !== "undefined";
  return {
    hasSw: "serviceWorker" in nav,
    hasPush: "PushManager" in window,
    hasNotification,
    ios,
    standalone,
    permission: hasNotification ? effectivePermission(Notification.permission) : "unsupported",
  };
}

/** Notification prefs the push sender honors, read from the shared notification settings (v2) in localStorage. */
export type PushPrefs = {
  dm: boolean;
  mention: boolean;
  thread_reply: boolean;
  needs_action: boolean;
};
export function pushPrefsFromSettings(raw: string | null): PushPrefs {
  const all = {
    dm: true,
    mention: true,
    thread_reply: true,
    needs_action: true,
  };
  if (!raw) return all;
  try {
    const s = JSON.parse(raw) as {
      desktopEnabled?: boolean;
      slotAlertsEnabled?: Partial<Record<keyof PushPrefs, boolean>>;
    };
    if (s.desktopEnabled === false)
      return {
        dm: false,
        mention: false,
        thread_reply: false,
        needs_action: false,
      };
    const a = s.slotAlertsEnabled ?? {};
    return {
      dm: a.dm !== false,
      mention: a.mention !== false,
      thread_reply: a.thread_reply !== false,
      needs_action: a.needs_action !== false,
    };
  } catch {
    return all;
  }
}
