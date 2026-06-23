import * as React from "react";
import type { LucideIcon } from "lucide-react";
import {
  Activity,
  ArrowUpRight,
  Brain,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Cpu,
  FileText,
  Hash,
  Info,
  MessageSquare,
  Pencil,
  Play,
  Power,
  Settings,
  Square,
  UserMinus,
  UserPlus,
} from "lucide-react";
import { toast } from "sonner";

import { MemorySection } from "@/features/agent-memory/ui/MemorySection";
import { useActiveAgentTurns } from "@/features/agents/activeAgentTurnsStore";
import { getManagedAgentPrimaryActionLabel } from "@/features/agents/lib/managedAgentControlActions";
import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import { ModelPicker } from "@/features/agents/ui/ModelPicker";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { getPresenceLabel } from "@/features/presence/lib/presence";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import type {
  useFollowMutation,
  useUnfollowMutation,
  useUserProfileQuery,
} from "@/features/profile/hooks";
import {
  type ProfileField,
  ProfileFieldGroup,
} from "@/features/profile/ui/UserProfilePanelFields";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import { BotIdenticon } from "@/features/messages/ui/BotIdenticon";
import { UserProfileAgentActions } from "@/features/profile/ui/UserProfileAgentActions";
import type {
  AgentPersona,
  ManagedAgent,
  RelayAgent,
} from "@/shared/api/types";
import { useFeatureEnabled } from "@/shared/features";
import { cn } from "@/shared/lib/cn";
import { useNow } from "@/shared/lib/useNow";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";

// ── Summary view ─────────────────────────────────────────────────────────────

export type ProfileSummaryViewProps = {
  canEditAgent: boolean;
  canOpenAgentLogs: boolean;
  canViewActivity: boolean;
  channelCount: number;
  channelIdToName: Record<string, string>;
  channelsLoading: boolean;
  displayName: string;
  followMutation: ReturnType<typeof useFollowMutation>;
  canInstantiateAgent: boolean;
  agentInstruction: string | null;
  handleAgentPrimaryAction: () => void;
  handleDeletePersona?: () => void;
  handleDuplicatePersona?: () => void;
  handleDeleteAgent: () => void;
  handleEditAgent: () => void;
  handleEditPersona?: () => void;
  handleExportPersona?: () => void;
  handleInstantiateAgent: () => void;
  handleMessage: () => void;
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
  diagnosticsSummary: string | null;
  modelLabel: string;
  onOpenAgentInfo: () => void;
  onOpenAgentSettings: () => void;
  onOpenChannels: () => void;
  onOpenDiagnostics: () => void;
  onOpenInstruction: () => void;
  onOpenMemories: () => void;
  onOpenModel: () => void;
  onOpenDm?: (pubkeys: string[]) => void;
  persona?: AgentPersona;
  presenceStatus: "online" | "away" | "offline" | undefined;
  profile: ReturnType<typeof useUserProfileQuery>["data"];
  pubkey: string | null;
  relayAgent: RelayAgent | undefined;
  unfollowMutation: ReturnType<typeof useUnfollowMutation>;
  userStatus: { text: string; emoji: string } | null | undefined;
};

export function ProfileSummaryView({
  canEditAgent,
  canOpenAgentLogs,
  canViewActivity,
  channelCount,
  channelIdToName,
  channelsLoading,
  displayName,
  followMutation,
  canInstantiateAgent,
  agentInstruction,
  handleAgentPrimaryAction,
  handleDeletePersona,
  handleDuplicatePersona,
  handleDeleteAgent,
  handleEditAgent,
  handleEditPersona,
  handleExportPersona,
  handleInstantiateAgent,
  handleMessage,
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
  diagnosticsSummary,
  modelLabel,
  onOpenAgentInfo,
  onOpenAgentSettings,
  onOpenChannels,
  onOpenDiagnostics,
  onOpenInstruction,
  onOpenMemories,
  onOpenModel,
  onOpenDm,
  persona,
  presenceStatus,
  profile,
  pubkey,
  relayAgent,
  unfollowMutation,
  userStatus,
}: ProfileSummaryViewProps) {
  const { goChannel } = useAppNavigation();
  const activeTurns = useActiveAgentTurns(isBot ? pubkey : null);

  const showMemoriesIngress = isOwner === true && Boolean(pubkey);
  const showInstructionIngress =
    isOwner === true &&
    (agentInstruction !== null || handleEditPersona !== undefined);
  const showChannelsIngress =
    channelsLoading || channelCount > 0 || isBot || relayAgent !== undefined;
  const showModelIngress = isOwner === true && isBot;
  const showAgentSettingsIngress =
    isOwner === true &&
    (agentSettingsFields.length > 0 || managedAgent?.backend.type === "local");
  const showDiagnosticsIngress =
    diagnosticsFields.length > 0 || canOpenAgentLogs || canViewActivity;
  const showAgentInfoIngress = agentInfoFields.length > 0;
  const personaActionKey = persona?.id;

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
          onMessage={onOpenDm ? handleMessage : undefined}
          pubkey={pubkey}
          unfollowMutation={unfollowMutation}
        />
      ) : null}

      {activeTurns.length > 0 ? (
        <div className="flex flex-wrap justify-center gap-1.5">
          {activeTurns.map(({ channelId, anchorAt }) => (
            <ProfileWorkingBadge
              key={channelId}
              channelId={channelId}
              name={channelIdToName[channelId] ?? channelId}
              anchorAt={anchorAt}
              onNavigate={goChannel}
            />
          ))}
        </div>
      ) : null}

      {showInstructionIngress ||
      showModelIngress ||
      showMemoriesIngress ||
      showChannelsIngress ||
      showAgentSettingsIngress ||
      showDiagnosticsIngress ||
      showAgentInfoIngress ? (
        <section className="space-y-2">
          {showInstructionIngress ? (
            <ProfileIngressRow
              icon={FileText}
              label="Agent instruction"
              onClick={onOpenInstruction}
              testId="user-profile-agent-instruction-ingress"
              trailing={handleEditPersona ? "Edit" : "View"}
            />
          ) : null}
          {showModelIngress ? (
            <ProfileIngressRow
              icon={Cpu}
              label="Model"
              onClick={onOpenModel}
              testId="user-profile-model-ingress"
              trailing={modelLabel}
            />
          ) : null}
          {showMemoriesIngress ? (
            <ProfileIngressRow
              icon={Brain}
              label="Memories"
              onClick={onOpenMemories}
              testId="user-profile-memories-ingress"
              trailing={
                memoriesLoading
                  ? "Loading…"
                  : memoryCount !== undefined
                    ? String(memoryCount)
                    : "View"
              }
            />
          ) : null}
          {showChannelsIngress ? (
            <ProfileIngressRow
              icon={Hash}
              label="Channels"
              onClick={onOpenChannels}
              testId="user-profile-channels-ingress"
              trailing={
                channelsLoading
                  ? "Loading…"
                  : channelCount > 0
                    ? String(channelCount)
                    : "None"
              }
            />
          ) : null}
          {showAgentSettingsIngress ? (
            <ProfileIngressRow
              icon={Settings}
              label="Agent settings"
              onClick={onOpenAgentSettings}
              testId="user-profile-agent-settings-ingress"
              trailing="View"
            />
          ) : null}
          {showDiagnosticsIngress ? (
            <ProfileIngressRow
              icon={Activity}
              label="Diagnostics"
              onClick={onOpenDiagnostics}
              testId="user-profile-diagnostics-ingress"
              trailing={diagnosticsSummary ?? "View"}
            />
          ) : null}
          {showAgentInfoIngress ? (
            <ProfileIngressRow
              icon={Info}
              label="Agent info"
              onClick={onOpenAgentInfo}
              testId="user-profile-agent-info-ingress"
              trailing="View"
            />
          ) : null}
        </section>
      ) : null}

      {isOwner === true && managedAgent ? (
        <UserProfileAgentActions
          isPending={isAgentActionPending}
          managedAgent={managedAgent}
          onDelete={handleDeleteAgent}
          onDuplicatePersona={handleDuplicatePersona}
          onExportPersona={handleExportPersona}
          personaActionKey={personaActionKey}
        />
      ) : null}
      {canInstantiateAgent ? (
        <UserProfileAgentActions
          isPending={isAgentActionPending}
          onDelete={handleDeletePersona}
          onDuplicatePersona={handleDuplicatePersona}
          onExportPersona={handleExportPersona}
          personaActionKey={personaActionKey}
        />
      ) : null}
    </div>
  );
}

function ProfileWorkingBadge({
  channelId,
  name,
  anchorAt,
  onNavigate,
}: {
  channelId: string;
  name: string;
  anchorAt: number;
  onNavigate: (channelId: string) => void;
}) {
  const now = useNow(1000);

  return (
    <Badge
      className="cursor-pointer motion-safe:animate-pulse normal-case tracking-normal hover:opacity-80"
      variant="default"
      onClick={() => onNavigate(channelId)}
    >
      Working in #{name} · {formatElapsed(now - anchorAt)}
    </Badge>
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
  return (
    <div className="flex flex-col items-center gap-3 text-center">
      <div className="relative">
        <ProfileAvatar
          avatarUrl={profile?.avatarUrl ?? null}
          className="h-20 w-20 text-xl"
          iconClassName="h-8 w-8"
          label={displayName}
          plain
          testId="user-profile-avatar"
        />
        {presenceStatus ? (
          <span
            aria-label={getPresenceLabel(presenceStatus)}
            className="absolute bottom-0 right-0 flex h-6 w-6 items-center justify-center rounded-full bg-background"
            data-testid="user-profile-presence-badge"
            role="img"
          >
            <PresenceDot className="h-3.5 w-3.5" status={presenceStatus} />
          </span>
        ) : null}
      </div>

      <div className="flex flex-col items-center gap-1">
        <div className="flex items-center justify-center gap-2">
          <h3 className="text-xl font-semibold tracking-tight">
            {displayName}
          </h3>
          {isBot ? (
            <BotIdenticon
              className="shrink-0 rounded"
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
    <div className="flex items-start justify-center gap-8">
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
          icon={MessageSquare}
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
    <div className="flex items-start justify-center gap-8">
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
  label,
  onClick,
  testId,
}: {
  active?: boolean;
  disabled?: boolean;
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  testId?: string;
}) {
  return (
    <button
      className="flex flex-col items-center gap-2 disabled:cursor-not-allowed disabled:opacity-50"
      data-testid={testId}
      disabled={disabled}
      onClick={onClick}
      type="button"
    >
      <span
        className={cn(
          "flex h-14 w-14 items-center justify-center rounded-full transition-colors",
          active
            ? "bg-foreground text-background hover:bg-foreground/90"
            : "bg-muted/60 text-foreground hover:bg-muted/80",
        )}
      >
        <Icon className="h-4 w-4" />
      </span>
      <span
        className={cn(
          "text-xs",
          active ? "text-foreground" : "text-muted-foreground",
        )}
      >
        {label}
      </span>
    </button>
  );
}

// ── Ingress rows ─────────────────────────────────────────────────────────────

function ProfileIngressRow({
  disabled,
  icon: Icon,
  label,
  onClick,
  testId,
  trailing,
}: {
  disabled?: boolean;
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  testId: string;
  trailing?: string;
}) {
  return (
    <button
      className="flex w-full items-center gap-3 rounded-2xl bg-muted/20 px-4 py-2 text-left transition-colors hover:bg-muted/40 disabled:cursor-not-allowed disabled:opacity-50"
      data-testid={testId}
      disabled={disabled}
      onClick={onClick}
      type="button"
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
        <Icon className="h-4 w-4 text-muted-foreground" />
      </span>
      <span className="min-w-0 flex-1 text-sm font-medium text-foreground">
        {label}
      </span>
      {trailing ? (
        <span
          className="max-w-[45%] truncate text-right text-sm text-muted-foreground"
          title={trailing}
        >
          {trailing}
        </span>
      ) : null}
      <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
    </button>
  );
}

// ── Focused views ────────────────────────────────────────────────────────────

export function MemoryFocusedView({
  agentPubkey,
  isOwner,
}: {
  agentPubkey: string;
  isOwner: boolean | undefined;
}) {
  if (isOwner !== true) {
    return null;
  }

  return (
    <div className="pt-4">
      <MemorySection agentPubkey={agentPubkey} />
    </div>
  );
}

type ProfileChannelLink = {
  id: string;
  name: string;
};

export function ChannelsFocusedView({
  canAddToChannel,
  channels,
  isActionPending,
  isLoading,
  onAddToChannel,
  onOpenChannel,
}: {
  canAddToChannel: boolean;
  channels: ProfileChannelLink[];
  isActionPending: boolean;
  isLoading: boolean;
  onAddToChannel: () => void;
  onOpenChannel: (channelId: string) => void;
}) {
  return (
    <div className="space-y-3 pt-4">
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
        <p
          className="text-base leading-7 italic text-muted-foreground"
          data-testid="user-profile-channels-empty"
        >
          No visible channel memberships.
        </p>
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

export function ModelFocusedView({
  managedAgent,
  modelLabel,
  onModelChanged,
}: {
  managedAgent: ManagedAgent | undefined;
  modelLabel: string;
  onModelChanged: () => void;
}) {
  return (
    <div className="space-y-3 pt-4">
      <div className="flex items-center gap-3 rounded-2xl bg-muted/20 px-4 py-3">
        <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
          <Cpu className="h-4 w-4 text-muted-foreground" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block text-xs font-medium text-foreground">
            Model
          </span>
          <span className="mt-0.5 block truncate text-sm text-muted-foreground">
            {modelLabel}
          </span>
        </span>
        {managedAgent ? (
          <ModelPicker agent={managedAgent} onModelChanged={onModelChanged} />
        ) : null}
      </div>
    </div>
  );
}

export function AgentSettingsFocusedView({
  fields,
  isActionPending,
  managedAgent,
  onToggleAutoStart,
}: {
  fields: ProfileField[];
  isActionPending: boolean;
  managedAgent: ManagedAgent | undefined;
  onToggleAutoStart: () => void;
}) {
  const canToggleAutoStart =
    managedAgent !== undefined && managedAgent.backend.type === "local";

  if (fields.length === 0 && !canToggleAutoStart) {
    return null;
  }

  return (
    <div className="space-y-3 pt-4">
      {canToggleAutoStart && managedAgent ? (
        <ProfileIngressRow
          disabled={isActionPending}
          icon={Power}
          label={
            managedAgent.startOnAppLaunch
              ? "Disable auto-start"
              : "Enable auto-start"
          }
          onClick={onToggleAutoStart}
          testId={`user-profile-agent-auto-start-${managedAgent.pubkey}`}
          trailing={managedAgent.startOnAppLaunch ? "On" : "Off"}
        />
      ) : null}
      {fields.length > 0 ? <ProfileFieldGroup fields={fields} /> : null}
    </div>
  );
}

export function DiagnosticsFocusedView({
  canOpenAgentLogs,
  canViewActivity,
  fields,
  managedAgent,
  onOpenActivity,
  onOpenAgentLogs,
  pubkey,
}: {
  canOpenAgentLogs: boolean;
  canViewActivity: boolean;
  fields: ProfileField[];
  managedAgent: ManagedAgent | undefined;
  onOpenActivity: () => void;
  onOpenAgentLogs: () => void;
  pubkey: string | null;
}) {
  const hasActions = canOpenAgentLogs || canViewActivity;

  if (fields.length === 0 && !hasActions) {
    return null;
  }

  return (
    <div className="space-y-3 pt-4">
      {fields.length > 0 ? <ProfileFieldGroup fields={fields} /> : null}
      {canOpenAgentLogs && managedAgent ? (
        <ProfileIngressRow
          icon={FileText}
          label="Harness log"
          onClick={onOpenAgentLogs}
          testId={`user-profile-agent-logs-${managedAgent.pubkey}`}
          trailing="View"
        />
      ) : null}
      {canViewActivity ? (
        <ProfileIngressRow
          icon={Activity}
          label="Activity log"
          onClick={onOpenActivity}
          testId={`user-profile-view-activity-${pubkey}`}
          trailing="View"
        />
      ) : null}
    </div>
  );
}

export function AgentInstructionFocusedView({
  instruction,
  onEdit,
}: {
  instruction: string | null;
  onEdit?: () => void;
}) {
  const trimmedInstruction = instruction?.trim() ?? "";

  return (
    <div className="space-y-3 pt-4">
      <div className="rounded-2xl bg-muted/20 px-4 py-3">
        {trimmedInstruction ? (
          <div data-testid="user-profile-agent-instruction">
            <Markdown
              className="text-sm leading-6"
              content={trimmedInstruction}
              interactive={false}
            />
          </div>
        ) : (
          <p
            className="text-sm leading-6 text-muted-foreground"
            data-testid="user-profile-agent-instruction-empty"
          >
            No instruction set.
          </p>
        )}
      </div>
      {onEdit ? (
        <Button
          className="w-full justify-start gap-2"
          data-testid="user-profile-agent-instruction-edit"
          onClick={onEdit}
          type="button"
          variant="outline"
        >
          <Pencil className="h-4 w-4" />
          Edit agent
        </Button>
      ) : null}
    </div>
  );
}
