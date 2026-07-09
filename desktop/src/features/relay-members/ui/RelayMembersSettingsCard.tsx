import {
  ArrowDown,
  ArrowUp,
  ChevronDown,
  Crown,
  Search,
  Shield,
  Trash2,
  UserPlus,
} from "lucide-react";
import { nip19 } from "nostr-tools";
import * as React from "react";
import { toast } from "sonner";

import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import {
  useAddRelayMemberMutation,
  useChangeRelayMemberRoleMutation,
  useMyRelayMembershipQuery,
  useRelayMembersQuery,
  useRemoveRelayMemberMutation,
} from "@/features/relay-members/hooks";
import type { RelayMember, RelayMemberRole } from "@/shared/api/types";
import type { UserProfileSummary } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Input } from "@/shared/ui/input";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";
import { WorkspaceIconSettingsCard } from "@/features/workspaces/ui/WorkspaceIconSettingsCard";
import { VirtualizedList } from "@/shared/ui/VirtualizedList";
import { InviteLinkSection } from "./InviteLinkSection";

type AssignableRelayRole = Exclude<RelayMemberRole, "owner">;

function normalizeRelayPubkeyInput(value: string): string {
  const trimmed = value.trim();
  const withoutPrefix = trimmed.toLowerCase().startsWith("nostr:")
    ? trimmed.slice("nostr:".length)
    : trimmed;
  const normalized = normalizePubkey(withoutPrefix);
  if (normalized.startsWith("npub1")) {
    try {
      const decoded = nip19.decode(normalized);
      if (decoded.type === "npub" && typeof decoded.data === "string") {
        return decoded.data.toLowerCase();
      }
    } catch {
      // fall through to return the normalized npub so validation can flag it
    }
  }
  return normalized;
}

function isValidHexPubkey(value: string): boolean {
  return /^[0-9a-f]{64}$/.test(value);
}

function formatDisplayName(member: RelayMember, displayName?: string | null) {
  return displayName?.trim() || truncatePubkey(member.pubkey);
}

function npubFromPubkey(pubkey: string): string | null {
  try {
    return nip19.npubEncode(pubkey);
  } catch {
    return null;
  }
}

function formatDate(dateString: string): string {
  return new Date(dateString).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

function roleBadgeClass(role: RelayMemberRole): string {
  switch (role) {
    case "owner":
      return "bg-amber-500/10 text-amber-500";
    case "admin":
      return "bg-blue-500/10 text-blue-500";
    case "member":
      return "bg-muted text-muted-foreground";
  }
}

function RoleBadge({ role }: { role: RelayMemberRole }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium capitalize",
        roleBadgeClass(role),
      )}
    >
      {role}
    </span>
  );
}

function RelayMemberRow({
  currentRole,
  currentPubkey,
  profile,
  member,
}: {
  currentRole: RelayMemberRole;
  currentPubkey?: string;
  profile?: UserProfileSummary;
  member: RelayMember;
}) {
  const removeMutation = useRemoveRelayMemberMutation();
  const changeRoleMutation = useChangeRelayMemberRoleMutation();
  const isSelf = currentPubkey
    ? normalizePubkey(currentPubkey) === normalizePubkey(member.pubkey)
    : false;
  const isBusy = removeMutation.isPending || changeRoleMutation.isPending;
  const canRemove =
    !isSelf &&
    member.role !== "owner" &&
    (currentRole === "owner" || member.role === "member");
  const canPromote = currentRole === "owner" && member.role === "member";
  const canDemote = currentRole === "owner" && member.role === "admin";

  const displayName = formatDisplayName(member, profile?.displayName);
  const npub = npubFromPubkey(member.pubkey);

  async function mutateWithToast(
    action: () => Promise<unknown>,
    success: string,
  ) {
    try {
      await action();
      toast.success(success);
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Relay membership update failed",
      );
    }
  }

  return (
    <div
      className="flex items-center gap-3 rounded-lg border border-border/60 bg-background/60 px-3 py-2.5"
      data-testid={`relay-member-row-${member.pubkey}`}
    >
      <ProfileAvatar
        avatarUrl={profile?.avatarUrl ?? null}
        className="h-9 w-9 text-xs"
        label={displayName}
      />
      <div className="min-w-0 flex-1 space-y-0.5">
        <div className="flex flex-wrap items-center gap-1.5">
          {member.role === "owner" ? (
            <Crown className="h-4 w-4 text-amber-500" />
          ) : null}
          {member.role === "admin" ? (
            <Shield className="h-4 w-4 text-blue-500" />
          ) : null}
          <span className="truncate text-sm font-medium">{displayName}</span>
          {isSelf ? (
            <span className="text-xs text-muted-foreground">you</span>
          ) : null}
          <RoleBadge role={member.role} />
        </div>
        <p className="truncate font-mono text-xs text-muted-foreground">
          {npub ?? member.pubkey}
        </p>
        <p className="text-xs text-muted-foreground">
          Added {formatDate(member.createdAt)}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-0.5">
        {canPromote ? (
          <Button
            data-testid={`promote-relay-member-${member.pubkey}`}
            disabled={isBusy}
            onClick={() =>
              void mutateWithToast(
                () =>
                  changeRoleMutation.mutateAsync({
                    pubkey: member.pubkey,
                    role: "admin",
                  }),
                "Promoted relay admin",
              )
            }
            size="icon"
            title="Promote to admin"
            variant="ghost"
          >
            <ArrowUp className="h-4 w-4" />
            <span className="sr-only">Promote to admin</span>
          </Button>
        ) : null}
        {canDemote ? (
          <Button
            data-testid={`demote-relay-member-${member.pubkey}`}
            disabled={isBusy}
            onClick={() =>
              void mutateWithToast(
                () =>
                  changeRoleMutation.mutateAsync({
                    pubkey: member.pubkey,
                    role: "member",
                  }),
                "Demoted to relay member",
              )
            }
            size="icon"
            title="Demote to member"
            variant="ghost"
          >
            <ArrowDown className="h-4 w-4" />
            <span className="sr-only">Demote to member</span>
          </Button>
        ) : null}
        {canRemove ? (
          <Button
            className="text-muted-foreground hover:text-destructive"
            data-testid={`remove-relay-member-${member.pubkey}`}
            disabled={isBusy}
            onClick={() =>
              void mutateWithToast(
                () => removeMutation.mutateAsync(member.pubkey),
                "Removed relay member",
              )
            }
            size="icon"
            title="Remove from relay"
            variant="ghost"
          >
            <Trash2 className="h-4 w-4" />
            <span className="sr-only">Remove from relay</span>
          </Button>
        ) : null}
      </div>
    </div>
  );
}

const ROLE_OPTIONS: {
  description: string;
  label: string;
  value: AssignableRelayRole;
}[] = [
  {
    value: "member",
    label: "Member",
    description: "Can connect and participate",
  },
  {
    value: "admin",
    label: "Admin",
    description: "Can also invite members",
  },
];

export function RelayMembersSettingsCard({
  currentPubkey,
}: {
  currentPubkey?: string;
}) {
  const myMembershipQuery = useMyRelayMembershipQuery();
  const currentRole = myMembershipQuery.data?.role ?? null;
  const canManageRelay = currentRole === "owner" || currentRole === "admin";
  const membersQuery = useRelayMembersQuery(canManageRelay);
  const members = React.useMemo(
    () => membersQuery.data ?? [],
    [membersQuery.data],
  );
  const profilesQuery = useUsersBatchQuery(
    members.map((member) => member.pubkey),
    {
      enabled: canManageRelay && members.length > 0,
    },
  );
  const profiles = profilesQuery.data?.profiles;
  const addMutation = useAddRelayMemberMutation();
  const [pubkeyInput, setPubkeyInput] = React.useState("");
  const [role, setRole] = React.useState<AssignableRelayRole>("member");
  const [search, setSearch] = React.useState("");

  const filteredMembers = React.useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return members;
    return members.filter((member) => {
      const profile = profiles?.[normalizePubkey(member.pubkey)];
      const displayName = profile?.displayName?.toLowerCase() ?? "";
      const nip05 = profile?.nip05Handle?.toLowerCase() ?? "";
      const npub = npubFromPubkey(member.pubkey)?.toLowerCase() ?? "";
      return (
        displayName.includes(q) ||
        nip05.includes(q) ||
        npub.includes(q) ||
        member.pubkey.toLowerCase().includes(q) ||
        member.role.includes(q)
      );
    });
  }, [members, profiles, search]);

  if (myMembershipQuery.isLoading) {
    return (
      <section className="min-w-0" data-testid="settings-relay-members">
        <p className="text-sm text-muted-foreground">
          Checking relay permissions…
        </p>
      </section>
    );
  }

  if (!canManageRelay || !currentRole) {
    return null;
  }

  const normalizedInput = normalizeRelayPubkeyInput(pubkeyInput);
  const canGrantAdmin = currentRole === "owner";
  const canAdd =
    isValidHexPubkey(normalizedInput) &&
    !addMutation.isPending &&
    (role === "member" || canGrantAdmin);

  async function handleAddMember(event: React.FormEvent) {
    event.preventDefault();
    if (!canAdd) return;
    try {
      await addMutation.mutateAsync({ pubkey: normalizedInput, role });
      toast.success(
        role === "admin" ? "Added relay admin" : "Added relay member",
      );
      setPubkeyInput("");
      setRole("member");
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to add relay member",
      );
    }
  }

  return (
    <section className="min-w-0" data-testid="settings-relay-members">
      <SettingsSectionHeader
        title="Relay Access"
        description={
          <>
            Manage who can connect to this relay. Owners can invite admins or
            members; admins can invite members.
          </>
        }
      />

      <div className="space-y-6">
        <WorkspaceIconSettingsCard />
        <form className="space-y-1.5" onSubmit={handleAddMember}>
          <label className="text-sm font-medium" htmlFor="relay-member-pubkey">
            Invite a person
          </label>
          <div className="flex flex-col gap-2 sm:flex-row">
            <Input
              className="flex-1"
              id="relay-member-pubkey"
              onChange={(event) => setPubkeyInput(event.target.value)}
              placeholder="npub1… or 64-char hex pubkey"
              value={pubkeyInput}
            />
            <div className="inline-flex shrink-0 self-stretch sm:self-auto">
              <Button
                className={cn(
                  canGrantAdmin && "rounded-r-none",
                  "flex-1 sm:flex-none",
                )}
                data-testid="invite-relay-member"
                disabled={!canAdd}
                type="submit"
              >
                <UserPlus className="h-4 w-4" />
                <span className="inline-block min-w-[3.5rem] text-left capitalize">
                  {role}
                </span>
              </Button>
              {canGrantAdmin ? (
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button
                      aria-label="Choose invite role"
                      className="rounded-l-none border-l border-primary-foreground/20 px-2"
                      data-testid="relay-invite-role-trigger"
                      disabled={addMutation.isPending}
                      type="button"
                    >
                      <ChevronDown className="h-4 w-4" />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end" className="w-56">
                    <DropdownMenuLabel>Invite as</DropdownMenuLabel>
                    <DropdownMenuSeparator />
                    <DropdownMenuRadioGroup
                      onValueChange={(value) =>
                        setRole(value as AssignableRelayRole)
                      }
                      value={role}
                    >
                      {ROLE_OPTIONS.map((option) => (
                        <DropdownMenuRadioItem
                          data-testid={`relay-invite-role-${option.value}`}
                          key={option.value}
                          value={option.value}
                        >
                          <div className="flex flex-col">
                            <span className="text-sm font-medium">
                              {option.label}
                            </span>
                            <span className="text-sm font-normal text-muted-foreground">
                              {option.description}
                            </span>
                          </div>
                        </DropdownMenuRadioItem>
                      ))}
                    </DropdownMenuRadioGroup>
                  </DropdownMenuContent>
                </DropdownMenu>
              ) : null}
            </div>
          </div>
          {pubkeyInput.trim().length > 0 &&
          !isValidHexPubkey(normalizedInput) ? (
            <p className="text-xs text-destructive">
              Enter a valid npub or 64-character hex pubkey.
            </p>
          ) : null}
        </form>

        <InviteLinkSection />

        {membersQuery.error instanceof Error ? (
          <p className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {membersQuery.error.message}
          </p>
        ) : null}

        <div className="space-y-3">
          <div className="flex items-baseline justify-between gap-2">
            <h3 className="text-sm font-medium">
              Members
              {members.length > 0 ? (
                <span className="ml-1.5 text-xs font-normal text-muted-foreground">
                  ({members.length})
                </span>
              ) : null}
            </h3>
          </div>

          <div className="relative">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <input
              autoCapitalize="none"
              autoCorrect="off"
              className="w-full rounded-lg border border-border/70 bg-background/70 py-2 pl-9 pr-3 text-sm placeholder:text-muted-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              data-testid="relay-members-search"
              onChange={(event) => setSearch(event.target.value)}
              placeholder="Search members by name, npub, or role…"
              spellCheck={false}
              type="text"
              value={search}
            />
          </div>

          {membersQuery.isLoading ? (
            <p className="text-sm text-muted-foreground">
              Loading relay members…
            </p>
          ) : members.length === 0 ? (
            <p className="rounded-lg border border-dashed border-border/70 bg-background/40 px-3 py-6 text-center text-sm text-muted-foreground">
              No relay members yet. Invite someone above to get started.
            </p>
          ) : filteredMembers.length === 0 ? (
            <p className="rounded-lg border border-dashed border-border/70 bg-background/40 px-3 py-6 text-center text-sm text-muted-foreground">
              No members match your search.
            </p>
          ) : (
            <VirtualizedList
              className="max-h-[28rem] pr-1"
              estimateSize={56}
              getItemKey={(member) => member.pubkey}
              items={filteredMembers}
              renderItem={(member) => (
                <div className="pb-2">
                  <RelayMemberRow
                    currentPubkey={currentPubkey}
                    currentRole={currentRole}
                    member={member}
                    profile={profiles?.[normalizePubkey(member.pubkey)]}
                  />
                </div>
              )}
            />
          )}
        </div>
      </div>
    </section>
  );
}
