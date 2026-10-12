import { isTauri } from "@tauri-apps/api/core";

import { requestDesktopNotificationAccess, sendDesktopNotification } from "./desktop";

const TITLE = "Buzz test notification";
const BODY = "Notifications are working on this device.";

function isIos(): boolean {
  return /iPad|iPhone|iPod/.test(navigator.userAgent) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
}
function isStandalone(): boolean {
  return window.matchMedia?.("(display-mode: standalone)").matches || (navigator as { standalone?: boolean }).standalone === true;
}

/** Shows one real notification through the same path the app uses. Returns a message for the user when it can't. */
export async function sendTestNotification(): Promise<{ ok: boolean; message: string }> {
  if (isTauri()) {
    const perm = await requestDesktopNotificationAccess();
    if (perm !== "granted") return { ok: false, message: "Notifications are blocked for Buzz. Allow them in System Settings → Notifications." };
    const ok = await sendDesktopNotification({ title: TITLE, body: BODY });
    return ok ? { ok, message: "Test notification sent." } : { ok, message: "The notification couldn't be shown." };
  }
  if (!("Notification" in window)) {
    return {
      ok: false,
      message:
        isIos() && !isStandalone()
          ? "On iPhone and iPad, notifications only work in the installed app: tap Share → Add to Home Screen, open Buzz from the home screen, then try again."
          : "This browser doesn't support notifications.",
    };
  }
  let perm = Notification.permission;
  if (perm === "default") perm = await Notification.requestPermission();
  if (perm !== "granted")
    return {
      ok: false,
      message:
        perm === "denied"
          ? `Notifications are blocked for this site. Allow them in your browser's site settings${isIos() ? " (or in Settings → Notifications → Buzz on iOS)" : ""}, then try again.`
          : "Notification permission wasn't granted.",
    };
  try {
    const reg = "serviceWorker" in navigator ? await navigator.serviceWorker.getRegistration() : undefined;
    if (reg?.showNotification) await reg.showNotification(TITLE, { body: BODY, tag: "buzz-test", icon: "/app-icon@2x.png" });
    else new Notification(TITLE, { body: BODY });
    return { ok: true, message: "Test notification sent." };
  } catch (e) {
    return { ok: false, message: `The notification couldn't be shown: ${e instanceof Error ? e.message : String(e)}` };
  }
}
