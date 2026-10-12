// Web "Buzz term" page (route /buzz-term): the desktop's real Buzz Term — same emulator, renderer, splash, tabs, touch handling and
// settings as the app — running on the member's desktop and streamed over the direct listener. The terminal itself is the app-wide
// TerminalBootstrap (mounted in the shell's dock); this page registers the remote backend, opens the left-nav-style maximized panel,
// and, whenever the panel isn't showing, is the launcher/status/error screen. While the desktop doesn't offer terminals it shows the
// exact steps to turn it on.
import * as React from "react";

import { useIdentityQuery } from "@/shared/api/hooks";
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
      return "The connection to your desktop was lost. Sessions on it ended. Reconnect to start a new one.";
    case "no_desktop":
      return "Your desktop isn't advertising remote access right now.";
    case "emu_unsupported":
      return `${deviceName} needs to be updated (and restarted) for the full Buzz Term on web.`;
    case "terminals_off":
    case "terminals_disabled":
      return `"Allow remote terminal" is off on ${deviceName} (Settings → Buzz Term on that computer).`;
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
  const ready = hosts.find((h) => h.supports.terminal && !h.stale && reachOf(h) !== "unreachable");
  const why = whyTermUnavailable(hosts[0], !!b.heartbeat?.remote, hosts[0] ? reachOf(hosts[0]) : "unknown");
  const c = BUZZ_TERM_UNAVAILABLE;
  const usable = (nav.terminal || !!ready) && b.available;

  const info = b.heartbeat?.remote;
  configureWebTermHub({ info, sign: b.signJson });
  const hub = getWebTermHub();
  const status = React.useSyncExternalStore(hub.subscribe, hub.getStatus);

  // Register the backend while this page is up; leaving it closes the left-nav-style panel (sessions stay alive in the hub).
  React.useLayoutEffect(() => {
    if (!usable) return;
    setTerminalBackend(hub);
    return () => {
      setTerminalBackend(null);
      setTerminalPanelMode("closed");
    };
  }, [usable, hub]);

  // Open the terminal by itself on arrival (like the desktop's left-nav Buzz Term).
  const openedRef = React.useRef(false);
  React.useEffect(() => {
    if (!usable || openedRef.current) return;
    openedRef.current = true;
    // a pending handoff opens the panel itself with its own session; a second blank one would sit next to it
    if (peekPendingTermHandoff()) return;
    openTerminalPanel("maximized", "all");
  }, [usable]);

  // A "Pass thread context to Buzz Term" handoff from the web: start its session in the terminal (a plain shell on the desktop).
  const handoffReady = usable && !!info && info.status === "on" && info.terminals;
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
    hub.reset();
    openTerminalPanel("maximized", "all");
  };

  return (
    <div className="flex-1 overflow-y-auto" data-testid="buzz-term-page">
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-3 p-6">
        <h1 className="text-xl font-semibold" data-testid="buzz-term-title">
          {usable ? "Buzz term" : c.title}
        </h1>
        {usable ? (
          <div data-testid="buzz-term-launcher" className="flex flex-col gap-3">
            <p className="text-sm text-muted-foreground">
              {panel.mode === "closed"
                ? `Buzz Term running on ${deviceName}.`
                : `Opening Buzz Term on ${deviceName}…`}
            </p>
            {status.state === "connecting" && <p className="text-sm">Waiting for approval in your signer…</p>}
            {failure && (
              <p className="text-sm font-medium text-destructive" data-testid="buzz-term-error">
                {failure}
              </p>
            )}
            {info && info.status === "on" && !info.terminals && (
              <p className="text-sm" data-testid="remote-terminals-off">
                "Allow remote terminal" is off on {deviceName}.
              </p>
            )}
            <div>
              <button
                type="button"
                data-testid="buzz-term-open"
                className="rounded-md border px-3 py-1.5 text-sm"
                onClick={open}
                disabled={status.state === "connecting"}
              >
                {status.state === "lost" || status.code ? "Reconnect" : "Open Buzz Term"}
              </button>
            </div>
          </div>
        ) : (
          <div data-testid="remote-unavailable-page" data-feature="term">
            <p className="text-sm text-muted-foreground">{c.summary}</p>
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
