import * as React from "react";

import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import { findLatestReactableMessage } from "@/features/messages/lib/latestReactableMessage";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import { recordQuickReactionEmoji } from "@/features/messages/ui/useQuickReactionEmojis";
import {
  REACT_TO_LATEST_SHORTCUT_EVENT,
  type ReactToLatestShortcutDetail,
} from "@/shared/lib/keyboard-shortcuts";
import { Popover, PopoverAnchor, PopoverContent } from "@/shared/ui/popover";

/**
 * Emoji picker for the react-to-latest shortcut, anchored above the composer's
 * left edge (not on the message row): the shortcut fires while the user is at
 * the composer, so the picker opens where their attention already is, and the
 * target message may be scrolled far out of view. Must be rendered inside the
 * composer dock (a `position: relative` container) so the anchor can pin to
 * the dock's top-left corner.
 */
export function ReactToLatestPicker({
  channelId,
  entries,
  onToggleReaction,
}: {
  channelId: string | null;
  entries: readonly MainTimelineEntry[];
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
}) {
  const [targetId, setTargetId] = React.useState<string | null>(null);
  const anchorRef = React.useRef<HTMLDivElement | null>(null);

  // Resolve the target when the shortcut fires, not at select time: the
  // reaction belongs to the message that was newest when the picker opened,
  // even if more messages arrive while it is up.
  React.useEffect(() => {
    if (!channelId || !onToggleReaction) return;

    function handleShortcut(event: Event) {
      const { detail } = event as CustomEvent<ReactToLatestShortcutDetail>;
      if (detail.channelId !== channelId) return;
      setTargetId(findLatestReactableMessage(entries)?.id ?? null);
    }

    window.addEventListener(REACT_TO_LATEST_SHORTCUT_EVENT, handleShortcut);
    return () => {
      window.removeEventListener(
        REACT_TO_LATEST_SHORTCUT_EVENT,
        handleShortcut,
      );
    };
  }, [channelId, entries, onToggleReaction]);

  // Read the live entry so the add/remove decision sees current reactions;
  // a target deleted while the picker is open closes it.
  const target = React.useMemo(() => {
    if (!targetId) return null;
    return (
      entries.find((entry) => entry.message.id === targetId)?.message ?? null
    );
  }, [entries, targetId]);

  const handleSelect = React.useCallback(
    (emoji: string) => {
      if (!target || !onToggleReaction) return;
      const remove = Boolean(
        target.reactions?.some(
          (reaction) =>
            reaction.emoji === emoji && reaction.reactedByCurrentUser,
        ),
      );
      void onToggleReaction(target, emoji, remove)
        .then(() => recordQuickReactionEmoji(emoji))
        .catch(() => {});
      setTargetId(null);
    },
    [onToggleReaction, target],
  );

  if (!onToggleReaction) {
    return null;
  }

  return (
    <Popover
      onOpenChange={(open) => {
        if (!open) setTargetId(null);
      }}
      open={Boolean(target)}
    >
      {/* left-5 lines the picker up with the composer's visible left edge
          (the dock backdrop's inset-x-5 gutter). */}
      <PopoverAnchor
        className="pointer-events-none absolute top-0 left-5"
        ref={anchorRef}
      />
      <PopoverContent
        align="start"
        className="w-auto overflow-hidden rounded-2xl border-0 bg-transparent p-0 shadow-none"
        data-testid="react-to-latest-picker"
        onCloseAutoFocus={(event) => {
          // There is no trigger to return focus to; hand it back to the
          // composer input the user was in when the shortcut fired.
          event.preventDefault();
          anchorRef.current?.parentElement
            ?.querySelector<HTMLElement>('[data-testid="message-input"]')
            ?.focus();
        }}
        side="top"
        sideOffset={8}
      >
        <EmojiPicker autoFocus onSelect={handleSelect} />
      </PopoverContent>
    </Popover>
  );
}
