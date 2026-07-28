import * as React from "react";
import {
  CheckCircle2,
  CircleSlash,
  Cloud,
  Loader2,
  Plus,
  RefreshCw,
  Unplug,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { ExtensionEntry, RemoteMcpConnection } from "@/shared/api/types";
import {
  connectPatina,
  disconnectRemoteMcp,
  listRemoteMcpConnections,
  setRemoteMcpEnabled,
  testPatinaConnection,
} from "@/shared/api/tauri";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";

type McpServersSectionProps = {
  extensions: ExtensionEntry[];
  pubkey: string;
  runtimeId: string | null;
  variant?: "compact" | "profile";
  buzzAgentSlot?: React.ReactNode;
};

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function McpServersSection({
  buzzAgentSlot,
  extensions,
  pubkey,
  runtimeId,
  variant = "compact",
}: McpServersSectionProps) {
  const isBuzzAgent = runtimeId === "buzz-agent";
  const [connections, setConnections] = React.useState<RemoteMcpConnection[]>(
    [],
  );
  const [loading, setLoading] = React.useState(true);
  const [busyAction, setBusyAction] = React.useState<string | null>(null);
  const [showPatinaForm, setShowPatinaForm] = React.useState(false);
  const [workspaceSlug, setWorkspaceSlug] = React.useState("");
  const [apiKey, setApiKey] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);

  const patina = connections.find(
    (connection) => connection.provider === "patina",
  );

  React.useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setConnections([]);
    setError(null);
    setBusyAction(null);
    setShowPatinaForm(false);
    setWorkspaceSlug("");
    setApiKey("");
    listRemoteMcpConnections(pubkey)
      .then((result) => {
        if (!cancelled) {
          setConnections(result);
        }
      })
      .catch((loadError) => {
        if (!cancelled) {
          setError(errorMessage(loadError));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [pubkey]);

  const replaceConnection = React.useCallback((next: RemoteMcpConnection) => {
    setConnections((current) => [
      ...current.filter((connection) => connection.id !== next.id),
      next,
    ]);
  }, []);

  const submitPatina = async (event: React.FormEvent) => {
    event.preventDefault();
    setError(null);
    setBusyAction("connect");
    try {
      const connection = await connectPatina({
        pubkey,
        workspaceSlug,
        apiKey,
      });
      replaceConnection(connection);
      setApiKey("");
      setShowPatinaForm(false);
    } catch (connectError) {
      setError(errorMessage(connectError));
    } finally {
      setApiKey("");
      setBusyAction(null);
    }
  };

  const runConnectionAction = async (
    action: "test" | "toggle" | "disconnect",
  ) => {
    if (!patina) {
      return;
    }
    setError(null);
    setBusyAction(action);
    try {
      if (action === "test") {
        replaceConnection(await testPatinaConnection(pubkey));
      } else if (action === "toggle") {
        replaceConnection(
          await setRemoteMcpEnabled(pubkey, patina.id, !patina.enabled),
        );
      } else {
        await disconnectRemoteMcp(pubkey, patina.id);
        setConnections((current) =>
          current.filter((connection) => connection.id !== patina.id),
        );
      }
    } catch (actionError) {
      setError(errorMessage(actionError));
    } finally {
      setBusyAction(null);
    }
  };

  return (
    <div
      className={cn(
        "border-t border-border/50",
        variant === "compact" ? "mt-3 pt-2" : "divide-y divide-border/50",
      )}
    >
      <div
        className={cn(
          "flex items-center justify-between gap-3",
          variant === "compact" ? "py-2" : "px-4 py-3",
        )}
      >
        <p className="text-xs font-medium text-foreground">MCP Servers</p>
        {!patina && !showPatinaForm ? (
          <Button
            data-testid="connect-patina"
            onClick={() => {
              setError(null);
              setShowPatinaForm(true);
            }}
            size="xs"
            type="button"
            variant="outline"
          >
            <Plus />
            Connect Patina
          </Button>
        ) : null}
      </div>

      {isBuzzAgent && buzzAgentSlot ? buzzAgentSlot : null}

      {showPatinaForm ? (
        <form
          className={cn(
            "space-y-3 border-t border-border/50",
            variant === "compact" ? "py-3" : "px-4 py-4",
          )}
          onSubmit={submitPatina}
        >
          <div>
            <p className="text-sm font-medium text-foreground">
              Connect Patina
            </p>
            <p className="mt-1 text-xs text-muted-foreground">
              Use an expiring, viewer-scoped Patina agent key. Buzz verifies
              workspace scope and the read-only tool set before saving it to
              your OS keyring.
            </p>
          </div>
          <label className="block space-y-1" htmlFor="patina-workspace-slug">
            <span className="text-xs font-medium text-foreground">
              Workspace slug
            </span>
            <Input
              autoComplete="off"
              data-testid="patina-workspace-slug"
              id="patina-workspace-slug"
              onChange={(event) => setWorkspaceSlug(event.target.value)}
              placeholder="acme"
              required
              value={workspaceSlug}
            />
          </label>
          <label className="block space-y-1" htmlFor="patina-api-key">
            <span className="text-xs font-medium text-foreground">
              Viewer agent key
            </span>
            <Input
              autoComplete="new-password"
              data-testid="patina-api-key"
              id="patina-api-key"
              onChange={(event) => setApiKey(event.target.value)}
              placeholder="pk_…"
              required
              type="password"
              value={apiKey}
            />
          </label>
          <p className="text-2xs text-muted-foreground/70">
            HTTP MCP compatibility is verified when the agent starts. V1 is
            designed for Codex ACP.
          </p>
          <button
            className="inline-flex text-xs font-medium text-primary hover:underline"
            onClick={() =>
              void openUrl(
                `https://patina.so/${encodeURIComponent(workspaceSlug.trim() || "workspace")}/settings`,
              )
            }
            type="button"
          >
            Open Patina key management
          </button>
          <div className="flex items-center gap-2">
            <Button
              data-testid="patina-test-connect"
              disabled={busyAction !== null}
              size="sm"
              type="submit"
            >
              {busyAction === "connect" ? (
                <Loader2 className="animate-spin" />
              ) : (
                <Cloud />
              )}
              Test & connect
            </Button>
            <Button
              disabled={busyAction !== null}
              onClick={() => {
                setApiKey("");
                setShowPatinaForm(false);
              }}
              size="sm"
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
          </div>
        </form>
      ) : null}

      {error ? (
        <p
          className={cn(
            "border-t border-border/50 py-2 text-xs text-destructive",
            variant === "profile" && "px-4",
          )}
          role="alert"
        >
          {error}
        </p>
      ) : null}

      {loading ? (
        <p
          className={cn(
            "flex items-center gap-2 py-3 text-sm text-muted-foreground",
            variant === "profile" && "px-4",
          )}
        >
          <Loader2 className="h-4 w-4 animate-spin" />
          Loading connections…
        </p>
      ) : patina && !showPatinaForm ? (
        <PatinaConnectionRow
          busyAction={busyAction}
          connection={patina}
          onAction={runConnectionAction}
          onReconnect={() => {
            setWorkspaceSlug(patina.workspaceSlug);
            setApiKey("");
            setError(null);
            setShowPatinaForm(true);
          }}
          variant={variant}
        />
      ) : null}

      {extensions.length > 0 ? (
        <div className="divide-y divide-border/50">
          {extensions.map((extension) => (
            <McpServerRow
              extension={extension}
              key={`${extension.kind}:${extension.name}`}
              variant={variant}
            />
          ))}
        </div>
      ) : !loading && !patina && !showPatinaForm ? (
        <p
          className={cn(
            "text-sm text-muted-foreground",
            variant === "compact" ? "py-2" : "px-4 py-3",
          )}
        >
          No custom servers configured
        </p>
      ) : null}
    </div>
  );
}

function PatinaConnectionRow({
  busyAction,
  connection,
  onAction,
  onReconnect,
  variant,
}: {
  busyAction: string | null;
  connection: RemoteMcpConnection;
  onAction: (action: "test" | "toggle" | "disconnect") => Promise<void>;
  onReconnect: () => void;
  variant: "compact" | "profile";
}) {
  const StatusIcon = connection.enabled ? CheckCircle2 : CircleSlash;
  return (
    <div
      className={cn(
        "border-t border-border/50",
        variant === "compact" ? "py-3" : "px-4 py-3",
      )}
      data-testid="patina-connection"
    >
      <div className="flex min-w-0 items-center gap-3">
        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted/50">
          <StatusIcon
            className={cn(
              "h-4 w-4",
              connection.enabled ? "text-emerald-600" : "text-muted-foreground",
            )}
          />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-medium text-foreground">
            Patina · {connection.workspaceName ?? connection.workspaceSlug}
          </span>
          <span className="mt-0.5 block truncate text-2xs text-muted-foreground/70">
            {connection.principalName ?? "Agent key"} · {connection.status}
          </span>
        </span>
      </div>
      <div className="mt-3 flex flex-wrap gap-2">
        <Button
          disabled={busyAction !== null}
          onClick={() => void onAction("test")}
          size="xs"
          type="button"
          variant="outline"
        >
          {busyAction === "test" ? (
            <Loader2 className="animate-spin" />
          ) : (
            <RefreshCw />
          )}
          Test
        </Button>
        <Button
          disabled={busyAction !== null}
          onClick={() => void onAction("toggle")}
          size="xs"
          type="button"
          variant="outline"
        >
          {connection.enabled ? "Disable" : "Enable"}
        </Button>
        <Button
          disabled={busyAction !== null}
          onClick={onReconnect}
          size="xs"
          type="button"
          variant="ghost"
        >
          Reconnect
        </Button>
        <Button
          disabled={busyAction !== null}
          onClick={() => void onAction("disconnect")}
          size="xs"
          type="button"
          variant="ghost"
        >
          <Unplug />
          Disconnect
        </Button>
      </div>
    </div>
  );
}

function McpServerRow({
  extension,
  variant,
}: {
  extension: ExtensionEntry;
  variant: "compact" | "profile";
}) {
  const StatusIcon = extension.enabled ? CheckCircle2 : CircleSlash;

  return (
    <div
      className={cn(
        "flex min-w-0 items-center gap-3",
        variant === "compact" ? "py-2" : "px-4 py-3",
      )}
    >
      <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted/50">
        <StatusIcon
          className={cn(
            "h-4 w-4",
            extension.enabled ? "text-emerald-600" : "text-muted-foreground",
          )}
        />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-foreground">
          {extension.name}
        </span>
        <span className="mt-0.5 block truncate text-2xs text-muted-foreground/70">
          {extension.kind}
          {extension.enabled ? " enabled" : " disabled"}
        </span>
      </span>
    </div>
  );
}
