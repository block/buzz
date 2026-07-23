import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Activity, Headphones, MessageSquare } from "lucide-react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useHuddle } from "@/features/huddle";
import {
  channelsQueryKey,
  useChannelsQuery,
  useOpenDmMutation,
} from "@/features/channels/hooks";
import {
  useProfileQuery,
  useUserProfileQuery,
  useUsersBatchQuery,
} from "@/features/profile/hooks";
import { channelMessagesKey } from "@/features/messages/lib/messageQueryKeys";
import {
  useRelayAgentsQuery,
  useManagedAgentsQuery,
} from "@/features/agents/hooks";
import { useIsManagedAgent } from "@/features/agent-memory/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useAgentWorking } from "@/features/agents/agentWorkingSignal";
import {
  formatOwnerLabel,
  ownsAuthorAgent,
} from "@/features/profile/lib/identity";
import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import { usePresenceQuery } from "@/features/presence/hooks";
import { useUserStatusQuery } from "@/features/user-status/hooks";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import { ProfileAvatarWithStatus } from "@/features/profile/ui/ProfileAvatarWithStatus";
import {
  createOptimisticMessage,
  mergeTimelineCacheMessages,
} from "@/features/messages/hooks";
import { buildWaveMessageContent } from "@/features/messages/lib/waveMessage";
import { useOpenAgentActivity } from "@/features/agents/useOpenAgentActivity";
import { useProfilePanel } from "@/shared/context/ProfilePanelContext";
import { sendChannelMessage } from "@/shared/api/tauri";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { KIND_STREAM_MESSAGE } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";

import { Popover, PopoverAnchor, PopoverContent } from "@/shared/ui/popover";
import { BotIdenticon } from "@/features/messages/ui/BotIdenticon";
import { useNow } from "@/shared/lib/useNow";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";

type UserProfilePopoverProps = {
  children: React.ReactNode;
  pubkey: string;
  triggerElement?: "div" | "span";
  /** Accessible name for interactive trigger content that is visually hidden. */
  triggerAriaLabel?: string;
  /** Set false when the trigger is inside another interactive control. */
  enableProfilePanel?: boolean;
  /** When set to "bot", a BotIdenticon badge renders next to the display name. */
  role?: string;
  /** Value used to generate the BotIdenticon glyph (typically the author name). */
  botIdenticonValue?: string;
};

const HOVER_OPEN_DELAY_MS = 500;
const HOVER_CLOSE_DELAY_MS = 200;

const RUNTIME_LABELS: Record<string, string> = {
  goose: "Goose",
  "claude-code": "Claude Code",
  "codex-acp": "Codex",
  aider: "Aider",
};

function runtimeLabel(command: string): string {
  return RUNTIME_LABELS[command] ?? command;
}

function InfoBadge({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex items-center rounded-full bg-muted/50 px-2 py-0.5 text-xs text-muted-foreground">
      {children}
    </span>
  );
}

function findCachedOneToOneDm(
  channels: Channel[] | undefined,
  targetPubkey: string,
  currentPubkey: string | undefined,
) {
  const normalizedTargetPubkey = normalizePubkey(targetPubkey);
  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;

  return (
    channels?.find((channel) => {
      if (channel.channelType !== "dm") {
        return false;
      }

      const participantPubkeys =
        channel.participantPubkeys.map(normalizePubkey);
      if (!participantPubkeys.includes(normalizedTargetPubkey)) {
        return false;
      }

      const otherParticipantPubkeys = normalizedCurrentPubkey
        ? participantPubkeys.filter(
            (participantPubkey) =>
              participantPubkey !== normalizedCurrentPubkey,
          )
        : participantPubkeys;

      return (
        otherParticipantPubkeys.length === 1 &&
        otherParticipantPubkeys[0] === normalizedTargetPubkey
      );
    }) ?? null
  );
}

const TEXT_SWAP_BASE_CLASS =
  "col-start-1 row-start-1 min-w-0 truncate transition-[opacity,filter] duration-[250ms] ease-in-out motion-reduce:transition-none";
const TEXT_SWAP_VISIBLE_CLASS = "opacity-100 blur-0";
const TEXT_SWAP_HIDDEN_CLASS = "opacity-0 blur-0";
const TEXT_SWAP_HOVER_VISIBLE_CLASS =
  "group-hover/name:opacity-100 group-hover/name:blur-0";
const TEXT_SWAP_HOVER_HIDDEN_CLASS =
  "group-hover/name:opacity-0 group-hover/name:blur-[2px]";

function HoverPubkeyName({
  displayName,
  pubkey,
}: {
  displayName: string;
  pubkey: string;
}) {
  return (
    <span className="group/name inline-grid h-5 min-w-0 flex-1 overflow-hidden text-sm font-semibold leading-5">
      <span
        className={`${TEXT_SWAP_BASE_CLASS} ${TEXT_SWAP_VISIBLE_CLASS} ${TEXT_SWAP_HOVER_HIDDEN_CLASS}`}
      >
        {displayName}
      </span>
      <span
        className={`${TEXT_SWAP_BASE_CLASS} ${TEXT_SWAP_HIDDEN_CLASS} ${TEXT_SWAP_HOVER_VISIBLE_CLASS}`}
      >
        {truncatePubkey(pubkey)}
      </span>
    </span>
  );
}

function StatusLine({ children }: { children: React.ReactNode }) {
  return (
    <div
      className="flex w-full min-w-0 items-center gap-1 py-1 text-xs leading-4 text-muted-foreground"
      data-testid="user-profile-status"
    >
      {children}
    </div>
  );
}

export function UserProfilePopover({
  children,
  pubkey,
  triggerElement = "div",
  triggerAriaLabel,
  enableProfilePanel = true,
  role,
  botIdenticonValue,
}: UserProfilePopoverProps) {
  const [open, setOpen] = React.useState(false);
  const [pendingAction, setPendingAction] = React.useState<
    "message" | "huddle" | "wave" | null
  >(null);
  const isMountedRef = React.useRef(false);
  const hoverTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const queryClient = useQueryClient();
  const { goChannel } = useAppNavigation();
  const openDmMutation = useOpenDmMutation();
  const { isStarting: isStartingHuddle, startHuddle } = useHuddle();
  const profileQuery = useUserProfileQuery(open ? pubkey : undefined);
  const usersBatchQuery = useUsersBatchQuery(open ? [pubkey] : [], {
    enabled: open,
  });
  const relayAgentsQuery = useRelayAgentsQuery({
    enabled: open,
  });
  const managedAgentsQuery = useManagedAgentsQuery({
    enabled: open,
  });
  const presenceQuery = usePresenceQuery(open ? [pubkey] : [], {
    enabled: open,
  });
  const userStatusQuery = useUserStatusQuery(open ? [pubkey] : []);

  const { canOpenAgentActivity, openAgentActivity } = useOpenAgentActivity();
  const { openProfilePanel } = useProfilePanel();
  const canOpenProfilePanel = enableProfilePanel && Boolean(openProfilePanel);
  const relayAgent = relayAgentsQuery.data?.find((a) => a.pubkey === pubkey);
  const managedAgent = managedAgentsQuery.data?.find(
    (a) => a.pubkey === pubkey,
  );
  const profile = profileQuery.data;
  const ownerPubkey = profile?.ownerPubkey ?? null;
  const ownerProfileQuery = useUsersBatchQuery(
    ownerPubkey ? [ownerPubkey] : [],
    { enabled: open && Boolean(ownerPubkey) },
  );
  const normalizedPubkey = normalizePubkey(pubkey);
  const isAgentByOaOwner = Boolean(
    usersBatchQuery.data?.profiles[normalizedPubkey]?.isAgent,
  );
  const isAgentByProfileOwner = profile?.ownerPubkey != null;
  const isBotProfile =
    role === "bot" ||
    Boolean(relayAgent || managedAgent) ||
    isAgentByProfileOwner ||
    isAgentByOaOwner;
  const isAgentClassificationPending =
    open &&
    role !== "bot" &&
    (profileQuery.isPending ||
      relayAgentsQuery.isPending ||
      managedAgentsQuery.isPending ||
      usersBatchQuery.isPending);
  const displayName = profile?.displayName ?? truncatePubkey(pubkey);
  // Owner signal mirrors UserProfilePanel: a declared NIP-OA owner whose agent
  // runs elsewhere holds no local seckey, so key custody (`isOwner`) alone
  // wrongly hides the affordance from them — and gating on bot-ness alone shows
  // it to every viewer. Combine declared ownership with local management, same
  // shape as the pane/sidebar/memory fixes. Every real boundary is server-side;
  // this only decides whether to paint the "View activity log" button.
  const isOwner = useIsManagedAgent(isBotProfile ? pubkey : null);
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;
  const ownerLabel = isBotProfile
    ? formatOwnerLabel(
        ownerPubkey,
        currentPubkey,
        ownerProfileQuery.data?.profiles,
      )
    : null;
  const isSelf =
    currentPubkey !== undefined &&
    currentPubkey.toLowerCase() === pubkey.toLowerCase();
  const showProfileActions = currentPubkey !== undefined && !isSelf;
  const showHumanProfileActions =
    showProfileActions && !isBotProfile && !isAgentClassificationPending;
  const selfProfileQuery = useProfileQuery(open && showProfileActions);
  const isCurrentUserOwner = ownsAuthorAgent(profile, currentPubkey);
  const viewerIsOwner = isCurrentUserOwner || isOwner === true;
  const showMessageAction =
    showProfileActions &&
    !isAgentClassificationPending &&
    (!isBotProfile || viewerIsOwner);
  const showAnyProfileActions = showHumanProfileActions || showMessageAction;
  const canViewActivity =
    isBotProfile && viewerIsOwner && canOpenAgentActivity(pubkey);
  const presenceStatus = presenceQuery.data?.[pubkey.toLowerCase()];
  const userStatus = userStatusQuery.data?.[pubkey.toLowerCase()];
  const userStatusText = userStatus?.text.trim() ?? "";
  const hasUserStatus = Boolean(userStatusText || userStatus?.emoji);
  const profileDescription = profile?.about?.trim() ?? "";
  const profileSubheader = profileDescription || profile?.nip05Handle?.trim();
  const activeTurns = useAgentWorking(isBotProfile ? pubkey : null).channels;
  const channelsQuery = useChannelsQuery();
  const channelIdToName = React.useMemo(() => {
    const map: Record<string, string> = {};
    for (const channel of channelsQuery.data ?? []) {
      map[channel.id] = channel.name;
    }
    return map;
  }, [channelsQuery.data]);

  const clearHoverTimer = React.useCallback(() => {
    if (hoverTimerRef.current !== null) {
      clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
  }, []);

  const handleTriggerMouseEnter = React.useCallback(() => {
    clearHoverTimer();
    hoverTimerRef.current = setTimeout(() => {
      setOpen(true);
    }, HOVER_OPEN_DELAY_MS);
  }, [clearHoverTimer]);

  const handleMouseLeave = React.useCallback(() => {
    clearHoverTimer();
    hoverTimerRef.current = setTimeout(() => {
      setOpen(false);
    }, HOVER_CLOSE_DELAY_MS);
  }, [clearHoverTimer]);

  const handleContentMouseEnter = React.useCallback(() => {
    clearHoverTimer();
  }, [clearHoverTimer]);

  const handleTriggerClick = React.useCallback(
    (event: React.MouseEvent) => {
      clearHoverTimer();
      if (canOpenProfilePanel && openProfilePanel) {
        event.preventDefault();
        event.stopPropagation();
        setOpen(false);
        openProfilePanel(pubkey);
      }
    },
    [canOpenProfilePanel, clearHoverTimer, openProfilePanel, pubkey],
  );

  const handleMessage = React.useCallback(async () => {
    if (!showMessageAction || pendingAction !== null) return;

    clearHoverTimer();
    setPendingAction("message");

    try {
      const dm = await openDmMutation.mutateAsync({ pubkeys: [pubkey] });
      await goChannel(dm.id);
      if (isMountedRef.current) {
        setOpen(false);
      }
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to open direct message.",
      );
    } finally {
      if (isMountedRef.current) {
        setPendingAction(null);
      }
    }
  }, [
    clearHoverTimer,
    goChannel,
    openDmMutation,
    pendingAction,
    pubkey,
    showMessageAction,
  ]);

  const handleHuddle = React.useCallback(async () => {
    if (
      !showProfileActions ||
      !showHumanProfileActions ||
      pendingAction !== null ||
      isStartingHuddle
    ) {
      return;
    }

    clearHoverTimer();
    setPendingAction("huddle");

    try {
      const dm = await openDmMutation.mutateAsync({ pubkeys: [pubkey] });
      await goChannel(dm.id);
      await startHuddle(dm.id, []);
      await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
      if (isMountedRef.current) {
        setOpen(false);
      }
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to start huddle.",
      );
    } finally {
      if (isMountedRef.current) {
        setPendingAction(null);
      }
    }
  }, [
    clearHoverTimer,
    goChannel,
    isStartingHuddle,
    openDmMutation,
    pendingAction,
    pubkey,
    queryClient,
    showHumanProfileActions,
    showProfileActions,
    startHuddle,
  ]);

  const handleWave = React.useCallback(async () => {
    if (
      !showProfileActions ||
      !showHumanProfileActions ||
      pendingAction !== null
    ) {
      return;
    }

    clearHoverTimer();
    setPendingAction("wave");

    try {
      const identity = identityQuery.data;
      if (!identity) {
        throw new Error("No identity available for sending messages.");
      }

      const dm =
        findCachedOneToOneDm(channelsQuery.data, pubkey, currentPubkey) ??
        (await openDmMutation.mutateAsync({ pubkeys: [pubkey] }));
      const senderName =
        selfProfileQuery.data?.displayName?.trim() ||
        identity.displayName.trim() ||
        truncatePubkey(identity.pubkey);
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
          setOpen(false);
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
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to send wave.",
      );
    } finally {
      if (isMountedRef.current) {
        setPendingAction(null);
      }
    }
  }, [
    channelsQuery.data,
    clearHoverTimer,
    currentPubkey,
    goChannel,
    identityQuery.data,
    openDmMutation,
    pendingAction,
    pubkey,
    queryClient,
    selfProfileQuery.data?.displayName,
    showHumanProfileActions,
    showProfileActions,
  ]);

  React.useEffect(() => {
    isMountedRef.current = true;

    return () => {
      isMountedRef.current = false;
      clearHoverTimer();
    };
  }, [clearHoverTimer]);

  const TriggerElement = triggerElement;
  const profileHeaderContent = (
    <>
      <ProfileAvatarWithStatus
        avatarClassName="text-xs"
        avatarUrl={profile?.avatarUrl ?? null}
        className="h-10 w-10"
        iconClassName="h-5 w-5"
        label={displayName}
        size={40}
        status={presenceStatus ?? "offline"}
        statusTestId="user-profile-popover-presence-badge"
        testId="user-profile-popover-avatar"
      />

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <HoverPubkeyName displayName={displayName} pubkey={pubkey} />
          {isBotProfile && botIdenticonValue ? (
            <BotIdenticon
              value={botIdenticonValue}
              size={20}
              className="shrink-0 rounded"
            />
          ) : null}
        </div>
        {isBotProfile && ownerLabel ? (
          <p
            className="mt-0.5 truncate text-xs leading-4 text-muted-foreground"
            data-testid={`user-profile-popover-owner-${pubkey}`}
          >
            managed by {ownerLabel}
          </p>
        ) : null}
        {profileSubheader ? (
          <p
            className="mt-0.5 truncate text-xs leading-4 text-muted-foreground"
            data-testid="user-profile-description"
          >
            {profileSubheader}
          </p>
        ) : null}
      </div>
    </>
  );

  return (
    <Popover onOpenChange={setOpen} open={open}>
      <PopoverAnchor asChild>
        <TriggerElement
          aria-label={triggerAriaLabel}
          role={canOpenProfilePanel ? "button" : undefined}
          tabIndex={canOpenProfilePanel ? 0 : undefined}
          onClick={handleTriggerClick}
          onKeyDown={(e) => {
            if (
              (e.key === "Enter" || e.key === " ") &&
              canOpenProfilePanel &&
              openProfilePanel
            ) {
              e.preventDefault();
              e.stopPropagation();
              clearHoverTimer();
              setOpen(false);
              openProfilePanel(pubkey);
            }
          }}
          onMouseEnter={handleTriggerMouseEnter}
          onMouseLeave={handleMouseLeave}
          className={cn(
            "inline-flex",
            canOpenProfilePanel && "cursor-pointer [&_*]:cursor-pointer",
          )}
        >
          {children}
        </TriggerElement>
      </PopoverAnchor>
      <PopoverContent
        align="start"
        className="w-80"
        data-testid="user-profile-popover"
        onMouseEnter={handleContentMouseEnter}
        onMouseLeave={handleMouseLeave}
        side="top"
        sideOffset={8}
      >
        <div className="flex flex-col gap-3">
          {canOpenProfilePanel ? (
            <button
              className="flex w-full min-w-0 cursor-pointer items-center gap-3 rounded-lg text-left text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring [&_*]:cursor-pointer"
              onClick={handleTriggerClick}
              type="button"
            >
              {profileHeaderContent}
            </button>
          ) : (
            <div className="flex w-full min-w-0 items-center gap-3 text-left text-foreground">
              {profileHeaderContent}
            </div>
          )}

          {isBotProfile && (managedAgent || relayAgent) ? (
            <div className="flex flex-wrap gap-1.5">
              {managedAgent?.agentCommand ? (
                <InfoBadge>{runtimeLabel(managedAgent.agentCommand)}</InfoBadge>
              ) : relayAgent?.agentType ? (
                <InfoBadge>{runtimeLabel(relayAgent.agentType)}</InfoBadge>
              ) : null}
              {managedAgent?.model ? (
                <InfoBadge>{managedAgent.model}</InfoBadge>
              ) : null}
              {managedAgent?.acpCommand ? (
                <InfoBadge>ACP: {managedAgent.acpCommand}</InfoBadge>
              ) : null}
            </div>
          ) : null}

          {activeTurns.length > 0 ? (
            <div className="flex flex-wrap gap-1.5">
              {activeTurns.map(({ channelId, anchorAt }) => (
                <PopoverWorkingBadge
                  key={channelId}
                  name={channelIdToName[channelId] ?? channelId}
                  anchorAt={anchorAt}
                />
              ))}
            </div>
          ) : null}

          {canViewActivity ? (
            <button
              className="flex w-full items-center gap-2 rounded-lg border border-border/60 px-3 py-2 text-left text-xs font-medium text-foreground transition-colors hover:bg-muted/50"
              data-testid={`user-profile-view-activity-${pubkey}`}
              onClick={() => {
                setOpen(false);
                openAgentActivity(pubkey);
              }}
              type="button"
            >
              <Activity className="h-4 w-4 text-muted-foreground" />
              View activity log
            </button>
          ) : null}

          {hasUserStatus || showAnyProfileActions ? (
            <>
              <div
                aria-hidden="true"
                className="my-1 border-t border-border/60"
              />
              {hasUserStatus ? (
                <StatusLine>
                  {userStatus?.emoji ? (
                    <StatusEmoji
                      className="h-3.5 w-3.5 shrink-0"
                      value={userStatus.emoji}
                    />
                  ) : null}
                  {userStatusText ? (
                    <span className="truncate">{userStatusText}</span>
                  ) : null}
                </StatusLine>
              ) : null}
              {showAnyProfileActions ? (
                <div className="flex gap-2">
                  {showHumanProfileActions ? (
                    <Button
                      aria-label="Wave"
                      className="buzz-wave-hover-trigger shrink-0 px-3 transition-transform duration-100 ease-out motion-reduce:transition-none motion-safe:active:scale-[0.97]"
                      data-testid={`user-profile-popover-wave-${pubkey}`}
                      disabled={
                        pendingAction !== null || openDmMutation.isPending
                      }
                      onClick={() => {
                        void handleWave();
                      }}
                      size="sm"
                      type="button"
                      variant="outline"
                    >
                      {pendingAction === "wave" ? (
                        <Spinner
                          aria-hidden="true"
                          className="h-3.5 w-3.5 border-2"
                        />
                      ) : (
                        <span
                          aria-hidden="true"
                          className="buzz-wave-hand text-sm leading-none"
                        >
                          👋
                        </span>
                      )}
                    </Button>
                  ) : null}
                  {showMessageAction ? (
                    <Button
                      className="min-w-0 flex-1"
                      data-testid={`user-profile-popover-message-${pubkey}`}
                      disabled={
                        pendingAction !== null || openDmMutation.isPending
                      }
                      onClick={() => {
                        void handleMessage();
                      }}
                      size="sm"
                      type="button"
                      variant="outline"
                    >
                      {pendingAction === "message" ? (
                        <Spinner
                          aria-hidden="true"
                          className="h-3.5 w-3.5 border-2"
                        />
                      ) : (
                        <MessageSquare />
                      )}
                      Message
                    </Button>
                  ) : null}
                  {showHumanProfileActions ? (
                    <Button
                      className="min-w-0 flex-1"
                      data-testid={`user-profile-popover-huddle-${pubkey}`}
                      disabled={
                        pendingAction !== null ||
                        openDmMutation.isPending ||
                        isStartingHuddle
                      }
                      onClick={() => {
                        void handleHuddle();
                      }}
                      size="sm"
                      type="button"
                      variant="outline"
                    >
                      {pendingAction === "huddle" ? (
                        <Spinner
                          aria-hidden="true"
                          className="h-3.5 w-3.5 border-2"
                        />
                      ) : (
                        <Headphones />
                      )}
                      Huddle
                    </Button>
                  ) : null}
                </div>
              ) : null}
            </>
          ) : null}
        </div>
      </PopoverContent>
    </Popover>
  );
}

function PopoverWorkingBadge({
  name,
  anchorAt,
}: {
  name: string;
  anchorAt: number;
}) {
  const now = useNow(1000);

  return (
    <span className="inline-flex items-center rounded-full bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary motion-safe:animate-pulse">
      Working in #{name} · {formatElapsed(now - anchorAt)}
    </span>
  );
}
