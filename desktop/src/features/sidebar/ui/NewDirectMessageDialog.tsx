import { Search, X } from "lucide-react";
import * as React from "react";

import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import { useUserSearchQuery } from "@/features/profile/hooks";
import { truncatePubkey } from "@/features/profile/lib/identity";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { UserSearchResult } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";

function formatUserName(user: UserSearchResult) {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncatePubkey(user.pubkey)
  );
}

function formatUserSecondary(user: UserSearchResult) {
  const displayName = user.displayName?.trim();
  const nip05Handle = user.nip05Handle?.trim();

  if (displayName && nip05Handle) {
    return nip05Handle;
  }

  return truncatePubkey(user.pubkey);
}

export function NewDirectMessageDialog({
  currentPubkey,
  isPending,
  onOpenChange,
  onSubmit,
  open,
}: {
  currentPubkey?: string;
  isPending: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (input: { pubkeys: string[] }) => Promise<void>;
  open: boolean;
}) {
  const [searchQuery, setSearchQuery] = React.useState("");
  const [selectedUsers, setSelectedUsers] = React.useState<UserSearchResult[]>(
    [],
  );
  const [submitErrorMessage, setSubmitErrorMessage] = React.useState<
    string | null
  >(null);
  const searchInputRef = React.useRef<HTMLInputElement>(null);
  const deferredSearchQuery = React.useDeferredValue(searchQuery.trim());
  const hasReachedRecipientLimit = selectedUsers.length >= 8;
  const selectedPubkeys = React.useMemo(
    () => new Set(selectedUsers.map((user) => user.pubkey.toLowerCase())),
    [selectedUsers],
  );
  const userSearchQuery = useUserSearchQuery(deferredSearchQuery, {
    enabled:
      open && deferredSearchQuery.length > 0 && !hasReachedRecipientLimit,
    limit: 8,
  });
  const isArchivedDiscovery = useIsArchivedPredicate();
  const searchResults = React.useMemo(
    () =>
      (userSearchQuery.data ?? []).filter((user) => {
        const normalizedPubkey = user.pubkey.toLowerCase();
        return (
          normalizedPubkey !== currentPubkey?.toLowerCase() &&
          !selectedPubkeys.has(normalizedPubkey) &&
          !isArchivedDiscovery(user.pubkey)
        );
      }),
    [currentPubkey, isArchivedDiscovery, selectedPubkeys, userSearchQuery.data],
  );

  React.useEffect(() => {
    if (!open) {
      setSearchQuery("");
      setSelectedUsers([]);
      setSubmitErrorMessage(null);
      return;
    }

    searchInputRef.current?.focus();
  }, [open]);

  function handleSelectUser(user: UserSearchResult) {
    if (hasReachedRecipientLimit) {
      return;
    }

    setSelectedUsers((current) => {
      if (current.some((candidate) => candidate.pubkey === user.pubkey)) {
        return current;
      }

      return [...current, user];
    });
    setSearchQuery("");
    setSubmitErrorMessage(null);
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-xl" data-testid="new-dm-dialog">
        <DialogHeader>
          <DialogTitle>New direct message</DialogTitle>
          <DialogDescription>
            Pick 1 to 8 people. If the conversation already exists, Buzz will
            reopen it.
          </DialogDescription>
        </DialogHeader>

        <form
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault();

            if (selectedUsers.length === 0) {
              return;
            }

            setSubmitErrorMessage(null);

            void onSubmit({
              pubkeys: selectedUsers.map((user) => user.pubkey),
            })
              .then(() => {
                onOpenChange(false);
              })
              .catch((error) => {
                setSubmitErrorMessage(
                  error instanceof Error
                    ? error.message
                    : "Failed to open direct message.",
                );
              });
          }}
        >
          <div className="overflow-hidden rounded-2xl border border-border/80 bg-muted/20">
            <div className="flex items-center gap-2 px-3 py-2">
              <Search className="h-4 w-4 text-muted-foreground" />
              <Input
                className="h-auto border-0 bg-transparent px-0 py-0 shadow-none focus-visible:ring-0"
                data-testid="new-dm-search"
                disabled={isPending}
                onChange={(event) => {
                  setSearchQuery(event.target.value);
                  setSubmitErrorMessage(null);
                }}
                onKeyDown={(event) => {
                  if (event.key !== "Enter" || searchResults.length === 0) {
                    return;
                  }

                  event.preventDefault();
                  handleSelectUser(searchResults[0]);
                }}
                placeholder="Search by name, NIP-05, or pubkey."
                ref={searchInputRef}
                value={searchQuery}
              />
            </div>

            {selectedUsers.length > 0 ? (
              <div className="flex flex-wrap gap-2 border-t border-border/70 px-3 py-2">
                {selectedUsers.map((user) => (
                  <div
                    className="inline-flex items-center gap-2 rounded-full border border-border/80 bg-background/80 px-2.5 py-1 text-xs"
                    data-testid={`new-dm-selected-${user.pubkey}`}
                    key={user.pubkey}
                  >
                    <ProfileAvatar
                      avatarUrl={user.avatarUrl}
                      className="h-5 w-5 text-[10px] shadow-none"
                      iconClassName="h-4 w-4"
                      label={formatUserName(user)}
                    />
                    <span className="font-medium">{formatUserName(user)}</span>
                    <button
                      aria-label={`Remove ${formatUserName(user)}`}
                      className="text-muted-foreground transition-colors hover:text-foreground"
                      disabled={isPending}
                      onClick={() => {
                        setSelectedUsers((current) =>
                          current.filter(
                            (candidate) => candidate.pubkey !== user.pubkey,
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

            <div className="border-t border-border/70 px-2 py-2">
              {hasReachedRecipientLimit ? (
                <p
                  className="px-2 py-1 text-sm text-muted-foreground"
                  data-testid="new-dm-limit"
                >
                  Direct messages support up to 9 people including you.
                </p>
              ) : deferredSearchQuery.length === 0 ? (
                <p
                  className="px-2 py-1 text-sm text-muted-foreground"
                  data-testid="new-dm-empty"
                >
                  Search for someone to start a conversation.
                </p>
              ) : userSearchQuery.isLoading ? (
                <p className="px-2 py-1 text-sm text-muted-foreground">
                  Searching…
                </p>
              ) : searchResults.length > 0 ? (
                <div className="space-y-1">
                  {searchResults.map((user) => (
                    <button
                      className="flex w-full items-center gap-3 rounded-xl px-3 py-2 text-left transition-colors hover:bg-accent hover:text-accent-foreground"
                      data-testid={`new-dm-result-${user.pubkey}`}
                      key={user.pubkey}
                      onClick={() => {
                        handleSelectUser(user);
                      }}
                      type="button"
                    >
                      <ProfileAvatar
                        avatarUrl={user.avatarUrl}
                        className="h-9 w-9 text-xs shadow-none"
                        iconClassName="h-4 w-4"
                        label={formatUserName(user)}
                      />
                      <div className="min-w-0 flex-1">
                        <p className="truncate text-sm font-medium">
                          {formatUserName(user)}
                        </p>
                        <p className="truncate text-xs text-muted-foreground">
                          {formatUserSecondary(user)}
                        </p>
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
          </div>

          {userSearchQuery.error instanceof Error ? (
            <p className="text-sm text-destructive">
              {userSearchQuery.error.message}
            </p>
          ) : null}

          {submitErrorMessage ? (
            <p className="text-sm text-destructive">{submitErrorMessage}</p>
          ) : null}

          <div className="flex items-center justify-between gap-3">
            <p className="text-sm text-muted-foreground">
              {selectedUsers.length === 0
                ? "No recipients selected yet."
                : `${selectedUsers.length} recipient${
                    selectedUsers.length === 1 ? "" : "s"
                  } selected.`}
            </p>
            <div className="flex items-center gap-2">
              <Button
                disabled={isPending}
                onClick={() => onOpenChange(false)}
                type="button"
                variant="ghost"
              >
                Cancel
              </Button>
              <Button
                data-testid="new-dm-submit"
                disabled={isPending || selectedUsers.length === 0}
                type="submit"
              >
                {isPending
                  ? "Opening..."
                  : selectedUsers.length > 1
                    ? "Start group DM"
                    : "Message"}
              </Button>
            </div>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
