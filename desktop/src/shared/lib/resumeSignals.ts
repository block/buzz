// App-resume signals for the web build (iOS PWAs freeze in the background and wake with half-dead sockets).
// `onResume` fires (coalesced) when the page becomes visible, is restored from bfcache, regains focus, or the network
// comes back; `onOffline` fires when the browser reports the network gone. Both are no-ops outside a browser.
type Fn = () => void;
const resumeFns = new Set<Fn>();
const offlineFns = new Set<Fn>();
let installed = false;
let last = 0;

function fireResume() {
  const now = Date.now();
  if (now - last < 750) return; // visibilitychange + focus + pageshow arrive together on iOS
  last = now;
  for (const f of [...resumeFns]) {
    try {
      f();
    } catch {
      /* one listener must not stop the others */
    }
  }
}

function install() {
  if (
    installed ||
    typeof window === "undefined" ||
    typeof document === "undefined"
  )
    return;
  installed = true;
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") fireResume();
  });
  window.addEventListener("pageshow", fireResume);
  window.addEventListener("focus", fireResume);
  window.addEventListener("online", fireResume);
  window.addEventListener("offline", () => {
    for (const f of [...offlineFns]) f();
  });
}

export function onResume(fn: Fn): () => void {
  install();
  resumeFns.add(fn);
  return () => void resumeFns.delete(fn);
}
export function onOffline(fn: Fn): () => void {
  install();
  offlineFns.add(fn);
  return () => void offlineFns.delete(fn);
}
export const isPageVisible = () =>
  typeof document === "undefined" || document.visibilityState === "visible";
