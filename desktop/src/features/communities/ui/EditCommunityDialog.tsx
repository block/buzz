import { CommunityNameSettings } from "./CommunityNameSettings";
import { reconcileCommunityName } from "../communityName";
import * as React from "react";

import { CommunityIconSettingsCard } from "@/features/communities/ui/CommunityIconSettingsCard";
import { useMyRelayMembershipLookupQuery } from "@/features/community-members/hooks";
import type { Community } from "@/features/communities/types";
import {
  expandTilde,
  normalizeRelayUrl,
} from "@/features/communities/communityStorage";
import { validateReposDir } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";

type EditCommunityDialogProps = {
  community: Community | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (
    id: string,
    updates: Partial<
      Pick<Community, "name" | "localName" | "relayUrl" | "token" | "reposDir">
    >,
  ) => void;
  showIconEditor?: boolean;
};

export function EditCommunityDialog({
  community,
  open,
  onOpenChange,
  onSave,
  showIconEditor = false,
}: EditCommunityDialogProps) {
  const [name, setName] = React.useState("");
  const [relayUrl, setRelayUrl] = React.useState("");
  const [token, setToken] = React.useState("");
  const [reposDir, setReposDir] = React.useState("");
  const [reposDirError, setReposDirError] = React.useState<string | null>(null);
  const membershipQuery = useMyRelayMembershipLookupQuery();
  const activeRole = membershipQuery.data?.membership?.role;
  const canEditIcon =
    showIconEditor &&
    (membershipQuery.data?.membershipRequired === false ||
      activeRole === "owner" ||
      activeRole === "admin");

  const initializedCommunity = React.useRef<string | null>(null);

  // Do not discard draft connection/nickname edits when relay metadata refreshes.
  React.useEffect(() => {
    if (!open) initializedCommunity.current = null;
    if (community && open && initializedCommunity.current !== community.id) {
      initializedCommunity.current = community.id;
      setName(
        reconcileCommunityName(community, {
          name: community.canonicalName ?? null,
        }).localName ?? "",
      );
      setRelayUrl(community.relayUrl);
      setToken(community.token ?? "");
      setReposDir(community.reposDir ?? "");
      setReposDirError(null);
    }
  }, [community, open]);

  const handleClose = React.useCallback(() => {
    onOpenChange(false);
  }, [onOpenChange]);

  const handleSubmit = React.useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      if (!community || !relayUrl.trim()) {
        return;
      }

      const updates: Partial<
        Pick<
          Community,
          "name" | "localName" | "relayUrl" | "token" | "reposDir"
        >
      > = {};

      const trimmedName = name.trim();
      if (trimmedName !== (community.localName ?? "")) {
        updates.localName = trimmedName;
      }

      const normalizedUrl = normalizeRelayUrl(relayUrl.trim());
      if (normalizedUrl !== community.relayUrl) {
        updates.relayUrl = normalizedUrl;
      }

      const trimmedToken = token.trim() || undefined;
      if (trimmedToken !== community.token) {
        updates.token = trimmedToken;
      }

      // Expand `~` to an absolute path before save — the backend rejects
      // tilde paths. An empty field clears the override (REPOS reverts to a
      // real dir). Validate the expanded value (the bytes the backend
      // canonicalizes) before save so a bad path is caught here instead of
      // bricking a later boot. Only emit when the resolved value actually
      // changed so a no-op edit doesn't trigger a backend re-apply.
      const expandedReposDir = await expandTilde(reposDir);
      if (expandedReposDir !== community.reposDir) {
        try {
          await validateReposDir(expandedReposDir ?? "");
        } catch (error) {
          setReposDirError(String(error));
          return;
        }
        updates.reposDir = expandedReposDir;
      }

      if (Object.keys(updates).length > 0) {
        onSave(community.id, updates);
      }

      handleClose();
    },
    [community, name, relayUrl, token, reposDir, onSave, handleClose],
  );

  if (!community) {
    return null;
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-h-[calc(100dvh-2rem)] max-w-md overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Edit Community</DialogTitle>
          <DialogDescription>
            Manage the shared name and this device’s connection settings.
          </DialogDescription>
        </DialogHeader>
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => void handleSubmit(e)}
        >
          <CommunityNameSettings
            key={community.id}
            community={community}
            canRename={canEditIcon}
          />
          {canEditIcon ? (
            <div className="flex items-center justify-between gap-4 rounded-xl border border-border/70 bg-muted/20 p-3">
              <div className="min-w-0">
                <p className="text-sm font-medium">Community icon</p>
                <p className="text-xs text-muted-foreground">
                  Shown in the community rail and switcher.
                </p>
              </div>
              <CommunityIconSettingsCard compact />
            </div>
          ) : null}
          <div className="flex flex-col gap-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="edit-ws-name"
            >
              Local nickname (optional)
            </label>
            <Input
              autoFocus
              id="edit-ws-name"
              onChange={(e) => setName(e.target.value)}
              placeholder="Use shared name"
              type="text"
              value={name}
            />
            <p className="text-xs text-muted-foreground">
              Only on this device. Leave blank to follow the shared name.
              {` Previous device label: ${community.fallbackName ?? community.name}.`}
            </p>
          </div>
          <div className="flex flex-col gap-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="edit-ws-relay-url"
            >
              Relay URL
            </label>
            <Input
              id="edit-ws-relay-url"
              onChange={(e) => setRelayUrl(e.target.value)}
              placeholder="wss://relay.example.com"
              type="text"
              value={relayUrl}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="edit-ws-token"
            >
              API Token
              <span className="ml-1 text-xs font-normal text-muted-foreground">
                (optional)
              </span>
            </label>
            <Input
              id="edit-ws-token"
              onChange={(e) => setToken(e.target.value)}
              placeholder="buzz_..."
              type="password"
              value={token}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="edit-ws-repos-dir"
            >
              Repos Directory
              <span className="ml-1 text-xs font-normal text-muted-foreground">
                (optional)
              </span>
            </label>
            <Input
              id="edit-ws-repos-dir"
              onChange={(e) => {
                setReposDir(e.target.value);
                setReposDirError(null);
              }}
              placeholder="~/Development"
              type="text"
              value={reposDir}
            />
            {reposDirError ? (
              <p className="text-xs text-destructive">{reposDirError}</p>
            ) : null}
            <p className="text-xs text-muted-foreground">
              Point the agent's <code>REPOS</code> directory at an existing
              folder so agents work in your local checkouts. Leave blank to use
              the default location.
            </p>
          </div>
          <div className="flex justify-end gap-2 pt-2">
            <Button onClick={handleClose} type="button" variant="outline">
              Cancel
            </Button>
            <Button disabled={!relayUrl.trim()} type="submit">
              Save Changes
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
