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
};

export function WaveMessageAttachment({
  channelId,
  fallbackText,
}: WaveMessageAttachmentProps) {
  const queryClient = useQueryClient();
  const { isStarting, startHuddle } = useHuddle();

  const handleStartHuddle = React.useCallback(
    async (event: React.MouseEvent<HTMLButtonElement>) => {
      event.preventDefault();
      event.stopPropagation();

      if (!channelId || isStarting) {
        return;
      }

      try {
        await startHuddle(channelId, []);
        await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to start huddle.",
        );
      }
    },
    [channelId, isStarting, queryClient, startHuddle],
  );

  return (
    <Attachment
      className="buzz-wave-hover-trigger mt-1 max-w-md"
      data-testid="message-wave-attachment"
      size="default"
    >
      <AttachmentMedia
        aria-hidden="true"
        className="bg-primary/10 text-2xl text-foreground"
        variant="image"
      >
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
          disabled={!channelId || isStarting}
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
