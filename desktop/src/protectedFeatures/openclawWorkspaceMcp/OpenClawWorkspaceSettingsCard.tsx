import { useCallback, useEffect, useState } from "react";
import { useFeatureEnabled } from "@/shared/features";
import { Button } from "@/shared/ui/button";
import {
  disconnectOpenClawWorkspace,
  fetchOpenClawWorkspaceStatus,
  type OpenClawWorkspaceStatus,
} from "./api";

function formatExpiry(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = Date.parse(iso);
  if (Number.isNaN(d)) return iso;
  return new Date(d).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

export function OpenClawWorkspaceSettingsCard() {
  const enabled = useFeatureEnabled("openclaw-workspace-mcp");
  const [status, setStatus] = useState<OpenClawWorkspaceStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setError(null);
      setStatus(await fetchOpenClawWorkspaceStatus());
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  useEffect(() => {
    if (!enabled) return;
    void refresh();
  }, [enabled, refresh]);

  if (!enabled) return null;

  const connected = status?.connected === true;

  return (
    <div
      className="rounded-xl border border-border/60 bg-card/40 p-4"
      data-testid="settings-openclaw-workspace"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="font-medium text-foreground">OpenClaw workspace</h3>
          <p className="mt-0.5 text-sm text-muted-foreground/70">
            {connected
              ? status?.connectedViaRelay
                ? "Connected via relay"
                : "Connected"
              : "Not connected"}
          </p>
          {connected ? (
            <p className="mt-1 text-sm text-muted-foreground/70">
              Expires {formatExpiry(status?.expiresAt)}
              {status?.url ? (
                <span className="block truncate opacity-80">{status.url}</span>
              ) : null}
              <span className="mt-1 block">
                OpenClaw FS + skill pack mode is available. Enable it per agent
                in Advanced → “Use OpenClaw workspace (MCP)”.
              </span>
            </p>
          ) : (
            <p className="mt-1 text-sm text-muted-foreground/70">
              Join a Hula relay that provisions workspace MCP to connect
              automatically. After connecting, turn on “Use OpenClaw workspace
              (MCP)” on each agent that should use remote FS + skills_list /
              skills_get (default stays local).
            </p>
          )}
          {error ? (
            <p className="mt-2 text-sm text-destructive">{error}</p>
          ) : null}
        </div>
        {connected ? (
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => {
              setBusy(true);
              void disconnectOpenClawWorkspace()
                .then(setStatus)
                .catch((err) =>
                  setError(err instanceof Error ? err.message : String(err)),
                )
                .finally(() => setBusy(false));
            }}
          >
            Disconnect
          </Button>
        ) : null}
      </div>
    </div>
  );
}
