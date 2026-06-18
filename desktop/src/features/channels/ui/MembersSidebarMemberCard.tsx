import {
  Activity,
  Bot,
  Ellipsis,
  Pencil,
  Play,
  RotateCcw,
  Shield,
  Square,
  Trash2,
} from "lucide-react";

import {
  getManagedAgentPrimaryActionLabel,
  isManagedAgentActive,
} from "@/features/agents/lib/managedAgentControlActions";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import { truncatePubkey } from "@/features/profile/lib/identity";
import type {
  ChannelMember,
  ManagedAgent,
  PresenceStatus,
} from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
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

type MembersSidebarMemberCardProps = {
  canChangeRole: boolean;
  canRemoveMember: boolean;
  isActionPending: boolean;
  isArchived: boolean;
  managedAgent?: ManagedAgent;
  member: ChannelMember;
  memberAvatarLabel: string;
  memberIsBot: boolean;
  memberLabel: string;
  onChangeRole: (member: ChannelMember, role: string) => void;
  onEditRespondTo?: (agent: ManagedAgent) => void;
  onManagedAgentAction: (agent: ManagedAgent) => void;
  onOpenProfile?: (pubkey: string) => void;
  onRemoveMember: (member: ChannelMember) => void;
  onViewActivity?: (pubkey: string) => void;
  presenceStatus?: PresenceStatus | null;
  profileAvatarUrl?: string | null;
};

const MEMBER_ROW_INSET_DIVIDER_CLASS =
  "after:pointer-events-none after:absolute after:bottom-0 after:left-[3.75rem] after:right-0 after:h-px after:bg-border/60 after:content-[''] last:after:hidden";

function formatRoleLabel(member: ChannelMember, memberIsBot: boolean) {
  if (memberIsBot) {
    return "agent";
  }

  if (member.role === "owner" || member.role === "admin") {
    return member.role;
  }

  return null;
}

function formatRespondToLabel(agent: ManagedAgent) {
  switch (agent.respondTo) {
    case "anyone":
      return "Anyone";
    case "allowlist":
      return `Allowlist (${agent.respondToAllowlist.length})`;
    default:
      return "Owner only";
  }
}

function formatManagedAgentStatus(agent: ManagedAgent) {
  switch (agent.status) {
    case "running":
      return "Running";
    case "stopped":
      return "Stopped";
    case "deployed":
      return "Deployed";
    case "not_deployed":
      return "Not deployed";
  }
}

export function MembersSidebarMemberCard({
  canChangeRole,
  canRemoveMember,
  isActionPending,
  isArchived,
  managedAgent,
  member,
  memberAvatarLabel,
  memberIsBot,
  memberLabel,
  onChangeRole,
  onEditRespondTo,
  onManagedAgentAction,
  onOpenProfile,
  onRemoveMember,
  onViewActivity,
  presenceStatus,
  profileAvatarUrl,
}: MembersSidebarMemberCardProps) {
  const roleLabel = formatRoleLabel(member, memberIsBot);
  const disabled = isActionPending || isArchived;
  const canViewActivity =
    memberIsBot &&
    managedAgent?.backend.type === "local" &&
    Boolean(onViewActivity);
  const hasActions = memberIsBot
    ? Boolean(managedAgent) || canRemoveMember || canViewActivity
    : canRemoveMember || canChangeRole;

  const memberIdentity = (
    <div className="pointer-events-none relative z-10 flex min-w-0 flex-1 items-center gap-3">
      <div className="relative shrink-0">
        <ProfileAvatar
          avatarUrl={profileAvatarUrl ?? null}
          className="h-8 w-8 text-xs shadow-none"
          iconClassName="h-4 w-4"
          label={memberAvatarLabel}
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
          <div className="relative min-w-0">
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
                {truncatePubkey(member.pubkey)}
              </span>
            </span>
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
        {managedAgent ? (
          <span
            className="sr-only"
            data-testid={`sidebar-managed-agent-status-${member.pubkey}`}
          >
            {formatManagedAgentStatus(managedAgent)}
          </span>
        ) : null}
        {managedAgent ? (
          <span
            className="sr-only"
            data-testid={`sidebar-managed-agent-respond-to-${member.pubkey}`}
          >
            {formatRespondToLabel(managedAgent)}
          </span>
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
          aria-label={`Open profile for ${memberLabel}`}
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
          canRemoveMember={canRemoveMember}
          canViewActivity={canViewActivity}
          disabled={disabled}
          managedAgent={managedAgent}
          member={member}
          memberIsBot={memberIsBot}
          onChangeRole={onChangeRole}
          onEditRespondTo={onEditRespondTo}
          onManagedAgentAction={onManagedAgentAction}
          onRemoveMember={onRemoveMember}
          onViewActivity={onViewActivity}
        />
      ) : null}
    </div>
  );
}

const PEOPLE_ROLES = ["admin", "member", "guest"] as const;

function MemberActionsMenu({
  canChangeRole,
  canRemoveMember,
  canViewActivity,
  disabled,
  managedAgent,
  member,
  memberIsBot,
  onChangeRole,
  onEditRespondTo,
  onManagedAgentAction,
  onRemoveMember,
  onViewActivity,
}: {
  canChangeRole: boolean;
  canRemoveMember: boolean;
  canViewActivity: boolean;
  disabled: boolean;
  managedAgent?: ManagedAgent;
  member: ChannelMember;
  memberIsBot: boolean;
  onChangeRole: (member: ChannelMember, role: string) => void;
  onEditRespondTo?: (agent: ManagedAgent) => void;
  onManagedAgentAction: (agent: ManagedAgent) => void;
  onRemoveMember: (member: ChannelMember) => void;
  onViewActivity?: (pubkey: string) => void;
}) {
  const showChangeRole =
    canChangeRole && !memberIsBot && member.role !== "owner";

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
            View activity
          </DropdownMenuItem>
        ) : null}
        {memberIsBot && managedAgent ? (
          <>
            {canViewActivity ? <DropdownMenuSeparator /> : null}
            <DropdownMenuItem
              data-testid={`sidebar-agent-action-${member.pubkey}`}
              disabled={disabled}
              onClick={() => onManagedAgentAction(managedAgent)}
            >
              {getManagedAgentActionIcon(managedAgent)}
              {getManagedAgentPrimaryActionLabel(managedAgent)}
            </DropdownMenuItem>
            {onEditRespondTo ? (
              <DropdownMenuItem
                data-testid={`sidebar-edit-respond-to-${member.pubkey}`}
                disabled={disabled}
                onClick={() => onEditRespondTo(managedAgent)}
              >
                <Pencil className="h-4 w-4" />
                Edit respond-to...
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
              Change role
            </DropdownMenuSubTrigger>
            <DropdownMenuSubContent>
              {PEOPLE_ROLES.map((role) => (
                <DropdownMenuItem
                  data-testid={`sidebar-role-${role}-${member.pubkey}`}
                  disabled={disabled || member.role === role}
                  key={role}
                  onClick={() => onChangeRole(member, role)}
                >
                  {role[0]?.toUpperCase()}
                  {role.slice(1)}
                  {member.role === role ? " (current)" : ""}
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
              Remove from channel
            </DropdownMenuItem>
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
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
