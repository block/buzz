// Pre-app unlock gate for the web build. Standalone (inline styles) so it renders before the app CSS loads.
import { QRCodeSVG } from "qrcode.react";
import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { BuzzMark } from "@/shared/ui/buzz-logo/BuzzMark";
import {
  adopt,
  detectNip07,
  signIn,
  signOutSigner,
  getSession,
} from "./signer";
import {
  DEFAULT_SIGNER_RELAYS,
  type NostrConnectFlow,
  beginNostrConnect,
  connectBunker,
} from "./signer/nip46";
import { dbgEnabled, nip46Debug } from "./signer/nip46Debug";
import { peekPendingConnect } from "./signer/nip46Store";

const isIos = () =>
  /iPad|iPhone|iPod/.test(navigator.userAgent) ||
  (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
// Matches the desktop onboarding neutral (light) theme: white card, ink #171717, pill CTAs.
const INK = "#171717";
const input: React.CSSProperties = {
  padding: "10px 14px",
  borderRadius: 12,
  border: "1px solid #e5e5e5",
  background: "#fff",
  color: INK,
  fontSize: 16, // 16px: iOS Safari zooms on smaller inputs
  width: "100%",
  boxSizing: "border-box",
  outline: "none",
};
const pill: React.CSSProperties = {
  height: 38,
  padding: "0 24px",
  borderRadius: 999,
  border: 0,
  fontWeight: 500,
  fontSize: 14,
  cursor: "pointer",
  textAlign: "center",
  textDecoration: "none",
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  gap: 8,
  boxSizing: "border-box",
  width: "100%",
};
const ghost: React.CSSProperties = {
  ...pill,
  height: 36,
  background: "rgba(23,23,23,0.1)",
  color: INK,
};
const errText = (e: unknown) => (e instanceof Error ? e.message : String(e));

function DebugLine() {
  const [, tick] = useState(0);
  useEffect(() => nip46Debug.subscribe(() => tick((n) => n + 1)), []);
  const d = nip46Debug.get();
  const urls = Object.keys(d.relays);
  const open = urls.filter((u) => d.relays[u]).length;
  return (
    <p
      style={{ ...note, fontFamily: "ui-monospace, monospace", fontSize: 11 }}
      data-testid="web-unlock-debug"
    >
      signer relays: connected {open}/{urls.length}, last event:{" "}
      {d.lastEvent ?? "none"}
      {d.note ? ` · ${d.note}` : ""}
    </p>
  );
}

function Nip46Panel({ onDone }: { onDone: (pubkey: string) => void }) {
  const [bunker, setBunker] = useState("");
  const [relay, setRelay] = useState(
    new URLSearchParams(location.search).get("nip46relay") ||
      DEFAULT_SIGNER_RELAYS.join(", "),
  );
  const [uri, setUri] = useState<string | null>(null);
  const [busy, setBusy] = useState<null | "bunker" | "connect">(
    peekPendingConnect() ? "connect" : null,
  );
  const [resumed, setResumed] = useState<boolean>(!!peekPendingConnect());
  const [err, setErr] = useState<string | null>(null);
  const flowRef = useRef<NostrConnectFlow | null>(null);
  // Unmount: stop listening but KEEP the pending attempt (a reload must be able to resume it).
  useEffect(() => () => flowRef.current?.detach(), []);

  const attach = async (flow: NostrConnectFlow) => {
    flowRef.current = flow;
    setResumed(flow.resumed);
    setUri(flow.uri);
    setBusy("connect");
    try {
      onDone(await adopt(await flow.connected));
    } catch (e) {
      if (flowRef.current !== flow) return; // detached/replaced
      setErr(errText(e));
      setUri(null);
      setBusy(null);
      setResumed(false);
    }
  };
  // Auto-resume a pending nostrconnect attempt after a reload / tab discard / return from the signer app.
  useEffect(() => {
    if (!peekPendingConnect()) return;
    let live = true;
    void beginNostrConnect(undefined, { resumeOnly: true }).then((flow) => {
      if (!live) return flow?.detach();
      if (flow) void attach(flow);
      else {
        setBusy(null);
        setResumed(false);
      }
    });
    return () => {
      live = false;
    };
    // biome-ignore lint/correctness/useExhaustiveDependencies: run once on mount
  }, []);

  const doBunker = async () => {
    setBusy("bunker");
    setErr(null);
    try {
      onDone(await adopt(await connectBunker(bunker)));
    } catch (e) {
      setErr(errText(e));
      setBusy(null);
    }
  };
  const doConnect = async () => {
    setErr(null);
    setBusy("connect");
    const flow = await beginNostrConnect(
      relay
        .split(/[\s,]+/)
        .filter(Boolean)
        .map((r) => r.trim()),
    );
    if (flow) void attach(flow);
  };
  return (
    <div
      style={{ display: "grid", gap: 10 }}
      data-testid="web-unlock-nip46-panel"
    >
      <input
        style={input}
        placeholder="bunker://…"
        value={bunker}
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        onChange={(e) => setBunker(e.target.value)}
        data-testid="web-unlock-bunker-input"
      />
      <button
        type="button"
        style={busy || !bunker.trim() ? btnOff : btn}
        disabled={!!busy || !bunker.trim()}
        onClick={doBunker}
        data-testid="web-unlock-bunker-go"
      >
        {busy === "bunker"
          ? "Waiting for your signer…"
          : "Connect with bunker URL"}
      </button>
      {busy === "connect" && (
        <p
          role="status"
          style={{ ...note, color: WARN, fontSize: 15 }}
          data-testid="web-unlock-finishing"
        >
          {resumed ? "Finishing sign-in…" : "Waiting for your signer…"}
        </p>
      )}
      <p style={note}>— or connect by scanning / opening a link —</p>
      {!uri && (
        <>
          <input
            style={input}
            value={relay}
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            onChange={(e) => setRelay(e.target.value)}
            aria-label="Signer relay"
            data-testid="web-unlock-signer-relay"
          />
          <button
            type="button"
            style={busy ? btnOff : ghost}
            disabled={!!busy}
            onClick={doConnect}
            data-testid="web-unlock-nostrconnect"
          >
            Show connect QR / link
          </button>
        </>
      )}
      {uri && (
        <div
          style={{ display: "grid", gap: 10, justifyItems: "center" }}
          data-testid="web-unlock-nostrconnect-panel"
        >
          <div style={{ background: "#fff", padding: 10, borderRadius: 16, border: "1px solid #e5e5e5" }}>
            <QRCodeSVG value={uri} size={200} />
          </div>
          <a
            href={uri}
            style={{
              ...btn,
              textDecoration: "none",
              textAlign: "center",
              width: "100%",
              boxSizing: "border-box",
            }}
            data-testid="web-unlock-open-signer"
          >
            Open in signer app
          </a>
          <a
            // Clave (iOS NIP-46 signer) universal link: opens Clave's approval sheet, or its install page.
            href={`https://clave.casa/connect?uri=${encodeURIComponent(uri)}`}
            style={ghost}
            data-testid="web-unlock-open-clave"
          >
            <img src="/signers/clave.png" alt="" width={20} height={20} style={{ borderRadius: 5 }} />
            Open in Clave
          </a>
          <input
            style={input}
            readOnly
            value={uri}
            onFocus={(e) => e.currentTarget.select()}
            data-testid="web-unlock-nostrconnect-uri"
          />
          <p style={note}>
            Approve in your signer app, then come back here. This page will
            finish signing in on its own (valid 10 min).
          </p>
          <button
            type="button"
            style={ghost}
            data-testid="web-unlock-cancel"
            onClick={() => {
              flowRef.current?.cancel();
              flowRef.current = null;
              setUri(null);
              setBusy(null);
              setResumed(false);
            }}
          >
            Cancel
          </button>
        </div>
      )}
      {dbgEnabled() && <DebugLine />}
      {err && (
        <p
          role="alert"
          style={{ color: ERR }}
          data-testid="web-unlock-nip46-error"
        >
          {err}
        </p>
      )}
    </div>
  );
}

const box: React.CSSProperties = {
  minHeight: "100dvh",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  color: INK,
  font: "15px/1.5 Inter, system-ui, -apple-system, sans-serif",
  // Desktop onboarding backdrop: dot grid over the chartreuse -> light-blue gradient.
  backgroundColor: "#d7e7f6",
  backgroundImage:
    "radial-gradient(circle, rgb(0 0 0 / 0.08) 1px, transparent 1px), linear-gradient(to bottom, #d7d72e, #d7e7f6)",
  backgroundSize: "24px 24px, auto",
  backgroundAttachment: "fixed",
  boxSizing: "border-box",
  padding:
    "calc(env(safe-area-inset-top) + 24px) calc(env(safe-area-inset-right) + 16px) calc(env(safe-area-inset-bottom) + 24px) calc(env(safe-area-inset-left) + 16px)",
};
const card: React.CSSProperties = {
  width: "min(calc(100vw - 2rem), calc(38rem + 2px))",
  boxSizing: "border-box",
  display: "grid",
  gap: 14,
  background: "#fff",
  borderRadius: 32,
  padding: "28px 24px",
  boxShadow: "0 10px 15px -3px rgb(0 0 0 / 0.1), 0 4px 6px -4px rgb(0 0 0 / 0.1)",
};
const btn: React.CSSProperties = { ...pill, background: INK, color: "#fff" };
const btnOff: React.CSSProperties = {
  ...pill,
  background: "rgba(23,23,23,0.06)",
  color: "#a3a3a3",
  cursor: "not-allowed",
};
const note: React.CSSProperties = { color: "#737373", fontSize: 13, margin: 0 };
const ERR = "#dc2626";
const WARN = "#a16207";

function Unlock({ onDone }: { onDone: (pubkey: string) => void }) {
  const [ext, setExt] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    void detectNip07().then(setExt);
  }, []);
  const go = async () => {
    setBusy(true);
    setErr(null);
    try {
      onDone(await signIn("nip07"));
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };
  return (
    <div style={box}>
      <div style={card} data-testid="web-unlock">
        <div
          aria-hidden
          data-testid="web-unlock-bee"
          style={{
            justifySelf: "center",
            width: 64,
            height: 64,
            borderRadius: "50%",
            background: "#d7d72e",
            color: INK,
            display: "grid",
            placeItems: "center",
          }}
        >
          <span style={{ display: "block", width: 40, lineHeight: 0 }}>
            <BuzzMark className="web-unlock-bee-mark" />
          </span>
        </div>
        <h1 style={{ fontSize: 24, lineHeight: "32px", fontWeight: 600, margin: 0, textAlign: "center" }}>
          Sign in to HulaBuzz
        </h1>
        <p style={note}>
          Your private key never enters this page. Sign-in and every message is
          signed by your browser extension or remote signer.
        </p>
        {ext === null && <p style={note}>Looking for a Nostr extension…</p>}
        {ext === true && (
          <button
            type="button"
            style={btn}
            disabled={busy}
            onClick={go}
            data-testid="web-unlock-nip07"
          >
            {busy ? "Waiting for extension…" : "Sign in with browser extension"}
          </button>
        )}
        {ext === false && !isIos() && (
          <div
            data-testid="web-unlock-no-extension"
            style={{ display: "grid", gap: 8 }}
          >
            <button type="button" style={btnOff} disabled>
              Sign in with browser extension
            </button>
            <p style={{ ...note, color: WARN }}>
              No NIP-07 extension (window.nostr) found. Install one such as
              Alby, nos2x or Nostr Connect, then reload this page.
            </p>
            <button
              type="button"
              style={ghost}
              onClick={() => {
                setExt(null);
                void detectNip07(3000).then(setExt);
              }}
            >
              Check again
            </button>
          </div>
        )}
        <div
          style={{
            borderTop: ext ? "1px solid #e5e5e5" : undefined,
            paddingTop: 12,
            display: "grid",
            gap: 8,
          }}
        >
          <strong data-testid="web-unlock-nip46">Remote signer (NIP-46)</strong>
          <p style={note}>
            Sign with nsec.app, Amber, Primal, Clave or any bunker. Your key
            stays in that app. Using Clave on this iPhone? Paste its bunker://
            code below for the most reliable sign-in.
            {isIos() && " Extensions don't exist on iPhone/iPad, so use this."}
          </p>
          <Nip46Panel onDone={onDone} />
        </div>
        {getSession() && (
          <button
            type="button"
            style={ghost}
            data-testid="web-unlock-forget"
            onClick={async () => {
              await signOutSigner();
              location.reload();
            }}
          >
            Forget saved signer
          </button>
        )}
        {err && (
          <p role="alert" style={{ color: ERR }}>
            {err}
          </p>
        )}
      </div>
    </div>
  );
}

export function showUnlock(): Promise<string> {
  const el = document.getElementById("root") as HTMLElement;
  const root = createRoot(el);
  return new Promise((resolve) => {
    root.render(
      <Unlock
        onDone={(pk) => {
          root.unmount();
          resolve(pk);
        }}
      />,
    );
  });
}

/** Reconnect UI over a running app (mid-session): same unlock flow in its own full-screen layer, app stays mounted underneath. */
export function showUnlockOverlay(): Promise<string> {
  const host = document.createElement("div");
  host.setAttribute("data-testid", "signer-reconnect-overlay");
  host.style.cssText = "position:fixed;inset:0;z-index:2147483647;overflow:auto;background:#d7e7f6";
  document.body.appendChild(host);
  const root = createRoot(host);
  return new Promise((resolve) => {
    root.render(
      <Unlock
        onDone={(pk) => {
          root.unmount();
          host.remove();
          resolve(pk);
        }}
      />,
    );
  });
}
