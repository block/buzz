import * as React from "react";
import { createPortal } from "react-dom";

import type { TimelineMessage } from "@/features/messages/types";
import { Button } from "@/shared/ui/button";
import { ForwardMessageDialog } from "./ForwardMessageDialog";

/**
 * Multi-select state for the message timeline, scoped to a single channel
 * view (not persisted, not global app state — matches the plan's "local
 * component/context state scoped to the channel view").
 *
 * Entry point: Ctrl+Click (or Cmd+Click) on a row (`MessageRow`) calls
 * `toggle(message)` directly — there is no separate "enter selection mode"
 * step. `active` is derived from whether the selected set is non-empty, so
 * selection becomes active the moment the first message is toggled in, and
 * inactive again once the set is cleared (via `clear`, e.g. the floating
 * bar's "Cancel" button or a successful forward). `MessageRow` reads this
 * context directly (rather than threading a prop through
 * `TimelineMessageList`/`MessageTimeline`/`TimelineMessageRow`) to decide
 * whether to render its selection checkbox — the same pattern the row
 * already uses for pinned-message state (`usePinnedMessagesActions`) and
 * reminders (`useRemindLater`).
 *
 * The default context value is a safe no-op so `MessageRow` instances that
 * render outside a provider (thread panels, huddle transcripts, home inbox
 * previews, etc.) simply never accumulate selection instead of crashing.
 * `isAvailable` lets those call sites skip Ctrl+Click handling entirely
 * rather than toggling into a selection set nobody renders a bar for.
 */
export type MessageSelectionApi = {
  isAvailable: boolean;
  active: boolean;
  selectedCount: number;
  isSelected: (messageId: string) => boolean;
  clear: () => void;
  toggle: (message: TimelineMessage) => void;
};

const NOOP_SELECTION: MessageSelectionApi = {
  isAvailable: false,
  active: false,
  selectedCount: 0,
  isSelected: () => false,
  clear: () => {},
  toggle: () => {},
};

const MessageSelectionContext =
  React.createContext<MessageSelectionApi>(NOOP_SELECTION);

export function useMessageSelection(): MessageSelectionApi {
  return React.useContext(MessageSelectionContext);
}

export function MessageSelectionProvider({
  channelId,
  children,
}: {
  channelId?: string | null;
  children: React.ReactNode;
}) {
  const [selected, setSelected] = React.useState<Map<string, TimelineMessage>>(
    () => new Map(),
  );
  const [isForwardDialogOpen, setIsForwardDialogOpen] = React.useState(false);
  // Same lazy-mount rationale as MessageActionBar's single-message Forward
  // entry — avoid running the recipient-search hook until the dialog is
  // actually opened once.
  const [hasOpenedForwardDialog, setHasOpenedForwardDialog] =
    React.useState(false);

  // Selection is scoped to a single channel view — leaving it clears state
  // rather than carrying stale ids into the next channel.
  const previousChannelIdRef = React.useRef(channelId);
  if (previousChannelIdRef.current !== channelId) {
    previousChannelIdRef.current = channelId;
    if (selected.size > 0) {
      setSelected(new Map());
    }
  }

  const clear = React.useCallback(() => {
    setSelected(new Map());
  }, []);

  const toggle = React.useCallback((message: TimelineMessage) => {
    setSelected((current) => {
      const next = new Map(current);
      if (next.has(message.id)) {
        next.delete(message.id);
      } else {
        next.set(message.id, message);
      }
      return next;
    });
  }, []);

  const isSelected = React.useCallback(
    (messageId: string) => selected.has(messageId),
    [selected],
  );

  const selectedMessagesChronological = React.useMemo(
    () => [...selected.values()].sort((a, b) => a.createdAt - b.createdAt),
    [selected],
  );

  const api = React.useMemo<MessageSelectionApi>(
    () => ({
      isAvailable: true,
      active: selected.size > 0,
      selectedCount: selected.size,
      isSelected,
      clear,
      toggle,
    }),
    [clear, isSelected, selected.size, toggle],
  );

  // Rendered via a portal to `document.body`, and pinned to the TOP of the
  // viewport, specifically to get out of the composer's territory entirely.
  // A previous version of this bar shared the composer's DOM subtree as a
  // plain sibling `<div>` at the bottom of the screen and lost a z-index tie
  // to the composer overlay (both were z-40; equal z-index ties resolve by
  // paint/DOM order, so the later composer overlay painted on top and fully
  // hid the bar) — then, even after bumping to z-50, it still had to guess
  // at a static bottom-offset large enough to clear the composer's dynamic
  // height (multi-line text, attachments, banners all grow it), which is a
  // fragile magic number. Portaling to `document.body` + docking at the top
  // sidesteps both problems at once: it's no longer sharing a stacking
  // context with the composer at all, and it no longer needs to know the
  // composer's height, because it isn't near it.
  const selectionBar =
    selected.size > 0
      ? createPortal(
          <div
            className="pointer-events-none fixed inset-x-0 top-4 z-50 flex justify-center px-4"
            data-testid="message-selection-toolbar"
          >
            <div className="pointer-events-auto flex items-center gap-2 rounded-full border border-border/70 bg-background/95 px-3 py-2 shadow-lg backdrop-blur-sm supports-[backdrop-filter]:bg-background/85">
              <span className="px-1 text-sm text-muted-foreground">
                {selected.size} selected
              </span>
              <Button
                data-testid="message-selection-forward"
                onClick={() => {
                  setHasOpenedForwardDialog(true);
                  setIsForwardDialogOpen(true);
                }}
                size="sm"
                type="button"
                variant="default"
              >
                Forward ({selected.size})
              </Button>
              <Button
                data-testid="message-selection-cancel"
                onClick={clear}
                size="sm"
                type="button"
                variant="ghost"
              >
                Cancel
              </Button>
            </div>
          </div>,
          document.body,
        )
      : null;

  return (
    <MessageSelectionContext value={api}>
      {children}
      {selectionBar}
      {hasOpenedForwardDialog ? (
        <ForwardMessageDialog
          messages={selectedMessagesChronological}
          onOpenChange={setIsForwardDialogOpen}
          onForwarded={clear}
          open={isForwardDialogOpen}
        />
      ) : null}
    </MessageSelectionContext>
  );
}
