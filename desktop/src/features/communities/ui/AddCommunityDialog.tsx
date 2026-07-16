import * as React from "react";

import {
  deriveCommunityName,
  expandTilde,
  normalizeRelayUrl,
} from "@/features/communities/communityStorage";
import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
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

type AddCommunityDialogProps = {
  onSubmit?: (
    community: import("@/features/communities/types").Community,
  ) => void;
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

export function AddCommunityDialog({
  open,
  onOpenChange,
}: AddCommunityDialogProps) {
  const [name, setName] = React.useState("");
  const [relayUrl, setRelayUrl] = React.useState("");
  const [token, setToken] = React.useState("");
  const [inviteCode, setInviteCode] = React.useState("");
  const [reposDir, setReposDir] = React.useState("");
  const communityOnboarding = useCommunityOnboarding();
  const [reposDirError, setReposDirError] = React.useState<string | null>(null);

  const handleClose = React.useCallback(() => {
    onOpenChange(false);
    setName("");
    setRelayUrl("");
    setToken("");
    setInviteCode("");
    setReposDir("");
    setReposDirError(null);
  }, [onOpenChange]);

  const handleSubmit = React.useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      if (!relayUrl.trim()) {
        return;
      }

      // Expand `~` before save — the backend rejects tilde paths. Empty input
      // resolves to `undefined` so REPOS keeps its default location. Validate
      // the expanded value (the bytes the backend canonicalizes) before save
      // so a bad path is caught here instead of bricking a later boot.
      const expandedReposDir = await expandTilde(reposDir);
      try {
        await validateReposDir(expandedReposDir ?? "");
      } catch (error) {
        setReposDirError(String(error));
        return;
      }

      const normalizedRelayUrl = normalizeRelayUrl(relayUrl.trim());
      communityOnboarding.start({
        source: "add-community",
        relayUrl: normalizedRelayUrl,
        inviteCode: inviteCode.trim() || undefined,
        communityName: name.trim() || deriveCommunityName(normalizedRelayUrl),
        token: token.trim() || undefined,
        reposDir: expandedReposDir,
      });
      handleClose();
    },
    [
      name,
      relayUrl,
      token,
      inviteCode,
      reposDir,
      communityOnboarding,
      handleClose,
    ],
  );

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Add Community</DialogTitle>
          <DialogDescription>
            Connect to another Buzz relay. Each community has its own channels,
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
              placeholder="My Community"
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
              htmlFor="ws-invite-code"
            >
              Invite Code
              <span className="ml-1 text-xs font-normal text-muted-foreground">
                (optional)
              </span>
            </label>
            <Input
              id="ws-invite-code"
              onChange={(e) => {
                setInviteCode(e.target.value);
              }}
              placeholder="Paste an invite code for a members-only relay"
              type="text"
              value={inviteCode}
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
          <p className="text-xs text-muted-foreground">
            Communities share your active identity. To use a different key,
            import it on the profile step (or in settings).
          </p>
          <div className="flex justify-end gap-2 pt-2">
            <Button onClick={handleClose} type="button" variant="outline">
              Cancel
            </Button>
            <Button disabled={!relayUrl.trim()} type="submit">
              Add Community
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
