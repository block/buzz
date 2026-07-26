import { Search, X } from "lucide-react";
import * as React from "react";

import {
  useUserSearchQuery,
  useUsersBatchQuery,
} from "@/features/profile/hooks";
import { isValidGroupHandle } from "@/features/groups/groupValidation";
import type { UserGroup } from "@/shared/api/relayGroups";
import type { Channel, UserSearchResult } from "@/shared/api/types";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type GroupDialogProps = {
  channels: Channel[];
  error: Error | null;
  group: UserGroup | null;
  isPending: boolean;
  mode: "create" | "edit";
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (group: UserGroup) => Promise<void>;
  viewerPubkey: string;
};

type SelectedMember = {
  avatarUrl: string | null;
  displayName: string;
  isAgent: boolean;
  pubkey: string;
};

function userLabel(user: UserSearchResult): string {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncatePubkey(user.pubkey)
  );
}

export function GroupDialog({
  channels,
  error,
  group,
  isPending,
  mode,
  open,
  onOpenChange,
  onSubmit,
  viewerPubkey,
}: GroupDialogProps) {
  const [name, setName] = React.useState("");
  const [handle, setHandle] = React.useState("");
  const [description, setDescription] = React.useState("");
  const [memberQuery, setMemberQuery] = React.useState("");
  const [selectedMemberPubkeys, setSelectedMemberPubkeys] = React.useState<
    string[]
  >([]);
  const [selectedMembersByPubkey, setSelectedMembersByPubkey] = React.useState<
    Map<string, SelectedMember>
  >(new Map());
  const [defaultChannelIds, setDefaultChannelIds] = React.useState<string[]>(
    [],
  );
  const deferredMemberQuery = React.useDeferredValue(memberQuery.trim());
  const initialProfilesQuery = useUsersBatchQuery(selectedMemberPubkeys, {
    enabled: open && selectedMemberPubkeys.length > 0,
  });
  const userSearchQuery = useUserSearchQuery(deferredMemberQuery, {
    enabled: open && deferredMemberQuery.length > 0,
    limit: 25,
  });

  React.useEffect(() => {
    if (!open) return;
    setName(group?.name ?? "");
    setHandle(group?.handle ?? "");
    setDescription(group?.description ?? "");
    setSelectedMemberPubkeys(group?.memberPubkeys ?? []);
    setSelectedMembersByPubkey(new Map());
    setDefaultChannelIds(group?.defaultChannelIds ?? []);
    setMemberQuery("");
  }, [group, open]);

  React.useEffect(() => {
    const profiles = initialProfilesQuery.data?.profiles;
    if (!profiles) return;
    setSelectedMembersByPubkey((current) => {
      const next = new Map(current);
      for (const pubkey of selectedMemberPubkeys) {
        const normalized = pubkey.toLowerCase();
        const profile = profiles[normalized];
        if (!profile || next.has(normalized)) continue;
        next.set(normalized, {
          avatarUrl: profile.avatarUrl,
          displayName:
            profile.displayName?.trim() ||
            profile.nip05Handle?.trim() ||
            truncatePubkey(pubkey),
          isAgent: profile.isAgent === true,
          pubkey: normalized,
        });
      }
      return next;
    });
  }, [initialProfilesQuery.data?.profiles, selectedMemberPubkeys]);

  const publicChannels = React.useMemo(
    () =>
      channels
        .filter(
          (channel) =>
            channel.visibility === "open" &&
            channel.channelType !== "dm" &&
            channel.archivedAt === null,
        )
        .sort((left, right) => left.name.localeCompare(right.name)),
    [channels],
  );
  const selectedSet = React.useMemo(
    () => new Set(selectedMemberPubkeys.map((pubkey) => pubkey.toLowerCase())),
    [selectedMemberPubkeys],
  );
  const memberResults = React.useMemo(
    () =>
      (userSearchQuery.data ?? []).filter(
        (user) => !selectedSet.has(user.pubkey.toLowerCase()),
      ),
    [selectedSet, userSearchQuery.data],
  );
  const normalizedHandle = handle.trim().toLowerCase();
  const showHandleError =
    normalizedHandle.length > 0 && !isValidGroupHandle(normalizedHandle);
  const canSubmit =
    name.trim().length > 0 &&
    isValidGroupHandle(normalizedHandle) &&
    !isPending;

  function addMember(user: UserSearchResult) {
    const pubkey = user.pubkey.toLowerCase();
    setSelectedMemberPubkeys((current) => [...current, pubkey]);
    setSelectedMembersByPubkey((current) => {
      const next = new Map(current);
      next.set(pubkey, {
        avatarUrl: user.avatarUrl,
        displayName: userLabel(user),
        isAgent: user.isAgent,
        pubkey,
      });
      return next;
    });
    setMemberQuery("");
  }

  function removeMember(pubkey: string) {
    setSelectedMemberPubkeys((current) =>
      current.filter((candidate) => candidate !== pubkey),
    );
    setSelectedMembersByPubkey((current) => {
      const next = new Map(current);
      next.delete(pubkey);
      return next;
    });
  }

  function toggleChannel(channelId: string) {
    setDefaultChannelIds((current) =>
      current.includes(channelId)
        ? current.filter((id) => id !== channelId)
        : [...current, channelId],
    );
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-2xl overflow-hidden p-0">
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="shrink-0 border-b border-border/60 px-6 py-5 pr-14">
            <DialogTitle>
              {mode === "create" ? "New group" : "Edit group"}
            </DialogTitle>
            <DialogDescription>
              Mention a shared set of people and agents with one handle.
            </DialogDescription>
          </DialogHeader>

          <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-6 py-5">
            <div className="space-y-1.5">
              <label className="text-sm font-medium" htmlFor="group-name">
                Name
              </label>
              <Input
                data-testid="group-dialog-name"
                disabled={isPending}
                id="group-name"
                onChange={(event) => setName(event.target.value)}
                placeholder="iOS team"
                value={name}
              />
            </div>

            <div className="space-y-1.5">
              <label className="text-sm font-medium" htmlFor="group-handle">
                Handle
              </label>
              <div className="flex items-center rounded-md border border-input bg-background focus-within:ring-1 focus-within:ring-ring">
                <span className="pl-3 text-sm text-muted-foreground">@</span>
                <Input
                  aria-invalid={showHandleError}
                  className="border-0 pl-0.5 shadow-none focus-visible:ring-0"
                  data-testid="group-dialog-handle"
                  disabled={isPending}
                  id="group-handle"
                  onChange={(event) =>
                    setHandle(
                      event.target.value.replace(/^@/, "").toLowerCase(),
                    )
                  }
                  placeholder="ios-team"
                  value={handle}
                />
              </div>
              {showHandleError ? (
                <p className="text-xs text-destructive">
                  Use 2–32 lowercase letters, numbers, underscores, or hyphens.
                </p>
              ) : null}
            </div>

            <div className="space-y-1.5">
              <label
                className="text-sm font-medium"
                htmlFor="group-description"
              >
                Description
              </label>
              <Textarea
                className="min-h-20"
                data-testid="group-dialog-description"
                disabled={isPending}
                id="group-description"
                onChange={(event) => setDescription(event.target.value)}
                placeholder="Optional description for this group."
                value={description}
              />
            </div>

            <div className="space-y-2">
              <span className="text-sm font-medium">Members</span>
              <p className="text-xs text-muted-foreground">
                Add people and agents. Empty groups are allowed.
              </p>
              {selectedMemberPubkeys.length === 0 ? (
                <p className="text-xs text-amber-600 dark:text-amber-400">
                  No one will be notified until members are added.
                </p>
              ) : null}
              <div className="rounded-lg border border-border/80 bg-background">
                <div className="flex items-center gap-2 px-2.5 py-2">
                  <Search className="h-4 w-4 text-muted-foreground" />
                  <Input
                    className="h-auto border-0 px-0 py-0 shadow-none focus-visible:ring-0"
                    data-testid="group-dialog-member-search"
                    disabled={isPending}
                    onChange={(event) => setMemberQuery(event.target.value)}
                    placeholder="Search people and agents"
                    value={memberQuery}
                  />
                </div>
                {selectedMemberPubkeys.length > 0 ? (
                  <div className="flex flex-wrap gap-1.5 border-t border-border/70 px-2.5 py-2">
                    {selectedMemberPubkeys.map((pubkey) => {
                      const member = selectedMembersByPubkey.get(pubkey);
                      const label =
                        member?.displayName ?? truncatePubkey(pubkey);
                      return (
                        <div
                          className="inline-flex items-center gap-1.5 rounded-full border border-border/80 bg-muted/60 px-2.5 py-1 text-2xs leading-none"
                          key={pubkey}
                        >
                          <UserAvatar
                            avatarUrl={member?.avatarUrl ?? null}
                            displayName={label}
                            size="xs"
                          />
                          <span className="font-medium">{label}</span>
                          {member?.isAgent ? (
                            <span className="text-muted-foreground">agent</span>
                          ) : null}
                          <button
                            aria-label={`Remove ${label}`}
                            className="text-muted-foreground hover:text-foreground"
                            onClick={() => removeMember(pubkey)}
                            type="button"
                          >
                            <X className="h-4 w-4" />
                          </button>
                        </div>
                      );
                    })}
                  </div>
                ) : null}
                {deferredMemberQuery.length > 0 ? (
                  <div className="border-t border-border/70 p-2">
                    {userSearchQuery.isLoading ? (
                      <p className="px-2 py-1 text-sm text-muted-foreground">
                        Searching…
                      </p>
                    ) : memberResults.length > 0 ? (
                      <div className="max-h-44 space-y-1 overflow-y-auto">
                        {memberResults.map((result) => (
                          <button
                            className="flex w-full items-center justify-between rounded-md px-2.5 py-1.5 text-left hover:bg-accent"
                            key={result.pubkey}
                            onClick={() => addMember(result)}
                            type="button"
                          >
                            <span className="flex min-w-0 items-center gap-2">
                              <UserAvatar
                                avatarUrl={result.avatarUrl}
                                displayName={userLabel(result)}
                                size="xs"
                              />
                              <span className="truncate text-sm font-medium">
                                {userLabel(result)}
                              </span>
                              {result.isAgent ? (
                                <span className="text-xs text-muted-foreground">
                                  agent
                                </span>
                              ) : null}
                            </span>
                            <span className="text-xs text-muted-foreground">
                              Add
                            </span>
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
            </div>

            <div className="space-y-2">
              <span className="text-sm font-medium">Default channels</span>
              <p className="text-xs text-muted-foreground">
                New members are automatically added to these channels.
              </p>
              <div
                aria-label="Default channels"
                aria-multiselectable="true"
                className="max-h-48 space-y-1 overflow-y-auto rounded-lg border border-border/70 p-2"
                role="listbox"
              >
                {publicChannels.length > 0 ? (
                  publicChannels.map((channel) => {
                    const selected = defaultChannelIds.includes(channel.id);
                    return (
                      <div
                        aria-selected={selected}
                        className="flex cursor-pointer items-center gap-3 rounded-md px-2 py-1.5 hover:bg-muted/50"
                        data-testid={`group-dialog-default-channel-${channel.id}`}
                        key={channel.id}
                        onClick={() => toggleChannel(channel.id)}
                        onKeyDown={(event) => {
                          if (event.key === "Enter" || event.key === " ") {
                            event.preventDefault();
                            toggleChannel(channel.id);
                          }
                        }}
                        role="option"
                        tabIndex={0}
                      >
                        <Checkbox
                          checked={selected}
                          className="pointer-events-none"
                          tabIndex={-1}
                        />
                        <span className="text-sm"># {channel.name}</span>
                      </div>
                    );
                  })
                ) : (
                  <p className="px-2 py-3 text-sm text-muted-foreground">
                    No public channels available.
                  </p>
                )}
              </div>
            </div>

            {error ? (
              <p className="rounded-xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                {error.message}
              </p>
            ) : null}
          </div>

          <div className="flex shrink-0 justify-end gap-3 border-t border-border/60 px-6 py-4">
            <Button
              disabled={isPending}
              onClick={() => onOpenChange(false)}
              size="sm"
              type="button"
              variant="outline"
            >
              Cancel
            </Button>
            <Button
              data-testid="group-dialog-submit"
              disabled={!canSubmit}
              onClick={() => {
                void onSubmit({
                  id: group?.id ?? crypto.randomUUID(),
                  handle: normalizedHandle,
                  name: name.trim(),
                  description: description.trim(),
                  creator: group?.creator ?? viewerPubkey.toLowerCase(),
                  memberPubkeys: selectedMemberPubkeys,
                  defaultChannelIds,
                }).catch(() => {});
              }}
              size="sm"
              type="button"
            >
              {isPending
                ? "Saving…"
                : mode === "create"
                  ? "Create group"
                  : "Save changes"}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
