import * as React from "react";
import { useTerminalContextOverride } from "@/app/TerminalContextOverrideContext";
import { useProfileQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import { MainInsetProvider } from "@/shared/layout/MainInsetContext";
import { MessageBubbleContext } from "@/features/messages/ui/MessageBubbleContext";
import { PULSE_CONVERSATION_KEYS } from "../lib/pulsePanelState";

const ChannelScreen = React.lazy(async () => {
  const module = await import("@/features/channels/ui/ChannelScreen");
  return { default: module.ChannelScreen };
});
const EMPTY_EVENTS: RelayEvent[] = [];

export function PulseChannelDetail({ channel }: { channel: Channel }) {
  const identity = useIdentityQuery();
  const profile = useProfileQuery();
  const detailRef = React.useRef<HTMLDivElement>(null);
  const { values, applyPatch } = useHistorySearchState(PULSE_CONVERSATION_KEYS);
  const terminalContext = React.useMemo(
    () => ({
      channelId: channel.id,
      channelName: channel.name,
      threadId: values.thread,
    }),
    [channel.id, channel.name, values.thread],
  );
  useTerminalContextOverride(terminalContext);
  return (
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
            <ChannelScreen
              key={channel.id}
              activeChannel={channel}
              currentIdentity={identity.data}
              currentProfile={profile.data}
              drillInThreads
              autoSendDraftKey={null}
              onCloseForumPost={() => applyPatch({ post: null, reply: null })}
              onSelectForumPost={(post) => applyPatch({ post, reply: null })}
              selectedForumPostId={values.post}
              targetForumReplyId={values.reply}
              targetMessageEvents={EMPTY_EVENTS}
              targetMessageId={null}
            />
          </React.Suspense>
        </MainInsetProvider>
      </MessageBubbleContext.Provider>
    </div>
  );
}
