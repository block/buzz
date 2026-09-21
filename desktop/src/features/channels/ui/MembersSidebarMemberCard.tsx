import { agentPresenceStartBlockReason } from "@/features/agents/lib/useAgentAvailability";
import {
  Activity,
  Ban,
  Bot,
  CircleSlash,
  Clock,
  Ellipsis,
  Pencil,
  Play,
  RotateCcw,
  Shield,
  ShieldCheck,
  Square,
  Trash2,
} from "lucide-react";

import {
  getManagedAgentPrimaryActionLabel,
  isManagedAgentActive,
} from "@/features/agents/lib/managedAgentControlActions";
import { AgentManagementMarker } from "@/features/agents/ui/OtherSetupAgentMarker";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import {
  agentCommunityAvailability,
  MANAGED_AGENT_PAIR_ACTION_LABELS,
  type ManagedAgentPairAction,
} from "@/features/agents/managedAgentRuntimeStatus";
import { truncateNpub } from "@/shared/lib/pubkey";
import type {
  ChannelMember,
  ManagedAgent,
  ManagedAgentRuntimeStatus,
  PresenceStatus,
} from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { useTranslation } from "@/i18n";

type MembersSidebarMemberCardProps = {
  canChangeRole: boolean;
  canModerate: boolean;
  canRemoveMember: boolean;
  isActionPending: boolean;
  isArchived: boolean;
  managedAgent?: ManagedAgent;
  managedAgentRuntime?: ManagedAgentRuntimeStatus;
  /** When set, the lifecycle menu item controls this agent+community pair
   * (local agents in a community context) instead of the whole agent. */
  pairAction?: ManagedAgentPairAction;
  member: ChannelMember;
  memberAvatarLabel: string;
  memberIsBot: boolean;
  memberLabel: string;
  moderationState?: MemberModerationState;
  onBan: (member: ChannelMember) => void;
  onChangeRole: (member: ChannelMember, role: string) => void;
  onEditRespondTo?: (agent: ManagedAgent) => void;
  onManagedAgentAction: (agent: ManagedAgent) => void;
  onOpenProfile?: (pubkey: string) => void;
  onRemoveMember: (member: ChannelMember) => void;
  onTimeout: (member: ChannelMember, expiresAtSecs: number) => void;
  onUnban: (member: ChannelMember) => void;
  onUntimeout: (member: ChannelMember) => void;
  onViewActivity?: (pubkey: string) => void;
  presenceStatus?: PresenceStatus | null;
  profileAvatarUrl?: string | null;
  profileOwnerPubkey?: string | null;
  viewerIsOwner: boolean;
};

/** Whether a member is currently banned / timed out (from the restriction read). */
export type MemberModerationState = {
  banned: boolean;
  timedOut: boolean;
};

/** Timeout durations offered in the member menu, in seconds. The `id` selects
 * the menu label at the render site, where every label is a literal `t()` key
 * (module constants cannot call `t()`). */
const TIMEOUT_PRESETS: { id: TimeoutPresetId; seconds: number }[] = [
  { id: "1-hour", seconds: 60 * 60 },
  { id: "24-hours", seconds: 24 * 60 * 60 },
  { id: "7-days", seconds: 7 * 24 * 60 * 60 },
];

type TimeoutPresetId = "1-hour" | "24-hours" | "7-days";

const MEMBER_ROW_INSET_DIVIDER_CLASS =
  "after:pointer-events-none after:absolute after:bottom-0 after:left-[3.75rem] after:right-0 after:h-px after:bg-border/60 after:content-[''] last:after:hidden";

/** Which badge text a member row shows: the bot marker, or the elevated role. */
function formatRoleLabel(member: ChannelMember, memberIsBot: boolean) {
  if (memberIsBot) {
    return "agent";
  }

  if (member.role === "owner" || member.role === "admin") {
    return member.role;
  }

  return null;
}

export function MembersSidebarMemberCard({
  canChangeRole,
  canModerate,
  canRemoveMember,
  isActionPending,
  isArchived,
  managedAgent,
  managedAgentRuntime,
  pairAction,
  member,
  memberAvatarLabel,
  memberIsBot,
  memberLabel,
  moderationState,
  onBan,
  onChangeRole,
  onEditRespondTo,
  onManagedAgentAction,
  onOpenProfile,
  onRemoveMember,
  onTimeout,
  onUnban,
  onUntimeout,
  onViewActivity,
  presenceStatus,
  profileAvatarUrl,
  profileOwnerPubkey,
  viewerIsOwner,
}: MembersSidebarMemberCardProps) {
  const { t } = useTranslation();
  const roleBadgeId = formatRoleLabel(member, memberIsBot);
  const roleLabel =
    roleBadgeId === "agent"
      ? t("channels.members.agent-label")
      : roleBadgeId === "owner"
        ? t("channels.members.role-owner")
        : roleBadgeId === "admin"
          ? t("channels.members.role-admin")
          : null;
  const respondToLabel = managedAgent
    ? managedAgent.respondTo === "anyone"
      ? t("channels.members.respond-anyone")
      : managedAgent.respondTo === "allowlist"
        ? t("channels.members.respond-selected-people", {
            total: managedAgent.respondToAllowlist.length,
          })
        : t("channels.members.respond-only-me")
    : null;
  const disabled = isActionPending || isArchived;
  const canViewActivity =
    memberIsBot &&
    (viewerIsOwner || managedAgent?.backend.type === "local") &&
    Boolean(onViewActivity);
  // Community ban/timeout applies to people, never bots, and never the community
  // owner (whom no moderator can restrict).
  const canModerateMember =
    canModerate && !memberIsBot && member.role !== "owner";
  const hasActions = memberIsBot
    ? Boolean(managedAgent) || canRemoveMember || canViewActivity
    : canRemoveMember || canChangeRole || canModerateMember;

  const memberIdentity = (
    <div className="pointer-events-none relative z-10 flex min-w-0 flex-1 items-center gap-3">
      <div className="relative shrink-0">
        <ProfileAvatar
          avatarUrl={profileAvatarUrl ?? null}
          className="h-8 w-8 text-xs shadow-none"
          iconClassName="h-4 w-4"
          label={memberAvatarLabel}
          shape={memberIsBot ? "squircle" : "circle"}
        />
        {presenceStatus ? (
          <span
            className="absolute -bottom-0.5 -right-0.5 flex h-3.5 w-3.5 items-center justify-center rounded-full bg-background"
            data-testid={`sidebar-member-presence-${member.pubkey}`}
          >
            <PresenceDot className="h-2 w-2" status={presenceStatus} />
          </span>
        ) : null}
      </div>
      <div className="min-w-0 flex-1">
        {memberIsBot ? (
          <div className="flex min-w-0 items-center gap-2">
            <div className="relative min-w-0 flex-1">
              <div className="flex min-w-0 items-center gap-2 transition-opacity duration-150 ease-out group-hover/member:opacity-0 group-focus-within/member:opacity-0">
                <span className="truncate text-sm font-medium tracking-tight">
                  {memberLabel}
                </span>
                <span className="inline-flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
                  <Bot aria-hidden="true" className="h-4 w-4" />
                  {roleLabel}
                </span>
              </div>
              <span className="absolute inset-0 flex items-center opacity-0 transition-opacity duration-150 ease-out group-hover/member:opacity-100 group-focus-within/member:opacity-100">
                <span className="truncate font-mono text-sm text-muted-foreground">
                  {truncateNpub(member.pubkey)}
                </span>
              </span>
            </div>
            <AgentManagementMarker
              pubkey={member.pubkey}
              ownerPubkey={profileOwnerPubkey}
              testId={`sidebar-member-agent-provenance-${member.pubkey}`}
            />
          </div>
        ) : (
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate text-sm font-medium tracking-tight">
              {memberLabel}
            </span>
            {roleLabel ? (
              <span className="inline-flex shrink-0 items-center text-xs text-muted-foreground">
                {roleLabel}
              </span>
            ) : null}
          </div>
        )}
        {managedAgentRuntime || managedAgent ? (
          <div className="mt-1 flex flex-wrap items-center gap-1.5">
            <Badge
              className="normal-case tracking-normal"
              data-testid={`sidebar-managed-agent-status-${member.pubkey}`}
              variant={
                managedAgentRuntime
                  ? agentCommunityAvailability(managedAgentRuntime) === "Here"
                    ? "default"
                    : "secondary"
                  : managedAgent && isManagedAgentActive(managedAgent)
                    ? "default"
                    : "secondary"
              }
            >
              {managedAgentRuntime
                ? agentCommunityAvailability(managedAgentRuntime)
                : managedAgent && isManagedAgentActive(managedAgent)
                  ? t("channels.members.status-running")
                  : t("channels.members.status-stopped")}
            </Badge>
            {managedAgent ? (
              <Badge
                className="normal-case tracking-normal"
                data-testid={`sidebar-managed-agent-respond-to-${member.pubkey}`}
                variant="outline"
              >
                {respondToLabel}
              </Badge>
            ) : null}
          </div>
        ) : null}
      </div>
    </div>
  );

  return (
    <div
      className={cn(
        "group/member relative isolate flex min-h-14 items-center gap-3 px-4 py-3.5 text-left transition-colors duration-150 ease-out hover:bg-muted/40 focus-within:bg-muted/40",
        MEMBER_ROW_INSET_DIVIDER_CLASS,
      )}
      data-testid={`sidebar-member-${member.pubkey}`}
    >
      {onOpenProfile ? (
        <button
          aria-label={t("channels.members.open-profile", { name: memberLabel })}
          className="absolute inset-0 z-0 cursor-pointer focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
          data-testid={`sidebar-member-open-profile-${member.pubkey}`}
          onClick={() => onOpenProfile(member.pubkey)}
          type="button"
        />
      ) : (
        <span aria-hidden="true" className="absolute inset-0 z-0" />
      )}
      {memberIdentity}
      {hasActions ? (
        <MemberActionsMenu
          canChangeRole={canChangeRole}
          canModerateMember={canModerateMember}
          canRemoveMember={canRemoveMember}
          canViewActivity={canViewActivity}
          disabled={disabled}
          managedAgent={managedAgent}
          member={member}
          memberIsBot={memberIsBot}
          moderationState={moderationState}
          onBan={onBan}
          onChangeRole={onChangeRole}
          onEditRespondTo={onEditRespondTo}
          onManagedAgentAction={onManagedAgentAction}
          onRemoveMember={onRemoveMember}
          onTimeout={onTimeout}
          onUnban={onUnban}
          onUntimeout={onUntimeout}
          onViewActivity={onViewActivity}
          pairAction={pairAction}
          availability={presenceStatus ?? undefined}
        />
      ) : null}
    </div>
  );
}

const PEOPLE_ROLES = ["admin", "member", "guest"] as const;

function MemberActionsMenu({
  availability,
  canChangeRole,
  canModerateMember,
  canRemoveMember,
  canViewActivity,
  disabled,
  managedAgent,
  member,
  memberIsBot,
  moderationState,
  onBan,
  onChangeRole,
  onEditRespondTo,
  onManagedAgentAction,
  onRemoveMember,
  onTimeout,
  onUnban,
  onUntimeout,
  onViewActivity,
  pairAction,
}: {
  canChangeRole: boolean;
  availability: PresenceStatus | undefined;
  canModerateMember: boolean;
  canRemoveMember: boolean;
  canViewActivity: boolean;
  disabled: boolean;
  managedAgent?: ManagedAgent;
  member: ChannelMember;
  memberIsBot: boolean;
  moderationState?: MemberModerationState;
  onBan: (member: ChannelMember) => void;
  onChangeRole: (member: ChannelMember, role: string) => void;
  onEditRespondTo?: (agent: ManagedAgent) => void;
  onManagedAgentAction: (agent: ManagedAgent) => void;
  onRemoveMember: (member: ChannelMember) => void;
  onTimeout: (member: ChannelMember, expiresAtSecs: number) => void;
  onUnban: (member: ChannelMember) => void;
  onUntimeout: (member: ChannelMember) => void;
  onViewActivity?: (pubkey: string) => void;
  pairAction?: ManagedAgentPairAction;
}) {
  const { t } = useTranslation();
  const roleOptionLabels: Record<(typeof PEOPLE_ROLES)[number], string> = {
    admin: t("channels.members.role-option-admin"),
    member: t("channels.members.role-option-member"),
    guest: t("channels.members.role-option-guest"),
  };
  const timeoutLabels: Record<TimeoutPresetId, string> = {
    "1-hour": t("channels.members.timeout-1-hour"),
    "24-hours": t("channels.members.timeout-24-hours"),
    "7-days": t("channels.members.timeout-7-days"),
  };
  const showChangeRole =
    canChangeRole && !memberIsBot && member.role !== "owner";
  const isBanned = moderationState?.banned ?? false;
  const isTimedOut = moderationState?.timedOut ?? false;

  const startBlockReason = managedAgent
    ? agentPresenceStartBlockReason(
        pairAction ? pairAction === "stop" : isManagedAgentActive(managedAgent),
        availability,
      )
    : undefined;

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <button
          className="invisible relative z-20 flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground group-hover/member:visible hover:bg-muted hover:text-foreground data-[state=open]:visible"
          data-testid={`sidebar-member-menu-${member.pubkey}`}
          type="button"
        >
          <Ellipsis className="h-4 w-4" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        onCloseAutoFocus={(event) => event.preventDefault()}
      >
        {canViewActivity ? (
          <DropdownMenuItem
            data-testid={`sidebar-view-activity-${member.pubkey}`}
            onClick={() => onViewActivity?.(member.pubkey)}
          >
            <Activity className="h-4 w-4" />
            {t("channels.members.view-activity")}
          </DropdownMenuItem>
        ) : null}
        {memberIsBot && managedAgent ? (
          <>
            {canViewActivity ? <DropdownMenuSeparator /> : null}
            <DropdownMenuItem
              data-testid={`sidebar-agent-action-${member.pubkey}`}
              disabled={disabled || Boolean(startBlockReason)}
              title={startBlockReason}
              onClick={() => {
                if (!startBlockReason) onManagedAgentAction(managedAgent);
              }}
            >
              {pairAction
                ? getPairActionIcon(pairAction)
                : getManagedAgentActionIcon(managedAgent)}
              {pairAction
                ? MANAGED_AGENT_PAIR_ACTION_LABELS[pairAction]
                : getManagedAgentPrimaryActionLabel(managedAgent)}
            </DropdownMenuItem>
            {onEditRespondTo ? (
              <DropdownMenuItem
                data-testid={`sidebar-edit-respond-to-${member.pubkey}`}
                disabled={disabled}
                onClick={() => onEditRespondTo(managedAgent)}
              >
                <Pencil className="h-4 w-4" />
                {t("channels.members.manage-access")}
              </DropdownMenuItem>
            ) : null}
            {canRemoveMember || showChangeRole ? (
              <DropdownMenuSeparator />
            ) : null}
          </>
        ) : null}
        {showChangeRole ? (
          <DropdownMenuSub>
            <DropdownMenuSubTrigger
              data-testid={`sidebar-change-role-${member.pubkey}`}
              disabled={disabled}
            >
              <Shield className="h-4 w-4" />
              {t("channels.members.change-role")}
            </DropdownMenuSubTrigger>
            <DropdownMenuSubContent>
              {PEOPLE_ROLES.map((role) => (
                <DropdownMenuItem
                  data-testid={`sidebar-role-${role}-${member.pubkey}`}
                  disabled={disabled || member.role === role}
                  key={role}
                  onClick={() => onChangeRole(member, role)}
                >
                  {roleOptionLabels[role]}
                  {member.role === role
                    ? t("channels.members.current-suffix")
                    : ""}
                </DropdownMenuItem>
              ))}
            </DropdownMenuSubContent>
          </DropdownMenuSub>
        ) : null}
        {canRemoveMember ? (
          <>
            {showChangeRole ? <DropdownMenuSeparator /> : null}
            <DropdownMenuItem
              className="text-destructive focus:text-destructive"
              data-testid={`sidebar-remove-member-${member.pubkey}`}
              disabled={disabled}
              onClick={() => onRemoveMember(member)}
            >
              <Trash2 className="h-4 w-4" />
              {t("channels.members.remove-from-channel")}
            </DropdownMenuItem>
          </>
        ) : null}
        {canModerateMember ? (
          <>
            {canRemoveMember || showChangeRole ? (
              <DropdownMenuSeparator />
            ) : null}
            {isTimedOut ? (
              <DropdownMenuItem
                data-testid={`sidebar-untimeout-${member.pubkey}`}
                disabled={disabled}
                onClick={() => onUntimeout(member)}
              >
                <ShieldCheck className="h-4 w-4" />
                {t("channels.members.lift-timeout")}
              </DropdownMenuItem>
            ) : (
              <DropdownMenuSub>
                <DropdownMenuSubTrigger
                  data-testid={`sidebar-timeout-${member.pubkey}`}
                  disabled={disabled}
                >
                  <Clock className="h-4 w-4" />
                  {t("channels.members.time-out")}
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent>
                  {TIMEOUT_PRESETS.map((preset) => (
                    <DropdownMenuItem
                      data-testid={`sidebar-timeout-${preset.seconds}-${member.pubkey}`}
                      disabled={disabled}
                      key={preset.seconds}
                      onClick={() =>
                        onTimeout(
                          member,
                          Math.floor(Date.now() / 1000) + preset.seconds,
                        )
                      }
                    >
                      {timeoutLabels[preset.id]}
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuSubContent>
              </DropdownMenuSub>
            )}
            {isBanned ? (
              <DropdownMenuItem
                data-testid={`sidebar-unban-${member.pubkey}`}
                disabled={disabled}
                onClick={() => onUnban(member)}
              >
                <CircleSlash className="h-4 w-4" />
                {t("channels.members.lift-ban")}
              </DropdownMenuItem>
            ) : (
              <DropdownMenuItem
                className="text-destructive focus:text-destructive"
                data-testid={`sidebar-ban-${member.pubkey}`}
                disabled={disabled}
                onClick={() => onBan(member)}
              >
                <Ban className="h-4 w-4" />
                {t("channels.members.ban-from-community")}
              </DropdownMenuItem>
            )}
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function getPairActionIcon(action: ManagedAgentPairAction) {
  if (action === "stop") return <Square className="h-4 w-4" />;
  if (action === "restart") return <RotateCcw className="h-4 w-4" />;
  return <Play className="h-4 w-4" />;
}

function getManagedAgentActionIcon(agent: ManagedAgent) {
  if (isManagedAgentActive(agent)) {
    return <Square className="h-4 w-4" />;
  }

  if (agent.backend.type === "local" && agent.status === "stopped") {
    return <RotateCcw className="h-4 w-4" />;
  }

  return <Play className="h-4 w-4" />;
}
