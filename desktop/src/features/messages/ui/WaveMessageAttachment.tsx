import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { channelsQueryKey } from "@/features/channels/hooks";
import { useHuddle } from "@/features/huddle";
import { normalizePubkey } from "@/shared/lib/pubkey";
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
  targetPubkey?: string;
  targetIsAgent?: boolean;
  agentPubkeys?: ReadonlySet<string>;
  huddleMemberPubkeys?: readonly string[];
  huddleMemberPubkeysPending?: boolean;
};

export function WaveMessageAttachment({
  channelId,
  fallbackText,
  targetPubkey,
  targetIsAgent = false,
  agentPubkeys,
  huddleMemberPubkeys = [],
  huddleMemberPubkeysPending = false,
}: WaveMessageAttachmentProps) {
  const queryClient = useQueryClient();
  const { isStarting, startHuddle } = useHuddle();
  const normalizedTargetPubkey = targetPubkey
    ? normalizePubkey(targetPubkey)
    : null;
  const targetAgentPubkey = React.useMemo(() => {
    if (!targetPubkey || !normalizedTargetPubkey) return null;
    if (targetIsAgent) return targetPubkey;
    if (!agentPubkeys) return null;
    return agentPubkeys.has(normalizedTargetPubkey) ? targetPubkey : null;
  }, [agentPubkeys, normalizedTargetPubkey, targetIsAgent, targetPubkey]);
  const resolvedHuddleMemberPubkeys = React.useMemo(() => {
    if (!targetAgentPubkey) return huddleMemberPubkeys;
    const seen = new Set<string>();
    return [...huddleMemberPubkeys, targetAgentPubkey].filter((pubkey) => {
      const normalizedPubkey = normalizePubkey(pubkey);
      if (seen.has(normalizedPubkey)) return false;
      seen.add(normalizedPubkey);
      return true;
    });
  }, [huddleMemberPubkeys, targetAgentPubkey]);
  const resolvedIncludesTarget =
    normalizedTargetPubkey !== null &&
    resolvedHuddleMemberPubkeys.some(
      (pubkey) => normalizePubkey(pubkey) === normalizedTargetPubkey,
    );
  const targetAgentLookupPending =
    normalizedTargetPubkey !== null &&
    huddleMemberPubkeysPending &&
    targetAgentPubkey === null &&
    !resolvedIncludesTarget;
  const legacyAgentLookupPending =
    normalizedTargetPubkey === null &&
    !targetIsAgent &&
    huddleMemberPubkeysPending &&
    resolvedHuddleMemberPubkeys.length === 0;
  const startHuddleDisabled =
    !channelId ||
    isStarting ||
    targetAgentLookupPending ||
    legacyAgentLookupPending;

  const handleStartHuddle = React.useCallback(
    async (event: React.MouseEvent<HTMLButtonElement>) => {
      event.preventDefault();
      event.stopPropagation();

      if (startHuddleDisabled) {
        return;
      }

      try {
        await startHuddle(channelId, [...resolvedHuddleMemberPubkeys]);
        await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to start huddle.",
        );
      }
    },
    [
      channelId,
      resolvedHuddleMemberPubkeys,
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
