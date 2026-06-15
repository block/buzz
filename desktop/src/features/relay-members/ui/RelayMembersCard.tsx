import * as React from "react";
import { MoreHorizontal, Plus, Shield, ShieldCheck, User } from "lucide-react";
import { toast } from "sonner";

import { useUsersBatchQuery } from "@/features/profile/hooks";
import { truncatePubkey } from "@/features/profile/lib/identity";
import {
  useChangeRelayMemberRoleMutation,
  useMyRelayMembershipQuery,
  useRelayMembersQuery,
} from "@/features/relay-members/hooks";
import type { RelayMember, RelayMemberRole } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { AddMemberDialog } from "./AddMemberDialog";
import { ConfirmRemoveDialog } from "./ConfirmRemoveDialog";

function formatRelativeDate(dateString: string): string {
  const date = new Date(dateString);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffMins = Math.floor(diffMs / 60_000);
  if (diffMins < 1) return "just now";
  if (diffMins < 60) return `${diffMins}m ago`;
  const diffHours = Math.floor(diffMins / 60);
  if (diffHours < 24) return `${diffHours}h ago`;
  const diffDays = Math.floor(diffHours / 24);
  if (diffDays < 30) return `${diffDays}d ago`;
  return date.toLocaleDateString();
}

function RoleBadge({ role }: { role: RelayMemberRole }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium",
        role === "owner" && "bg-primary/10 text-primary",
        role === "admin" && "bg-blue-500/10 text-blue-500",
        role === "member" && "bg-muted text-muted-foreground",
      )}
    >
      {role}
    </span>
  );
}

function RoleIcon({ role }: { role: RelayMemberRole }) {
  switch (role) {
    case "owner":
      return <ShieldCheck className="h-4 w-4 shrink-0 text-primary" />;
    case "admin":
      return <Shield className="h-4 w-4 shrink-0 text-blue-500" />;
    default:
      return <User className="h-4 w-4 shrink-0 text-muted-foreground" />;
  }
}

function MemberRow({
  member,
  displayName,
  currentPubkey,
  viewerRole,
  onRemove,
  onChangeRole,
}: {
  member: RelayMember;
  displayName: string | null;
  currentPubkey?: string;
  viewerRole: RelayMemberRole | null;
  onRemove: (member: RelayMember) => void;
  onChangeRole: (pubkey: string, newRole: string) => void;
}) {
  const isSelf = currentPubkey?.toLowerCase() === member.pubkey.toLowerCase();
  const isOwner = viewerRole === "owner";
  const isAdmin = viewerRole === "admin";

  const canRemove =
    !isSelf &&
    ((isAdmin && member.role === "member") ||
      (isOwner && member.role !== "owner"));

  const canChangeRole = !isSelf && isOwner && member.role !== "owner";

  const showActions = canRemove || canChangeRole;

  return (
    <div
      className="flex items-center justify-between gap-3 rounded-lg border border-border/60 bg-background/60 px-3 py-2.5"
      data-testid={`relay-member-row-${member.pubkey}`}
    >
      <div className="flex min-w-0 items-center gap-3">
        <RoleIcon role={member.role} />
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-medium">
              {displayName || truncatePubkey(member.pubkey)}
            </span>
            <RoleBadge role={member.role} />
            {isSelf ? (
              <span className="text-xs text-muted-foreground">(you)</span>
            ) : null}
          </div>
          <p className="text-xs text-muted-foreground">
            Joined {formatRelativeDate(member.createdAt)}
          </p>
        </div>
      </div>

      {showActions ? (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              data-testid={`relay-member-actions-${member.pubkey}`}
              size="sm"
              variant="ghost"
            >
              <MoreHorizontal className="h-4 w-4" />
              <span className="sr-only">Actions</span>
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {canChangeRole ? (
              <>
                {member.role === "member" ? (
                  <DropdownMenuItem
                    onClick={() => onChangeRole(member.pubkey, "admin")}
                  >
                    Make Admin
                  </DropdownMenuItem>
                ) : null}
                {member.role === "admin" ? (
                  <DropdownMenuItem
                    onClick={() => onChangeRole(member.pubkey, "member")}
                  >
                    Make Member
                  </DropdownMenuItem>
                ) : null}
              </>
            ) : null}
            {canRemove && canChangeRole ? <DropdownMenuSeparator /> : null}
            {canRemove ? (
              <DropdownMenuItem
                className="text-destructive focus:text-destructive"
                onClick={() => onRemove(member)}
              >
                Remove
              </DropdownMenuItem>
            ) : null}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </div>
  );
}

const ROLE_ORDER: Record<string, number> = { owner: 0, admin: 1, member: 2 };

export function RelayMembersCard({
  currentPubkey,
}: {
  currentPubkey?: string;
}) {
  const membersQuery = useRelayMembersQuery();
  const myMembershipQuery = useMyRelayMembershipQuery();
  const changeRoleMutation = useChangeRelayMemberRoleMutation();

  const [addDialogOpen, setAddDialogOpen] = React.useState(false);
  const [removeTarget, setRemoveTarget] = React.useState<RelayMember | null>(
    null,
  );

  const members = React.useMemo(() => {
    const raw = membersQuery.data ?? [];
    return [...raw].sort(
      (a, b) => (ROLE_ORDER[a.role] ?? 3) - (ROLE_ORDER[b.role] ?? 3),
    );
  }, [membersQuery.data]);
  const myMembership = myMembershipQuery.data;
  const isOwner = myMembership?.role === "owner";
  const isAdmin = myMembership?.role === "admin";
  const canManage = isOwner || isAdmin;

  const memberPubkeys = React.useMemo(
    () => members.map((m) => m.pubkey),
    [members],
  );
  const profilesQuery = useUsersBatchQuery(memberPubkeys);
  const profiles = profilesQuery.data?.profiles ?? {};

  function handleChangeRole(pubkey: string, newRole: string) {
    changeRoleMutation.mutate(
      { pubkey, newRole },
      {
        onSuccess: () => {
          toast.success(`Role changed to ${newRole}`);
        },
        onError: (error) => {
          toast.error(
            error instanceof Error ? error.message : "Failed to change role",
          );
        },
      },
    );
  }

  return (
    <section className="min-w-0" data-testid="settings-relay-members">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <Shield className="h-4 w-4 text-muted-foreground" />
            <h2 className="text-sm font-semibold tracking-tight">
              Relay Members
            </h2>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            Manage who has access to this relay.
          </p>
        </div>

        {canManage ? (
          <Button
            data-testid="add-relay-member"
            onClick={() => setAddDialogOpen(true)}
            size="sm"
          >
            <Plus className="h-4 w-4" />
            Add Member
          </Button>
        ) : null}
      </div>

      {membersQuery.error instanceof Error ? (
        <p className="mt-3 rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {membersQuery.error.message}
        </p>
      ) : null}

      {membersQuery.isLoading ? (
        <p className="mt-4 text-sm text-muted-foreground">Loading members...</p>
      ) : members.length > 0 ? (
        <div className="mt-4 space-y-2">
          {members.map((member) => (
            <MemberRow
              currentPubkey={currentPubkey}
              displayName={
                profiles[member.pubkey.toLowerCase()]?.displayName ?? null
              }
              key={member.pubkey}
              member={member}
              onChangeRole={handleChangeRole}
              onRemove={setRemoveTarget}
              viewerRole={myMembership?.role ?? null}
            />
          ))}
        </div>
      ) : membersQuery.isSuccess ? (
        <p className="mt-4 text-sm text-muted-foreground">No members yet.</p>
      ) : null}

      <AddMemberDialog
        isOwner={isOwner}
        onOpenChange={setAddDialogOpen}
        open={addDialogOpen}
      />
      <ConfirmRemoveDialog
        member={removeTarget}
        displayName={
          removeTarget
            ? (profiles[removeTarget.pubkey.toLowerCase()]?.displayName ?? null)
            : null
        }
        onOpenChange={(open) => {
          if (!open) setRemoveTarget(null);
        }}
        open={removeTarget !== null}
      />
    </section>
  );
}
