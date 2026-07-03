import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { channelsQueryKey } from "@/features/channels/hooks";
import { useHuddle } from "@/features/huddle";
import {
  Attachment,
  AttachmentAction,
  AttachmentActions,
  AttachmentContent,
  AttachmentDescription,
  AttachmentMedia,
  AttachmentTitle,
} from "@/shared/ui/attachment";

type WaveMessageAttachmentProps = {
  channelId?: string | null;
  fallbackText: string;
  huddleMemberPubkeys?: readonly string[];
  huddleMemberPubkeysPending?: boolean;
};

export function WaveMessageAttachment({
  channelId,
  fallbackText,
  huddleMemberPubkeys = [],
  huddleMemberPubkeysPending = false,
}: WaveMessageAttachmentProps) {
  const queryClient = useQueryClient();
  const { isStarting, startHuddle } = useHuddle();
  const startHuddleDisabled =
    !channelId || isStarting || huddleMemberPubkeysPending;

  const handleStartHuddle = React.useCallback(
    async (event: React.MouseEvent<HTMLButtonElement>) => {
      event.preventDefault();
      event.stopPropagation();

      if (startHuddleDisabled) {
        return;
      }

      try {
        await startHuddle(channelId, [...huddleMemberPubkeys]);
        await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to start huddle.",
        );
      }
    },
    [
      channelId,
      huddleMemberPubkeys,
      queryClient,
      startHuddle,
      startHuddleDisabled,
    ],
  );

  return (
    <Attachment
      className="buzz-wave-hover-trigger mt-1 max-w-md"
      data-testid="message-wave-attachment"
      size="default"
    >
      <AttachmentMedia aria-hidden="true" className="text-lg">
        <span className="buzz-wave-hand">👋</span>
      </AttachmentMedia>
      <AttachmentContent>
        <AttachmentTitle>{fallbackText}</AttachmentTitle>
        <AttachmentDescription>
          Start a huddle to talk to them.
        </AttachmentDescription>
      </AttachmentContent>
      <AttachmentActions>
        <AttachmentAction
          disabled={startHuddleDisabled}
          onClick={handleStartHuddle}
          size="xs"
          type="button"
        >
          Start huddle
        </AttachmentAction>
      </AttachmentActions>
    </Attachment>
  );
}
