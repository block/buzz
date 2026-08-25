import type * as React from "react";

export type AnchorState =
  | { kind: "at-bottom" }
  | { kind: "message"; messageId: string; topOffset: number }
  | { kind: "pinned-center"; messageId: string; contentTop: number };

export type ScrollToMessageResult = "centered" | "pending" | "missing";

export type UseAnchoredScrollOptions = {
  scrollContainerRef: React.RefObject<HTMLDivElement | null>;
  contentRef: React.RefObject<HTMLDivElement | null>;
  channelId?: string | null;
  isLoading: boolean;
  messages: Array<{ id: string }>;
  splitPanelOpen?: boolean;
  targetMessageId?: string | null;
  highlightTargetMessage?: boolean;
  pinTargetCentered?: boolean;
  topBoundaryReached?: boolean;
  onTargetReached?: (messageId: string) => void;
  onTargetSettled?: (messageId: string) => void;
  virtualCancelBottomIntent?: () => void;
  virtualScrollToMessage?: (
    messageId: string,
    options?: { behavior?: ScrollBehavior },
  ) => boolean;
  virtualScrollBy?: (offset: number) => void;
  virtualScrollToBottom?: (behavior?: ScrollBehavior) => void;
  virtualSettleAtBottom?: () => void;
  virtualizerOwnsPrependAnchoring?: boolean;
  virtualizerRenderVersion?: number;
};

export type UseAnchoredScrollResult = {
  onScroll: () => void;
  isAtBottom: boolean;
  newMessageCount: number;
  highlightedMessageId: string | null;
  scrollToBottom: (behavior?: ScrollBehavior) => void;
  settleAtBottomAfterLayout: () => boolean;
  scrollToBottomOnNextUpdate: () => void;
  scrollToMessage: (
    messageId: string,
    options?: { highlight?: boolean; behavior?: ScrollBehavior },
  ) => ScrollToMessageResult;
  onVirtualizerAtBottomStateChange: (atBottom: boolean) => void;
};
