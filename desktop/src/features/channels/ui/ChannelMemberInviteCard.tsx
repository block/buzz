import { Search, UserPlus, X } from "lucide-react";
import * as React from "react";

import { truncatePubkey } from "@/shared/lib/pubkey";
import { PubKey } from "@/shared/ui/PubKey";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import { useUserSearchQuery } from "@/features/profile/hooks";
import type {
  AddChannelMembersResult,
  ChannelMember,
  UserSearchResult,
} from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { UserAvatar } from "@/shared/ui/UserAvatar";

function formatSearchUserName(user: UserSearchResult) {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncatePubkey(user.pubkey)
  );
}

export function ChannelMemberInviteCard({
  canAssignElevatedRoles,
  existingMembers,
  isPending,
  onSubmit,
  open,
  requestErrorMessage,
}: {
  canAssignElevatedRoles: boolean;
  existingMembers: ChannelMember[];
  isPending: boolean;
  onSubmit: (input: {
    pubkeys: string[];
    role: Exclude<ChannelMember["role"], "owner">;
  }) => Promise<AddChannelMembersResult>;
  open: boolean;
  requestErrorMessage?: string | null;
}) {
  const [inviteQuery, setInviteQuery] = React.useState("");
  const [selectedInvitees, setSelectedInvitees] = React.useState<
    UserSearchResult[]
  >([]);
  const [inviteRole, setInviteRole] =
    React.useState<Exclude<ChannelMember["role"], "owner">>("member");
  const [submissionErrors, setSubmissionErrors] = React.useState<
    AddChannelMembersResult["errors"]
  >([]);

  // Only owners/admins may grant the elevated "admin" role — the relay rejects
  // it for everyone else. Hide it from the dropdown so we never offer a role
  // the server will refuse. "member", "guest", and "bot" are non-elevated and
  // any channel member may grant them.
  const availableRoles = React.useMemo<
    Exclude<ChannelMember["role"], "owner">[]
  >(
    () =>
      canAssignElevatedRoles
        ? ["member", "admin", "guest", "bot"]
        : ["member", "guest", "bot"],
    [canAssignElevatedRoles],
  );

  // Guard against a stale elevated selection if the caller's permissions change
  // while the card is mounted (e.g. the member gets demoted).
  React.useEffect(() => {
    if (!availableRoles.includes(inviteRole)) {
      setInviteRole("member");
    }
  }, [availableRoles, inviteRole]);

  const deferredInviteQuery = React.useDeferredValue(inviteQuery.trim());
  const selectedInviteePubkeys = React.useMemo(
    () =>
      new Set(selectedInvitees.map((invitee) => invitee.pubkey.toLowerCase())),
    [selectedInvitees],
  );
  const memberPubkeys = React.useMemo(
    () => new Set(existingMembers.map((member) => member.pubkey.toLowerCase())),
    [existingMembers],
  );
  const userSearchQuery = useUserSearchQuery(deferredInviteQuery, {
    enabled: open && deferredInviteQuery.length > 0,
    // Ask for more than we'll display so server-side ranking has room to be
    // refined client-side. The Tauri command clamps at 50.
    limit: 25,
  });
  const isArchivedDiscovery = useIsArchivedPredicate();
  const inviteSearchResults = React.useMemo(
    () =>
      (userSearchQuery.data ?? []).filter(
        (user) =>
          !memberPubkeys.has(user.pubkey.toLowerCase()) &&
          !selectedInviteePubkeys.has(user.pubkey.toLowerCase()) &&
          !isArchivedDiscovery(user.pubkey),
      ),
    [
      isArchivedDiscovery,
      memberPubkeys,
      selectedInviteePubkeys,
      userSearchQuery.data,
    ],
  );

  React.useEffect(() => {
    if (!open) {
      setInviteQuery("");
      setSelectedInvitees([]);
      setSubmissionErrors([]);
    }
  }, [open]);

  const inviteTargets = selectedInvitees.map((invitee) => invitee.pubkey);

  return (
    <form
      className="space-y-2.5 rounded-xl border border-border/70 bg-background/70 p-3"
      onSubmit={(event) => {
        event.preventDefault();
        void onSubmit({
          pubkeys: inviteTargets,
          role: inviteRole,
        }).then((result) => {
          const addedPubkeys = new Set(
            result.added.map((pubkey) => pubkey.toLowerCase()),
          );
          setSelectedInvitees((current) =>
            current.filter(
              (invitee) => !addedPubkeys.has(invitee.pubkey.toLowerCase()),
            ),
          );
          setInviteQuery("");
          setSubmissionErrors(result.errors);
        });
      }}
    >
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-sm font-medium">
          <UserPlus className="h-4 w-4" />
          <span>Add members</span>
        </div>
        {inviteTargets.length > 0 ? (
          <span className="rounded-full bg-background px-2 py-1 text-2xs font-medium leading-none text-muted-foreground">
            {inviteTargets.length} selected
          </span>
        ) : null}
      </div>
      <div className="space-y-2">
        <label className="sr-only" htmlFor="channel-management-search-users">
          Search people
        </label>
        <div className="rounded-lg border border-border/80 bg-background">
          <div className="flex items-center gap-2 px-2.5 py-2">
            <Search className="h-4 w-4 text-muted-foreground" />
            <Input
              className="h-auto border-0 px-0 py-0 shadow-none focus-visible:ring-0"
              data-testid="channel-management-search-users"
              disabled={isPending}
              id="channel-management-search-users"
              onChange={(event) => setInviteQuery(event.target.value)}
              placeholder="Search people and agents"
              value={inviteQuery}
            />
          </div>
          {selectedInvitees.length > 0 ? (
            <div className="flex flex-wrap gap-1.5 border-t border-border/70 px-2.5 py-2">
              {selectedInvitees.map((invitee) => (
                <div
                  className="inline-flex items-center gap-1.5 rounded-full border border-border/80 bg-muted/60 px-2.5 py-1 text-2xs leading-none"
                  data-testid={`selected-invitee-${invitee.pubkey}`}
                  key={invitee.pubkey}
                >
                  <UserAvatar
                    avatarUrl={invitee.avatarUrl ?? null}
                    displayName={formatSearchUserName(invitee)}
                    size="xs"
                  />
                  <span className="font-medium">
                    {formatSearchUserName(invitee)}
                  </span>
                  <button
                    aria-label={`Remove ${formatSearchUserName(invitee)}`}
                    className="text-muted-foreground transition-colors hover:text-foreground"
                    onClick={() => {
                      setSelectedInvitees((current) =>
                        current.filter(
                          (candidate) => candidate.pubkey !== invitee.pubkey,
                        ),
                      );
                    }}
                    type="button"
                  >
                    <X className="h-4 w-4" />
                  </button>
                </div>
              ))}
            </div>
          ) : null}
          {selectedInvitees.length > 0 ? (
            <div className="space-y-1 border-t border-border/70 px-2.5 py-2">
              {selectedInvitees.map((invitee) => (
                <div
                  className="flex min-w-0 flex-wrap items-baseline gap-x-2 text-2xs text-muted-foreground"
                  data-testid={`invitee-pubkey-${invitee.pubkey}`}
                  key={invitee.pubkey}
                >
                  <span className="font-medium">
                    {formatSearchUserName(invitee)}
                  </span>
                  <PubKey pubkey={invitee.pubkey} variant="full" />
                </div>
              ))}
            </div>
          ) : null}
          {deferredInviteQuery.length > 0 ? (
            <div className="border-t border-border/70 px-2 py-2">
              {userSearchQuery.isLoading ? (
                <p className="px-2 py-1 text-sm text-muted-foreground">
                  Searching…
                </p>
              ) : inviteSearchResults.length > 0 ? (
                <div className="max-h-44 space-y-1 overflow-y-auto">
                  {inviteSearchResults.map((result) => (
                    <button
                      className="flex w-full items-center justify-between rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-accent hover:text-accent-foreground"
                      data-testid={`channel-user-search-result-${result.pubkey}`}
                      key={result.pubkey}
                      onClick={() => {
                        setSelectedInvitees((current) => [...current, result]);
                        setInviteQuery("");
                      }}
                      type="button"
                    >
                      <div className="flex items-center gap-2 min-w-0">
                        <UserAvatar
                          avatarUrl={result.avatarUrl}
                          displayName={formatSearchUserName(result)}
                          size="xs"
                        />
                        <p className="truncate text-sm font-medium leading-5">
                          {formatSearchUserName(result)}
                        </p>
                        {result.isAgent ? (
                          <span className="shrink-0 text-xs text-muted-foreground">
                            agent
                          </span>
                        ) : null}
                      </div>
                      <span className="text-xs text-muted-foreground">Add</span>
                    </button>
                  ))}
                </div>
              ) : (
                <p className="px-2 py-1 text-sm text-muted-foreground">
                  No matching users.
                </p>
              )}
            </div>
          ) : null}
        </div>
        {userSearchQuery.error instanceof Error ? (
          <p className="text-sm text-destructive">
            {userSearchQuery.error.message}
          </p>
        ) : null}
      </div>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <label className="sr-only" htmlFor="channel-member-role">
          Role
        </label>
        <div className="flex items-center gap-2">
          <span className="text-xs text-muted-foreground">Role</span>
          <select
            className="h-8 rounded-md border border-input bg-background px-2.5 text-sm"
            data-testid="channel-management-add-role"
            disabled={isPending}
            id="channel-member-role"
            onChange={(event) =>
              setInviteRole(
                event.target.value as Exclude<ChannelMember["role"], "owner">,
              )
            }
            value={inviteRole}
          >
            {availableRoles.map((role) => (
              <option key={role} value={role}>
                {role}
              </option>
            ))}
          </select>
        </div>
        <Button
          className="min-w-24"
          data-testid="channel-management-add-members"
          disabled={isPending || inviteTargets.length === 0}
          size="sm"
          type="submit"
        >
          {isPending ? "Adding..." : "Add members"}
        </Button>
      </div>
      {requestErrorMessage ? (
        <p className="text-sm text-destructive">{requestErrorMessage}</p>
      ) : null}
      {submissionErrors.length > 0 ? (
        <div className="space-y-1 text-sm text-destructive">
          {submissionErrors.map((error) => (
            <p key={`${error.pubkey}-${error.error}`}>
              {truncatePubkey(error.pubkey)}: {error.error}
            </p>
          ))}
        </div>
      ) : null}
    </form>
  );
}
