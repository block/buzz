import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useOpenDmMutation } from "@/features/channels/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import type { InboxItem } from "@/features/home/lib/inbox";
import { getThreadReference } from "@/features/messages/lib/threading";
import { getChannelMembers } from "@/shared/api/tauriChannels";
import { openHiddenDmInboxContext } from "./hiddenDmInboxAction";

type UseHiddenDmInboxNavigationOptions = {
  availableChannelIds: ReadonlySet<string>;
  currentPubkey: string | undefined;
  onOpenContext: (
    channelId: string,
    messageId: string,
    threadRootId?: string | null,
  ) => void;
  selectedItem: InboxItem | null;
};

export function useHiddenDmInboxNavigation({
  availableChannelIds,
  currentPubkey,
  onOpenContext,
  selectedItem,
}: UseHiddenDmInboxNavigationOptions) {
  const { goChannel } = useAppNavigation();
  const { activeCommunity } = useCommunities();
  const openDm = useOpenDmMutation().mutateAsync;
  const expectedRelayUrl = activeCommunity?.relayUrl ?? "";
  const expectedSignerPubkey = currentPubkey?.trim().toLowerCase() ?? "";
  const scopeKey =
    expectedRelayUrl && expectedSignerPubkey
      ? `${expectedRelayUrl}\u0000${expectedSignerPubkey}`
      : "";
  const pendingChannelIdsRef = React.useRef(new Set<string>());
  const errorChannelIdsRef = React.useRef(new Set<string>());
  const generationRef = React.useRef(0);
  const [, setPendingVersion] = React.useState(0);
  React.useEffect(() => {
    if (!scopeKey) return;
    generationRef.current += 1;
    pendingChannelIdsRef.current.clear();
    errorChannelIdsRef.current.clear();
    setPendingVersion((version) => version + 1);
    return () => {
      generationRef.current += 1;
      pendingChannelIdsRef.current.clear();
      errorChannelIdsRef.current.clear();
    };
  }, [scopeKey]);

  const openContext = React.useCallback(
    async (
      item: InboxItem,
      channelId: string,
      messageId: string,
      threadRootId?: string | null,
    ) => {
      const generation = generationRef.current;
      // A retry starts fresh: drop any prior error so the pending state shows
      // and the error/retry affordance clears until this attempt resolves.
      errorChannelIdsRef.current.delete(channelId);
      await openHiddenDmInboxContext({
        item,
        channelId,
        messageId,
        threadRootId,
        availableChannelIds,
        expectedRelayUrl,
        expectedSignerPubkey,
        pendingChannelIds: pendingChannelIdsRef.current,
        fetchMembers: getChannelMembers,
        openDm,
        isCurrent: () => generationRef.current === generation,
        onOpenContext,
        onError: () => {
          if (generationRef.current === generation) {
            errorChannelIdsRef.current.add(channelId);
          }
          toast.error("Could not reopen conversation. Try again.");
        },
        onPendingChange: () => setPendingVersion((version) => version + 1),
      });
    },
    [
      availableChannelIds,
      expectedRelayUrl,
      expectedSignerPubkey,
      onOpenContext,
      openDm,
    ],
  );

  const selectedChannelId = selectedItem?.item.channelId ?? null;

  return {
    canOpenSelected: Boolean(
      selectedChannelId &&
        !pendingChannelIdsRef.current.has(selectedChannelId) &&
        (availableChannelIds.has(selectedChannelId) ||
          (selectedItem?.item.channelType === "dm" &&
            expectedRelayUrl.length > 0 &&
            expectedSignerPubkey.length > 0)),
    ),
    isReopenPending: React.useCallback(
      (channelId: string | null | undefined) =>
        Boolean(channelId && pendingChannelIdsRef.current.has(channelId)),
      [],
    ),
    isReopenErrored: React.useCallback(
      (channelId: string | null | undefined) =>
        Boolean(channelId && errorChannelIdsRef.current.has(channelId)),
      [],
    ),
    handleOpenDirect: React.useCallback(
      (item: InboxItem) => {
        const channelId = item.item.channelId;
        if (!channelId) return;
        void openContext(
          item,
          channelId,
          item.id,
          getThreadReference(item.item.tags).rootId,
        );
      },
      [openContext],
    ),
    handleOpenDm: React.useCallback(
      async (pubkeys: string[]) => {
        const dm = await openDm({
          pubkeys,
          expectedRelayUrl,
          expectedSignerPubkey,
        });
        await goChannel(dm.id);
      },
      [expectedRelayUrl, expectedSignerPubkey, goChannel, openDm],
    ),
    handleOpenSelectedContext: React.useCallback(
      (channelId: string, messageId: string, threadRootId?: string | null) => {
        if (selectedItem) {
          void openContext(selectedItem, channelId, messageId, threadRootId);
        }
      },
      [openContext, selectedItem],
    ),
  };
}
