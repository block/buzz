// Web "Buzz term" page (route /buzz-term): the desktop's real Buzz Term — same emulator, renderer, splash, tabs, touch handling and
// settings as the app — running on the member's desktop and streamed over the direct listener. The terminal itself is the app-wide
// TerminalBootstrap (mounted in the shell's dock); this page registers the remote backend, opens the left-nav-style maximized panel,
// and, whenever the panel isn't showing, is the launcher/status/error screen. While the desktop doesn't offer terminals it shows the
// exact steps to turn it on.
import * as React from "react";

import { useIdentityQuery } from "@/shared/api/hooks";
import { ReconnectCard } from "@/shared/ui/reconnect-card";
import { openTerminalPanel, setTerminalPanelMode, useTerminalPanel } from "@/features/terminal/terminalPanelStore";
import { setTerminalBackend } from "@/features/terminal/terminalClient";
import { requestTermSessionOpen } from "@/features/term-session/lib/openRequestStore";
import { useBridgeAgents } from "../useBridgeAgents";
import { describeRemote } from "../remoteCopy";
import { whyTermUnavailable } from "../remoteHosts";
import { configureWebTermHub, getWebTermHub } from "../webTermHub";
import { peekPendingTermHandoff, takePendingTermHandoff } from "../webTermHandoff";
import { BUZZ_TERM_UNAVAILABLE, useRemoteHostsFor, useWebRemoteNav } from "./WebRemoteSettings";

/** What a failed/closed launch means in words (codes come from RemoteError and the desktop). */
export function termFailureMessage(code: string | undefined, detail: string | undefined, deviceName: string): string | null {
  switch (code) {
    case undefined:
      return null;
    case "lost":
    case "closed":
      return "The connection to your desktop was lost. Your terminals keep running there for 10 minutes; Reconnect to bring them back.";
    case "no_desktop":
      return "Your desktop isn't advertising remote access right now.";
    case "emu_unsupported":
      return `${deviceName} needs to be updated (and restarted) for the full Buzz Term on web.`;
    case "terminals_off":
    case "terminals_disabled":
      return `"Allow remote terminal" is off on ${deviceName} (Settings → Buzz Term on that computer).`;
    case "too_many_terminals":
      return `${deviceName} already has 3 Buzz Term sessions running from an earlier page. Reconnect to bring them back, or close them and start fresh.`;
    case "launch_failed":
      return `${deviceName} couldn't start the terminal${detail ? ` (${detail})` : ""}.`;
    default:
      return null;
  }
}

export function BuzzTermPage() {
  const pubkey = useIdentityQuery().data?.pubkey;
  const b = useBridgeAgents(pubkey);
  const nav = useWebRemoteNav();
  const { hosts, reachOf } = useRemoteHostsFor(b);
  const panel = useTerminalPanel();
  // Usable as soon as the desktop advertises terminals and answers on the tailnet; the per-device switch in web Settings
  // no longer has to be found and flipped first (it can still be used to switch the nav entry off).
  // Reachability gates everything: the panel only opens once the desktop's listener actually answered on the tailnet.
  const cand = hosts.find((h) => h.supports.terminal && !h.stale);
  const reach = cand ? reachOf(cand) : "unknown";
  // No answer within ~5 s counts as unreachable (explain right away instead of an empty panel); re-armed whenever reach changes.
  const [probeTimedOut, setProbeTimedOut] = React.useState(false);
  React.useEffect(() => {
    setProbeTimedOut(false);
    if (reach !== "unknown") return;
    const t = window.setTimeout(() => setProbeTimedOut(true), 5_000);
    return () => window.clearTimeout(t);
  }, [reach, cand?.id]);
  const reachable = reach === "reachable";
  const unreachable = reach === "unreachable" || (probeTimedOut && !reachable);
  const why = whyTermUnavailable(cand ?? hosts[0], !!b.heartbeat?.remote, unreachable ? "unreachable" : reach);
  const c = BUZZ_TERM_UNAVAILABLE;
  void nav; // the web Settings switch no longer forces the page "usable" while the desktop can't be reached
  const usable = !!cand && !unreachable && b.available;

  const info = b.heartbeat?.remote;
  configureWebTermHub({ info, sign: b.signJson });
  const hub = getWebTermHub();
  const status = React.useSyncExternalStore(hub.subscribe, hub.getStatus);

  // The backend is registered app-wide (WebRemoteWatcher → useWebTermBackend) so the in-channel Term works too; leaving this
  // page closes the left-nav-style panel (sessions stay alive in the hub).
  React.useLayoutEffect(() => {
    if (usable) setTerminalBackend(hub); // already registered by the watcher; harmless, and covers a first render before it runs
  }, [usable, hub]);
  // Close the left-nav-style panel only when leaving the page (not on every usable/hub flap, which used to strand the launcher).
  React.useLayoutEffect(() => () => setTerminalPanelMode("closed"), []);

  // Terminals offered by the desktop and Direct up → open maximized immediately (like the desktop's left-nav Buzz Term).
  const termsOff = !!info && info.status === "on" && !info.terminals;
  const termReady = usable && reachable && !!info && info.status === "on" && info.terminals;
  // Terminals switched off on the desktop: close the panel so the page explains it (a brief Direct flap does not close it).
  React.useEffect(() => {
    if (termsOff) setTerminalPanelMode("closed");
  }, [termsOff]);
  // Unreachable, terminals off, or the terminal connection dropped: fold the panel back to this page (Reconnect lives here),
  // never leave an empty maximized shell with a "+" that can't work.
  const reconnecting = status.state === "reconnecting";
  const mustFold = (!!cand && unreachable) || termsOff || status.state === "lost" || reconnecting;
  React.useEffect(() => {
    if (mustFold && panel.mode !== "closed") setTerminalPanelMode("closed");
  }, [mustFold, panel.mode]);
  const openedRef = React.useRef(false);
  React.useEffect(() => {
    if (!termReady || status.state === "lost" || reconnecting) {
      openedRef.current = false; // re-arm: when Direct comes back, auto-open again
      return;
    }
    if (openedRef.current) return;
    openedRef.current = true;
    // a pending handoff opens the panel itself with its own session; a second blank one would sit next to it
    if (peekPendingTermHandoff()) return;
    openTerminalPanel("maximized", "all");
  }, [termReady, status.state, reconnecting]);

  // A "Pass thread context to Buzz Term" handoff from the web: start its session in the terminal (a plain shell on the desktop).
  const handoffReady = termReady;
  React.useEffect(() => {
    if (!handoffReady || !peekPendingTermHandoff()) return;
    const t = window.setTimeout(() => {
      const h = takePendingTermHandoff();
      if (!h) return;
      requestTermSessionOpen({
        sid: h.sid,
        channelId: "__buzz_term__",
        channelName: "Buzz Term",
        threadId: null,
        tool: h.tool,
        promptPath: "",
        launchCommand: h.launchCommand,
      });
    }, 0);
    return () => window.clearTimeout(t);
  }, [handoffReady]);

  const deviceName = info?.name || "your desktop";
  const d = describeRemote(b.presence, info, status.code);
  const failure = termFailureMessage(status.code, status.detail, deviceName) ?? (status.code ? d.message : "") ?? "";
  const open = () => {
    if (!termReady) return;
    hub.reset();
    openTerminalPanel("maximized", "all");
  };

  return (
    <div className="flex-1 overflow-y-auto" data-testid="buzz-term-page">
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-3 p-6">
        <h1 className="text-xl font-semibold" data-testid="buzz-term-title">
          {usable && !termsOff ? "Buzz term" : c.title}
        </h1>
        {usable && !termsOff ? (
          <div data-testid="buzz-term-launcher" className="flex flex-col gap-3">
            <p className="text-sm text-muted-foreground">
              {!termReady
                ? `Connecting to ${deviceName}…`
                : panel.mode === "closed"
                  ? `Buzz Term running on ${deviceName}.`
                  : `Opening Buzz Term on ${deviceName}…`}
            </p>
            <ReconnectCard
              reconnecting={reconnecting}
              title={`Reconnecting to ${deviceName}…`}
              description="Your terminals keep running there and come back on their own"
              failingSince={hub.failingSince()}
              onReconnectNow={hub.reconnectNow}
              testId="buzz-term-reconnecting"
            />
            {status.state === "connecting" && <p className="text-sm">Waiting for approval in your signer…</p>}
            {failure && !reconnecting && (
              <p className="text-sm text-muted-foreground" data-testid="buzz-term-error">
                {failure}
              </p>
            )}
            {(termReady || failure) && !reconnecting && <div>
              <button
                type="button"
                data-testid="buzz-term-open"
                className="rounded-md border px-3 py-1.5 text-sm"
                onClick={open}
                disabled={status.state === "connecting" || !termReady}
              >
                {status.state === "lost" || status.code ? "Reconnect" : "Open Buzz Term"}
              </button>
              {status.code === "too_many_terminals" || status.code === "attach_failed" || status.code === "not_found" ? (
                <button
                  type="button"
                  data-testid="buzz-term-close-old"
                  className="ml-2 rounded-md border px-3 py-1.5 text-sm"
                  disabled={!termReady}
                  onClick={() => {
                    void hub
                      .closeAllLive()
                      .catch(() => 0)
                      .then(() => {
                        hub.reset();
                        openTerminalPanel("maximized", "all");
                      });
                  }}
                >
                  Close old sessions
                </button>
              ) : null}
            </div>}
          </div>
        ) : (
          <div data-testid="remote-unavailable-page" data-feature="term">
            <p className="text-sm text-muted-foreground">{c.summary}</p>
            {termsOff && (
              <p className="mt-2 text-sm font-medium" data-testid="remote-terminals-off">
                "Allow remote terminal" is off on {deviceName}. Turn it on there in Settings → Buzz Term → Allow remote terminal.
              </p>
            )}
            {why && (
              <p className="mt-2 text-sm font-medium" data-testid="buzz-term-why">
                {why}
              </p>
            )}
            <ol className="mt-2 list-decimal space-y-1 pl-5 text-sm" data-testid="remote-unavailable-steps">
              {c.steps.map((t) => (
                <li key={t}>{t}</li>
              ))}
            </ol>
          </div>
        )}
      </div>
    </div>
  );
}
