import * as React from "react";
import { toast } from "sonner";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { pickBestieAgent } from "@/features/agents/lib/bestie";
import { useOpenDmMutation } from "@/features/channels/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { useSendMessageMutation } from "@/features/messages/hooks";
import { buildMessageLink } from "@/features/messages/lib/messageLink";
import { getThreadReference } from "@/features/messages/lib/threading";
import type { TimelineMessage } from "@/features/messages/types";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ManagedAgent } from "@/shared/api/types";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { VideoReviewCommentMarkdown } from "@/shared/ui/VideoReviewCommentMarkdown";

export function useBestieMessageAgent() {
  const { activeCommunity } = useCommunities();
  const managedAgentsQuery = useManagedAgentsQuery();

  return React.useMemo(
    () =>
      pickBestieAgent(managedAgentsQuery.data ?? [], activeCommunity?.relayUrl),
    [activeCommunity?.relayUrl, managedAgentsQuery.data],
  );
}

/** The Bestie destination rendered inside the persistent message-action shell. */
export function BestieMessagePanel({
  bestie,
  channelId,
  message,
  onClose,
}: {
  bestie: ManagedAgent;
  channelId: string;
  message: TimelineMessage;
  onClose: () => void;
}) {
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const openDmMutation = useOpenDmMutation();
  const sendMessageMutation = useSendMessageMutation(null, identityQuery.data);
  const isPending = openDmMutation.isPending || sendMessageMutation.isPending;

  const submit = async (
    note: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
  ) => {
    // The action can outlive its rendered community while opening the DM and
    // preparing optimistic state. Capture both ownership dimensions before
    // the first await and require the backend to reject publication if either
    // one changes in flight.
    const expectedRelayUrl = activeCommunity?.relayUrl;
    const expectedSignerPubkey = identityQuery.data?.pubkey;
    if (!expectedRelayUrl || !expectedSignerPubkey) {
      const error = new Error(
        "Bestie messages require an active community and signing identity.",
      );
      toast.error(`Couldn't send to ${bestie.name}`);
      throw error;
    }
    const { rootId } = getThreadReference(message.tags ?? []);
    const link = buildMessageLink({
      channelId,
      messageId: message.id,
      threadRootId: rootId,
    });
    const trimmedNote = note.trim();
    const content = [trimmedNote, `[Open original message](${link})`]
      .filter(Boolean)
      .join("\n\n");

    try {
      const dm = await openDmMutation.mutateAsync({
        pubkeys: [bestie.pubkey],
        expectedRelayUrl,
        expectedSignerPubkey,
      });
      await sendMessageMutation.mutateAsync({
        content,
        expectedRelayUrl,
        expectedSignerPubkey,
        mediaTags,
        mentionPubkeys,
        targetChannel: dm,
        transport: "http",
      });
      onClose();
      toast.success(`Sent to ${bestie.name}`);
    } catch (error) {
      console.error("Failed to send message to Bestie", error);
      toast.error(`Couldn't send to ${bestie.name}`);
      throw error;
    }
  };

  return (
    <div
      className="w-[min(328px,calc(100vw-2rem))] p-4"
      data-testid={`bestie-popover-${message.id}`}
    >
      <div className="space-y-3.5">
        <div className="flex items-center gap-2.5">
          <ProfileAvatar
            avatarUrl={bestie.avatarUrl}
            className="size-8 text-xs"
            label={bestie.name}
          />
          <p className="min-w-0 truncate text-sm font-semibold">Bestie</p>
        </div>

        <div
          className="w-3/4 overflow-hidden rounded-xl border border-border/60 bg-background p-2.5 shadow-sm"
          data-testid={`bestie-message-snapshot-${message.id}`}
        >
          <div className="flex min-w-0 items-start gap-2">
            <UserAvatar
              avatarUrl={message.avatarUrl ?? null}
              className="size-6 shrink-0"
              displayName={message.author}
              size="sm"
            />
            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 items-baseline gap-1">
                <p className="truncate text-2xs font-semibold leading-3.5">
                  {message.author}
                </p>
                <p className="shrink-0 text-3xs leading-3.5 text-muted-foreground/70">
                  {message.time}
                </p>
              </div>
              <div
                className="mt-0.5 max-h-3.5 overflow-hidden"
                data-testid={`bestie-message-snapshot-body-${message.id}`}
              >
                <VideoReviewCommentMarkdown
                  className="line-clamp-1 text-2xs leading-3.5 text-foreground/80 [&_p]:leading-3.5"
                  content={message.body}
                  interactive={false}
                  messageId={message.id}
                />
              </div>
            </div>
          </div>
        </div>

        <MessageComposer
          allowEmptySend
          channelName="Bestie"
          channelType="dm"
          containerClassName="px-0 pb-0"
          disabled={isPending}
          draftKey={`bestie-share:${message.id}`}
          isSending={isPending}
          onSend={submit}
          placeholder="Add a note (optional)"
          showBackgroundUploadProgress={false}
        />
      </div>
    </div>
  );
}
