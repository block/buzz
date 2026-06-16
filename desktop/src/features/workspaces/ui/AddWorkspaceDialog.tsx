import * as React from "react";

import {
  DEFAULT_PUBLIC_RELAYS,
  DEFAULT_SERVERLESS_RELAY,
  normalizeRelayList,
  relayListIncludes,
  toggleRelayInList,
} from "@/features/workspaces/defaultRelays";
import type { Workspace } from "@/features/workspaces/types";
import {
  deriveWorkspaceName,
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
  const [serverless, setServerless] = React.useState(false);

  const handleClose = React.useCallback(() => {
    onOpenChange(false);
    setName("");
    setRelayUrl("");
    setToken("");
    setServerless(false);
  }, [onOpenChange]);

  const handleSubmit = React.useCallback(
    (e: React.FormEvent) => {
      e.preventDefault();
      if (!relayUrl.trim()) {
        return;
      }

      const workspace: Workspace = {
        id: crypto.randomUUID(),
        name: name.trim() || deriveWorkspaceName(relayUrl.trim()),
        relayUrl: serverless
          ? normalizeRelayList(relayUrl.trim())
          : normalizeRelayUrl(relayUrl.trim()),
        // Serverless workspaces never use a Sprout API token.
        token: serverless ? undefined : token.trim() || undefined,
        mode: serverless ? "serverless" : "sprout",
        addedAt: new Date().toISOString(),
      };

      onSubmit(workspace);
      handleClose();
    },
    [name, relayUrl, token, serverless, onSubmit, handleClose],
  );

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Add Workspace</DialogTitle>
          <DialogDescription>
            Connect to another Buzz relay, or a generic public Nostr relay in
            serverless mode. Each workspace has its own channels, messages, and
            identity.
          </DialogDescription>
        </DialogHeader>
        <form className="flex flex-col gap-4" onSubmit={handleSubmit}>
          <label className="flex items-start gap-2.5 rounded-md border border-border bg-muted/40 p-3">
            <input
              checked={serverless}
              className="mt-0.5 h-4 w-4 accent-primary"
              onChange={(e) => {
                const on = e.target.checked;
                setServerless(on);
                // Turning serverless ON pre-fills the recommended relays so the
                // user doesn't have to tap every chip. Only auto-fill when the
                // field is empty or still holds the defaults, so we never stomp
                // a custom relay list the user typed. Turning it OFF clears the
                // default list (a single Buzz relay is expected instead).
                if (on) {
                  if (relayUrl.trim() === "") {
                    setRelayUrl(DEFAULT_SERVERLESS_RELAY);
                  }
                } else if (relayUrl.trim() === DEFAULT_SERVERLESS_RELAY) {
                  setRelayUrl("");
                }
              }}
              type="checkbox"
            />
            <span className="flex flex-col gap-0.5">
              <span className="text-sm font-medium text-foreground">
                Serverless mode
              </span>
              <span className="text-xs text-muted-foreground">
                Connect directly to a generic public Nostr relay - no Buzz
                server, database, or auth. Channels and DMs only; search,
                presence, and huddles are unavailable.
              </span>
            </span>
          </label>
          <div className="flex flex-col gap-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="ws-relay-url"
            >
              {serverless ? "Relay URLs" : "Relay URL"}
            </label>
            <Input
              autoFocus
              id="ws-relay-url"
              onChange={(e) => setRelayUrl(e.target.value)}
              placeholder={
                serverless
                  ? DEFAULT_SERVERLESS_RELAY
                  : "wss://relay.example.com"
              }
              type="text"
              value={relayUrl}
            />
            {serverless ? (
              <>
                <p className="text-xs text-muted-foreground">
                  Connects to all listed relays for redundancy. Tap to
                  add/remove.
                </p>
                <div className="flex flex-wrap gap-1.5 pt-0.5">
                  {DEFAULT_PUBLIC_RELAYS.map((relay) => {
                    const selected = relayListIncludes(relayUrl, relay);
                    return (
                      <button
                        className={`rounded-full border px-2 py-0.5 text-xs transition-colors ${
                          selected
                            ? "border-primary bg-primary/15 text-foreground"
                            : "border-border bg-muted/40 text-muted-foreground hover:border-primary/60 hover:text-foreground"
                        }`}
                        key={relay}
                        onClick={() =>
                          setRelayUrl(toggleRelayInList(relayUrl, relay))
                        }
                        type="button"
                      >
                        {relay.replace("wss://", "")}
                      </button>
                    );
                  })}
                </div>
              </>
            ) : null}
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
          {!serverless && (
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
          )}
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
