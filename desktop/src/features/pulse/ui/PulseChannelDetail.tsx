import * as React from "react";
import { useTerminalContextOverride } from "@/app/TerminalContextOverrideContext";
import { useLocation } from "@tanstack/react-router";
import { selectSearchHighlightRouteState } from "@/app/routes/searchHighlightRouteState";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel } from "@/shared/api/types";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import { MainInsetProvider } from "@/shared/layout/MainInsetContext";
import { MessageBubbleContext } from "@/features/messages/ui/MessageBubbleContext";
import { PULSE_CONVERSATION_KEYS } from "../lib/pulsePanelState";

const ChannelRouteScreen = React.lazy(async () => {
  const module = await import("@/app/routes/ChannelRouteScreen");
  return { default: module.ChannelRouteScreen };
});

export function PulseChannelDetail({
  channel,
  channelId = channel?.id,
}: {
  channel?: Channel;
  channelId?: string;
}) {
  const identity = useIdentityQuery();
  const searchHighlight = useLocation({
    select: selectSearchHighlightRouteState,
  });
  const detailRef = React.useRef<HTMLDivElement>(null);
  const { values } = useHistorySearchState(PULSE_CONVERSATION_KEYS);
  const terminalContext = React.useMemo(
    () => ({
      channelId: channelId ?? "",
      channelName: channel?.name ?? "Conversation",
      threadId: values.thread,
    }),
    [channelId, channel?.name, values.thread],
  );
  useTerminalContextOverride(terminalContext);
  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col"
      data-testid="pulse-message-view"
    >
      <div
        ref={detailRef}
        className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
      >
        <MessageBubbleContext.Provider value={identity.data?.pubkey ?? null}>
          <MainInsetProvider mainInsetRef={detailRef}>
            <React.Suspense
              fallback={
                <p role="status" className="p-6 text-sm text-muted-foreground">
                  Loading conversation…
                </p>
              }
            >
              {channelId && (
                <ChannelRouteScreen
                  key={channelId}
                  embedded
                  channelId={channelId}
                  autoSendDraftKey={null}
                  searchHighlight={searchHighlight}
                  selectedPostId={values.post}
                  targetReplyId={values.reply}
                  targetMessageId={values.messageId}
                  targetThreadRootId={values.threadRootId ?? values.thread}
                />
              )}
            </React.Suspense>
          </MainInsetProvider>
        </MessageBubbleContext.Provider>
      </div>
    </div>
  );
}
