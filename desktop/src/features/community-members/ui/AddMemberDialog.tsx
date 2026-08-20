import { ChevronDown, Search } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import * as React from "react";
import { toast } from "sonner";

import {
  useAddRelayMemberMutation,
  useRelayMembersQuery,
} from "@/features/community-members/hooks";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import { useUserSearchQuery } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { SelectedRecipientChip } from "@/features/profile/ui/SelectedRecipientChip";
import type { RelayMemberRole, UserSearchResult } from "@/shared/api/types";
import { parsePubkeyInput } from "@/shared/lib/nostrUtils";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Input } from "@/shared/ui/input";
import { Popover, PopoverAnchor, PopoverContent } from "@/shared/ui/popover";

const ROLE_OPTIONS: Array<{
  value: RelayMemberRole;
  label: string;
}> = [
  {
    value: "member",
    label: "Member",
  },
  {
    value: "admin",
    label: "Admin",
  },
];

function formatSearchUserName(user: UserSearchResult) {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncatePubkey(user.pubkey)
  );
}

export function DirectAddMemberForm({
  isOwner,
  onAdded,
  showLabel = true,
  submitLabel = "Add member",
}: {
  isOwner: boolean;
  onAdded?: () => void;
  showLabel?: boolean;
  submitLabel?: string;
}) {
  const addMutation = useAddRelayMemberMutation();
  const membersQuery = useRelayMembersQuery();
  const [query, setQuery] = React.useState("");
  const [selectedUsers, setSelectedUsers] = React.useState<UserSearchResult[]>(
    [],
  );
  const [role, setRole] = React.useState<RelayMemberRole>("member");
  const [isPickerOpen, setIsPickerOpen] = React.useState(false);
  const searchInputRef = React.useRef<HTMLInputElement>(null);
  const shouldReduceMotion = useReducedMotion();

  const deferredQuery = React.useDeferredValue(query.trim());
  const parsedPubkey = parsePubkeyInput(deferredQuery);
  const userSearchQuery = useUserSearchQuery(deferredQuery, {
    enabled: deferredQuery.length > 0,
    limit: 8,
  });
  const isArchived = useIsArchivedPredicate();
  const selectedPubkeys = React.useMemo(
    () => new Set(selectedUsers.map((user) => user.pubkey.toLowerCase())),
    [selectedUsers],
  );
  const isAlreadyMember =
    parsedPubkey !== null &&
    (membersQuery.data ?? []).some(
      (m) => m.pubkey.toLowerCase() === parsedPubkey.toLowerCase(),
    );
  const canAdd = selectedUsers.length > 0 && !addMutation.isPending;
  const searchResults = React.useMemo(
    () =>
      (userSearchQuery.data ?? []).filter(
        (user) =>
          !isArchived(user.pubkey) &&
          !selectedPubkeys.has(user.pubkey.toLowerCase()) &&
          !(membersQuery.data ?? []).some(
            (member) =>
              member.pubkey.toLowerCase() === user.pubkey.toLowerCase(),
          ),
      ),
    [isArchived, membersQuery.data, selectedPubkeys, userSearchQuery.data],
  );
  const directResult = React.useMemo<UserSearchResult | null>(() => {
    if (
      parsedPubkey === null ||
      isAlreadyMember ||
      selectedPubkeys.has(parsedPubkey.toLowerCase()) ||
      searchResults.some(
        (user) => user.pubkey.toLowerCase() === parsedPubkey.toLowerCase(),
      )
    ) {
      return null;
    }
    return {
      pubkey: parsedPubkey,
      displayName: null,
      avatarUrl: null,
      nip05Handle: null,
      ownerPubkey: null,
      isAgent: false,
    };
  }, [isAlreadyMember, parsedPubkey, searchResults, selectedPubkeys]);
  const roleOptions = React.useMemo(
    () => ROLE_OPTIONS.filter((option) => isOwner || option.value === "member"),
    [isOwner],
  );
  const selectedRoleLabel =
    roleOptions.find((option) => option.value === role)?.label ?? "Member";
  const actionTransition = shouldReduceMotion
    ? { duration: 0 }
    : { duration: 0.18, ease: [0.23, 1, 0.32, 1] as const };

  function reset() {
    setQuery("");
    setSelectedUsers([]);
    setRole("member");
    setIsPickerOpen(false);
    addMutation.reset();
  }

  function selectUser(user: UserSearchResult) {
    setSelectedUsers((currentUsers) =>
      currentUsers.some(
        (currentUser) =>
          currentUser.pubkey.toLowerCase() === user.pubkey.toLowerCase(),
      )
        ? currentUsers
        : [...currentUsers, user],
    );
    setQuery("");
    setIsPickerOpen(true);
    window.requestAnimationFrame(() => {
      searchInputRef.current?.focus({ preventScroll: true });
    });
  }

  function removeUser(pubkey: string) {
    setSelectedUsers((currentUsers) =>
      currentUsers.filter(
        (user) => user.pubkey.toLowerCase() !== pubkey.toLowerCase(),
      ),
    );
    searchInputRef.current?.focus({ preventScroll: true });
  }

  async function handleAdd() {
    if (!canAdd) return;

    try {
      for (const user of selectedUsers) {
        await addMutation.mutateAsync({ pubkey: user.pubkey, role });
      }
      toast.success(
        selectedUsers.length === 1
          ? role === "admin"
            ? "Admin added"
            : "Member added"
          : role === "admin"
            ? "Admins added"
            : "Members added",
      );
      reset();
      onAdded?.();
    } catch {
      // The mutation exposes the API error below the field.
    }
  }

  return (
    <form
      className="space-y-4"
      data-testid="direct-add-member-form"
      onSubmit={(event) => {
        event.preventDefault();
        handleAdd();
      }}
    >
      <div className="space-y-1.5">
        {showLabel ? (
          <label className="text-sm font-medium" htmlFor="member-search">
            Person
          </label>
        ) : null}
        <div className="flex gap-2">
          <Popover
            modal={false}
            onOpenChange={setIsPickerOpen}
            open={isPickerOpen && deferredQuery.length > 0}
          >
            <PopoverAnchor asChild>
              <div
                className="min-w-0 flex-1 rounded-md border border-input bg-background focus-within:ring-1 focus-within:ring-ring"
                data-testid="member-recipient-field"
              >
                <div
                  className={`grid min-h-10 min-w-0 grid-cols-[minmax(0,1fr)_auto] gap-x-3 p-1.5 ${selectedUsers.length > 1 ? "items-start" : "items-center"}`}
                >
                  <div className="flex min-w-0 flex-wrap items-center gap-1.5">
                    {selectedUsers.length === 0 ? (
                      <Search className="h-4 w-4 shrink-0 text-muted-foreground/55" />
                    ) : null}
                    {selectedUsers.map((user) => (
                      <motion.div
                        key={user.pubkey}
                        layout="position"
                        transition={actionTransition}
                      >
                        <SelectedRecipientChip
                          disabled={addMutation.isPending}
                          inspectable={false}
                          label={formatSearchUserName(user)}
                          onRemove={() => removeUser(user.pubkey)}
                          poofOnRemove={false}
                          testIds={{
                            chip: `member-search-selection-remove-${user.pubkey}`,
                          }}
                          user={user}
                        />
                      </motion.div>
                    ))}
                    <Input
                      aria-autocomplete="list"
                      aria-controls="member-search-results"
                      aria-expanded={isPickerOpen}
                      autoCapitalize="none"
                      autoCorrect="off"
                      className="h-7 w-auto min-w-16 flex-1 border-0 bg-transparent px-0 py-0.5 text-sm shadow-none outline-hidden placeholder:text-muted-foreground/55 focus-visible:ring-0"
                      data-testid="member-pubkey-input"
                      disabled={addMutation.isPending}
                      id="member-search"
                      onChange={(event) => {
                        setQuery(event.target.value);
                        setIsPickerOpen(true);
                      }}
                      onFocus={() => setIsPickerOpen(true)}
                      onKeyDown={(event) => {
                        if (
                          event.key === "Backspace" &&
                          query.length === 0 &&
                          selectedUsers.length > 0
                        ) {
                          event.preventDefault();
                          const lastUser = selectedUsers.at(-1);
                          if (lastUser) removeUser(lastUser.pubkey);
                        }
                      }}
                      placeholder={
                        selectedUsers.length === 0
                          ? "Search people or paste an npub"
                          : ""
                      }
                      ref={searchInputRef}
                      role="combobox"
                      spellCheck={false}
                      value={query}
                    />
                  </div>
                  <AnimatePresence initial={false}>
                    {selectedUsers.length > 0 ? (
                      <motion.div
                        animate={{ opacity: 1, scale: 1, x: 0 }}
                        className="shrink-0"
                        exit={{ opacity: 0, scale: 0.96, x: -4 }}
                        initial={{ opacity: 0, scale: 0.96, x: -4 }}
                        transition={actionTransition}
                      >
                        <DropdownMenu modal={false}>
                          <DropdownMenuTrigger asChild>
                            <button
                              aria-label="Choose member role"
                              className="inline-flex items-center gap-1.5 bg-transparent text-sm text-muted-foreground outline-hidden transition-colors hover:text-foreground focus-visible:text-foreground disabled:cursor-not-allowed disabled:opacity-60"
                              data-testid="member-role"
                              disabled={addMutation.isPending}
                              type="button"
                            >
                              {selectedRoleLabel}
                              <ChevronDown className="h-3.5 w-3.5 shrink-0" />
                            </button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent
                            align="end"
                            onCloseAutoFocus={(event) => event.preventDefault()}
                            sideOffset={4}
                            style={{ minWidth: "13rem" }}
                          >
                            <DropdownMenuRadioGroup
                              onValueChange={(value) =>
                                setRole(value as RelayMemberRole)
                              }
                              value={role}
                            >
                              {roleOptions.map((option) => (
                                <DropdownMenuRadioItem
                                  key={option.value}
                                  value={option.value}
                                >
                                  {option.label}
                                </DropdownMenuRadioItem>
                              ))}
                            </DropdownMenuRadioGroup>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </motion.div>
                    ) : null}
                  </AnimatePresence>
                </div>
              </div>
            </PopoverAnchor>
            <PopoverContent
              align="start"
              className="w-(--radix-popover-trigger-width) overflow-hidden p-0"
              data-testid="member-search-popover"
              onCloseAutoFocus={(event) => event.preventDefault()}
              onOpenAutoFocus={(event) => event.preventDefault()}
              sideOffset={6}
            >
              <div
                className="max-h-64 overflow-y-auto overscroll-contain py-1"
                data-testid="member-search-results"
                id="member-search-results"
                role="listbox"
              >
                {userSearchQuery.isLoading ? (
                  <p className="px-3 py-3 text-sm text-muted-foreground">
                    Searching…
                  </p>
                ) : searchResults.length > 0 || directResult ? (
                  <>
                    {directResult ? (
                      <SearchResult
                        onSelect={() => selectUser(directResult)}
                        user={directResult}
                      />
                    ) : null}
                    {searchResults.map((user) => (
                      <SearchResult
                        key={user.pubkey}
                        onSelect={() => selectUser(user)}
                        user={user}
                      />
                    ))}
                  </>
                ) : (
                  <p className="px-3 py-3 text-sm text-muted-foreground">
                    No people found. Paste a full npub or hex public key to add
                    someone directly.
                  </p>
                )}
              </div>
            </PopoverContent>
          </Popover>
          <AnimatePresence initial={false}>
            {selectedUsers.length > 0 ? (
              <motion.div
                animate={{ opacity: 1, scale: 1, x: 0 }}
                className="shrink-0"
                exit={{ opacity: 0, scale: 0.96, x: -4 }}
                initial={{ opacity: 0, scale: 0.96, x: -4 }}
                transition={actionTransition}
              >
                <Button
                  className="h-11"
                  data-testid="confirm-add-member"
                  disabled={!canAdd}
                  size="sm"
                  type="submit"
                >
                  {addMutation.isPending ? "Inviting…" : submitLabel}
                </Button>
              </motion.div>
            ) : null}
          </AnimatePresence>
        </div>
        {isAlreadyMember ? (
          <p className="text-xs text-destructive">
            This person is already a community member.
          </p>
        ) : null}
        {userSearchQuery.error instanceof Error ? (
          <p className="text-xs text-destructive">
            {userSearchQuery.error.message}
          </p>
        ) : null}
      </div>

      {addMutation.error instanceof Error ? (
        <p className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {addMutation.error.message}
        </p>
      ) : null}
    </form>
  );
}

function SearchResult({
  onSelect,
  user,
}: {
  onSelect: () => void;
  user: UserSearchResult;
}) {
  const name = formatSearchUserName(user);
  const isDirectPubkey = user.displayName === null && user.nip05Handle === null;

  return (
    <button
      className="flex min-h-11 w-full items-center gap-3 px-3 py-2 text-left transition-colors hover:bg-muted/50 focus-visible:bg-muted/50 focus-visible:outline-hidden"
      data-testid={`member-search-result-${user.pubkey}`}
      onClick={onSelect}
      role="option"
      type="button"
    >
      <ProfileAvatar
        avatarUrl={user.avatarUrl}
        className="h-8 w-8 text-xs shadow-none"
        iconClassName="h-4 w-4"
        label={name}
      />
      <span className="min-w-0 flex-1 truncate text-sm font-medium">
        {name}
      </span>
      {isDirectPubkey ? (
        <span className="shrink-0 text-xs text-muted-foreground">
          public key
        </span>
      ) : null}
    </button>
  );
}

export function AddMemberDialog({
  isOwner,
  open,
  onOpenChange,
}: {
  isOwner: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        className="max-w-md overflow-hidden p-0"
        data-testid="add-relay-member-dialog"
      >
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="border-b border-border/60 px-6 py-5 pr-14">
            <DialogTitle>Add member</DialogTitle>
            <DialogDescription>
              Add a person to this community by their public key.
            </DialogDescription>
          </DialogHeader>
          <div className="px-6 py-4">
            <DirectAddMemberForm
              isOwner={isOwner}
              onAdded={() => onOpenChange(false)}
            />
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
