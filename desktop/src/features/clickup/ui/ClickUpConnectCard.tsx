import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { KeyRound, ShieldCheck } from "lucide-react";

import { Button } from "@/shared/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/shared/ui/card";
import { Input } from "@/shared/ui/input";

const CLICKUP_APPS_SETTINGS_URL = "https://app.clickup.com/settings/apps";

type ClickUpConnectCardProps = {
  errorMessage?: string | null;
  isPending: boolean;
  onConnect: (token: string) => Promise<void>;
};

export function ClickUpConnectCard({
  errorMessage,
  isPending,
  onConnect,
}: ClickUpConnectCardProps) {
  const [token, setToken] = React.useState("");

  const handleSubmit = React.useCallback(
    async (event: React.FormEvent) => {
      event.preventDefault();
      const value = token.trim();
      if (!value) return;
      setToken("");
      try {
        await onConnect(value);
      } catch {
        // The parent renders the typed connection error. Sensitive values are
        // cleared before native IPC and must be entered again after failure.
      }
    },
    [onConnect, token],
  );

  return (
    <div className="flex flex-1 items-center justify-center px-4 py-10 sm:px-6">
      <Card className="w-full max-w-xl" data-testid="clickup-connect-card">
        <CardHeader className="space-y-3">
          <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-primary/10 text-primary">
            <KeyRound className="h-5 w-5" />
          </div>
          <div className="space-y-1">
            <CardTitle className="text-xl">Connect ClickUp locally</CardTitle>
            <p className="text-sm leading-6 text-muted-foreground">
              Connect a personal token to see tasks assigned to you. This
              prototype only reads ClickUp data.
            </p>
          </div>
        </CardHeader>
        <CardContent>
          <form className="space-y-4" onSubmit={handleSubmit}>
            <div className="space-y-2">
              <label className="text-sm font-medium" htmlFor="clickup-token">
                Personal API token
              </label>
              <Input
                autoComplete="off"
                data-testid="clickup-token-input"
                disabled={isPending}
                id="clickup-token"
                onChange={(event) => setToken(event.target.value)}
                placeholder="pk_…"
                type="password"
                value={token}
              />
              <p className="text-xs leading-5 text-muted-foreground">
                The token is sent once to the native Buzz process, verified
                against ClickUp, and stored in your operating system keyring. It
                is never returned to this screen.
              </p>
            </div>
            {errorMessage ? (
              <p
                className="rounded-xl bg-destructive/10 px-3 py-2 text-xs text-destructive"
                role="alert"
              >
                {errorMessage}
              </p>
            ) : null}
            <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
              <Button
                data-testid="connect-clickup"
                disabled={isPending || token.trim().length === 0}
                type="submit"
              >
                {isPending ? "Connecting…" : "Connect ClickUp"}
              </Button>
              <Button
                onClick={() => void openUrl(CLICKUP_APPS_SETTINGS_URL)}
                type="button"
                variant="ghost"
              >
                Create or copy a token
              </Button>
            </div>
          </form>
          <div className="mt-5 flex gap-2 rounded-xl bg-muted/40 px-3 py-3 text-xs leading-5 text-muted-foreground">
            <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-emerald-500" />
            <p>
              Buzz will not create, update, comment on, move, or delete ClickUp
              tasks in this version. Disconnecting only removes the local
              credential.
            </p>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
