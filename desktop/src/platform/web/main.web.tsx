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
  probe: () => probeActiveSigner(),
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
await import("@/main");
