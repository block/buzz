import * as React from "react";

import type { Workspace } from "@/features/workspaces/types";
import {
  expandTilde,
  normalizeRelayUrl,
} from "@/features/workspaces/workspaceStorage";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";

type EditWorkspaceDialogProps = {
  workspace: Workspace | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (
    id: string,
    updates: Partial<
      Pick<Workspace, "name" | "relayUrl" | "token" | "reposDir">
    >,
  ) => void;
  onRemove?: (id: string) => void;
  canRemove?: boolean;
};

export function EditWorkspaceDialog({
  workspace,
  open,
  onOpenChange,
  onSave,
  onRemove,
  canRemove,
}: EditWorkspaceDialogProps) {
  const [name, setName] = React.useState("");
  const [relayUrl, setRelayUrl] = React.useState("");
  const [token, setToken] = React.useState("");
  const [reposDir, setReposDir] = React.useState("");

  // Sync form state when the dialog opens with a workspace
  React.useEffect(() => {
    if (workspace && open) {
      setName(workspace.name);
      setRelayUrl(workspace.relayUrl);
      setToken(workspace.token ?? "");
      setReposDir(workspace.reposDir ?? "");
    }
  }, [workspace, open]);

  const handleClose = React.useCallback(() => {
    onOpenChange(false);
  }, [onOpenChange]);

  const handleSubmit = React.useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      if (!workspace || !relayUrl.trim()) {
        return;
      }

      const updates: Partial<
        Pick<Workspace, "name" | "relayUrl" | "token" | "reposDir">
      > = {};

      const trimmedName = name.trim();
      if (trimmedName && trimmedName !== workspace.name) {
        updates.name = trimmedName;
      }

      const normalizedUrl = normalizeRelayUrl(relayUrl.trim());
      if (normalizedUrl !== workspace.relayUrl) {
        updates.relayUrl = normalizedUrl;
      }

      const trimmedToken = token.trim() || undefined;
      if (trimmedToken !== workspace.token) {
        updates.token = trimmedToken;
      }

      // Expand `~` to an absolute path before save — the backend rejects
      // tilde paths. An empty field clears the override (REPOS reverts to a
      // real dir). Only emit when the resolved value actually changed so a
      // no-op edit doesn't trigger a backend re-apply.
      const expandedReposDir = await expandTilde(reposDir);
      if (expandedReposDir !== workspace.reposDir) {
        updates.reposDir = expandedReposDir;
      }

      if (Object.keys(updates).length > 0) {
        onSave(workspace.id, updates);
      }

      handleClose();
    },
    [workspace, name, relayUrl, token, reposDir, onSave, handleClose],
  );

  const handleRemove = React.useCallback(() => {
    if (workspace && onRemove) {
      onRemove(workspace.id);
      handleClose();
    }
  }, [workspace, onRemove, handleClose]);

  if (!workspace) {
    return null;
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Edit Workspace</DialogTitle>
          <DialogDescription>
            Update this workspace's name or relay URL.
          </DialogDescription>
        </DialogHeader>
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => void handleSubmit(e)}
        >
          <div className="flex flex-col gap-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="edit-ws-name"
            >
              Name
            </label>
            <Input
              autoFocus
              id="edit-ws-name"
              onChange={(e) => setName(e.target.value)}
              placeholder="My Workspace"
              type="text"
              value={name}
            />
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
              onChange={(e) => setReposDir(e.target.value)}
              placeholder="~/Development"
              type="text"
              value={reposDir}
            />
            <p className="text-xs text-muted-foreground">
              Point the agent's <code>REPOS</code> directory at an existing
              folder so agents work in your local checkouts. Leave blank to use
              the default location.
            </p>
          </div>
          <div className="flex items-center justify-between pt-2">
            <div>
              {canRemove && onRemove ? (
                <Button
                  className="text-destructive hover:text-destructive"
                  onClick={handleRemove}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  Remove Workspace
                </Button>
              ) : null}
            </div>
            <div className="flex gap-2">
              <Button onClick={handleClose} type="button" variant="outline">
                Cancel
              </Button>
              <Button disabled={!name.trim() || !relayUrl.trim()} type="submit">
                Save Changes
              </Button>
            </div>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
