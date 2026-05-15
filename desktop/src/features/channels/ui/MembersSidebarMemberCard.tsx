import {
  Activity,
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
import { Badge } from "@/shared/ui/badge";

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
  onRemoveMember: (member: ChannelMember) => void;
  onViewActivity?: (pubkey: string) => void;
  presenceStatus?: PresenceStatus | null;
  profileAvatarUrl?: string | null;
};

function formatRoleLabel(member: ChannelMember, memberIsBot: boolean) {
  if (memberIsBot) {
    return "Bot";
  }

  return `${member.role[0]?.toUpperCase() ?? ""}${member.role.slice(1)}`;
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

  return (
    <div
      className="group flex items-center justify-between gap-3 rounded-lg px-3 py-2 transition-colors hover:bg-muted/40"
      data-testid={`sidebar-member-${member.pubkey}`}
    >
      <div className="flex min-w-0 items-center gap-3">
        <div className="relative shrink-0">
          <ProfileAvatar
            avatarUrl={profileAvatarUrl ?? null}
            className="h-9 w-9 rounded-full text-[11px] shadow-none"
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
        <div className="min-w-0">
          <div className="flex items-center gap-1.5">
            <p className="truncate text-sm font-medium leading-5">
              {memberLabel}
            </p>
            <Badge className="shrink-0" variant="secondary">
              {roleLabel}
            </Badge>
            {managedAgent ? (
              <>
                <Badge
                  className="shrink-0"
                  data-testid={`sidebar-managed-agent-status-${member.pubkey}`}
                  variant="secondary"
                >
                  {formatManagedAgentStatus(managedAgent)}
                </Badge>
                <Badge
                  className="shrink-0"
                  data-testid={`sidebar-managed-agent-respond-to-${member.pubkey}`}
                  variant="outline"
                >
                  {formatRespondToLabel(managedAgent)}
                </Badge>
              </>
            ) : null}
          </div>
          <p className="truncate font-mono text-[10px] text-muted-foreground/50">
            {truncatePubkey(member.pubkey)}
          </p>
        </div>
      </div>
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
          className="invisible flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground group-hover:visible hover:bg-muted hover:text-foreground data-[state=open]:visible"
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
