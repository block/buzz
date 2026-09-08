import * as React from "react";
import { Activity, Headphones, MessageSquare } from "lucide-react";

import { AgentManagementMarker } from "@/features/agents/ui/OtherSetupAgentMarker";

import { useChannelsQuery } from "@/features/channels/hooks";
import {
  useUserProfileQuery,
  useUsersBatchQuery,
} from "@/features/profile/hooks";
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
import { useAgentAvailability } from "@/features/agents/lib/useAgentAvailability";
import {
  useUserStatusQuery,
  visibleUserStatus,
} from "@/features/user-status/hooks";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import { ProfileAvatarWithStatus } from "@/features/profile/ui/ProfileAvatarWithStatus";
import { useOpenAgentActivity } from "@/features/agents/useOpenAgentActivity";
import { useProfilePanel } from "@/shared/context/ProfilePanelContext";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { useProfileInteractionActions } from "@/features/profile/ui/useProfileInteractionActions";

import {
  DEFAULT_POPOVER_HOVER_OPEN_DELAY_MS,
  Popover,
  PopoverAnchor,
  PopoverContent,
} from "@/shared/ui/popover";
import { BotIdenticon } from "@/features/messages/ui/BotIdenticon";
import { useNow } from "@/shared/lib/useNow";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";
import { resolveModelLabel } from "@/features/agents/lib/formatAgentModelLabel";

type UserProfilePopoverProps = {
  children: React.ReactNode;
  pubkey: string;
  triggerElement?: "div" | "span";
  /** Accessible name for interactive trigger content that is visually hidden. */
  triggerAriaLabel?: string;
  /** Set false when the trigger is inside another interactive control. */
  enableProfilePanel?: boolean;
  /** Set false when a smaller, context-specific hover treatment is provided. */
  enableHoverPopover?: boolean;
  /** When set to "bot", a BotIdenticon badge renders next to the display name. */
  role?: string;
  /** Value used to generate the BotIdenticon glyph (typically the author name). */
  botIdenticonValue?: string;
};

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
  enableHoverPopover = true,
  role,
  botIdenticonValue,
}: UserProfilePopoverProps) {
  const [open, setOpen] = React.useState(false);
  const hoverTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const { openProfilePanel } = useProfilePanel();
  const canOpenProfilePanel = enableProfilePanel && Boolean(openProfilePanel);

  const clearHoverTimer = React.useCallback(() => {
    if (hoverTimerRef.current !== null) {
      clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
  }, []);

  const handleTriggerMouseEnter = React.useCallback(() => {
    if (!enableHoverPopover) {
      return;
    }
    clearHoverTimer();
    hoverTimerRef.current = setTimeout(() => {
      setOpen(true);
    }, DEFAULT_POPOVER_HOVER_OPEN_DELAY_MS);
  }, [clearHoverTimer, enableHoverPopover]);

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

  React.useEffect(() => {
    return clearHoverTimer;
  }, [clearHoverTimer]);

  const TriggerElement = triggerElement;
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
      {open ? (
        <UserProfilePopoverBody
          botIdenticonValue={botIdenticonValue}
          canOpenProfilePanel={canOpenProfilePanel}
          onBeforeAction={clearHoverTimer}
          onContentMouseEnter={handleContentMouseEnter}
          onMouseLeave={handleMouseLeave}
          onTriggerClick={handleTriggerClick}
          pubkey={pubkey}
          role={role}
          setOpen={setOpen}
        />
      ) : null}
    </Popover>
  );
}

/**
 * Everything behind the popover surface: seven query subscriptions, agent
 * classification, and the interaction actions. Mounted only while the
 * popover is open — the trigger shell above stays cheap enough for grids
 * that render hundreds of instances (~40ms per card when this was eager).
 */
function UserProfilePopoverBody({
  botIdenticonValue,
  canOpenProfilePanel,
  onBeforeAction,
  onContentMouseEnter,
  onMouseLeave,
  onTriggerClick,
  pubkey,
  role,
  setOpen,
}: {
  botIdenticonValue?: string;
  canOpenProfilePanel: boolean;
  onBeforeAction: () => void;
  onContentMouseEnter: () => void;
  onMouseLeave: () => void;
  onTriggerClick: (event: React.MouseEvent) => void;
  pubkey: string;
  role?: string;
  setOpen: (open: boolean) => void;
}) {
  const profileQuery = useUserProfileQuery(pubkey);
  const usersBatchQuery = useUsersBatchQuery([pubkey]);
  const relayAgentsQuery = useRelayAgentsQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const { status: presenceStatus } = useAgentAvailability(pubkey);
  const userStatusQuery = useUserStatusQuery([pubkey]);

  const { canOpenAgentActivity, openAgentActivity } = useOpenAgentActivity();
  const relayAgent = relayAgentsQuery.data?.find((a) => a.pubkey === pubkey);
  const managedAgent = managedAgentsQuery.data?.find(
    (a) => a.pubkey === pubkey,
  );
  const profile = profileQuery.data;
  const ownerPubkey = profile?.ownerPubkey ?? null;
  const ownerProfileQuery = useUsersBatchQuery(
    ownerPubkey ? [ownerPubkey] : [],
    { enabled: Boolean(ownerPubkey) },
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
  const isCurrentUserOwner = ownsAuthorAgent(profile, currentPubkey);
  const viewerIsOwner = isCurrentUserOwner || isOwner === true;
  const showHuddleAction =
    showHumanProfileActions ||
    (showProfileActions &&
      isBotProfile &&
      viewerIsOwner &&
      !isAgentClassificationPending);
  const showMessageAction =
    showProfileActions &&
    !isAgentClassificationPending &&
    (!isBotProfile || viewerIsOwner);
  const showAnyProfileActions =
    showHumanProfileActions || showMessageAction || showHuddleAction;
  const canViewActivity =
    isBotProfile && viewerIsOwner && canOpenAgentActivity(pubkey);
  const userStatus = visibleUserStatus(
    userStatusQuery.data?.[pubkey.toLowerCase()],
  );
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

  const closeProfileActions = React.useCallback(
    () => setOpen(false),
    [setOpen],
  );
  const {
    handleHuddle,
    handleMessage,
    handleWave,
    isOpeningDm,
    isStartingHuddle,
    pendingAction,
  } = useProfileInteractionActions({
    availability: {
      huddle: showHuddleAction,
      message: showMessageAction,
      wave: showHumanProfileActions,
    },
    effectivePubkey: pubkey,
    enabled: true,
    isBot: isBotProfile,
    isSelf,
    onBeforeAction: onBeforeAction,
    onClose: closeProfileActions,
    viewerIsOwner,
  });

  const profileHeaderContent = (
    <>
      <ProfileAvatarWithStatus
        avatarClassName="text-xs"
        avatarUrl={profile?.avatarUrl ?? null}
        className="h-10 w-10"
        iconClassName="h-5 w-5"
        label={displayName}
        shape={isBotProfile ? "squircle" : "circle"}
        size={40}
        status={presenceStatus}
        statusTestId="user-profile-popover-presence-badge"
        testId="user-profile-popover-avatar"
      />

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <HoverPubkeyName displayName={displayName} pubkey={pubkey} />
          <AgentManagementMarker
            pubkey={pubkey}
            ownerPubkey={ownerPubkey}
            testId="user-profile-popover-agent-provenance"
          />
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
    <PopoverContent
      align="start"
      className="w-80"
      data-testid="user-profile-popover"
      onMouseEnter={onContentMouseEnter}
      onMouseLeave={onMouseLeave}
      // This is a hover card: moving focus into its first button on open
      // makes the profile header look keyboard-selected before the user has
      // interacted with it. Keep focus on the trigger; Tab still enters the
      // card and shows its normal focus treatment when needed.
      onOpenAutoFocus={(event) => event.preventDefault()}
      side="top"
      sideOffset={8}
    >
      <div className="flex flex-col gap-3">
        {canOpenProfilePanel ? (
          <button
            className="flex w-full min-w-0 cursor-pointer items-center gap-3 rounded-lg text-left text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring [&_*]:cursor-pointer"
            onClick={onTriggerClick}
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
              <InfoBadge>
                {resolveModelLabel(
                  managedAgent.model,
                  null,
                  managedAgent.provider,
                )}
              </InfoBadge>
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
                    disabled={pendingAction !== null || isOpeningDm}
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
                    disabled={pendingAction !== null || isOpeningDm}
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
                {showHuddleAction ? (
                  <Button
                    className="min-w-0 flex-1"
                    data-testid={`user-profile-popover-huddle-${pubkey}`}
                    disabled={
                      pendingAction !== null || isOpeningDm || isStartingHuddle
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
