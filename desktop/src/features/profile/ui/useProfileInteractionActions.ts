import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  channelsQueryKey,
  useChannelsQuery,
  useOpenDmMutation,
} from "@/features/channels/hooks";
import { findCachedOneToOneDm } from "@/features/channels/lib/oneToOneDm";
import { useHuddle } from "@/features/huddle";
import { formatHuddleActionError } from "@/features/huddle/lib/huddleError";
import {
  createOptimisticMessage,
  mergeTimelineCacheMessages,
} from "@/features/messages/hooks";
import { channelMessagesKey } from "@/features/messages/lib/messageQueryKeys";
import { buildWaveMessageContent } from "@/features/messages/lib/waveMessage";
import { useProfileQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { sendChannelMessage } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_STREAM_MESSAGE } from "@/shared/constants/kinds";
import { truncateNpub } from "@/shared/lib/pubkey";

export type ProfileInteractionAction = "huddle" | "message" | "wave";

type ProfileInteractionAvailability = {
  huddle: boolean;
  message: boolean;
  wave: boolean;
};

export function useProfileInteractionActions({
  availability,
  effectivePubkey,
  enabled,
  isBot,
  isSelf,
  onBeforeAction,
  onClose,
  viewerIsOwner,
}: {
  availability?: ProfileInteractionAvailability;
  effectivePubkey: string | null;
  enabled: boolean;
  isBot: boolean;
  isSelf: boolean;
  onBeforeAction?: () => void;
  onClose: () => void;
  viewerIsOwner: boolean | undefined;
}) {
  const [pendingAction, setPendingAction] =
    React.useState<ProfileInteractionAction | null>(null);
  const isMountedRef = React.useRef(false);
  const queryClient = useQueryClient();
  const { goChannel } = useAppNavigation();
  const openDmMutation = useOpenDmMutation();
  const openDm = openDmMutation.mutateAsync;
  const channelsQuery = useChannelsQuery();
  const identityQuery = useIdentityQuery();
  const { isStarting: isStartingHuddle, startHuddle } = useHuddle();

  const canInteract = enabled && !isSelf && effectivePubkey !== null;
  const canWave = canInteract && (availability?.wave ?? !isBot);
  const canMessage =
    canInteract &&
    (availability?.message ?? (!isBot || viewerIsOwner === true));
  const canHuddle = canInteract && (availability?.huddle ?? canMessage);
  const selfProfileQuery = useProfileQuery(enabled && canWave);

  React.useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
    };
  }, []);

  const runAction = React.useCallback(
    async (
      action: ProfileInteractionAction,
      operation: (targetPubkey: string) => Promise<void>,
    ) => {
      if (!effectivePubkey || pendingAction !== null) {
        return;
      }

      onBeforeAction?.();
      setPendingAction(action);
      try {
        await operation(effectivePubkey);
      } catch (error) {
        if (action === "huddle") {
          toast.error(formatHuddleActionError(error, "start"));
        } else {
          toast.error(
            error instanceof Error
              ? error.message
              : action === "wave"
                ? "Couldn't send the wave."
                : "Couldn't open the direct message.",
          );
        }
      } finally {
        if (isMountedRef.current) {
          setPendingAction(null);
        }
      }
    },
    [effectivePubkey, onBeforeAction, pendingAction],
  );

  const handleMessage = React.useCallback(() => {
    if (!canMessage) {
      return;
    }

    void runAction("message", async (targetPubkey) => {
      const dm = await openDm({ pubkeys: [targetPubkey] });
      await goChannel(dm.id);
      if (isMountedRef.current) {
        onClose();
      }
    });
  }, [canMessage, goChannel, onClose, openDm, runAction]);

  const handleHuddle = React.useCallback(() => {
    if (!canHuddle || isStartingHuddle) {
      return;
    }

    void runAction("huddle", async (targetPubkey) => {
      const dm = await openDm({ pubkeys: [targetPubkey] });
      await goChannel(dm.id);
      await startHuddle(dm.id, isBot ? [targetPubkey] : []);
      await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
      if (isMountedRef.current) {
        onClose();
      }
    });
  }, [
    canHuddle,
    goChannel,
    isBot,
    isStartingHuddle,
    onClose,
    openDm,
    queryClient,
    runAction,
    startHuddle,
  ]);

  const handleWave = React.useCallback(() => {
    if (!canWave) {
      return;
    }

    void runAction("wave", async (targetPubkey) => {
      const identity = identityQuery.data;
      if (!identity) {
        throw new Error("No identity is available to send messages.");
      }

      const dm =
        findCachedOneToOneDm(
          channelsQuery.data,
          targetPubkey,
          identity.pubkey,
        ) ?? (await openDm({ pubkeys: [targetPubkey] }));
      const senderName =
        selfProfileQuery.data?.displayName?.trim() ||
        identity.displayName.trim() ||
        truncateNpub(identity.pubkey);
      const content = buildWaveMessageContent(senderName);
      const queryKey = channelMessagesKey(dm.id);

      await queryClient.cancelQueries({ queryKey });
      const previousMessages =
        queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
      const optimisticMessage = createOptimisticMessage(
        dm.id,
        content,
        identity,
        previousMessages,
      );

      queryClient.setQueryData<RelayEvent[]>(
        queryKey,
        mergeTimelineCacheMessages(previousMessages, optimisticMessage),
      );

      try {
        await goChannel(dm.id);
        if (isMountedRef.current) {
          onClose();
        }

        const result = await sendChannelMessage(dm.id, content);
        queryClient.setQueryData<RelayEvent[]>(queryKey, (current = []) =>
          mergeTimelineCacheMessages(current, {
            id: result.eventId,
            localKey: optimisticMessage.id,
            pubkey: identity.pubkey,
            created_at: result.createdAt,
            kind: KIND_STREAM_MESSAGE,
            tags: [
              ["h", dm.id],
              ["p", identity.pubkey],
            ],
            content: content.trim(),
            sig: "",
          }),
        );
      } catch (error) {
        queryClient.setQueryData<RelayEvent[]>(queryKey, (current = []) =>
          current.filter(
            (message) =>
              message.id !== optimisticMessage.id &&
              message.localKey !== optimisticMessage.localKey,
          ),
        );
        throw error;
      }
    });
  }, [
    canWave,
    channelsQuery.data,
    goChannel,
    identityQuery.data,
    onClose,
    openDm,
    queryClient,
    runAction,
    selfProfileQuery.data?.displayName,
  ]);

  return {
    canHuddle,
    canMessage,
    canWave,
    handleHuddle,
    handleMessage,
    handleWave,
    isOpeningDm: openDmMutation.isPending,
    isStartingHuddle,
    pendingAction,
  };
}
