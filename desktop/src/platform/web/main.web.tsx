// Web entry: register SW (prod only), unlock via signer (NIP-07), then boot the shared desktop app.
import "./web.css";
import { resumeSession, signOutSigner } from "./signer";
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
if (!(await resumeSession())) {
  const { showUnlock } = await import("./UnlockScreen");
  await showUnlock();
}
await import("@/main");
