import type { ComponentProps } from "react";

import { BotActivityComposerAction } from "@/features/channels/ui/BotActivityBar";
import { ComposerActivityAccessory } from "@/features/messages/ui/ComposerActivityAccessory";
import { TypingIndicatorRow } from "@/features/messages/ui/TypingIndicatorRow";

type ChannelComposerActivityAccessoryProps = {
  agents: ComponentProps<typeof BotActivityComposerAction>["agents"];
  channel: ComponentProps<typeof TypingIndicatorRow>["channel"];
  currentPubkey: ComponentProps<typeof TypingIndicatorRow>["currentPubkey"];
  onOpenAgentSession: ComponentProps<
    typeof BotActivityComposerAction
  >["onOpenAgentSession"];
  openAgentSessionPubkey: ComponentProps<
    typeof BotActivityComposerAction
  >["openAgentSessionPubkey"];
  profiles: ComponentProps<typeof BotActivityComposerAction>["profiles"];
  typingPubkeys: string[];
  visible: boolean;
  workingBotPubkeys: string[];
};

export function ChannelComposerActivityAccessory({
  agents,
  channel,
  currentPubkey,
  onOpenAgentSession,
  openAgentSessionPubkey,
  profiles,
  typingPubkeys,
  visible,
  workingBotPubkeys,
}: ChannelComposerActivityAccessoryProps) {
  return (
    <ComposerActivityAccessory
      className="px-5"
      testId="channel-composer-activity-row"
      visible={visible}
    >
      <div className="flex w-full items-center gap-2 overflow-visible pl-2">
        {workingBotPubkeys.length > 0 ? (
          <div className="flex min-w-0 flex-1 overflow-visible">
            <BotActivityComposerAction
              agents={agents}
              channelId={channel?.id ?? null}
              onOpenAgentSession={onOpenAgentSession}
              openAgentSessionPubkey={openAgentSessionPubkey}
              profiles={profiles}
              workingBotPubkeys={workingBotPubkeys}
              variant="inline"
            />
          </div>
        ) : null}
        {typingPubkeys.length > 0 ? (
          <TypingIndicatorRow
            channel={channel}
            className="min-w-0 flex-1 py-0 pl-[calc(0.75rem+1px)] pr-0 sm:pl-[calc(1rem+1px)]"
            currentPubkey={currentPubkey}
            profiles={profiles}
            typingPubkeys={typingPubkeys}
          />
        ) : null}
      </div>
    </ComposerActivityAccessory>
  );
}
