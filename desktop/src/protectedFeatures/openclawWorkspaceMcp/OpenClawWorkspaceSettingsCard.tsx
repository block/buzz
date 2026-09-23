import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { relayClient } from "@/shared/api/relayClient";
import { useReconnectRelay } from "@/shared/api/useReconnectRelay";
import { useFeatureEnabled } from "@/shared/features";
import { Button } from "@/shared/ui/button";
import {
  disconnectOpenClawWorkspace,
  emitOpenClawWorkspaceStatusChanged,
  fetchOpenClawWorkspaceStatus,
  refreshOpenClawWorkspace,
  testOpenClawWorkspace,
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

const RECONNECT_POLL_MS = 15_000;
const RECONNECT_POLL_INTERVAL_MS = 800;

type BusyAction = "test" | "refresh" | "disconnect" | "reconnect" | null;

async function pollOpenClawUntilConnected(
  onTick: (status: OpenClawWorkspaceStatus) => void,
  timeoutMs = RECONNECT_POLL_MS,
  intervalMs = RECONNECT_POLL_INTERVAL_MS,
): Promise<OpenClawWorkspaceStatus> {
  const deadline = Date.now() + timeoutMs;
  let last = await fetchOpenClawWorkspaceStatus();
  onTick(last);
  while (!last.connected && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, intervalMs));
    last = await fetchOpenClawWorkspaceStatus();
    onTick(last);
  }
  return last;
}

export function OpenClawWorkspaceSettingsCard() {
  const enabled = useFeatureEnabled("openclaw-workspace-mcp");
  const { reconnect, isPending: isRelayReconnectPending } = useReconnectRelay();
  const [status, setStatus] = useState<OpenClawWorkspaceStatus | null>(null);
  const [busy, setBusy] = useState<BusyAction>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      setError(null);
      setStatus(await fetchOpenClawWorkspaceStatus());
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  useEffect(() => {
    if (!enabled) return;
    void reload();
  }, [enabled, reload]);

  if (!enabled) return null;

  const connected = status?.connected === true;
  const anyBusy = busy !== null || isRelayReconnectPending;

  async function runAction(
    action: Exclude<BusyAction, null>,
    work: () => Promise<void>,
  ) {
    setBusy(action);
    setError(null);
    try {
      await work();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      toast.error(message);
    } finally {
      setBusy(null);
    }
  }

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
                with “Use OpenClaw workspace (MCP)” when creating or editing an
                agent.
              </span>
            </p>
          ) : (
            <p className="mt-1 text-sm text-muted-foreground/70">
              Join a Hula relay that provisions workspace MCP to connect
              automatically, or use Reconnect to force a relay AUTH so a new
              grant can arrive. After connecting, turn on “Use OpenClaw
              workspace (MCP)” on each agent that should use remote FS +
              skills_list / skills_get (default stays local).
            </p>
          )}
          {error ? (
            <p className="mt-2 text-sm text-destructive">{error}</p>
          ) : null}
        </div>
        {connected ? (
          <div className="flex shrink-0 flex-wrap justify-end gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={anyBusy}
              data-testid="settings-openclaw-workspace-test"
              onClick={() => {
                void runAction("test", async () => {
                  const result = await testOpenClawWorkspace();
                  if (result.ok) {
                    toast.success(result.message);
                  } else {
                    setError(result.message);
                    toast.error(result.message);
                  }
                });
              }}
            >
              {busy === "test" ? "Testing…" : "Test"}
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={anyBusy}
              data-testid="settings-openclaw-workspace-refresh"
              onClick={() => {
                void runAction("refresh", async () => {
                  const next = await refreshOpenClawWorkspace();
                  setStatus(next);
                  emitOpenClawWorkspaceStatusChanged();
                  toast.success(
                    "Re-applied the stored grant to Claude MCP configs. A new JWT arrives on the next relay AUTH.",
                  );
                });
              }}
            >
              {busy === "refresh" ? "Refreshing…" : "Refresh"}
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={anyBusy}
              data-testid="settings-openclaw-workspace-disconnect"
              onClick={() => {
                void runAction("disconnect", async () => {
                  const next = await disconnectOpenClawWorkspace();
                  setStatus(next);
                  emitOpenClawWorkspaceStatusChanged();
                  const stopped = next.agentsUpdated ?? 0;
                  toast.success(
                    stopped > 0
                      ? `Disconnected OpenClaw workspace. Stopped ${stopped} OpenClaw agent${stopped === 1 ? "" : "s"}.`
                      : "Disconnected OpenClaw workspace.",
                  );
                });
              }}
            >
              {busy === "disconnect" ? "Disconnecting…" : "Disconnect"}
            </Button>
          </div>
        ) : (
          <div className="flex shrink-0 flex-wrap justify-end gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={anyBusy}
              data-testid="settings-openclaw-workspace-reconnect"
              onClick={() => {
                void runAction("reconnect", async () => {
                  // Mint happens only after NIP-42 AUTH. If the relay socket is
                  // already up, drop it so preconnect runs a full AUTH and the
                  // relay can remint + push a HULA capability (same path as
                  // initial auto-connect). OpenClawWorkspaceRelayListener /
                  // handleProtectedRelayPayload still apply the grant.
                  if (relayClient.getConnectionState() === "connected") {
                    relayClient.disconnect();
                  }
                  await reconnect();
                  const next = await pollOpenClawUntilConnected(setStatus);
                  emitOpenClawWorkspaceStatusChanged();
                  if (next.connected) {
                    toast.success(
                      next.connectedViaRelay
                        ? "OpenClaw workspace connected via relay."
                        : "OpenClaw workspace connected.",
                    );
                  } else {
                    toast.message(
                      "Still waiting for relay AUTH / grant — stay joined to the Hula relay.",
                    );
                  }
                });
              }}
            >
              {busy === "reconnect" || isRelayReconnectPending
                ? "Reconnecting…"
                : "Reconnect"}
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}
