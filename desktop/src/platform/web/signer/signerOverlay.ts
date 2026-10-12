// Vanilla-DOM UI for signer health (independent of the React tree, which may not be mounted yet):
//  * runBootGate(): before the app boots with a saved NIP-46 session, prove the signer answers; otherwise a full-screen
//    "Your signer isn't responding" state with Retry / Reconnect signer / Sign out replaces the blank splash.
//  * startSignerBanner(): mid-session, a non-blocking banner with the same actions once health turns unresponsive.
import { BANNER_SILENCE_MS, signerHealth, type SignerHealthReason } from "./signerHealth";

export type SignerActions = {
  /** Round-trip probe; true if the signer answered. */
  probe(onAttempt?: (attempt: number, timeoutMs: number) => void): Promise<boolean>;
  /** Re-pair (nostrconnect QR / bunker paste, the unlock UI). Resolves when a signer is connected again. */
  reconnect(): Promise<void>;
  signOut(): Promise<void>;
};

const BTN = "background:#fff;color:#111;border:0;border-radius:8px;padding:10px 14px;font:600 14px Inter,system-ui,sans-serif;margin:4px";
const BTN2 = "background:transparent;color:#f5f5f5;border:1px solid #666;border-radius:8px;padding:10px 14px;font:14px Inter,system-ui,sans-serif;margin:4px";

function mk<K extends keyof HTMLElementTagNameMap>(tag: K, css: string, text?: string, testid?: string) {
  const e = document.createElement(tag);
  e.style.cssText = css;
  if (text) e.textContent = text;
  if (testid) e.setAttribute("data-testid", testid);
  return e;
}

export const EXT_DOWN_TITLE = "Your browser extension isn't responding";
export const EXT_DOWN_MSG = "Check that it's unlocked and approve any prompts.";
const isExt = () => signerHealth.signerKind() === "nip07";

export function describeDown(reason: SignerHealthReason): string {
  if (isExt()) return EXT_DOWN_MSG; // browser extension (NIP-07): no remote-signer wording
  switch (reason) {
    case "revoked":
      return "Your signer says this connection is no longer authorized (the session was revoked or removed). Reconnect it to continue.";
    case "long-wait":
      return "Your signer has been silent for a while. Open your signer app (Primal, Amber, nsec.app…) and make sure it's running, then retry.";
    case "probe-failed":
    case "timeout":
      return "Your signer didn't answer. Open your signer app and make sure it's running and online, then retry — or reconnect it.";
    default:
      return "Your signer isn't available.";
  }
}

function actions(a: SignerActions, status: HTMLElement, after: () => void) {
  const row = mk("div", "margin-top:14px;display:flex;flex-wrap:wrap;justify-content:center");
  const retry = mk("button", BTN, "Retry", "signer-retry");
  const rec = mk("button", BTN2, "Reconnect signer", "signer-reconnect");
  const out = mk("button", BTN2, "Sign out of this device", "signer-signout");
  const busy = (on: boolean) => {
    for (const b of [retry, rec, out]) (b as HTMLButtonElement).disabled = on;
  };
  retry.onclick = async () => {
    busy(true);
    status.textContent = isExt() ? "Checking your extension…" : "Checking your signer…";
    const ok = await a.probe().catch(() => false);
    busy(false);
    if (ok) after();
    else status.textContent = isExt() ? "Still no answer from your extension." : "Still no answer from your signer.";
  };
  rec.onclick = async () => {
    busy(true);
    try {
      await a.reconnect();
      after();
    } catch {
      status.textContent = "Couldn't reconnect.";
    }
    busy(false);
  };
  out.onclick = async () => {
    busy(true);
    try {
      await a.signOut();
    } finally {
      location.reload();
    }
  };
  if (isExt()) row.append(retry); // extension: Retry only (nothing to re-pair, no remote session to sign out of)
  else row.append(retry, rec, out);
  return row;
}

/**
 * Live "Connecting to your signer…" text: seconds left for the current probe attempt (matches the real probe timeout) and
 * dots cycling 1→2→3 in a fixed-width slot. Returns the attempt callback and a stop function.
 */
export function startProbeTicker(title: HTMLElement, status: HTMLElement) {
  let label = "Connecting to your signer";
  let deadline = 0;
  let tick = 0;
  const dots = mk("span", "display:inline-block;width:1.2em;text-align:left", "", "signer-gate-dots");
  const secs = mk("span", "font-variant-numeric:tabular-nums;display:inline-block;min-width:1ch", "", "signer-gate-secs");
  const render = () => {
    dots.textContent = ".".repeat((tick % 3) + 1);
    const left = deadline ? Math.max(1, Math.ceil((deadline - Date.now()) / 1000)) : 0;
    secs.textContent = left ? String(left) : "";
    title.firstChild && (title.firstChild.textContent = label);
  };
  title.textContent = label;
  title.append(dots);
  status.textContent = "";
  status.append("Time left: ", secs, "s");
  const dotTimer = setInterval(() => {
    tick++;
    render();
  }, 450);
  const secTimer = setInterval(render, 100);
  render();
  return {
    onAttempt(attempt: number, timeoutMs: number) {
      label = attempt === 0 ? "Connecting to your signer" : "Retrying";
      deadline = Date.now() + timeoutMs;
      render();
    },
    stop() {
      clearInterval(dotTimer);
      clearInterval(secTimer);
    },
  };
}

/** Resolves once the signer is proven alive (or the user reconnected). Never resolves while it's down: the user acts. */
export async function runBootGate(a: SignerActions): Promise<void> {
  const root = mk(
    "div",
    "position:fixed;inset:0;z-index:2147483647;background:#0b0b0b;color:#f5f5f5;display:flex;flex-direction:column;align-items:center;justify-content:center;padding:24px;text-align:center;font:15px/1.5 Inter,system-ui,sans-serif",
  );
  root.setAttribute("role", "alert");
  document.body.appendChild(root);
  const title = mk("h1", "font:600 20px Inter,system-ui,sans-serif;margin:0 0 8px", "Connecting to your signer", "signer-gate-title");
  const msg = mk("p", "max-width:420px;margin:0;opacity:.85", "Checking that your remote signer is reachable.", "signer-gate-msg");
  const status = mk("p", "margin:10px 0 0;font-size:13px;opacity:.7", "", "signer-gate-status");
  root.append(title, msg, status);
  const done = () => root.remove();
  const ticker = startProbeTicker(title, status);
  const ok = await a.probe(ticker.onAttempt).catch(() => false);
  ticker.stop();
  status.textContent = "";
  if (ok) return done();
  if (signerHealth.get().state !== "unresponsive") signerHealth.probeFailed(); // keep a more specific reason (revoked) if set
  root.setAttribute("data-testid", "signer-down-screen");
  title.textContent = "Your signer isn't responding";
  msg.textContent = describeDown(signerHealth.get().reason === "none" ? "probe-failed" : signerHealth.get().reason);
  return new Promise<void>((resolve) => {
    root.appendChild(
      actions(a, status, () => {
        signerHealth.success();
        done();
        resolve();
      }),
    );
  });
}

/** Non-blocking banner shown while health is unresponsive after the app has booted. */
export function startSignerBanner(a: SignerActions) {
  let el: HTMLDivElement | null = null;
  let recheck: ReturnType<typeof setTimeout> | undefined;
  const render = () => {
    const h = signerHealth.get();
    clearTimeout(recheck);
    // Timeouts/long waits: only a sustained silence counts (one slow request that later answers must not flash the banner).
    const sustained = h.reason === "revoked" || h.reason === "probe-failed" || Date.now() - signerHealth.lastAlive() >= BANNER_SILENCE_MS;
    if (h.state === "unresponsive" && !sustained && !el) {
      recheck = setTimeout(render, Math.max(250, BANNER_SILENCE_MS - (Date.now() - signerHealth.lastAlive())));
      return;
    }
    if (h.state !== "unresponsive") {
      el?.remove();
      el = null;
      return;
    }
    if (el) return;
    el = mk(
      "div",
      "position:fixed;left:8px;right:8px;top:calc(env(safe-area-inset-top) + 8px);z-index:2147483646;background:#3a1212;color:#fff;border:1px solid #c0392b;border-radius:12px;padding:12px 14px;font:14px/1.4 Inter,system-ui,sans-serif;text-align:center;box-shadow:0 4px 20px #0008",
      undefined,
      "signer-down-banner",
    ) as HTMLDivElement;
    el.setAttribute("role", "alert");
    const t = mk("div", "font-weight:600", isExt() ? EXT_DOWN_TITLE : "Your signer isn't responding", "signer-down-title");
    const m = mk("div", "font-size:13px;opacity:.9;margin-top:2px", describeDown(h.reason));
    const status = mk("div", "font-size:12px;opacity:.8;margin-top:6px", "");
    el.append(t, m, status, actions(a, status, () => signerHealth.success()));
    document.body.appendChild(el);
  };
  signerHealth.subscribe(render);
  render();
}
