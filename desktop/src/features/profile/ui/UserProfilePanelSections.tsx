import * as React from "react";
import type { LucideIcon } from "lucide-react";
import {
  ArrowUpRight,
  ChevronDown,
  ChevronUp,
  CircleAlert,
  MessageSquare,
  Pencil,
  Play,
  Square,
  UserMinus,
  UserPlus,
} from "lucide-react";
import { toast } from "sonner";

import { MemorySection } from "@/features/agent-memory/ui/MemorySection";
import { useActiveAgentTurns } from "@/features/agents/activeAgentTurnsStore";
import { getManagedAgentPrimaryActionLabel } from "@/features/agents/lib/managedAgentControlActions";
import { ManagedAgentLogPanel } from "@/features/agents/ui/ManagedAgentLogPanel";
import { AgentConfigPanel } from "@/features/agents/ui/AgentConfigPanel";
import { getPresenceLabel } from "@/features/presence/lib/presence";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import type { ProfileActivityAgent } from "@/features/profile/lib/profileActivityAgent";
import type {
  useFollowMutation,
  useUnfollowMutation,
  useUserProfileQuery,
} from "@/features/profile/hooks";
import {
  type ProfileField,
  ProfileFieldGroup,
} from "@/features/profile/ui/UserProfilePanelFields";
import { AGENT_DETAILS_FIELD_LABELS } from "@/features/profile/ui/UserProfilePanelAgentDetails";
import {
  ProfileInfoTabContent,
  ProfileIngressRow,
  ProfileRuntimeTabContent,
  ProfileTabBar,
} from "@/features/profile/ui/UserProfilePanelTabs";
import {
  MaskedAvatarBadgeFrame,
  STATUS_DOT_MASK_CURVE,
} from "@/features/profile/ui/MaskedAvatarBadgeFrame";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import { BotIdenticon } from "@/features/messages/ui/BotIdenticon";
import type { ManagedAgent, RelayAgent } from "@/shared/api/types";
import { Spinner } from "@/shared/ui/spinner";
import type {
  ProfileChannelLink,
  ProfilePanelTab,
} from "@/features/profile/ui/UserProfilePanelUtils";
import { useFeatureEnabled } from "@/shared/features";
import { cn } from "@/shared/lib/cn";
import { Alert, AlertDescription, AlertTitle } from "@/shared/ui/alert";
import { Badge } from "@/shared/ui/badge";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

export { AgentInstructionsFocusedView } from "@/features/profile/ui/UserProfilePanelAgentDetails";

// ── Summary view ─────────────────────────────────────────────────────────────

export type ProfileSummaryViewProps = {
  activityAgent: ProfileActivityAgent | null;
  canAddToChannel: boolean;
  canEditAgent: boolean;
  canOpenAgentLogs: boolean;
  canViewActivity: boolean;
  channelCount: number;
  channelIdToName: Record<string, string>;
  channels: ProfileChannelLink[];
  channelsLoading: boolean;
  displayName: string;
  followMutation: ReturnType<typeof useFollowMutation>;
  canInstantiateAgent: boolean;
  agentInstruction: string | null;
  handleAgentPrimaryAction: () => void;
  handleEditAgent: () => void;
  handleEditPersona?: () => void;
  handleInstantiateAgent: () => void;
  handleMessage: () => void;
  isArchived: boolean;
  isMessagePending: boolean;
  isBot: boolean;
  isAgentActionPending: boolean;
  isFollowing: boolean;
  isOwner: boolean | undefined;
  isSelf: boolean;
  managedAgent: ManagedAgent | undefined;
  memoriesLoading: boolean;
  memoryCount: number | undefined;
  agentInfoFields: ProfileField[];
  agentSettingsFields: ProfileField[];
  diagnosticsFields: ProfileField[];
  onAddToChannel: () => void;
  onOpenActivity: (channelId?: string | null) => void;
  onOpenChannel: (channelId: string) => void;
  onOpenDiagnostics: () => void;
  onOpenInstructions: () => void;
  onTabChange: (tab: ProfilePanelTab, options?: { replace?: boolean }) => void;
  onOpenDm?: (pubkeys: string[]) => Promise<void> | void;
  presenceStatus: "online" | "away" | "offline" | undefined;
  profile: ReturnType<typeof useUserProfileQuery>["data"];
  pubkey: string | null;
  relayAgent: RelayAgent | undefined;
  tab: ProfilePanelTab;
  unfollowMutation: ReturnType<typeof useUnfollowMutation>;
  userStatus: { text: string; emoji: string } | null | undefined;
};

type RuntimeTabStatus = "running" | "stopped" | "error";

const PROFILE_HERO_SPACING = {
  "0": 0,
  "6": 24,
} as const;

const PROFILE_HERO_PRESENCE_BADGE = {
  cutout: { cx: 68, cy: 68, r: 15 },
  shell: {
    bottom: PROFILE_HERO_SPACING["0"],
    height: PROFILE_HERO_SPACING["6"],
    right: PROFILE_HERO_SPACING["0"],
    width: PROFILE_HERO_SPACING["6"],
  },
} as const;

function resolveRuntimeTabStatus({
  diagnosticsError,
  managedAgent,
}: {
  diagnosticsError: boolean;
  managedAgent: ManagedAgent | undefined;
}): RuntimeTabStatus | undefined {
  if (diagnosticsError || managedAgent?.lastError) {
    return "error";
  }

  if (!managedAgent) {
    return undefined;
  }

  if (managedAgent.status === "running" || managedAgent.status === "deployed") {
    return "running";
  }

  return "stopped";
}

function RuntimeTabStatusDot({ status }: { status: RuntimeTabStatus }) {
  const label =
    status === "error" ? "Error" : status === "running" ? "Running" : "Stopped";

  return (
    <span
      aria-hidden="true"
      className={cn(
        "block h-1.5 w-1.5 rounded-full ring-1 ring-background",
        status === "error"
          ? "bg-destructive"
          : status === "running"
            ? "bg-emerald-500"
            : "bg-muted-foreground/50",
      )}
      data-status={status}
      data-testid="user-profile-runtime-status"
      title={label}
    />
  );
}

export function ProfileSummaryView({
  activityAgent,
  canAddToChannel,
  canEditAgent,
  canOpenAgentLogs,
  canViewActivity,
  channelCount,
  channelIdToName,
  channels,
  channelsLoading,
  displayName,
  followMutation,
  canInstantiateAgent,
  agentInstruction,
  handleAgentPrimaryAction,
  handleEditAgent,
  handleEditPersona,
  handleInstantiateAgent,
  handleMessage,
  isArchived,
  isMessagePending,
  isBot,
  isAgentActionPending,
  isFollowing,
  isOwner,
  isSelf,
  managedAgent,
  memoriesLoading,
  memoryCount,
  agentInfoFields,
  agentSettingsFields,
  diagnosticsFields,
  onAddToChannel,
  onOpenActivity,
  onOpenChannel,
  onOpenDiagnostics,
  onOpenInstructions,
  onTabChange,
  onOpenDm,
  presenceStatus,
  profile,
  pubkey,
  relayAgent,
  tab,
  unfollowMutation,
  userStatus,
}: ProfileSummaryViewProps) {
  const activeTurns = useActiveAgentTurns(isBot ? pubkey : null);

  const showMemoriesTab = isOwner === true && Boolean(pubkey);
  const showInstructionBlock =
    isOwner === true &&
    (agentInstruction !== null || handleEditPersona !== undefined);
  const showChannelsTab =
    channelsLoading || channelCount > 0 || isBot || relayAgent !== undefined;
  const runtimeConfigurationFields = agentSettingsFields.filter((field) =>
    AGENT_DETAILS_FIELD_LABELS.has(field.label),
  );
  const runtimeSettingsFields = agentSettingsFields.filter(
    (field) => !AGENT_DETAILS_FIELD_LABELS.has(field.label),
  );
  const showRuntimeTab =
    isOwner === true &&
    isBot &&
    (runtimeConfigurationFields.length > 0 ||
      runtimeSettingsFields.length > 0 ||
      managedAgent !== undefined ||
      diagnosticsFields.length > 0 ||
      canOpenAgentLogs ||
      showInstructionBlock);
  const showDiagnosticsIngress =
    diagnosticsFields.some((field) => field.label !== "Status") ||
    canOpenAgentLogs;
  const showActivityIngress = canViewActivity;
  const showInfoTab =
    agentInfoFields.length > 0 ||
    isArchived ||
    showActivityIngress ||
    !showRuntimeTab;

  const diagnosticsErrorField = diagnosticsFields.find(
    (field) => field.label === "Last error",
  );
  const diagnosticsTrailing =
    diagnosticsErrorField !== undefined ? (
      <Badge title={diagnosticsErrorField.displayValue} variant="destructive">
        Error
      </Badge>
    ) : (
      "View"
    );
  const runtimeTabStatus = resolveRuntimeTabStatus({
    diagnosticsError: diagnosticsErrorField !== undefined,
    managedAgent,
  });

  const tabs = React.useMemo(() => {
    const items: Array<{
      id: ProfilePanelTab;
      label: string;
      trailing?: React.ReactNode;
    }> = [];
    if (showInfoTab) {
      items.push({ id: "info", label: "Info" });
    }
    if (showRuntimeTab) {
      items.push({
        id: "runtime",
        label: "Runtime",
        trailing: runtimeTabStatus ? (
          <RuntimeTabStatusDot status={runtimeTabStatus} />
        ) : undefined,
      });
    }
    if (showChannelsTab) {
      items.push({
        id: "channels",
        label: "Channels",
        trailing: channelsLoading
          ? "…"
          : channelCount > 0
            ? String(channelCount)
            : undefined,
      });
    }
    if (showMemoriesTab) {
      items.push({
        id: "memories",
        label: "Memories",
        trailing: memoriesLoading
          ? "…"
          : memoryCount !== undefined
            ? String(memoryCount)
            : undefined,
      });
    }
    return items;
  }, [
    channelCount,
    channelsLoading,
    memoriesLoading,
    memoryCount,
    runtimeTabStatus,
    showChannelsTab,
    showInfoTab,
    showMemoriesTab,
    showRuntimeTab,
  ]);

  const showTabSection = tabs.length > 0;
  const showTabBar = !(tabs.length === 1 && tabs[0]?.id === "info");
  const activeTab = tabs.some((item) => item.id === tab)
    ? tab
    : (tabs[0]?.id ?? "info");

  return (
    <div className="flex flex-col gap-6 pt-4">
      <ProfileHero
        displayName={displayName}
        isBot={isBot}
        presenceStatus={presenceStatus}
        profile={profile}
        userStatus={userStatus}
      />

      {canInstantiateAgent ? (
        <ProfilePersonaPrimaryActions
          canEditAgent={canEditAgent}
          disabled={isAgentActionPending}
          onEditAgent={handleEditAgent}
          onStartAgent={handleInstantiateAgent}
        />
      ) : !isSelf && pubkey ? (
        <ProfilePrimaryActions
          canEditAgent={canEditAgent}
          followMutation={followMutation}
          onEditAgent={handleEditAgent}
          agentActionDisabled={isAgentActionPending}
          agentActionLabel={
            isOwner === true && managedAgent
              ? getManagedAgentPrimaryActionLabel(managedAgent)
              : undefined
          }
          agentActionLive={
            managedAgent?.status === "running" ||
            managedAgent?.status === "deployed"
          }
          onAgentPrimaryAction={
            isOwner === true && managedAgent
              ? handleAgentPrimaryAction
              : undefined
          }
          isFollowing={isFollowing}
          messagePending={isMessagePending}
          onMessage={onOpenDm ? handleMessage : undefined}
          pubkey={pubkey}
          unfollowMutation={unfollowMutation}
        />
      ) : null}

      {showTabSection ? (
        <section className="space-y-3">
          {showTabBar ? (
            <ProfileTabBar
              activeTab={activeTab}
              onTabChange={onTabChange}
              tabs={tabs}
            />
          ) : null}
          {activeTab === "info" ? (
            <ProfileInfoTabContent
              activeTurns={activeTurns}
              activityAgent={activityAgent}
              agentInfoFields={agentInfoFields}
              channelIdToName={channelIdToName}
              isArchived={isArchived}
              onOpenActivity={onOpenActivity}
              pubkey={pubkey}
              showActivityIngress={showActivityIngress}
            />
          ) : null}
          {activeTab === "runtime" ? (
            <>
              <ProfileRuntimeTabContent
                agentInstruction={agentInstruction}
                diagnosticsFields={diagnosticsFields}
                diagnosticsSummary={diagnosticsTrailing}
                onOpenDiagnostics={onOpenDiagnostics}
                onOpenInstructions={onOpenInstructions}
                runtimeConfigurationFields={runtimeConfigurationFields}
                runtimeSettingsFields={runtimeSettingsFields}
                showDiagnosticsIngress={showDiagnosticsIngress}
                showInstructionBlock={showInstructionBlock}
              />
              {isOwner === true && managedAgent !== undefined ? (
                <div className="overflow-hidden rounded-2xl bg-muted/20">
                  <AgentConfigPanel
                    advancedMode="flat"
                    pubkey={managedAgent.pubkey}
                  />
                </div>
              ) : null}
            </>
          ) : null}
          {activeTab === "channels" ? (
            <ChannelsFocusedView
              canAddToChannel={canAddToChannel}
              channels={channels}
              isActionPending={isAgentActionPending}
              isLoading={channelsLoading}
              onAddToChannel={onAddToChannel}
              onOpenChannel={onOpenChannel}
              variant="embedded"
            />
          ) : null}
          {activeTab === "memories" && pubkey ? (
            <MemoryFocusedView
              agentPubkey={pubkey}
              variant="embedded"
              viewerIsOwner={isOwner}
            />
          ) : null}
        </section>
      ) : null}
    </div>
  );
}

// ── Hero & metadata ──────────────────────────────────────────────────────────

function ProfileHero({
  displayName,
  isBot,
  presenceStatus,
  profile,
  userStatus,
}: {
  displayName: string;
  isBot: boolean;
  presenceStatus: "online" | "away" | "offline" | undefined;
  profile: ProfileSummaryViewProps["profile"];
  userStatus: ProfileSummaryViewProps["userStatus"];
}) {
  const presenceDotClassName = isBot ? "h-4.5 w-4.5" : "h-3.5 w-3.5";

  return (
    <div className="flex flex-col items-center gap-3 text-center">
      <MaskedAvatarBadgeFrame
        badge={
          presenceStatus ? (
            <span
              aria-label={getPresenceLabel(presenceStatus)}
              className="flex h-6 w-6 items-center justify-center rounded-full"
              data-testid="user-profile-presence-badge"
              role="img"
            >
              <PresenceDot
                className={presenceDotClassName}
                status={presenceStatus}
              />
            </span>
          ) : null
        }
        badgeBox={PROFILE_HERO_PRESENCE_BADGE.shell}
        className="h-20 w-20"
        curve={STATUS_DOT_MASK_CURVE}
        cutout={PROFILE_HERO_PRESENCE_BADGE.cutout}
        size={80}
      >
        <ProfileAvatar
          avatarUrl={profile?.avatarUrl ?? null}
          className="h-full w-full text-xl"
          iconClassName="h-8 w-8"
          label={displayName}
          plain
          testId="user-profile-avatar"
        />
      </MaskedAvatarBadgeFrame>

      <div className="flex flex-col items-center gap-1">
        <div className="flex items-center justify-center gap-2">
          <h3 className="text-xl font-semibold tracking-tight">
            {displayName}
          </h3>
          {isBot ? (
            <BotIdenticon
              className="shrink-0 rounded"
              data-testid="profile-bot-indicator"
              size={20}
              value={displayName}
            />
          ) : null}
        </div>

        {profile?.about?.trim() ? (
          <ProfileHeroDescription
            about={profile.about.trim()}
            key={profile.about.trim()}
          />
        ) : null}

        {profile?.nip05Handle ? (
          <p className="text-sm text-muted-foreground">{profile.nip05Handle}</p>
        ) : null}

        {userStatus ? (
          <p className="text-sm text-muted-foreground">
            {userStatus.emoji ? (
              <StatusEmoji
                className="mr-1 inline h-3.5 w-3.5"
                value={userStatus.emoji}
              />
            ) : null}
            {userStatus.text}
          </p>
        ) : null}
      </div>
    </div>
  );
}

function ProfileHeroDescription({ about }: { about: string }) {
  const [expanded, setExpanded] = React.useState(false);
  const [isTruncated, setIsTruncated] = React.useState(false);
  const textRef = React.useRef<HTMLParagraphElement>(null);

  const measureTruncation = React.useCallback(() => {
    const element = textRef.current;
    if (!element || expanded) {
      return;
    }
    setIsTruncated(element.scrollHeight > element.clientHeight + 1);
  }, [expanded]);

  React.useLayoutEffect(() => {
    measureTruncation();
  }, [measureTruncation]);

  React.useEffect(() => {
    const element = textRef.current;
    if (!element) {
      return;
    }

    const observer = new ResizeObserver(() => {
      measureTruncation();
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [measureTruncation]);

  const toggleClassName =
    "inline-flex items-center gap-0.5 text-xs font-medium text-muted-foreground opacity-60 transition-opacity hover:text-foreground hover:opacity-100";

  return (
    <div className="flex w-full flex-col items-center gap-0.5">
      <div className="w-fit max-w-full px-2">
        <p
          className={cn(
            "text-center whitespace-pre-wrap text-sm leading-relaxed text-muted-foreground",
            !expanded && "line-clamp-3",
          )}
          data-testid="user-profile-description"
          ref={textRef}
        >
          {about}
        </p>
      </div>
      {!expanded && isTruncated ? (
        <button
          className={toggleClassName}
          data-testid="user-profile-description-toggle"
          onClick={() => setExpanded(true)}
          type="button"
        >
          more
          <ChevronDown className="h-4 w-4" />
        </button>
      ) : null}
      {expanded ? (
        <button
          className={toggleClassName}
          data-testid="user-profile-description-toggle"
          onClick={() => setExpanded(false)}
          type="button"
        >
          less
          <ChevronUp className="h-4 w-4" />
        </button>
      ) : null}
    </div>
  );
}

// ── Primary actions ──────────────────────────────────────────────────────────

function ProfilePrimaryActions({
  agentActionDisabled,
  agentActionLabel,
  agentActionLive,
  canEditAgent,
  followMutation,
  isFollowing,
  messagePending,
  onAgentPrimaryAction,
  onEditAgent,
  onMessage,
  pubkey,
  unfollowMutation,
}: {
  agentActionDisabled?: boolean;
  agentActionLabel?: string;
  agentActionLive?: boolean;
  canEditAgent: boolean;
  followMutation: ReturnType<typeof useFollowMutation>;
  isFollowing: boolean;
  messagePending?: boolean;
  onAgentPrimaryAction?: () => void;
  onEditAgent: () => void;
  onMessage?: () => void;
  pubkey: string;
  unfollowMutation: ReturnType<typeof useUnfollowMutation>;
}) {
  const showFollowAction = useFeatureEnabled("pulse");
  const followToggleMutation = isFollowing ? unfollowMutation : followMutation;

  const handleFollowClick = () => {
    followToggleMutation.mutate(pubkey, {
      onError: (error) =>
        toast.error(
          `${isFollowing ? "Unfollow" : "Follow"} failed: ${error.message}`,
        ),
    });
  };

  return (
    <div className="flex items-center justify-center gap-8">
      {showFollowAction ? (
        <ProfileQuickAction
          active={isFollowing}
          disabled={followToggleMutation.isPending}
          icon={isFollowing ? UserMinus : UserPlus}
          label={isFollowing ? "Unfollow" : "Follow"}
          onClick={handleFollowClick}
        />
      ) : null}
      {onMessage ? (
        <ProfileQuickAction
          disabled={messagePending}
          icon={MessageSquare}
          isLoading={messagePending}
          label="Message"
          onClick={onMessage}
          testId="user-profile-message"
        />
      ) : null}
      {canEditAgent ? (
        <ProfileQuickAction
          icon={Pencil}
          label="Edit"
          onClick={onEditAgent}
          testId="user-profile-edit-agent"
        />
      ) : null}
      {onAgentPrimaryAction && agentActionLabel ? (
        <ProfileQuickAction
          active={agentActionLive}
          disabled={agentActionDisabled}
          icon={agentActionLive ? Square : Play}
          label={agentActionLabel}
          onClick={onAgentPrimaryAction}
          testId="user-profile-agent-primary-action"
        />
      ) : null}
    </div>
  );
}

function ProfilePersonaPrimaryActions({
  canEditAgent,
  disabled,
  onEditAgent,
  onStartAgent,
}: {
  canEditAgent: boolean;
  disabled: boolean;
  onEditAgent: () => void;
  onStartAgent: () => void;
}) {
  return (
    <div className="flex items-center justify-center gap-8">
      <ProfileQuickAction
        disabled={disabled}
        icon={Play}
        label="Start agent"
        onClick={onStartAgent}
        testId="user-profile-start-agent"
      />
      {canEditAgent ? (
        <ProfileQuickAction
          disabled={disabled}
          icon={Pencil}
          label="Edit"
          onClick={onEditAgent}
          testId="user-profile-edit-agent"
        />
      ) : null}
    </div>
  );
}

function ProfileQuickAction({
  active,
  disabled,
  icon: Icon,
  isLoading,
  label,
  onClick,
  testId,
}: {
  active?: boolean;
  disabled?: boolean;
  icon: LucideIcon;
  isLoading?: boolean;
  label: string;
  onClick: () => void;
  testId?: string;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          aria-label={label}
          className={cn(
            "flex h-12 w-12 items-center justify-center rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-50",
            active
              ? "bg-foreground text-background hover:bg-foreground/90"
              : "bg-muted/60 text-foreground hover:bg-muted/80",
          )}
          data-testid={testId}
          disabled={disabled}
          onClick={onClick}
          type="button"
        >
          {isLoading ? (
            <Spinner aria-hidden="true" className="h-4 w-4 border-2" />
          ) : (
            <Icon className="h-4 w-4" />
          )}
        </button>
      </TooltipTrigger>
      <TooltipContent align="center" side="top">
        {label}
      </TooltipContent>
    </Tooltip>
  );
}

// ── Focused views ────────────────────────────────────────────────────────────

export function MemoryFocusedView({
  agentPubkey,
  variant = "focused",
  viewerIsOwner,
}: {
  agentPubkey: string;
  variant?: "embedded" | "focused";
  viewerIsOwner: boolean | undefined;
}) {
  if (viewerIsOwner !== true) {
    return null;
  }

  return (
    <div className={variant === "focused" ? "pt-4" : undefined}>
      <MemorySection agentPubkey={agentPubkey} viewerIsOwner={viewerIsOwner} />
    </div>
  );
}

export function ChannelsFocusedView({
  canAddToChannel,
  channels,
  isActionPending,
  isLoading,
  onAddToChannel,
  onOpenChannel,
  variant = "focused",
}: {
  canAddToChannel: boolean;
  channels: ProfileChannelLink[];
  isActionPending: boolean;
  isLoading: boolean;
  onAddToChannel: () => void;
  onOpenChannel: (channelId: string) => void;
  variant?: "embedded" | "focused";
}) {
  return (
    <div className={cn("space-y-3", variant === "focused" && "pt-4")}>
      {canAddToChannel ? (
        <ProfileIngressRow
          disabled={isActionPending}
          icon={UserPlus}
          label="Add to channel"
          onClick={onAddToChannel}
          testId="user-profile-agent-add-channel"
          trailing={isActionPending ? "Working…" : undefined}
        />
      ) : null}
      {isLoading ? (
        <p className="text-base leading-7 text-muted-foreground">
          Loading channels…
        </p>
      ) : channels.length === 0 ? (
        <div
          className={cn(
            "flex flex-col items-center justify-center px-6 text-center",
            canAddToChannel ? "min-h-20 py-4" : "min-h-56 py-10",
          )}
          data-testid="user-profile-channels-empty"
        >
          <UserPlus className="mx-auto h-4 w-4 text-muted-foreground" />
          <p className="mt-3 text-sm font-medium">
            {canAddToChannel
              ? "Add this agent to a channel"
              : "Channels appear here"}
          </p>
          <p className="mt-1 text-sm text-muted-foreground">
            {canAddToChannel
              ? "Choose a channel above so it can join the conversation."
              : "Visible memberships appear as this agent joins channels."}
          </p>
        </div>
      ) : (
        <ul
          className="overflow-hidden rounded-2xl bg-muted/20"
          data-testid="user-profile-channels-list"
        >
          {channels.map((channel) => (
            <li key={channel.id}>
              <button
                aria-label={`Open #${channel.name}`}
                className="group flex w-full items-center gap-3 px-4 py-3 text-left text-base leading-7 text-foreground transition-colors hover:bg-muted/40"
                data-testid={`user-profile-channel-link-${channel.name}`}
                onClick={() => onOpenChannel(channel.id)}
                type="button"
              >
                <span className="min-w-0 flex-1 truncate">#{channel.name}</span>
                <ArrowUpRight
                  aria-hidden="true"
                  className="h-4 w-4 shrink-0 text-muted-foreground transition-colors group-hover:text-foreground"
                />
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export function AgentInfoFocusedView({
  metadataFields,
}: {
  metadataFields: ProfileField[];
}) {
  if (metadataFields.length === 0) {
    return null;
  }

  return (
    <div className="pt-4">
      <ProfileFieldGroup fields={metadataFields} />
    </div>
  );
}

export function DiagnosticsFocusedView({
  canOpenAgentLogs,
  fields,
  logContent,
  logError,
  logLoading,
  managedAgent,
}: {
  canOpenAgentLogs: boolean;
  fields: ProfileField[];
  logContent: string | null;
  logError: Error | null;
  logLoading: boolean;
  managedAgent: ManagedAgent | undefined;
}) {
  const hasLog = canOpenAgentLogs && managedAgent !== undefined;
  const lastErrorField = fields.find((field) => field.label === "Last error");
  const detailFields = fields.filter(
    (field) => field.label !== "Last error" && field.label !== "Status",
  );

  if (!lastErrorField && detailFields.length === 0 && !hasLog) {
    return null;
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 pt-4">
      {lastErrorField ? (
        <Alert
          className="flex gap-3"
          data-testid={lastErrorField.testId}
          variant="destructive"
        >
          <CircleAlert className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
          <div className="min-w-0">
            <AlertTitle>Last error</AlertTitle>
            <AlertDescription className="wrap-break-word">
              {lastErrorField.displayValue}
            </AlertDescription>
          </div>
        </Alert>
      ) : null}
      {detailFields.length > 0 ? (
        <ProfileFieldGroup fields={detailFields} />
      ) : null}
      {hasLog ? (
        <div className="min-h-0 flex-1">
          <ManagedAgentLogPanel
            chrome="bare"
            error={logError}
            isLoading={logLoading}
            logContent={logContent}
            selectedAgent={managedAgent}
            variant="inline"
          />
        </div>
      ) : null}
    </div>
  );
}
