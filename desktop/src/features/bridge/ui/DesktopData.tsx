// Web only: the Agents settings, dialogs and page show the DESKTOP's data over the direct connection. These helpers say whether it
// is available and explain, instead of showing the web app's own (empty) local values, when it is not.
import { isTauri } from "@tauri-apps/api/core";
import * as React from "react";

import { getProxyDirect, NEEDS_DIRECT, proxyCmd, subscribeProxy } from "../remoteCmd";

export const isWebBuild = () => !isTauri();

/** { web, live }: `live` = this web app currently has a signed direct connection to the desktop. Always live on the desktop app. */
export function useDesktopData(): { web: boolean; live: boolean; reason: string } {
  const web = isWebBuild();
  const [, bump] = React.useReducer((x: number) => x + 1, 0);
  const direct = React.useSyncExternalStore(subscribeProxy, getProxyDirect);
  React.useEffect(() => (direct ? direct.subscribe(bump) : undefined), [direct]);
  return { web, live: !web || !!direct?.live, reason: NEEDS_DIRECT };
}

/** Connects once when a settings page opens (never in the background). */
export function useAutoDirectOnOpen() {
  const direct = React.useSyncExternalStore(subscribeProxy, getProxyDirect);
  React.useEffect(() => {
    if (isWebBuild() && direct) void direct.autoConnect();
    return () => direct?.resetAuto();
  }, [direct]);
}

/** Shown on web when the desktop's data isn't available: says why and offers a Retry. */
export function DesktopDataNotice(props: { what?: string }) {
  const { web, live, reason } = useDesktopData();
  const direct = React.useSyncExternalStore(subscribeProxy, getProxyDirect);
  if (!web || live) return null;
  const connecting = direct?.state === "connecting";
  return (
    <div className="mb-4 rounded-lg border border-border/60 px-4 py-3 text-sm" data-testid="desktop-data-notice">
      <strong>{props.what ?? "Your desktop's settings"} aren't available right now.</strong>
      <p className="mt-1 text-muted-foreground">{reason}</p>
      {direct && (
        <button type="button" className="mt-2 text-xs underline" disabled={connecting} data-testid="desktop-data-retry" onClick={() => void direct.connect().catch(() => {})}>
          {connecting ? "Waiting for your signer…" : "Retry"}
        </button>
      )}
    </div>
  );
}

/** Replaces a desktop-only setting on web with an explanation. */
export function DesktopOnlyNote(props: { title: string; children: React.ReactNode }) {
  return (
    <div className="mb-4 rounded-lg border border-border/60 px-4 py-3 text-sm" data-testid="desktop-only-note">
      <strong>{props.title}</strong>
      <p className="mt-1 text-muted-foreground">{props.children}</p>
    </div>
  );
}

export type ChipTone = "green" | "amber" | "red" | "grey";
const CHIP: Record<ChipTone, string> = {
  green: "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400",
  amber: "bg-amber-500/15 text-amber-600 dark:text-amber-400",
  red: "bg-red-500/15 text-red-600 dark:text-red-400",
  grey: "bg-muted-foreground/15 text-muted-foreground",
};
/** Small status pill (same styling as the presence chip). */
export function StatusChip(props: { tone: ChipTone; children: React.ReactNode; testId?: string }) {
  return (
    <span className={`inline-flex shrink-0 items-center rounded-full px-2 py-0.5 text-xs font-medium ${CHIP[props.tone]}`} data-testid={props.testId} data-tone={props.tone}>
      {props.children}
    </span>
  );
}

/** Web: the desktop's OpenClaw workspace status over Direct (read-only, no secrets). */
export function useDesktopOpenClawStatus(): { tone: ChipTone; label: string } {
  const { live } = useDesktopData();
  const [st, setSt] = React.useState<{ tone: ChipTone; label: string } | null>(null);
  React.useEffect(() => {
    if (!live) return setSt(null);
    let dead = false;
    const run = () =>
      proxyCmd<{ connected: boolean; expiresAt: string | null }>("get_openclaw_workspace_mcp_status")
        .then((r) => {
          if (dead) return;
          const expired = !!r.expiresAt && Date.parse(r.expiresAt) < Date.now();
          setSt(r.connected && !expired ? { tone: "green", label: "Set up" } : expired ? { tone: "amber", label: "Expired" } : { tone: "amber", label: "Not set up" });
        })
        .catch(() => !dead && setSt({ tone: "red", label: "Error" }));
    void run();
    const t = window.setInterval(run, 30_000);
    return () => {
      dead = true;
      window.clearInterval(t);
    };
  }, [live]);
  return st ?? { tone: "grey", label: "Unknown" };
}

/** Web Settings → Agents: OpenClaw workspace, read-only status from the desktop. */
export function WebOpenClawStatusNote() {
  const s = useDesktopOpenClawStatus();
  return (
    <div className="mb-4 rounded-lg border border-border/60 px-4 py-3 text-sm" data-testid="desktop-only-note">
      <div className="flex items-center justify-between gap-2">
        <strong>OpenClaw workspace</strong>
        <StatusChip tone={s.tone} testId="openclaw-status-chip">{s.label}</StatusChip>
      </div>
      <p className="mt-1 text-muted-foreground">
        The OpenClaw workspace is set up and checked on your desktop (Settings → Agents). It can't be changed from the web yet.
      </p>
    </div>
  );
}
