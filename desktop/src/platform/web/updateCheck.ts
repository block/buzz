import { showStatus } from "@/shared/ui/statusPills";
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
  // Action pill in the shared status host: persists until tapped (statusPills.ts timing policy).
  showStatus("web-update", {
    tone: "action",
    title: "New version available",
    testId: "web-update-prompt",
    action: { label: "Reload", testId: "web-update-reload", onClick: reloadNow },
  });
}

function reloadNow() {
  showStatus("web-update", { tone: "progress", title: "Reloading…", testId: "web-update-prompt" });
  void (async () => {
    try {
      const reg = await navigator.serviceWorker?.getRegistration();
      await reg?.update();
    } catch {
      /* reload anyway */
    }
    location.reload();
  })();
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
