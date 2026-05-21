import * as React from "react";

import { getIdentity } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";

import { initFirstWorkspace, deriveWorkspaceName } from "../workspaceStorage";

const LOCAL_RELAY_URL = "ws://localhost:3000";

type WelcomeSetupProps = {
  defaultRelayUrl: string;
  onComplete: () => void;
};

export function WelcomeSetup({
  defaultRelayUrl,
  onComplete,
}: WelcomeSetupProps) {
  const isInternalBuild = defaultRelayUrl !== LOCAL_RELAY_URL;
  const [relayUrl, setRelayUrl] = React.useState(defaultRelayUrl);
  const [isConnecting, setIsConnecting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const handleConnect = React.useCallback(async () => {
    const trimmedUrl = relayUrl.trim();
    if (!trimmedUrl) {
      setError("Please enter a relay URL.");
      return;
    }

    setIsConnecting(true);
    setError(null);

    try {
      // We snapshot only the pubkey for display purposes (workspace switcher
      // labels, etc.). The private key lives on disk in `identity.key` and
      // is the single source of truth — never copied into localStorage.
      const identity = await getIdentity();
      initFirstWorkspace(trimmedUrl, identity.pubkey);

      // The reload triggered by onComplete() will re-run useWorkspaceInit,
      // which calls applyWorkspace with the saved config. No need to apply here.
      onComplete();
    } catch (err) {
      setError(
        err instanceof Error ? err.message : "Failed to connect. Try again.",
      );
      setIsConnecting(false);
    }
  }, [relayUrl, onComplete]);

  const workspaceName = React.useMemo(
    () => deriveWorkspaceName(relayUrl.trim() || LOCAL_RELAY_URL),
    [relayUrl],
  );

  return (
    <div className="flex min-h-dvh items-center justify-center bg-[radial-gradient(circle_at_top,hsl(var(--primary)/0.14),transparent_48%),linear-gradient(180deg,hsl(var(--background)),hsl(var(--muted)/0.55))] px-4 py-8">
      <div className="w-full max-w-sm rounded-[28px] border border-border/70 bg-background/92 p-8 shadow-2xl backdrop-blur">
        <p className="text-xs font-medium uppercase tracking-[0.2em] text-muted-foreground">
          Sprout
        </p>
        <h1 className="mt-3 text-2xl font-semibold tracking-tight text-foreground">
          Welcome
        </h1>
        <p className="mt-2 text-sm leading-6 text-muted-foreground">
          {isInternalBuild
            ? "Connect to your workspace to get started."
            : "Running a local relay? Connect now. Or enter a custom relay URL."}
        </p>

        <div className="mt-6 space-y-4">
          {!isInternalBuild ? (
            <div className="space-y-1.5">
              <label
                className="text-xs font-medium text-muted-foreground"
                htmlFor="relay-url"
              >
                Relay URL
              </label>
              <Input
                id="relay-url"
                onChange={(e) => {
                  setRelayUrl(e.target.value);
                  setError(null);
                }}
                placeholder="ws://localhost:3000"
                type="url"
                value={relayUrl}
              />
            </div>
          ) : null}

          {error ? <p className="text-sm text-destructive">{error}</p> : null}

          <Button
            className="w-full"
            disabled={isConnecting || !relayUrl.trim()}
            onClick={handleConnect}
            size="default"
            type="button"
          >
            {isConnecting
              ? "Connecting..."
              : isInternalBuild
                ? `Connect to ${workspaceName}`
                : "Connect"}
          </Button>
        </div>
      </div>
    </div>
  );
}
