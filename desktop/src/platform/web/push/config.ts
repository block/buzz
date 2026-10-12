// Web Push feature flag + config. VITE_BUZZ_WEB_PUSH_URL is the relay-hosted push gateway
// (prod default https://push.huladesk.com, see vite.config.web.ts); empty hides every push surface.
export const WEB_PUSH_URL = (
  (import.meta.env.VITE_BUZZ_WEB_PUSH_URL as string | undefined) || ""
).replace(/\/+$/, "");
export const WEB_PUSH_FLAG = WEB_PUSH_URL !== "";

const ENABLED_KEY = "buzz-web-push-enabled.v1";
const BANNER_DISMISSED_KEY = "buzz-web-push-banner-dismissed.v1";
export const WEB_PUSH_CHANGE_EVENT = "buzz:web-push-change";

function get(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
function set(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* storage denied */
  }
}

/** Push is ON by default; only an explicit settings toggle turns it off. */
export const readPushEnabled = () => get(ENABLED_KEY) !== "0";
export function writePushEnabled(on: boolean) {
  set(ENABLED_KEY, on ? "1" : "0");
  window.dispatchEvent(new CustomEvent(WEB_PUSH_CHANGE_EVENT));
}
export const bannerDismissed = () => get(BANNER_DISMISSED_KEY) === "1";
export const dismissBanner = () => set(BANNER_DISMISSED_KEY, "1");
