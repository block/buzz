import type * as React from "react";
import { Pin } from "lucide-react";

import {
  makeThreadRailPin,
  useThreadRailContext,
} from "@/features/channels/ThreadRailContext";
import type { TimelineMessage } from "@/features/messages/types";
import {
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelTitle,
} from "@/shared/layout/AuxiliaryPanel";
import { Button } from "@/shared/ui/button";

export function MessageThreadPanelHeader({
  channelId,
  channelName,
  expandedReplyIds,
  headerLeading,
  headerTitle = "Thread",
  headerTitleAriaLabel,
  isFocusMode,
  isSinglePanelView,
  onClose,
  onHeaderTitleClick,
  returnAnchorId,
  showBackButton,
  threadHead,
}: {
  channelId: string | null;
  channelName: string;
  expandedReplyIds: ReadonlySet<string>;
  headerLeading?: React.ReactNode;
  headerTitle?: string;
  headerTitleAriaLabel?: string;
  isFocusMode: boolean;
  isSinglePanelView: boolean;
  onClose: () => void;
  onHeaderTitleClick?: () => void;
  returnAnchorId: string | undefined;
  showBackButton?: boolean;
  threadHead: TimelineMessage;
}) {
  const threadRail = useThreadRailContext();
  const isPinned =
    channelId !== null &&
    threadRail.pins.some(
      (pin) => pin.channelId === channelId && pin.rootId === threadHead.id,
    );

  const title = onHeaderTitleClick ? (
    <button
      aria-label={headerTitleAriaLabel ?? `Open ${headerTitle}`}
      className="min-w-0 max-w-full truncate text-left hover:underline"
      data-testid="message-thread-open-channel"
      onClick={onHeaderTitleClick}
      title={headerTitleAriaLabel ?? `Open ${headerTitle}`}
      type="button"
    >
      {headerTitle}
    </button>
  ) : (
    headerTitle
  );

  return (
    <AuxiliaryPanelHeader backdrop>
      <AuxiliaryPanelHeaderGroup
        backButtonAriaLabel="Back to conversation"
        backButtonTestId="message-thread-back"
        leading={headerLeading}
        onBack={
          (showBackButton ?? (isSinglePanelView && !isFocusMode))
            ? onClose
            : undefined
        }
      >
        <AuxiliaryPanelTitle>{title}</AuxiliaryPanelTitle>
        {threadRail.isScoped && channelId ? (
        <Button
          aria-label={isPinned ? "Thread pinned" : "Pin to thread rail"}
          aria-pressed={isPinned}
          data-testid="pin-thread-to-rail"
          disabled={isPinned}
          onClick={() =>
            threadRail.pin(
              makeThreadRailPin({
                channelId,
                channelName,
                rootExcerpt: threadHead.body.slice(0, 96),
                rootId: threadHead.id,
                returnAnchorId: returnAnchorId ?? undefined,
                expandedReplyIds: [...expandedReplyIds],
              }),
            )
          }
          size="icon-xs"
          type="button"
          variant="ghost"
        >
          <Pin aria-hidden />
        </Button>
        ) : null}
      </AuxiliaryPanelHeaderGroup>
    </AuxiliaryPanelHeader>
  );
}
