import * as React from "react";

import type { Workspace } from "@/features/workspaces/types";
import {
  deriveWorkspaceName,
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

type AddWorkspaceDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (workspace: Workspace) => void;
};

export function AddWorkspaceDialog({
  open,
  onOpenChange,
  onSubmit,
}: AddWorkspaceDialogProps) {
  const [name, setName] = React.useState("");
  const [relayUrl, setRelayUrl] = React.useState("");
  const [token, setToken] = React.useState("");
  const [reposDir, setReposDir] = React.useState("");

  const handleClose = React.useCallback(() => {
    onOpenChange(false);
    setName("");
    setRelayUrl("");
    setToken("");
    setReposDir("");
  }, [onOpenChange]);

  const handleSubmit = React.useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      if (!relayUrl.trim()) {
        return;
      }

      // Expand `~` before save — the backend rejects tilde paths. Empty input
      // resolves to `undefined` so REPOS keeps its default location.
      const expandedReposDir = await expandTilde(reposDir);

      const workspace: Workspace = {
        id: crypto.randomUUID(),
        name: name.trim() || deriveWorkspaceName(relayUrl.trim()),
        relayUrl: normalizeRelayUrl(relayUrl.trim()),
        token: token.trim() || undefined,
        reposDir: expandedReposDir,
        addedAt: new Date().toISOString(),
      };

      onSubmit(workspace);
      handleClose();
    },
    [name, relayUrl, token, reposDir, onSubmit, handleClose],
  );

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Add Workspace</DialogTitle>
          <DialogDescription>
            Connect to another Buzz relay. Each workspace has its own channels,
            messages, and identity.
          </DialogDescription>
        </DialogHeader>
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => void handleSubmit(e)}
        >
          <div className="flex flex-col gap-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="ws-relay-url"
            >
              Relay URL
            </label>
            <Input
              autoFocus
              id="ws-relay-url"
              onChange={(e) => setRelayUrl(e.target.value)}
              placeholder="wss://relay.example.com"
              type="text"
              value={relayUrl}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="ws-name"
            >
              Name
              <span className="ml-1 text-xs font-normal text-muted-foreground">
                (optional)
              </span>
            </label>
            <Input
              id="ws-name"
              onChange={(e) => setName(e.target.value)}
              placeholder="My Workspace"
              type="text"
              value={name}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="ws-token"
            >
              API Token
              <span className="ml-1 text-xs font-normal text-muted-foreground">
                (optional)
              </span>
            </label>
            <Input
              id="ws-token"
              onChange={(e) => setToken(e.target.value)}
              placeholder="buzz_..."
              type="password"
              value={token}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="ws-repos-dir"
            >
              Repos Directory
              <span className="ml-1 text-xs font-normal text-muted-foreground">
                (optional)
              </span>
            </label>
            <Input
              id="ws-repos-dir"
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
          <p className="text-xs text-muted-foreground">
            Workspaces share your active identity. To use a different key,
            import it on the profile step (or in settings).
          </p>
          <div className="flex justify-end gap-2 pt-2">
            <Button onClick={handleClose} type="button" variant="outline">
              Cancel
            </Button>
            <Button disabled={!relayUrl.trim()} type="submit">
              Add Workspace
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
