import * as React from "react";

import type { Channel, RelayEvent } from "@/shared/api/types";

type ShellView = "fresh" | "navigator" | "channel";

/** ↑/↓ across the navigator list, cards, and ⌥↑/⌥↓ channel stepping. */
export function useDevShellNavigation({
  activeMainId,
  navigatorId,
  onOpenChannel,
  orderedChannels,
  roots,
  selectedRootId,
  setNavigatorId,
  setSelectedRootId,
  setThreadOpen,
  setView,
  view,
}: {
  activeMainId: string | null;
  navigatorId: string | null;
  onOpenChannel: (channelId: string) => void;
  orderedChannels: Channel[];
  roots: RelayEvent[];
  selectedRootId: string | null;
  setNavigatorId: (id: string | null) => void;
  setSelectedRootId: (id: string | null) => void;
  setThreadOpen: (open: boolean) => void;
  setView: (view: ShellView) => void;
  view: ShellView;
}) {
  const navigateChannels = React.useCallback(
    (direction: 1 | -1) => {
      if (orderedChannels.length === 0) return;
      const currentIndex = orderedChannels.findIndex(
        (session) => session.id === navigatorId,
      );
      if (currentIndex === -1) {
        setNavigatorId(orderedChannels[orderedChannels.length - 1].id);
        return;
      }
      // ↑ walks up the visible list; ↓ back down. The navigator stays
      // highlighted at the ends — only Enter or Escape leave it.
      const nextIndex = Math.min(
        orderedChannels.length - 1,
        Math.max(0, currentIndex + direction),
      );
      setNavigatorId(orderedChannels[nextIndex].id);
    },
    [navigatorId, orderedChannels, setNavigatorId],
  );

  // ⌥↑/⌥↓ from the composer: open the previous/next channel in the visible
  // list directly — focus stays in the box the whole time.
  const stepChannel = React.useCallback(
    (direction: 1 | -1) => {
      if (orderedChannels.length === 0) return;
      const referenceId = view === "channel" ? activeMainId : navigatorId;
      const currentIndex = orderedChannels.findIndex(
        (session) => session.id === referenceId,
      );
      if (currentIndex === -1) {
        // Nothing open — ⌥↑ enters the list at the bottom (nearest channel).
        if (direction === -1) {
          onOpenChannel(orderedChannels[orderedChannels.length - 1].id);
        }
        return;
      }
      const nextIndex = Math.min(
        orderedChannels.length - 1,
        Math.max(0, currentIndex + direction),
      );
      if (nextIndex === currentIndex) return;
      onOpenChannel(orderedChannels[nextIndex].id);
    },
    [activeMainId, navigatorId, onOpenChannel, orderedChannels, view],
  );

  const navigateCards = React.useCallback(
    (direction: 1 | -1) => {
      if (roots.length === 0) return;
      const currentIndex = roots.findIndex(
        (root) => root.id === selectedRootId,
      );
      if (currentIndex === -1) {
        // ArrowUp enters the cards at the newest prompt; ArrowDown is a no-op.
        if (direction === -1) {
          setSelectedRootId(roots[roots.length - 1].id);
        }
        return;
      }
      const nextIndex = currentIndex + direction;
      if (nextIndex >= roots.length) {
        // Past the newest card — back to plain channel input.
        setSelectedRootId(null);
        setThreadOpen(false);
        return;
      }
      setSelectedRootId(roots[Math.max(0, nextIndex)].id);
    },
    [roots, selectedRootId, setSelectedRootId, setThreadOpen],
  );

  const handleNavigate = React.useCallback(
    (direction: 1 | -1) => {
      if (view === "channel") {
        navigateCards(direction);
        return;
      }
      if (view === "fresh") {
        if (direction === -1) {
          setView("navigator");
          setNavigatorId(
            orderedChannels.length > 0
              ? orderedChannels[orderedChannels.length - 1].id
              : null,
          );
        }
        return;
      }
      navigateChannels(direction);
    },
    [
      navigateChannels,
      navigateCards,
      orderedChannels,
      setNavigatorId,
      setView,
      view,
    ],
  );

  return { handleNavigate, navigateCards, stepChannel };
}
