/**
 * Community identity shared across the console: the navigation context, the
 * community badge and the not-connected warning.
 */

import { createContext, useContext } from "react";
import { cn } from "@/shared/lib/cn";
import { getConnectedCommunityHost } from "./api";
import { useAsyncLoad } from "./AdminConsolePanelHelpers";
/** A community as a row or badge knows it: id may be unknown, never host. */
export type CommunityRef = { id: string; host: string; icon?: string | null };

type CommunityNav = {
  /** Opens a community's page. */
  open: (community: CommunityRef) => void;
  /** Host the active relay serves, resolved natively. */
  connectedHost: string | null;
};

export const CommunityNavContext = createContext<CommunityNav>({
  open: () => {},
  connectedHost: null,
});

export function useCommunityNav(): CommunityNav {
  return useContext(CommunityNavContext);
}

/** The connected community's host, resolved natively; reloads on `key`. */
export function useConnectedHost(key: string): string | null {
  const state = useAsyncLoad(getConnectedCommunityHost, [key], 0);
  return state.status === "ok" ? state.data : null;
}

/**
 * Host plus icon (or the host's first letter). Opens the community's page;
 * a row whose source community was purged
 * (`id` null) gets a non-navigable "community removed" badge.
 */
export function CommunityBadge({
  id,
  host,
  icon,
}: {
  id: string | null;
  host: string | null;
  icon?: string | null;
}) {
  const { open } = useCommunityNav();
  const base =
    "inline-flex max-w-full items-center gap-1.5 rounded-full border border-border/60 px-2 py-0.5 text-xs";
  if (id === null || !host) {
    return (
      <span
        className={cn(base, "text-muted-foreground")}
        data-testid="community-badge-removed"
      >
        community removed
      </span>
    );
  }
  const content = (
    <>
      {icon ? (
        <img alt="" className="h-4 w-4 rounded-full" src={icon} />
      ) : (
        <span
          className="flex h-4 w-4 items-center justify-center rounded-full bg-muted text-2xs font-semibold uppercase"
          data-testid="community-badge-initial"
        >
          {host[0]}
        </span>
      )}
      <span className="truncate" data-testid="community-badge-host">
        {host}
      </span>
    </>
  );
  return (
    <button
      className={cn(base, "hover:bg-muted/50")}
      data-testid={`community-badge-${host}`}
      onClick={() => open({ id, host, icon })}
      title={`Open ${host}`}
      type="button"
    >
      {content}
    </button>
  );
}

/** "Not the community you're connected to", when that applies. */
export function NotConnectedWarning({ host }: { host: string }) {
  const { connectedHost } = useCommunityNav();
  if (connectedHost === null || connectedHost === host) return null;
  return (
    <span
      className="text-xs text-amber-600 dark:text-amber-400"
      data-testid="community-not-connected"
    >
      Not the community you're connected to.
    </span>
  );
}
