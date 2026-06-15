import { ChevronDown, Search, UserPlus, X } from "lucide-react";
import * as React from "react";

import { formatPubkey } from "@/features/channels/lib/memberUtils";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import { useUserSearchQuery } from "@/features/profile/hooks";
import type {
  AddChannelMembersResult,
  ChannelMember,
  UserSearchResult,
} from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { UserAvatar } from "@/shared/ui/UserAvatar";

function formatSearchUserName(user: UserSearchResult) {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    formatPubkey(user.pubkey)
  );
}

function formatSearchUserSecondary(user: UserSearchResult) {
  const displayName = user.displayName?.trim();
  const nip05Handle = user.nip05Handle?.trim();

  if (displayName && nip05Handle) {
    return nip05Handle;
  }

  return formatPubkey(user.pubkey);
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
  const [invitePubkeys, setInvitePubkeys] = React.useState("");
  const [inviteQuery, setInviteQuery] = React.useState("");
  const [isDirectPubkeyEntryOpen, setIsDirectPubkeyEntryOpen] =
    React.useState(false);
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
      setInvitePubkeys("");
      setInviteQuery("");
      setIsDirectPubkeyEntryOpen(false);
      setSelectedInvitees([]);
      setSubmissionErrors([]);
    }
  }, [open]);

  const parsedInvitePubkeys = React.useMemo(
    () =>
      invitePubkeys
        .split(/[\s,]+/)
        .map((value) => value.trim())
        .filter((value) => value.length > 0),
    [invitePubkeys],
  );
  const inviteTargets = [
    ...new Set([
      ...selectedInvitees.map((invitee) => invitee.pubkey),
      ...parsedInvitePubkeys,
    ]),
  ];
  const directEntryLabel =
    parsedInvitePubkeys.length > 0 && !isDirectPubkeyEntryOpen
      ? `Direct pubkey entry (${parsedInvitePubkeys.length} ready)`
      : "Direct pubkey entry";

  return (
    <form
      className="space-y-2.5 rounded-xl border border-border/80 bg-muted/15 p-3"
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
          const remainingPubkeys = parsedInvitePubkeys
            .filter((pubkey) => !addedPubkeys.has(pubkey.toLowerCase()))
            .join("\n");
          setInvitePubkeys(remainingPubkeys);
          if (remainingPubkeys.length > 0) {
            setIsDirectPubkeyEntryOpen(true);
          }
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
          <span className="rounded-full bg-background px-2 py-1 text-[11px] font-medium leading-none text-muted-foreground">
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
              placeholder="Search by name or NIP-05."
              value={inviteQuery}
            />
          </div>
          {selectedInvitees.length > 0 ? (
            <div className="flex flex-wrap gap-1.5 border-t border-border/70 px-2.5 py-2">
              {selectedInvitees.map((invitee) => (
                <div
                  className="inline-flex items-center gap-1.5 rounded-full border border-border/80 bg-muted/60 px-2.5 py-1 text-[11px] leading-none"
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
                        <div className="min-w-0">
                          <p className="truncate text-sm font-medium leading-5">
                            {formatSearchUserName(result)}
                          </p>
                          <p className="truncate text-xs text-muted-foreground">
                            {formatSearchUserSecondary(result)}
                          </p>
                        </div>
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
      <div className="space-y-2">
        <button
          aria-controls="channel-management-direct-pubkeys-panel"
          aria-expanded={isDirectPubkeyEntryOpen}
          className="inline-flex items-center gap-1.5 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
          data-testid="channel-management-toggle-direct-pubkeys"
          onClick={() => {
            setIsDirectPubkeyEntryOpen((current) => !current);
          }}
          type="button"
        >
          <ChevronDown
            className={cn(
              "h-4 w-4 transition-transform",
              isDirectPubkeyEntryOpen && "rotate-180",
            )}
          />
          <span>{directEntryLabel}</span>
        </button>

        {isDirectPubkeyEntryOpen ? (
          <div
            className="space-y-1.5 rounded-lg border border-dashed border-border/80 bg-background/70 p-2.5"
            id="channel-management-direct-pubkeys-panel"
          >
            <label className="sr-only" htmlFor="channel-management-add-pubkeys">
              Paste pubkeys
            </label>
            <p className="text-xs text-muted-foreground">
              For exact pubkeys when search is not the right fit.
            </p>
            <Textarea
              className="min-h-24"
              data-testid="channel-management-add-pubkeys"
              disabled={isPending}
              id="channel-management-add-pubkeys"
              onChange={(event) => setInvitePubkeys(event.target.value)}
              placeholder="Paste one or more pubkeys, separated by spaces, commas, or new lines."
              value={invitePubkeys}
            />
          </div>
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
              {formatPubkey(error.pubkey)}: {error.error}
            </p>
          ))}
        </div>
      ) : null}
    </form>
  );
}
