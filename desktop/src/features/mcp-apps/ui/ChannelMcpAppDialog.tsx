import { AppWindow, LoaderCircle, Plus, Trash2 } from "lucide-react";
import * as React from "react";

import {
  installChannelMcpApp,
  removeChannelMcpApp,
  type ChannelMcpAppInstallation,
} from "@/features/mcp-apps/lib/channelMcpAppStorage";
import { mcpAppDisplayLabel } from "@/features/mcp-apps/lib/mcpAppMessage";
import {
  connectMcpAppServer,
  disconnectMcpAppServer,
  inspectMcpAppResource,
  type McpAppResourcePolicy,
  type McpAppServerDescriptor,
  type McpAppTool,
} from "@/shared/api/tauriMcpApps";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";

type ChannelMcpAppDialogProps = {
  apps: ChannelMcpAppInstallation[];
  channelId: string;
  open: boolean;
  pubkey: string;
  onOpenChange: (open: boolean) => void;
};

function installationId(endpoint: string, toolName: string): string {
  return `${encodeURIComponent(endpoint)}:${toolName}`;
}

function initialArguments(tool: McpAppTool): Record<string, unknown> {
  const required = Array.isArray(tool.inputSchema.required)
    ? tool.inputSchema.required.filter(
        (name): name is string => typeof name === "string",
      )
    : [];
  const properties =
    tool.inputSchema.properties &&
    typeof tool.inputSchema.properties === "object" &&
    !Array.isArray(tool.inputSchema.properties)
      ? (tool.inputSchema.properties as Record<string, unknown>)
      : {};
  return Object.fromEntries(
    required.map((name) => {
      const property = properties[name];
      const type =
        property && typeof property === "object" && !Array.isArray(property)
          ? (property as Record<string, unknown>).type
          : undefined;
      return [name, type === "array" ? [] : type === "boolean" ? false : ""];
    }),
  );
}

function parseArguments(value: string): Record<string, unknown> | null {
  try {
    const parsed: unknown = JSON.parse(value);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

function approvedDomains(policy: McpAppResourcePolicy | null): string[] {
  const csp = policy?.csp;
  return [
    csp?.connectDomains,
    csp?.resourceDomains,
    csp?.frameDomains,
    csp?.baseUriDomains,
  ]
    .flatMap((value) => (Array.isArray(value) ? value : []))
    .filter((value): value is string => typeof value === "string")
    .filter((value, index, values) => values.indexOf(value) === index);
}

function requestedPermissions(policy: McpAppResourcePolicy | null): string[] {
  const permissions = policy?.requestedPermissions;
  if (!permissions) return [];
  return [
    permissions.camera ? "Camera" : null,
    permissions.microphone ? "Microphone" : null,
    permissions.geolocation ? "Location" : null,
    permissions.clipboardWrite ? "Clipboard write" : null,
  ].filter((value): value is string => value !== null);
}

export function ChannelMcpAppDialog({
  apps,
  channelId,
  open,
  pubkey,
  onOpenChange,
}: ChannelMcpAppDialogProps) {
  const [endpoint, setEndpoint] = React.useState("");
  const [server, setServer] = React.useState<McpAppServerDescriptor | null>(
    null,
  );
  const [selectedTool, setSelectedTool] = React.useState<McpAppTool | null>(
    null,
  );
  const [argumentsJson, setArgumentsJson] = React.useState("{}");
  const [isConnecting, setIsConnecting] = React.useState(false);
  const [isInspecting, setIsInspecting] = React.useState(false);
  const [approvedPolicy, setApprovedPolicy] =
    React.useState<McpAppResourcePolicy | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const serverRef = React.useRef<McpAppServerDescriptor | null>(null);
  const connectionAttemptRef = React.useRef(0);
  const inspectionAttemptRef = React.useRef(0);

  const uiTools = React.useMemo(
    () => server?.tools.filter((tool) => tool.uiResourceUri) ?? [],
    [server],
  );
  const parsedArguments = React.useMemo(
    () => parseArguments(argumentsJson),
    [argumentsJson],
  );
  const selectedDomains = React.useMemo(
    () => approvedDomains(approvedPolicy),
    [approvedPolicy],
  );
  const selectedPermissions = React.useMemo(
    () => requestedPermissions(approvedPolicy),
    [approvedPolicy],
  );

  const clearConnectedServer = React.useCallback(() => {
    const current = serverRef.current;
    serverRef.current = null;
    setServer(null);
    setSelectedTool(null);
    setApprovedPolicy(null);
    setIsInspecting(false);
    if (current) void disconnectMcpAppServer(current.serverId);
  }, []);

  React.useEffect(() => {
    const serverId = server?.serverId;
    const resourceUri = selectedTool?.uiResourceUri;
    inspectionAttemptRef.current += 1;
    const attempt = inspectionAttemptRef.current;
    setApprovedPolicy(null);
    if (!serverId || !resourceUri) {
      setIsInspecting(false);
      return;
    }
    setIsInspecting(true);
    setError(null);
    void inspectMcpAppResource(serverId, resourceUri)
      .then((policy) => {
        if (inspectionAttemptRef.current === attempt) {
          setApprovedPolicy(policy);
        }
      })
      .catch((cause) => {
        if (inspectionAttemptRef.current !== attempt) return;
        setError(
          cause instanceof Error
            ? cause.message
            : "Buzz could not inspect this MCP App resource.",
        );
      })
      .finally(() => {
        if (inspectionAttemptRef.current === attempt) {
          setIsInspecting(false);
        }
      });
  }, [selectedTool?.uiResourceUri, server?.serverId]);

  React.useEffect(
    () => () => {
      connectionAttemptRef.current += 1;
      inspectionAttemptRef.current += 1;
      const current = serverRef.current;
      serverRef.current = null;
      if (current) void disconnectMcpAppServer(current.serverId);
    },
    [],
  );

  function handleOpenChange(nextOpen: boolean) {
    if (!nextOpen) {
      connectionAttemptRef.current += 1;
      clearConnectedServer();
      setError(null);
      setIsConnecting(false);
    }
    onOpenChange(nextOpen);
  }

  async function handleConnect() {
    const nextEndpoint = endpoint.trim();
    if (!nextEndpoint) return;
    const attempt = connectionAttemptRef.current + 1;
    connectionAttemptRef.current = attempt;
    clearConnectedServer();
    setIsConnecting(true);
    setError(null);
    try {
      const next = await connectMcpAppServer(nextEndpoint);
      if (connectionAttemptRef.current !== attempt) {
        await disconnectMcpAppServer(next.serverId);
        return;
      }
      const tools = next.tools.filter((tool) => tool.uiResourceUri);
      serverRef.current = next;
      setServer(next);
      setSelectedTool(tools[0] ?? null);
      setArgumentsJson(
        JSON.stringify(tools[0] ? initialArguments(tools[0]) : {}, null, 2),
      );
      if (tools.length === 0) {
        setError("This server did not advertise any MCP Apps.");
      }
    } catch (cause) {
      if (connectionAttemptRef.current !== attempt) return;
      clearConnectedServer();
      setError(
        cause instanceof Error
          ? cause.message
          : "Buzz could not connect to this MCP server.",
      );
    } finally {
      if (connectionAttemptRef.current === attempt) setIsConnecting(false);
    }
  }

  function handleSelectTool(tool: McpAppTool) {
    setSelectedTool(tool);
    setArgumentsJson(JSON.stringify(initialArguments(tool), null, 2));
    setError(null);
  }

  function handleInstall() {
    if (
      !server ||
      !selectedTool?.uiResourceUri ||
      !parsedArguments ||
      !approvedPolicy
    ) {
      return;
    }
    const installed = installChannelMcpApp(pubkey, channelId, {
      id: installationId(server.endpoint, selectedTool.name),
      endpoint: server.endpoint,
      serverName: mcpAppDisplayLabel(server.name, server.endpoint, 120),
      toolName: selectedTool.name,
      title: mcpAppDisplayLabel(
        selectedTool.title || selectedTool.name,
        selectedTool.name,
      ),
      resourceUri: selectedTool.uiResourceUri,
      arguments: parsedArguments,
      approvedPolicy,
    });
    if (!installed) {
      setError("Buzz could not save this channel app.");
      return;
    }
    handleOpenChange(false);
  }

  return (
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <DialogContent
        className="max-h-[min(760px,calc(100vh-2rem))] max-w-xl overflow-y-auto"
        data-testid="channel-mcp-app-dialog"
      >
        <DialogHeader>
          <DialogTitle>Channel apps</DialogTitle>
          <DialogDescription>
            Add an MCP App as a tab beside this channel’s conversation.
          </DialogDescription>
        </DialogHeader>

        {apps.length > 0 ? (
          <section className="space-y-2" aria-label="Installed channel apps">
            <h3 className="text-xs font-medium text-muted-foreground">
              Installed
            </h3>
            <div className="overflow-hidden rounded-xl border border-border/60">
              {apps.map((app) => (
                <div
                  className="flex items-center gap-3 border-b border-border/60 px-3 py-2.5 last:border-b-0"
                  key={app.id}
                >
                  <AppWindow className="h-4 w-4 text-muted-foreground" />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-medium">
                      {app.title}
                    </span>
                    <span className="block truncate text-2xs text-muted-foreground">
                      {app.serverName}
                    </span>
                  </span>
                  <Button
                    aria-label={`Remove ${app.title}`}
                    onClick={() =>
                      removeChannelMcpApp(pubkey, channelId, app.id)
                    }
                    size="icon-xs"
                    type="button"
                    variant="ghost"
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                </div>
              ))}
            </div>
          </section>
        ) : null}

        <section className="space-y-3" aria-label="Connect an MCP App server">
          <h3 className="text-xs font-medium text-muted-foreground">
            Connect a server
          </h3>
          <div className="flex gap-2">
            <Input
              aria-label="MCP server endpoint"
              onChange={(event) => setEndpoint(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") void handleConnect();
              }}
              placeholder="https://runtime.example.com/mcp"
              value={endpoint}
            />
            <Button
              disabled={!endpoint.trim() || isConnecting}
              onClick={() => void handleConnect()}
              type="button"
              variant="secondary"
            >
              {isConnecting ? (
                <LoaderCircle className="h-4 w-4 animate-spin" />
              ) : (
                "Connect"
              )}
            </Button>
          </div>

          {uiTools.length > 0 ? (
            <div className="grid gap-2 sm:grid-cols-2">
              {uiTools.map((tool) => (
                <button
                  aria-pressed={selectedTool?.name === tool.name}
                  className="rounded-xl border border-border/60 p-3 text-left transition-colors hover:bg-accent/50 aria-pressed:border-primary aria-pressed:bg-primary/5"
                  data-testid="channel-mcp-app-tool"
                  key={tool.name}
                  onClick={() => handleSelectTool(tool)}
                  type="button"
                >
                  <span className="block text-sm font-medium">
                    {mcpAppDisplayLabel(tool.title || tool.name, tool.name)}
                  </span>
                  {tool.description ? (
                    <span className="mt-1 line-clamp-2 block text-xs text-muted-foreground">
                      {mcpAppDisplayLabel(tool.description, "", 240)}
                    </span>
                  ) : null}
                </button>
              ))}
            </div>
          ) : null}

          {selectedTool ? (
            <>
              <div className="space-y-1.5 rounded-xl border border-border/60 bg-muted/20 p-3">
                <p className="text-xs font-medium text-foreground">
                  Requested network access
                </p>
                {isInspecting ? (
                  <p className="text-xs text-muted-foreground">
                    Reading the app resource…
                  </p>
                ) : selectedDomains.length > 0 ? (
                  <ul className="space-y-1 font-mono text-2xs text-muted-foreground">
                    {selectedDomains.map((domain) => (
                      <li className="break-all" key={domain}>
                        {domain}
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p className="text-xs text-muted-foreground">
                    No external domains.
                  </p>
                )}
                {selectedPermissions.length > 0 ? (
                  <>
                    <p className="pt-2 text-xs font-medium text-foreground">
                      Requested browser permissions
                    </p>
                    <p className="text-xs text-muted-foreground">
                      {selectedPermissions.join(", ")}. Buzz does not grant
                      these permissions in this version.
                    </p>
                  </>
                ) : null}
              </div>
              <label
                className="block space-y-1.5"
                htmlFor="mcp-app-tool-arguments"
              >
                <span className="text-xs font-medium text-muted-foreground">
                  Tool arguments
                </span>
                <Textarea
                  aria-invalid={parsedArguments === null}
                  className="min-h-28 font-mono text-xs"
                  id="mcp-app-tool-arguments"
                  onChange={(event) => setArgumentsJson(event.target.value)}
                  value={argumentsJson}
                />
                {parsedArguments === null ? (
                  <span className="text-xs text-destructive">
                    Enter one JSON object.
                  </span>
                ) : null}
              </label>
            </>
          ) : null}
        </section>

        {error ? (
          <p className="text-sm text-destructive" role="alert">
            {error}
          </p>
        ) : null}

        <DialogFooter>
          <Button
            onClick={() => handleOpenChange(false)}
            type="button"
            variant="ghost"
          >
            Cancel
          </Button>
          <Button
            data-testid="channel-mcp-app-add-tab"
            disabled={
              !server ||
              !selectedTool ||
              !parsedArguments ||
              !approvedPolicy ||
              isInspecting
            }
            onClick={handleInstall}
            type="button"
          >
            <Plus className="mr-1.5 h-4 w-4" />
            Add tab
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
