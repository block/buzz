// Web entry: register SW (prod only), unlock via signer (NIP-07), then boot the shared desktop app.
import "./web.css";
import { getSession, probeActiveSigner, resumeSession, signOutSigner, signerPubkey } from "./signer";
import { runBootGate, startSignerBanner, type SignerActions } from "./signer/signerOverlay";
import { startBootWatch } from "./signer/bootWatch";
import { sendStatus } from "@/shared/lib/sendStatus";

if ("serviceWorker" in navigator && import.meta.env.PROD) {
  window.addEventListener("load", () => {
    navigator.serviceWorker
      .register("/sw.js")
      .catch((e) => console.warn("sw register failed", e));
  });
}
// Slow remote signers: failed sends stay visible as "Failed — Retry" instead of silently rolling back.
sendStatus.configure({ keepFailedRows: true });
startBootWatch(signOutSigner);
const signerActions: SignerActions = {
  probe: (onAttempt) => probeActiveSigner(8000, onAttempt),
  reconnect: async () => {
    const before = signerPubkey();
    const { showUnlockOverlay } = await import("./UnlockScreen");
    const pk = await showUnlockOverlay();
    if (before && pk !== before) location.reload(); // a different account: restart cleanly
  },
  signOut: () => signOutSigner(),
};
if (!(await resumeSession())) {
  const { showUnlock } = await import("./UnlockScreen");
  await showUnlock();
} else if (getSession()?.kind === "nip46") {
  // A saved session can be dead (signer app removed it, offline, revoked): prove it before relay auth starts.
  await runBootGate(signerActions);
}
startSignerBanner(signerActions);
void import("./updateCheck").then((m) => m.startUpdateCheck()).catch(() => undefined);
if (import.meta.env.VITE_BUZZ_WEB_PUSH_URL) void import("./push/banner").then((m) => m.startWebPush()).catch(() => undefined);
try {
  await import("@/main");
} catch (e) {
  // A failed chunk/module import (stale bundle, dev reload on iOS) used to leave a blank page: show the error and a Reload.
  console.error("[web] app failed to load", e);
  const root = document.getElementById("root") ?? document.body;
  const box = document.createElement("div");
  box.setAttribute("data-testid", "web-boot-error");
  box.style.cssText = "display:flex;flex-direction:column;align-items:center;justify-content:center;gap:12px;height:100vh;padding:24px;font:14px system-ui;text-align:center";
  const t = document.createElement("p");
  t.textContent = "Buzz failed to load";
  t.style.fontWeight = "600";
  const m = document.createElement("pre");
  m.textContent = e instanceof Error ? e.message : String(e);
  m.style.cssText = "max-width:28rem;white-space:pre-wrap;word-break:break-word;font-size:12px;opacity:.8";
  const b = document.createElement("button");
  b.textContent = "Reload";
  b.style.cssText = "padding:8px 16px;border:1px solid #888;border-radius:6px";
  b.onclick = () => location.reload();
  box.append(t, m, b);
  root.replaceChildren(box);
}
