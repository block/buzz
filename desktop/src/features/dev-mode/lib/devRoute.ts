/**
 * Dev mode ↔ router URL selection sync.
 *
 * The standard layout keeps its open conversation in the URL
 * (`/channels/$channelId?thread=<rootId>`); dev mode holds selection in
 * component state, so toggling display styles (⇧⌘D) would otherwise lose
 * the open channel/thread in both directions. These helpers translate
 * between the two: the shell seeds its initial state from the URL and
 * writes its own selection back once the user navigates within dev mode.
 */

/** Channel/thread the URL carried when the dev shell mounted. */
export type DevRouteSeed = {
  channelId: string;
  threadRootId: string | null;
};

/** The dev shell's current logical selection. */
export type DevRouteSelection = {
  view: "fresh" | "navigator" | "channel";
  channelId: string | null;
  threadRootId: string | null;
};

const CHANNEL_PATH_PATTERN = /^\/channels\/([^/]+)$/;

export function readDevRouteSeed(location: {
  pathname: string;
  search: Record<string, unknown>;
}): DevRouteSeed | null {
  const match = CHANNEL_PATH_PATTERN.exec(location.pathname);
  if (!match) return null;
  // `thread` is the standard layout's open side panel; `threadRootId`
  // arrives with message deep links (see channels.$channelId.tsx).
  const thread = location.search.thread ?? location.search.threadRootId;
  return {
    channelId: decodeURIComponent(match[1]),
    threadRootId:
      typeof thread === "string" && thread.length > 0 ? thread : null,
  };
}

/**
 * Whether a selection change is the seeded state resolving (channel list or
 * message window loading in) rather than the user navigating. Settling
 * transitions only ever fill a null field with its seeded value; anything
 * else hands URL ownership to dev mode. Until then the URL must not be
 * rewritten — it may carry state the shell does not model (message targets,
 * profile panels) that the standard layout should get back unchanged.
 */
export function isSettlingTransition(
  previous: DevRouteSelection,
  current: DevRouteSelection,
  seed: DevRouteSeed | null,
): boolean {
  if (seed === null) return false;
  if (previous.view !== "channel" || current.view !== "channel") return false;
  const channelSettling =
    current.channelId === previous.channelId ||
    (previous.channelId === null && current.channelId === seed.channelId);
  const threadSettling =
    current.threadRootId === previous.threadRootId ||
    (previous.threadRootId === null &&
      current.threadRootId === seed.threadRootId);
  return channelSettling && threadSettling;
}
