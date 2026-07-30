import * as React from "react";
import { useNavigate, useRouter } from "@tanstack/react-router";

import {
  type DevRouteSeed,
  type DevRouteSelection,
  isSettlingTransition,
  readDevRouteSeed,
} from "./devRoute";

/**
 * One-shot read of the channel/thread the URL carried when dev mode
 * mounted — the conversation the standard layout had open.
 */
export function useDevRouteSeed(): DevRouteSeed | null {
  const router = useRouter();
  const [seed] = React.useState(() =>
    readDevRouteSeed(
      router.state.location as {
        pathname: string;
        search: Record<string, unknown>;
      },
    ),
  );
  return seed;
}

/**
 * Keeps the router URL in step with the dev shell's selection so ⇧⌘D back
 * to the standard layout lands on the same channel/thread — and reopens the
 * seeded `?thread=` side chat once the channel's message window is in.
 *
 * URL writes use `replace`: dev-mode channel walking (⌥↑/⌥↓ steps through
 * the list) must not flood the history stack.
 */
export function useDevRouteSync({
  seed,
  view,
  channelId,
  threadOpen,
  selectedRootId,
  messagesReady,
  roots,
  onOpenThread,
}: {
  seed: DevRouteSeed | null;
  view: "fresh" | "navigator" | "channel";
  /** The open channel's resolved id — null while loading or off-channel. */
  channelId: string | null;
  threadOpen: boolean;
  selectedRootId: string | null;
  /** Whether the open channel's message window has loaded. */
  messagesReady: boolean;
  roots: readonly { id: string }[];
  onOpenThread: (rootId: string) => void;
}): void {
  const navigate = useNavigate();

  // The side chat only reopens when the seeded root actually resolved in
  // the loaded window, so a stale or paged-out id degrades to just the
  // channel. Opening any other channel first cancels the seed.
  const pendingThreadSeedRef = React.useRef(seed?.threadRootId ?? null);
  React.useEffect(() => {
    const rootId = pendingThreadSeedRef.current;
    if (rootId === null || channelId === null) return;
    if (channelId !== seed?.channelId) {
      pendingThreadSeedRef.current = null;
      return;
    }
    if (!messagesReady) return;
    pendingThreadSeedRef.current = null;
    if (threadOpen || selectedRootId !== null) return;
    if (!roots.some((root) => root.id === rootId)) return;
    onOpenThread(rootId);
  }, [
    channelId,
    messagesReady,
    onOpenThread,
    roots,
    seed,
    selectedRootId,
    threadOpen,
  ]);

  const selection = React.useMemo<DevRouteSelection>(
    () => ({
      view,
      channelId: view === "channel" ? channelId : null,
      threadRootId: view === "channel" && threadOpen ? selectedRootId : null,
    }),
    [channelId, selectedRootId, threadOpen, view],
  );

  const previousRef = React.useRef<DevRouteSelection | null>(null);
  const dirtyRef = React.useRef(false);
  React.useEffect(() => {
    const previous = previousRef.current;
    previousRef.current = selection;
    if (previous === null) return;
    if (
      previous.view === selection.view &&
      previous.channelId === selection.channelId &&
      previous.threadRootId === selection.threadRootId
    ) {
      return;
    }
    if (!dirtyRef.current) {
      if (isSettlingTransition(previous, selection, seed)) return;
      // Peeking at the navigator from fresh (and backing out) opens
      // nothing — it should not claim the URL.
      if (previous.view !== "channel" && selection.view !== "channel") return;
      dirtyRef.current = true;
    }
    // Navigator previews don't move the URL; only a real open does.
    if (selection.view === "navigator") return;
    if (selection.view === "channel") {
      if (selection.channelId === null) return;
      void navigate({
        to: "/channels/$channelId",
        params: { channelId: selection.channelId },
        search: selection.threadRootId
          ? { thread: selection.threadRootId }
          : {},
        replace: true,
      });
      return;
    }
    void navigate({ to: "/", replace: true });
  }, [navigate, seed, selection]);
}
