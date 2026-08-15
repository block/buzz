import { Loader2 } from "lucide-react";
import * as React from "react";

import { useChannelsQuery, useOpenDmMutation } from "@/features/channels/hooks";
import { buildForwardedContent } from "@/features/messages/lib/forwardMessageContent";
import type { TimelineMessage } from "@/features/messages/types";
import { useIdentityQuery } from "@/shared/api/hooks";
import { sendChannelMessage } from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import {
  formatRecipientName,
  useNewMessageRecipients,
} from "./useNewMessageRecipients";

/**
 * Forward one or more selected messages to channels and/or people (as a new
 * DM), bundled into ONE combined new message per destination.
 *
 * Everything here is built from mechanisms that are already active on the
 * hosted, unmodified relay — no new event kind, no relay-side change:
 *  - "Send to a channel" and "send into a DM" are the same primitive: a
 *    normal channel message tagged with the destination channel id
 *    (`sendChannelMessage`).
 *  - Opening/ensuring a DM with a person uses the existing `open_dm`
 *    command (`useOpenDmMutation`) — it's idempotent, so it's called
 *    unconditionally for every selected person rather than checking
 *    client-side whether a DM already exists.
 *  - Attachments are carried forward by copying the original messages'
 *    parsed `imeta` tags onto the new outgoing message (no re-upload) —
 *    see `buildForwardedContent`.
 */
export function ForwardMessageDialog({
  messages,
  onForwarded,
  onOpenChange,
  open,
}: {
  /** Source messages, in chronological order. One combined message is
   *  published per destination — never one message per source message. */
  messages: TimelineMessage[];
  onForwarded?: () => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;
  const openDmMutation = useOpenDmMutation();
  const channelsQuery = useChannelsQuery({ enabled: open });

  const [channelQuery, setChannelQuery] = React.useState("");
  const [selectedChannelIds, setSelectedChannelIds] = React.useState<
    Set<string>
  >(() => new Set());
  const [isSending, setIsSending] = React.useState(false);
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);

  const {
    searchResults,
    selectedUsers,
    selectUser,
    removeUser,
    reset: resetRecipients,
    setSearchQuery,
  } = useNewMessageRecipients({ active: open, currentPubkey });

  React.useEffect(() => {
    if (!open) {
      setChannelQuery("");
      setSelectedChannelIds(new Set());
      setErrorMessage(null);
      resetRecipients();
    }
  }, [open, resetRecipients]);

  const forwardableChannels = React.useMemo(
    () =>
      // Forum channels don't support message sends (mirrors the guard in
      // `useSendMessageMutation`); DMs are offered via the People picker
      // instead, resolved through `open_dm` per selected person.
      (channelsQuery.data ?? []).filter(
        (channel) =>
          channel.channelType !== "dm" && channel.channelType !== "forum",
      ),
    [channelsQuery.data],
  );
  const filteredChannels = React.useMemo(() => {
    const trimmed = channelQuery.trim().toLowerCase();
    if (trimmed.length === 0) return forwardableChannels;
    return forwardableChannels.filter((channel) =>
      channel.name.toLowerCase().includes(trimmed),
    );
  }, [channelQuery, forwardableChannels]);

  const toggleChannel = React.useCallback((channelId: string) => {
    setSelectedChannelIds((current) => {
      const next = new Set(current);
      if (next.has(channelId)) {
        next.delete(channelId);
      } else {
        next.add(channelId);
      }
      return next;
    });
  }, []);

  const destinationCount = selectedChannelIds.size + selectedUsers.length;
  const canSend = destinationCount > 0 && messages.length > 0 && !isSending;

  const handleSend = React.useCallback(async () => {
    if (!canSend) return;
    setIsSending(true);
    setErrorMessage(null);

    try {
      const { content, mediaTags } = buildForwardedContent(messages);
      const targetChannelIds = new Set<string>(selectedChannelIds);

      // `open_dm` is idempotent — publishing it for an existing DM just
      // returns that DM's canonical channel id — so every selected person
      // is resolved the same way regardless of whether a DM is already open.
      for (const user of selectedUsers) {
        const dmChannel = await openDmMutation.mutateAsync({
          pubkeys: [normalizePubkey(user.pubkey)],
        });
        targetChannelIds.add(dmChannel.id);
      }

      const failureMessages: string[] = [];
      await Promise.all(
        [...targetChannelIds].map(async (targetChannelId) => {
          try {
            await sendChannelMessage(targetChannelId, content, null, mediaTags);
          } catch (error) {
            failureMessages.push(
              error instanceof Error ? error.message : String(error),
            );
          }
        }),
      );

      if (failureMessages.length > 0) {
        const allFailed = failureMessages.length === targetChannelIds.size;
        setErrorMessage(
          allFailed
            ? "Failed to forward the message."
            : `Forwarded to some destinations, but ${failureMessages.length} failed.`,
        );
        if (!allFailed) {
          onForwarded?.();
          onOpenChange(false);
        }
        return;
      }

      onForwarded?.();
      onOpenChange(false);
    } catch (error) {
      setErrorMessage(
        error instanceof Error
          ? error.message
          : "Failed to forward the message.",
      );
    } finally {
      setIsSending(false);
    }
  }, [
    canSend,
    messages,
    onForwarded,
    onOpenChange,
    openDmMutation,
    selectedChannelIds,
    selectedUsers,
  ]);

  return (
    <Dialog
      onOpenChange={(next) => {
        if (!isSending) onOpenChange(next);
      }}
      open={open}
    >
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {messages.length <= 1
              ? "Forward message"
              : `Forward ${messages.length} messages`}
          </DialogTitle>
          <DialogDescription>
            Choose one or more channels or people. Everything selected is sent
            as a single combined message to each destination.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3">
          {selectedUsers.length > 0 ? (
            <div className="flex flex-wrap gap-1.5">
              {selectedUsers.map((user) => (
                <button
                  className="flex items-center gap-1.5 rounded-full border border-border/70 bg-muted/60 py-1 pl-1 pr-2 text-xs"
                  key={user.pubkey}
                  onClick={() => removeUser(user.pubkey)}
                  type="button"
                >
                  <UserAvatar
                    avatarUrl={user.avatarUrl}
                    className="h-5 w-5 text-2xs"
                    displayName={formatRecipientName(user)}
                  />
                  {formatRecipientName(user)}
                  <span aria-hidden="true">×</span>
                </button>
              ))}
            </div>
          ) : null}

          <Input
            data-testid="forward-message-search"
            onChange={(event) => {
              setChannelQuery(event.target.value);
              setSearchQuery(event.target.value);
            }}
            placeholder="Search channels or people"
            value={channelQuery}
          />

          <div className="max-h-72 overflow-y-auto rounded-lg border border-border/60">
            {filteredChannels.length > 0 ? (
              <div className="border-b border-border/40 py-1">
                <p className="px-3 pb-1 pt-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                  Channels
                </p>
                {filteredChannels.map((channel: Channel) => (
                  /* biome-ignore lint/a11y/noLabelWithoutControl: Radix Checkbox button inside label */
                  <label
                    className="flex cursor-pointer items-center gap-2 px-3 py-2 text-sm hover:bg-muted/60"
                    key={channel.id}
                  >
                    <Checkbox
                      checked={selectedChannelIds.has(channel.id)}
                      data-testid={`forward-channel-${channel.id}`}
                      onCheckedChange={() => toggleChannel(channel.id)}
                    />
                    <span className="truncate">#{channel.name}</span>
                  </label>
                ))}
              </div>
            ) : null}

            {searchResults.length > 0 ? (
              <div className="py-1">
                <p className="px-3 pb-1 pt-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                  People
                </p>
                {searchResults.map((user) => {
                  const isSelected = selectedUsers.some(
                    (selectedUser) => selectedUser.pubkey === user.pubkey,
                  );
                  return (
                    /* biome-ignore lint/a11y/noLabelWithoutControl: Radix Checkbox button inside label */
                    <label
                      className="flex cursor-pointer items-center gap-2 px-3 py-2 text-sm hover:bg-muted/60"
                      key={user.pubkey}
                    >
                      <Checkbox
                        checked={isSelected}
                        data-testid={`forward-user-${user.pubkey}`}
                        onCheckedChange={() => {
                          if (isSelected) {
                            removeUser(user.pubkey);
                          } else {
                            selectUser(user);
                          }
                        }}
                      />
                      <UserAvatar
                        avatarUrl={user.avatarUrl}
                        className="h-6 w-6 text-xs"
                        displayName={formatRecipientName(user)}
                      />
                      <span className="truncate">
                        {formatRecipientName(user)}
                      </span>
                    </label>
                  );
                })}
              </div>
            ) : null}

            {filteredChannels.length === 0 && searchResults.length === 0 ? (
              <p className="px-3 py-6 text-center text-sm text-muted-foreground">
                No matching channels or people.
              </p>
            ) : null}
          </div>

          {errorMessage ? (
            <p className="text-sm text-destructive">{errorMessage}</p>
          ) : null}
        </div>

        <DialogFooter>
          <Button
            disabled={isSending}
            onClick={() => onOpenChange(false)}
            type="button"
            variant="outline"
          >
            Cancel
          </Button>
          <Button
            data-testid="forward-message-send"
            disabled={!canSend}
            onClick={() => void handleSend()}
            type="button"
          >
            {isSending ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : destinationCount > 0 ? (
              `Forward to ${destinationCount}`
            ) : (
              "Forward"
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
