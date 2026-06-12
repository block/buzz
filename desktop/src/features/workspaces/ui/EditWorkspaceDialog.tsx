import * as React from "react";

import type { Workspace } from "@/features/workspaces/types";
import { normalizeRelayUrl } from "@/features/workspaces/workspaceStorage";
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
    updates: Partial<Pick<Workspace, "name" | "relayUrl" | "token">>,
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

  // Sync form state when the dialog opens with a workspace
  React.useEffect(() => {
    if (workspace && open) {
      setName(workspace.name);
      setRelayUrl(workspace.relayUrl);
      setToken(workspace.token ?? "");
    }
  }, [workspace, open]);

  const handleClose = React.useCallback(() => {
    onOpenChange(false);
  }, [onOpenChange]);

  const handleSubmit = React.useCallback(
    (e: React.FormEvent) => {
      e.preventDefault();
      if (!workspace || !relayUrl.trim()) {
        return;
      }

      const updates: Partial<Pick<Workspace, "name" | "relayUrl" | "token">> =
        {};

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

      if (Object.keys(updates).length > 0) {
        onSave(workspace.id, updates);
      }

      handleClose();
    },
    [workspace, name, relayUrl, token, onSave, handleClose],
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
        <form className="flex flex-col gap-4" onSubmit={handleSubmit}>
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
