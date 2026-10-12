// Web only: the Agents settings, dialogs and page show the DESKTOP's data over the direct connection. These helpers say whether it
// is available and explain, instead of showing the web app's own (empty) local values, when it is not.
import { isTauri } from "@tauri-apps/api/core";
import * as React from "react";

import { getProxyDirect, NEEDS_DIRECT, subscribeProxy } from "../remoteCmd";

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
