// "New version available — tap to reload": each prod build writes /version.json with its build id (vite.config.web.ts) and
// bakes the same id into the bundle. We re-read it (no-store) when the app comes back to the foreground and every 10 min;
// a different id means a newer build is live. Reloading is safe: the service worker serves navigations network-first.
const BUILD_ID = (import.meta.env.VITE_BUZZ_WEB_BUILD_ID as string | undefined) ?? "";
const EVERY_MS = 10 * 60_000;
let shown = false;
let last = 0;

export async function checkForUpdate(fetchImpl: typeof fetch = fetch): Promise<boolean> {
  if (!BUILD_ID || shown) return shown;
  try {
    const res = await fetchImpl(`/version.json?t=${Date.now()}`, { cache: "no-store" });
    if (!res.ok) return false;
    const v = (await res.json()) as { build?: unknown };
    if (typeof v.build === "string" && v.build && v.build !== BUILD_ID) {
      showPrompt();
      return true;
    }
  } catch {
    /* offline: try again later */
  }
  return false;
}

function showPrompt() {
  if (shown) return;
  shown = true;
  const b = document.createElement("button");
  b.type = "button";
  b.setAttribute("data-testid", "web-update-prompt");
  b.textContent = "New version available · Tap to reload";
  b.style.cssText =
    "position:fixed;left:50%;transform:translateX(-50%);top:calc(env(safe-area-inset-top, 0px) + 10px);z-index:2147483000;" +
    "padding:8px 16px;border-radius:999px;border:1px solid rgba(127,127,127,.35);background:var(--popover, #1f2937);" +
    "color:var(--popover-foreground, #fff);font:600 13px/1.2 system-ui,-apple-system,sans-serif;box-shadow:0 6px 24px rgba(0,0,0,.25);cursor:pointer;max-width:calc(100vw - 24px)";
  b.onclick = () => {
    b.textContent = "Reloading…";
    void (async () => {
      try {
        const reg = await navigator.serviceWorker?.getRegistration();
        await reg?.update();
      } catch {
        /* reload anyway */
      }
      location.reload();
    })();
  };
  document.body.appendChild(b);
}

export function startUpdateCheck() {
  if (!BUILD_ID) return;
  const maybe = () => {
    if (document.visibilityState !== "visible" || Date.now() - last < 30_000) return;
    last = Date.now();
    void checkForUpdate();
  };
  document.addEventListener("visibilitychange", maybe);
  window.addEventListener("focus", maybe);
  window.addEventListener("online", maybe);
  setInterval(maybe, EVERY_MS);
  setTimeout(maybe, 15_000);
}
