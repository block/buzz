// Web: OpenClaw repos live on your Mac and are read over the Direct connection (features/bridge/directCmd.ts). While Direct is
// reconnecting this shows the themed reconnect card; when the Mac can't be reached (or runs a HulaBuzz without the handler yet)
// a calm card. Never red text, never a required click: every hula-* query refetches on its own once Direct is live again.
import { useQueryClient } from "@tanstack/react-query";
import { Laptop } from "lucide-react";
import * as React from "react";

import { getProxyDirect, isCmdStale, subscribeProxy } from "@/features/bridge/remoteCmd";
import { isWebBuild } from "@/features/bridge/ui/DesktopData";
import { cn } from "@/shared/lib/cn";
import { ReconnectCard } from "@/shared/ui/reconnect-card";

const DESKTOP_ERR =
  /not available in the web app|needs a direct connection|couldn't reach your desktop|reload your desktop|older version of the app|remote agent management is off|signed connection expired|desktop is busy/i;

/** True on web when an error only means "this needs your desktop over Direct" (not a real repository failure). */
export function isDesktopRepoError(e: unknown): boolean {
  if (!isWebBuild() || !e) return false;
  const code = (e as { code?: unknown }).code;
  if (code === "needs_direct" || code === "unknown_op") return true;
  const m = e instanceof Error ? e.message : typeof e === "string" ? e : "";
  return DESKTOP_ERR.test(m);
}
const isStale = (e: unknown) =>
  (e as { code?: unknown })?.code === "unknown_op" || /reload your desktop|older version of the app/i.test(e instanceof Error ? e.message : "");

const isHulaKey = (k: readonly unknown[]) => typeof k[0] === "string" && (k[0] as string).startsWith("hula-");

/** Live Direct state; refetches every OpenClaw read the moment Direct comes back. */
export function useDesktopRepoDirect() {
  const qc = useQueryClient();
  const direct = React.useSyncExternalStore(subscribeProxy, getProxyDirect);
  const [, bump] = React.useReducer((x: number) => x + 1, 0);
  React.useEffect(() => (direct ? direct.subscribe(bump) : undefined), [direct]);
  const live = !!direct?.live;
  const reconnecting = !!direct && !live && (direct.state === "connecting" || direct.reconnecting);
  const wasLive = React.useRef(live);
  React.useEffect(() => {
    if (live && !wasLive.current) void qc.invalidateQueries({ predicate: (q) => isHulaKey(q.queryKey) });
    wasLive.current = live;
  }, [live, qc]);
  return { direct, live, reconnecting };
}

/** Calm themed card (same look as the reconnect card). */
function CalmCard(props: { title: string; description: string; testId: string; compact?: boolean }) {
  return (
    <div
      role="status"
      data-testid={props.testId}
      className={cn(
        "buzz-reconnect-card flex items-center gap-3 rounded-2xl border border-border/60 bg-card/95 px-3.5 py-3 text-sm text-foreground shadow-sm",
        !props.compact && "mx-auto w-full max-w-md",
      )}
    >
      <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <Laptop aria-hidden="true" className="h-4 w-4" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block font-medium">{props.title}</span>
        <span className="mt-0.5 block text-xs text-muted-foreground">{props.description}</span>
      </span>
    </div>
  );
}

/**
 * Web only. `error` is the failed read (if any); pass none to describe the current Direct state.
 * Renders nothing on the desktop app or while Direct is live and the reads work.
 */
export function DesktopRepoCard(props: { error?: unknown; compact?: boolean; what?: string }) {
  const { direct, live, reconnecting } = useDesktopRepoDirect();
  const qc = useQueryClient();
  const what = props.what ?? "this repo";
  const stale = live && (isStale(props.error) || isCmdStale("call_openclaw_workspace_mcp_tool"));
  // Connect once when a repo page needs the desktop (the shared client keeps retrying on its own after that).
  React.useEffect(() => {
    if (isWebBuild() && direct && !direct.live) void direct.autoReconnect().catch(() => undefined);
  }, [direct]);
  // Live but a read failed (e.g. a desktop update landed later): check again quietly.
  React.useEffect(() => {
    if (!live || !props.error) return;
    const t = window.setInterval(() => void qc.invalidateQueries({ predicate: (q) => isHulaKey(q.queryKey) }), stale ? 60_000 : 15_000);
    return () => window.clearInterval(t);
  }, [live, props.error, stale, qc]);
  if (!isWebBuild()) return null;
  const wrap = (n: React.ReactNode) => <div className={cn(props.compact ? "" : "flex min-h-[40vh] items-center justify-center p-6")}>{n}</div>;
  if (reconnecting)
    return wrap(
      <ReconnectCard
        reconnecting
        title="Reconnecting to your Mac…"
        description="This comes back on its own"
        testId="desktop-repo-reconnecting"
        className={props.compact ? undefined : "mx-auto w-full max-w-md"}
      />,
    );
  if (stale)
    return wrap(
      <CalmCard
        compact={props.compact}
        testId="desktop-repo-update"
        title={`Update HulaBuzz on your Mac to browse ${what}`}
        description="Your Mac is running an older HulaBuzz. This shows up here on its own after the next update."
      />,
    );
  if (live && !props.error) return null;
  if (live)
    return wrap(
      <CalmCard
        compact={props.compact}
        testId="desktop-repo-retrying"
        title={`Checking ${what} on your Mac…`}
        description="This refreshes on its own."
      />,
    );
  return wrap(
    <CalmCard
      compact={props.compact}
      testId="desktop-repo-offline"
      title={`Open HulaBuzz on your Mac to browse ${what}`}
      description="It lives on your Mac and shows up here on its own once HulaBuzz is open there."
    />,
  );
}
