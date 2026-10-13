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

const COPY = {
  "ios-install":
    "Get notifications when Hula Buzz is closed: tap Share, then Add to Home Screen, open the app from your Home Screen and tap Enable.",
  "needs-tap":
    "Turn on notifications so mentions, DMs and approvals reach you even when the app is closed.",
} as const;

let el: HTMLDivElement | null = null;
function hide() {
  el?.remove();
  el = null;
}

function render() {
  const state = pushUiState();
  if (
    !readPushEnabled() ||
    bannerDismissed() ||
    (state !== "ios-install" && state !== "needs-tap")
  )
    return hide();
  hide();
  el = document.createElement("div");
  el.setAttribute("data-web-push-banner", state);
  el.setAttribute("role", "status");
  el.style.cssText =
    "position:fixed;left:8px;right:8px;top:calc(env(safe-area-inset-top,0px) + 8px);z-index:2147483000;display:flex;gap:10px;align-items:center;" +
    "padding:10px 12px;border-radius:14px;background:var(--popover,#1f1f23);color:var(--popover-foreground,#fff);" +
    "box-shadow:0 6px 24px rgba(0,0,0,.25);font:14px/1.35 system-ui,-apple-system,sans-serif;max-width:640px;margin:0 auto";
  const text = document.createElement("span");
  text.style.flex = "1";
  text.textContent = COPY[state];
  el.append(text);
  if (state === "needs-tap") {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.textContent = "Enable";
    btn.setAttribute("data-web-push-enable", "");
    btn.style.cssText =
      "border:0;border-radius:10px;padding:6px 12px;font-weight:600;background:var(--primary,#f5b400);color:var(--primary-foreground,#000)";
    btn.onclick = async () => {
      btn.disabled = true;
      await enablePushFromTap();
      render();
    };
    el.append(btn);
  }
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
  el.append(close);
  document.body.append(el);
}

/** Boot hook: no-op unless the push service flag is set at build time. */
export function startWebPush() {
  if (!WEB_PUSH_FLAG) return;
  installPushClickRouting();
  // Re-send the subscription + lease whenever the app opens or comes back (the relay may have dropped it).
  // After a failure, wait 10 minutes before asking the signer again so a rejecting relay can't prompt on every resume.
  let failedAt = 0;
  const sync = () => {
    if (failedAt && Date.now() - failedAt < 10 * 60_000) return;
    void ensurePushSubscribed()
      .then(() => {
        failedAt = 0;
      })
      .catch(() => {
        failedAt = Date.now();
      });
  };
  sync();
  setTimeout(render, 1500);
  window.addEventListener(WEB_PUSH_CHANGE_EVENT, () => {
    render();
    sync();
  });
  // visible / pageshow / focus / online: re-sends the lease on every resume (coalesced in ensurePushSubscribed) and picks up OS permission/pref changes
  onResume(() => {
    render();
    sync();
  });
}
