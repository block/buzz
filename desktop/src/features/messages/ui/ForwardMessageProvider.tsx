import * as React from "react";
import { toast } from "sonner";

import { useOpenDmMutation } from "@/features/channels/hooks";
import { resolveOneToOneDm } from "@/features/channels/lib/oneToOneDm";
import { useSendMessageMutation } from "@/features/messages/hooks";
import {
  buildForwardPayload,
  forwardPreview,
} from "@/features/messages/lib/forwardedMessage";
import type { TimelineMessage } from "@/features/messages/types";
import { useRelaySelfQuery } from "@/features/moderation/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel } from "@/shared/api/types";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { ForwardMessageDialog } from "./ForwardMessageDialog";

type ForwardRequest = {
  message: TimelineMessage;
  sourceChannelId: string;
};

type ForwardMessageContextValue = {
  /** Opens the destination picker; null when no provider is mounted. */
  requestForward: ((request: ForwardRequest) => void) | null;
};

const ForwardMessageContext = React.createContext<ForwardMessageContextValue>({
  requestForward: null,
});

function forwardedToLabel(channel: Channel): string {
  return channel.channelType === "dm"
    ? `Forwarded to ${channel.name}`
    : `Forwarded to #${channel.name}`;
}

/**
 * Owns the forward destination picker and the send it triggers.
 *
 * Forwarding is offered from the message action bar, which sits at the bottom
 * of a memoised, virtualised timeline. Routing the request through context
 * keeps the dialog and the mutation mounted once at the shell instead of
 * threading a callback (and a re-render) through every row.
 */
export function ForwardMessageProvider({
  children,
  hiddenChannelIds,
}: {
  children: React.ReactNode;
  /**
   * Channels the shell hides from the sidebar (unrevealed huddle backing
   * channels), which must not be offered as forward destinations either.
   */
  hiddenChannelIds?: ReadonlySet<string>;
}) {
  const { channels } = useChannelNavigation();
  const identityQuery = useIdentityQuery();
  const relaySelfQuery = useRelaySelfQuery();
  const openDm = useOpenDmMutation().mutateAsync;
  // Every forward names its destination explicitly, so there is no ambient
  // channel for this mutation to fall back to.
  const sendMessageMutation = useSendMessageMutation(null, identityQuery.data);
  const [request, setRequest] = React.useState<ForwardRequest | null>(null);

  const sendAsync = sendMessageMutation.mutateAsync;
  const forward = React.useCallback(
    (resolveTarget: () => Channel | Promise<Channel>) => {
      if (!request) return;
      setRequest(null);

      const payload = buildForwardPayload(
        request.message,
        request.sourceChannelId,
      );
      if (!payload) {
        toast.error("This message can't be forwarded.");
        return;
      }

      void (async () => {
        try {
          const target = await resolveTarget();
          await sendAsync({
            targetChannel: target,
            content: payload.content,
            mediaTags: payload.contentTags,
            forwardedFrom: payload.forwardedFrom,
          });
          toast.success(forwardedToLabel(target));
        } catch (error) {
          console.error("Failed to forward message", error);
          toast.error("Couldn't forward the message");
        }
      })();
    },
    [request, sendAsync],
  );

  const currentPubkey = identityQuery.data?.pubkey;
  const handleForwardPerson = React.useCallback(
    (person: { pubkey: string }) => {
      // Reuse the conversation that already exists: opening a second DM with
      // the same person would split the thread they read. `openDm` is only the
      // fallback, for someone the viewer has never messaged.
      forward(() =>
        resolveOneToOneDm({
          channels,
          currentPubkey,
          openDm,
          targetPubkey: person.pubkey,
        }),
      );
    },
    [channels, currentPubkey, forward, openDm],
  );

  const value = React.useMemo<ForwardMessageContextValue>(
    () => ({ requestForward: setRequest }),
    [],
  );

  return (
    <ForwardMessageContext.Provider value={value}>
      {children}
      <ForwardMessageDialog
        channels={channels}
        currentPubkey={currentPubkey}
        hiddenChannelIds={hiddenChannelIds}
        onForwardChannel={(channel) => forward(() => channel)}
        onForwardPerson={handleForwardPerson}
        onOpenChange={(open) => {
          if (!open) setRequest(null);
        }}
        open={request !== null}
        preview={forwardPreview(request?.message.body)}
        relaySelfPubkey={relaySelfQuery.data}
        sourceChannelId={request?.sourceChannelId}
      />
    </ForwardMessageContext.Provider>
  );
}

export function useForwardMessage() {
  return React.useContext(ForwardMessageContext);
}
