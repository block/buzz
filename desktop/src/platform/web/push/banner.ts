import { onResume } from "@/shared/lib/resumeSignals";
// Dismissible top banner nudging the user to turn on push (vanilla DOM, like the signer overlay; no AppShell edits).
import {
  bannerDismissed,
  dismissBanner,
  readPushEnabled,
  WEB_PUSH_CHANGE_EVENT,
  WEB_PUSH_FLAG,
} from "./config";
import {
  enablePushFromTap,
  ensurePushSubscribed,
  installPushClickRouting,
  pushUiState,
} from "./client";
import type { PushUiState } from "./env";

const COPY = {
  "ios-install":
    "Get notifications when Hula Buzz is closed: tap Share, then Add to Home Screen, open the app from your Home Screen and tap Enable.",
  "needs-tap":
    "Turn on notifications so mentions, DMs and approvals reach you even when the app is closed.",
  denied:
    "Notifications are blocked for HulaBuzz. On iPhone: Settings → Notifications → HulaBuzz → Allow Notifications.",
} as const;

type BannerState = keyof typeof COPY;

let el: HTMLDivElement | null = null;
let renderGen = 0;

function hide() {
  el?.remove();
  el = null;
}

function permissionGranted(): boolean {
  return (
    typeof Notification !== "undefined" && Notification.permission === "granted"
  );
}

/** True when OS permission is granted and the browser has an active push subscription. */
async function grantedAndSubscribed(): Promise<boolean> {
  if (!permissionGranted()) return false;
  try {
    if (!("serviceWorker" in navigator) || !("PushManager" in window))
      return true; // granted is enough to hide the nudge
    const reg =
      (await navigator.serviceWorker.getRegistration()) ??
      (await navigator.serviceWorker.ready.catch(() => null));
    const sub = await reg?.pushManager.getSubscription();
    return !!sub;
  } catch {
    return true;
  }
}

function bannerShell(): HTMLDivElement {
  const node = document.createElement("div");
  node.setAttribute("role", "status");
  // Theme tokens are HSL components (see ThemeProvider); wrap with hsl() so the
  // banner paints a solid themed surface instead of an invalid/transparent color.
  node.style.cssText =
    "position:fixed;left:8px;right:8px;top:calc(env(safe-area-inset-top,0px) + 8px);z-index:2147483000;display:flex;gap:10px;align-items:center;" +
    "padding:10px 12px;border-radius:14px;background:hsl(var(--card));color:hsl(var(--card-foreground));" +
    "border:1px solid hsl(var(--border));box-shadow:0 6px 24px rgba(0,0,0,.18);" +
    "font:14px/1.35 system-ui,-apple-system,sans-serif;max-width:640px;margin:0 auto";
  return node;
}

function appendDismiss(node: HTMLDivElement) {
  const close = document.createElement("button");
  close.type = "button";
  close.setAttribute("aria-label", "Dismiss");
  close.setAttribute("data-web-push-dismiss", "");
  close.textContent = "×";
  close.style.cssText =
    "border:0;background:transparent;color:inherit;font-size:20px;line-height:1;padding:0 4px";
  close.onclick = () => {
    dismissBanner();
    hide();
  };
  node.append(close);
}

async function render() {
  const gen = ++renderGen;
  // Hide immediately once the OS has granted notifications (and when already subscribed).
  if (
    !readPushEnabled() ||
    bannerDismissed() ||
    permissionGranted() ||
    (await grantedAndSubscribed())
  ) {
    if (gen === renderGen) hide();
    return;
  }
  if (gen !== renderGen) return;

  const state = pushUiState() as PushUiState;
  const show: BannerState | null =
    state === "ios-install" || state === "needs-tap" || state === "denied"
      ? state
      : null;
  if (!show) return hide();

  hide();
  el = bannerShell();
  el.setAttribute("data-web-push-banner", show);
  const text = document.createElement("span");
  text.style.flex = "1";
  text.textContent = COPY[show];
  el.append(text);

  if (show === "needs-tap") {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.textContent = "Enable";
    btn.setAttribute("data-web-push-enable", "");
    // Outlined Enable (matches Button variant="outline" + primary accent).
    btn.style.cssText =
      "border:1px solid hsl(var(--primary));border-radius:10px;padding:6px 12px;font-weight:600;" +
      "background:transparent;color:hsl(var(--primary))";
    btn.onclick = async () => {
      btn.disabled = true;
      const next = await enablePushFromTap();
      // Hide the moment the OS returns "granted" (subscribe already attempted).
      if (next === "ready" || permissionGranted()) hide();
      window.dispatchEvent(new CustomEvent(WEB_PUSH_CHANGE_EVENT));
      void render();
    };
    el.append(btn);
  }

  appendDismiss(el);
  document.body.append(el);
}

function watchPermissionChanges() {
  try {
    const perms = navigator.permissions;
    if (!perms?.query) return;
    void perms
      .query({ name: "notifications" as PermissionName })
      .then((status) => {
        status.onchange = () => {
          void render();
          if (permissionGranted()) {
            void ensurePushSubscribed().catch(() => false);
          }
        };
      })
      .catch(() => undefined);
  } catch {
    /* Permissions API missing or notifications name unsupported */
  }
}

/** Boot hook: no-op unless the push service flag is set at build time. */
export function startWebPush() {
  if (!WEB_PUSH_FLAG) return;
  installPushClickRouting();
  watchPermissionChanges();
  // Re-send the subscription + lease whenever the app opens or comes back (the relay may have dropped it).
  // After a failure, wait 10 minutes before asking the signer again so a rejecting relay can't prompt on every resume.
  let failedAt = 0;
  const sync = () => {
    if (failedAt && Date.now() - failedAt < 10 * 60_000) return;
    void ensurePushSubscribed()
      .then(() => {
        failedAt = 0;
        void render(); // hide once subscribe succeeds after a grant
      })
      .catch(() => {
        failedAt = Date.now();
      });
  };
  sync();
  void render();
  setTimeout(() => void render(), 1500);
  window.addEventListener(WEB_PUSH_CHANGE_EVENT, () => {
    void render();
    sync();
  });
  // visible / pageshow / focus / online: re-check permission + subscription and refresh the lease
  onResume(() => {
    void render();
    sync();
  });
}
