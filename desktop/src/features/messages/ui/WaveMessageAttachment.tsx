import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { channelsQueryKey } from "@/features/channels/hooks";
import { HighlightedSearchText } from "@/features/search/ui/HighlightedSearchText";
import { useHuddle } from "@/features/huddle/HuddleContext";
import { formatHuddleActionError } from "@/features/huddle/lib/huddleError";
import {
  Attachment,
  AttachmentAction,
  AttachmentActions,
  AttachmentContent,
  AttachmentDescription,
  AttachmentMedia,
  AttachmentTitle,
} from "@/shared/ui/attachment";
import { huddlesAvailable } from "@/features/huddle/lib/huddlesAvailable";
import { WebHuddleNotice } from "@/features/huddle/components/WebHuddleNotice";

type WaveMessageAttachmentProps = {
  channelId?: string | null;
  fallbackText: string;
  huddleMemberPubkeys?: readonly string[];
  huddleMemberPubkeysPending?: boolean;
  searchQuery?: string;
};

export function WaveMessageAttachment({
  channelId,
  fallbackText,
  huddleMemberPubkeys = [],
  huddleMemberPubkeysPending = false,
  searchQuery,
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
        toast.error(formatHuddleActionError(error, "start"));
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

  const card = (
    <Attachment
      className="buzz-wave-hover-trigger mt-1 max-w-md"
      data-testid="message-wave-attachment"
      size="default"
    >
      <AttachmentMedia aria-hidden="true" className="text-lg">
        <span className="buzz-wave-hand">👋</span>
      </AttachmentMedia>
      <AttachmentContent>
        <AttachmentTitle>
          <HighlightedSearchText
            query={searchQuery ?? ""}
            text={fallbackText}
          />
        </AttachmentTitle>
        {huddlesAvailable() ? (
          <AttachmentDescription>
            Start a huddle to talk to them.
          </AttachmentDescription>
        ) : null}
      </AttachmentContent>
      {huddlesAvailable() ? (
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
      ) : null}
    </Attachment>
  );
  return huddlesAvailable() ? card : <WebHuddleNotice>{card}</WebHuddleNotice>;
}
