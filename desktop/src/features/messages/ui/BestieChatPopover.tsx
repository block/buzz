import { Loader2 } from "lucide-react";
import * as React from "react";
import { createPortal } from "react-dom";
import { toast } from "sonner";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { pickBestieAgent } from "@/features/agents/lib/bestie";
import { useOpenDmMutation } from "@/features/channels/hooks";
import { useChannelOpenReadState } from "@/features/channels/ui/useChannelOpenReadState";
import { useCommunities } from "@/features/communities/useCommunities";
import {
  useChannelMessagesQuery,
  useChannelSubscription,
  useSendMessageMutation,
} from "@/features/messages/hooks";
import { formatTimelineMessages } from "@/features/messages/lib/formatTimelineMessages";
import { getThreadReference } from "@/features/messages/lib/threading";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { MessageThreadTranscript } from "@/features/messages/ui/MessageThreadTranscript";
import { useProfileQuery } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel } from "@/shared/api/types";
import { getPlatformKeysById } from "@/shared/lib/keyboard-shortcuts";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverAnchor, PopoverContent } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

export function BestieChatPopover({ showTrigger }: { showTrigger: boolean }) {
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const profileQuery = useProfileQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const openDmMutation = useOpenDmMutation();
  const sendMessageMutation = useSendMessageMutation(null, identityQuery.data);
  const [open, setOpen] = React.useState(false);
  const [channel, setChannel] = React.useState<Channel | null>(null);
  const [openError, setOpenError] = React.useState<string | null>(null);
  const [portalTarget, setPortalTarget] = React.useState<HTMLElement | null>(
    null,
  );
  const scrollRef = React.useRef<HTMLDivElement>(null);
  const openRequestRef = React.useRef(0);
  const positionedScrollRef = React.useRef(false);
  const stickToBottomRef = React.useRef(true);

  const bestie = React.useMemo(
    () =>
      pickBestieAgent(managedAgentsQuery.data ?? [], activeCommunity?.relayUrl),
    [activeCommunity?.relayUrl, managedAgentsQuery.data],
  );
  const conversationScope = `${activeCommunity?.relayUrl ?? ""}:${bestie?.pubkey ?? ""}`;
  const conversationScopeRef = React.useRef(conversationScope);

  useChannelSubscription(channel);
  const messagesQuery = useChannelMessagesQuery(channel);
  const currentPubkey = identityQuery.data?.pubkey ?? null;
  const profiles = React.useMemo(() => {
    if (!bestie) return undefined;
    return {
      [normalizePubkey(bestie.pubkey)]: {
        avatarUrl: bestie.avatarUrl,
        displayName: bestie.name,
        isAgent: true,
        name: bestie.name,
        nip05Handle: null,
        ownerPubkey: null,
      },
      ...(currentPubkey
        ? {
            [normalizePubkey(currentPubkey)]: {
              avatarUrl: profileQuery.data?.avatarUrl ?? null,
              displayName: "You",
              isAgent: false,
              name: null,
              nip05Handle: null,
              ownerPubkey: null,
            },
          }
        : {}),
    };
  }, [bestie, currentPubkey, profileQuery.data?.avatarUrl]);
  const messages = React.useMemo(
    () =>
      channel
        ? formatTimelineMessages(
            messagesQuery.data ?? [],
            channel,
            currentPubkey ?? undefined,
            profileQuery.data?.avatarUrl ?? null,
            profiles,
          )
        : [],
    [
      channel,
      currentPubkey,
      messagesQuery.data,
      profileQuery.data?.avatarUrl,
      profiles,
    ],
  );
  const lastMessageId = messages.at(-1)?.id ?? null;
  const latestTopLevelMessage = React.useMemo(() => {
    const rawMessages = messagesQuery.data;
    if (!rawMessages) return null;
    for (let index = rawMessages.length - 1; index >= 0; index -= 1) {
      if (getThreadReference(rawMessages[index].tags).parentId === null) {
        return rawMessages[index];
      }
    }
    return null;
  }, [messagesQuery.data]);
  const activeReadAt = latestTopLevelMessage
    ? new Date(latestTopLevelMessage.created_at * 1_000).toISOString()
    : null;
  useChannelOpenReadState(
    open ? (channel?.id ?? null) : null,
    channel?.isMember,
    activeReadAt,
  );

  React.useLayoutEffect(() => {
    setPortalTarget(
      showTrigger ? document.getElementById("app-top-chrome-trailing") : null,
    );
  }, [showTrigger]);

  React.useEffect(() => {
    if (!lastMessageId) return;
    const scrollElement = scrollRef.current;
    if (!scrollElement) return;
    if (!positionedScrollRef.current || stickToBottomRef.current) {
      scrollElement.scrollTo({
        behavior: positionedScrollRef.current ? "smooth" : "auto",
        top: scrollElement.scrollHeight,
      });
    }
    positionedScrollRef.current = true;
  }, [lastMessageId]);

  React.useEffect(() => {
    if (conversationScopeRef.current === conversationScope) return;
    conversationScopeRef.current = conversationScope;
    openRequestRef.current += 1;
    setOpen(false);
    setChannel(null);
    setOpenError(null);
  }, [conversationScope]);

  const openConversation = React.useCallback(() => {
    if (!bestie) return;
    const requestId = ++openRequestRef.current;
    setChannel(null);
    setOpenError(null);
    void openDmMutation
      .mutateAsync({
        pubkeys: [bestie.pubkey],
        expectedRelayUrl: activeCommunity?.relayUrl,
        expectedSignerPubkey: currentPubkey ?? undefined,
      })
      .then((openedChannel) => {
        if (openRequestRef.current === requestId) setChannel(openedChannel);
      })
      .catch((error) => {
        if (openRequestRef.current !== requestId) return;
        console.error("Failed to open Bestie conversation", error);
        setOpenError(`Couldn't load your conversation with ${bestie.name}.`);
        toast.error(`Couldn't open ${bestie.name}`);
      });
  }, [activeCommunity?.relayUrl, bestie, currentPubkey, openDmMutation]);

  const handleOpenChange = React.useCallback(
    (nextOpen: boolean) => {
      if (!bestie) return;
      setOpen(nextOpen);
      positionedScrollRef.current = false;
      stickToBottomRef.current = true;
      if (nextOpen) {
        openConversation();
        return;
      }
      openRequestRef.current += 1;
      setChannel(null);
      setOpenError(null);
    },
    [bestie, openConversation],
  );

  const handleBestieShortcut = React.useEffectEvent((event: KeyboardEvent) => {
    if (
      !bestie ||
      !hasPrimaryShortcutModifier(event) ||
      event.altKey ||
      event.shiftKey ||
      event.repeat ||
      event.defaultPrevented ||
      event.code !== "Digit1"
    ) {
      return;
    }
    event.preventDefault();
    handleOpenChange(!open);
  });

  React.useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => handleBestieShortcut(event);
    window.addEventListener("keydown", onKeyDown, { capture: true });
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, []);

  if (!bestie) return null;

  const submit = async (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
  ) => {
    if (!channel) return;
    await sendMessageMutation.mutateAsync({
      content,
      mediaTags,
      mentionPubkeys,
      targetChannel: channel,
    });
  };
  const isLoading =
    !openError &&
    (openDmMutation.isPending || (channel && messagesQuery.isLoading));
  const isSending = sendMessageMutation.isPending;

  return (
    <Popover onOpenChange={handleOpenChange} open={open}>
      {portalTarget ? (
        createPortal(
          <PopoverAnchor asChild>
            <div className="flex items-center">
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    aria-expanded={open}
                    aria-label={`Open ${bestie.name} chat`}
                    className="h-[28px] w-[28px] rounded-[6px] p-0"
                    data-testid="open-bestie-panel"
                    onClick={() => handleOpenChange(!open)}
                    size="icon"
                    type="button"
                    variant="ghost"
                  >
                    <ProfileAvatar
                      avatarUrl={bestie.avatarUrl}
                      className="size-6 text-3xs"
                      label={bestie.name}
                      plain
                      testId="bestie-header-avatar"
                    />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  Message {bestie.name} ({getPlatformKeysById("open-bestie")})
                </TooltipContent>
              </Tooltip>
            </div>
          </PopoverAnchor>,
          portalTarget,
        )
      ) : (
        <PopoverAnchor asChild>
          <span className="pointer-events-none fixed right-3 top-10 size-px" />
        </PopoverAnchor>
      )}

      <PopoverContent
        align="end"
        className="h-[min(480px,calc(100vh-6rem))] w-[min(360px,calc(100vw-2rem))] overflow-hidden p-0"
        data-testid="bestie-chat-popover"
        side="bottom"
        sideOffset={8}
      >
        <div className="flex h-full min-h-0 flex-col">
          <div className="flex shrink-0 items-center gap-2.5 border-b border-border/60 px-3.5 py-3">
            <ProfileAvatar
              avatarUrl={bestie.avatarUrl}
              className="size-8 text-xs"
              label={bestie.name}
            />
            <p className="min-w-0 truncate text-sm font-semibold">
              {bestie.name}
            </p>
          </div>

          <div
            className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto py-3"
            data-testid="bestie-chat-scroll"
            onScroll={(event) => {
              const element = event.currentTarget;
              stickToBottomRef.current =
                element.scrollHeight -
                  element.scrollTop -
                  element.clientHeight <
                64;
            }}
            ref={scrollRef}
          >
            {openError ? (
              <div
                className="flex h-full flex-col items-center justify-center gap-3 px-8 text-center"
                role="alert"
              >
                <p className="text-xs text-muted-foreground">{openError}</p>
                <Button onClick={openConversation} size="sm" type="button">
                  Retry
                </Button>
              </div>
            ) : isLoading ? (
              <div className="flex h-full items-center justify-center text-muted-foreground">
                <Loader2 className="size-4 animate-spin" />
              </div>
            ) : messages.length > 0 && channel ? (
              <MessageThreadTranscript
                channelId={channel.id}
                currentPubkey={currentPubkey ?? undefined}
                messages={messages}
                profiles={profiles}
                testId="bestie-chat-transcript"
              />
            ) : (
              <div className="flex h-full items-center justify-center px-8 text-center">
                <p className="text-xs text-muted-foreground">
                  Your messages with {bestie.name} will show up here.
                </p>
              </div>
            )}
          </div>

          <MessageComposer
            channelId={channel?.id ?? null}
            channelName={bestie.name}
            channelType="dm"
            containerClassName="shrink-0 border-t border-border/60 px-3 pb-3 pt-2"
            disabled={!channel || isSending}
            draftKey={`bestie-panel:${channel?.id ?? "loading"}`}
            isSending={isSending}
            onSend={submit}
            placeholder={`Message ${bestie.name}`}
            profiles={profiles}
            showBackgroundUploadProgress={false}
          />
        </div>
      </PopoverContent>
    </Popover>
  );
}
