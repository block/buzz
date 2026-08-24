import type { ComponentProps } from "react";

import { useCardMintJobs } from "@/features/agents/cardMintStore";
import { CardMintComposerChip } from "@/features/agents/ui/CardMintComposerChip";
import { ChannelComposerActivityRow } from "@/features/channels/ui/ChannelComposerActivityRow";
import { ComposerActivityAccessory } from "@/features/messages/ui/ComposerActivityAccessory";

type ChannelComposerActivityAccessoryProps = {
  agents: ComponentProps<typeof ChannelComposerActivityRow>["agents"];
  channel: ComponentProps<typeof ChannelComposerActivityRow>["channel"];
  currentPubkey: ComponentProps<
    typeof ChannelComposerActivityRow
  >["currentPubkey"];
  onOpenAgentSession: ComponentProps<
    typeof ChannelComposerActivityRow
  >["onOpenAgentSession"];
  profiles: ComponentProps<typeof ChannelComposerActivityRow>["profiles"];
  typingPubkeys: string[];
  visible: boolean;
};

/** Main's shared channel activity host with the unified live-activity strip. */
export function ChannelComposerActivityAccessory({
  agents,
  channel,
  currentPubkey,
  onOpenAgentSession,
  profiles,
  typingPubkeys,
  visible,
}: ChannelComposerActivityAccessoryProps) {
  const cardMintJobs = useCardMintJobs();

  return (
    <ComposerActivityAccessory
      className="px-5"
      testId="channel-composer-activity-row"
      visible={visible}
    >
      <div className="flex h-8.5 w-full items-center gap-2 overflow-visible pb-1.5 pl-2">
        {cardMintJobs.length > 0 ? <CardMintComposerChip /> : null}
        <ChannelComposerActivityRow
          agents={agents}
          channel={channel}
          currentPubkey={currentPubkey}
          onOpenAgentSession={onOpenAgentSession}
          profiles={profiles}
          typingPubkeys={typingPubkeys}
        />
      </div>
    </ComposerActivityAccessory>
  );
}
