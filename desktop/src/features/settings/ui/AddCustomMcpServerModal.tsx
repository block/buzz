import * as React from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Loader2,
  Play,
  Terminal,
  Globe,
  XCircle,
} from "lucide-react";

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
import { Badge } from "@/shared/ui/badge";
import { SegmentedControl } from "@/shared/ui/segmented-control";
import { cn } from "@/shared/lib/cn";

export interface McpDiscoveredTool {
  name: string;
  description?: string;
  input_schema?: Record<string, unknown>;
}

export interface McpTestResult {
  success: boolean;
  latency_ms: number;
  tools_count: number;
  tools: McpDiscoveredTool[];
  error?: string;
}

export interface McpServerConfig {
  id: string;
  name: string;
  transport: string;
  command?: string;
  args?: string[];
  cwd?: string;
  env?: Record<string, string>;
  url?: string;
  headers?: Record<string, string>;
  bearer_token?: string;
  enabled: boolean;
  created_at?: string;
}

interface AddCustomMcpServerModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onServerSaved: () => void;
  initialServer?: McpServerConfig | null;
}

export function AddCustomMcpServerModal({
  open,
  onOpenChange,
  onServerSaved,
  initialServer,
}: AddCustomMcpServerModalProps) {
  const [name, setName] = React.useState(initialServer?.name ?? "");
  const [id, setId] = React.useState(initialServer?.id ?? "");
  const [transport, setTransport] = React.useState<"stdio" | "sse">(
    (initialServer?.transport as "stdio" | "sse") ?? "stdio",
  );
  const [command, setCommand] = React.useState(initialServer?.command ?? "");
  const [args, setArgs] = React.useState(initialServer?.args?.join(" ") ?? "");
  const [cwd, setCwd] = React.useState(initialServer?.cwd ?? "");
  const [url, setUrl] = React.useState(initialServer?.url ?? "");
  const [bearerToken, setBearerToken] = React.useState(
    initialServer?.bearer_token ?? "",
  );

  const [testing, setTesting] = React.useState(false);
  const [testResult, setTestResult] = React.useState<McpTestResult | null>(
    null,
  );
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [showToolsList, setShowToolsList] = React.useState(false);

  React.useEffect(() => {
    if (initialServer) {
      setName(initialServer.name);
      setId(initialServer.id);
      setTransport((initialServer.transport as "stdio" | "sse") || "stdio");
      setCommand(initialServer.command ?? "");
      setArgs(initialServer.args?.join(" ") ?? "");
      setCwd(initialServer.cwd ?? "");
      setUrl(initialServer.url ?? "");
      setBearerToken(initialServer.bearer_token ?? "");
    } else {
      setName("");
      setId("");
      setTransport("stdio");
      setCommand("");
      setArgs("");
      setCwd("");
      setUrl("");
      setBearerToken("");
    }
    setTestResult(null);
    setError(null);
    setShowToolsList(false);
  }, [initialServer]);

  const handleNameChange = (val: string) => {
    setName(val);
    if (!initialServer) {
      setId(val.toLowerCase().replace(/[^a-z0-9_-]/g, "-"));
    }
  };

  const buildCurrentConfig = (): McpServerConfig => {
    const serverId =
      id.trim() || name.toLowerCase().replace(/[^a-z0-9_-]/g, "-");
    return {
      id: serverId,
      name: name.trim(),
      transport,
      command: transport === "stdio" ? command.trim() : undefined,
      args:
        transport === "stdio" && args.trim()
          ? args.trim().split(/\s+/)
          : undefined,
      cwd: transport === "stdio" && cwd.trim() ? cwd.trim() : undefined,
      url: transport === "sse" ? url.trim() : undefined,
      bearer_token:
        transport === "sse" && bearerToken.trim()
          ? bearerToken.trim()
          : undefined,
      enabled: initialServer ? initialServer.enabled : true,
      created_at: initialServer?.created_at ?? new Date().toISOString(),
    };
  };

  const handleTestConnection = async () => {
    setError(null);
    setTestResult(null);

    if (transport === "stdio" && !command.trim()) {
      setError("Please specify an executable command or binary path.");
      return;
    }
    if (transport === "sse" && !url.trim()) {
      setError("Please specify a valid server URL.");
      return;
    }

    try {
      setTesting(true);
      const config = buildCurrentConfig();
      const res = await invoke<McpTestResult>("test_mcp_connection", {
        config,
      });
      setTestResult(res);
      if (res.tools_count > 0) {
        setShowToolsList(true);
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setTesting(false);
    }
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) {
      setError("Server name is required.");
      return;
    }
    if (transport === "stdio" && !command.trim()) {
      setError("Command is required for stdio transport.");
      return;
    }
    if (transport === "sse" && !url.trim()) {
      setError("Server URL is required for SSE transport.");
      return;
    }

    try {
      setSaving(true);
      setError(null);
      const config = buildCurrentConfig();
      await invoke("save_mcp_server", { server: config });
      onServerSaved();
      onOpenChange(false);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl" data-testid="mcp-server-modal">
        <form onSubmit={handleSave} className="space-y-4">
          <DialogHeader>
            <DialogTitle>
              {initialServer ? "Edit MCP Server" : "Add Custom MCP Server"}
            </DialogTitle>
            <DialogDescription>
              Connect external Model Context Protocol (MCP) servers via local
              stdio subprocesses or remote SSE endpoints.
            </DialogDescription>
          </DialogHeader>

          {error && (
            <div className="rounded-md bg-destructive/15 p-3 text-sm text-destructive flex items-start gap-2">
              <XCircle className="h-4 w-4 mt-0.5 shrink-0" />
              <span>{error}</span>
            </div>
          )}

          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-1.5">
              <label
                htmlFor="mcp-server-name"
                className="text-sm font-medium text-foreground"
              >
                Server Name
              </label>
              <Input
                id="mcp-server-name"
                data-testid="mcp-server-name-input"
                placeholder="e.g. Postgres DB Explorer"
                value={name}
                onChange={(e) => handleNameChange(e.target.value)}
                required
              />
            </div>
            <div className="space-y-1.5">
              <label
                htmlFor="mcp-server-id"
                className="text-sm font-medium text-foreground"
              >
                Server Identifier
              </label>
              <Input
                id="mcp-server-id"
                data-testid="mcp-server-id-input"
                placeholder="e.g. postgres-mcp"
                value={id}
                onChange={(e) => setId(e.target.value)}
                required
              />
            </div>
          </div>

          <div className="space-y-1.5">
            <span className="text-sm font-medium text-foreground block">
              Transport Type
            </span>
            <SegmentedControl<"stdio" | "sse">
              legend="Transport Type"
              testId="mcp-transport-selector"
              optionTestIdPrefix="mcp-transport"
              value={transport}
              onValueChange={(val) => setTransport(val)}
              options={[
                {
                  value: "stdio",
                  label: "Stdio (Subprocess)",
                  Icon: Terminal,
                },
                {
                  value: "sse",
                  label: "SSE / HTTP",
                  Icon: Globe,
                },
              ]}
            />
          </div>

          {transport === "stdio" ? (
            <div className="space-y-3 rounded-lg border border-border/60 bg-muted/20 p-3.5">
              <div className="space-y-1.5">
                <label
                  htmlFor="mcp-cmd"
                  className="text-xs font-semibold uppercase tracking-wider text-muted-foreground"
                >
                  Executable Command or Binary Path
                </label>
                <Input
                  id="mcp-cmd"
                  data-testid="mcp-cmd-input"
                  placeholder="e.g. npx, uvx, python, or /usr/local/bin/server"
                  value={command}
                  onChange={(e) => setCommand(e.target.value)}
                  required={transport === "stdio"}
                />
              </div>

              <div className="space-y-1.5">
                <label
                  htmlFor="mcp-args"
                  className="text-xs font-semibold uppercase tracking-wider text-muted-foreground"
                >
                  Arguments (space-separated)
                </label>
                <Input
                  id="mcp-args"
                  data-testid="mcp-args-input"
                  placeholder="e.g. -y @modelcontextprotocol/server-postgres postgresql://localhost/mydb"
                  value={args}
                  onChange={(e) => setArgs(e.target.value)}
                />
              </div>

              <div className="space-y-1.5">
                <label
                  htmlFor="mcp-cwd"
                  className="text-xs font-semibold uppercase tracking-wider text-muted-foreground"
                >
                  Working Directory (Optional)
                </label>
                <Input
                  id="mcp-cwd"
                  placeholder="Defaults to workspace root"
                  value={cwd}
                  onChange={(e) => setCwd(e.target.value)}
                />
              </div>
            </div>
          ) : (
            <div className="space-y-3 rounded-lg border border-border/60 bg-muted/20 p-3.5">
              <div className="space-y-1.5">
                <label
                  htmlFor="mcp-url"
                  className="text-xs font-semibold uppercase tracking-wider text-muted-foreground"
                >
                  Server Endpoint URL
                </label>
                <Input
                  id="mcp-url"
                  data-testid="mcp-url-input"
                  placeholder="e.g. http://localhost:8000/sse or https://mcp.internal.net/events"
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  required={transport === "sse"}
                />
              </div>

              <div className="space-y-1.5">
                <label
                  htmlFor="mcp-token"
                  className="text-xs font-semibold uppercase tracking-wider text-muted-foreground"
                >
                  Bearer Token (Optional)
                </label>
                <Input
                  id="mcp-token"
                  type="password"
                  placeholder="eyJhbGciOi..."
                  value={bearerToken}
                  onChange={(e) => setBearerToken(e.target.value)}
                />
              </div>
            </div>
          )}

          {/* Test connection results container */}
          {testResult && (
            <div
              className={cn(
                "rounded-lg border p-3.5 text-sm transition-all",
                testResult.success
                  ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-950 dark:text-emerald-200"
                  : "border-destructive/30 bg-destructive/10 text-destructive",
              )}
              data-testid="mcp-test-result"
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  {testResult.success ? (
                    <CheckCircle2 className="h-4 w-4 text-emerald-500 shrink-0" />
                  ) : (
                    <XCircle className="h-4 w-4 text-destructive shrink-0" />
                  )}
                  <span className="font-medium">
                    {testResult.success
                      ? "Connection successful"
                      : "Handshake failed"}
                  </span>
                  <Badge variant="outline" className="text-xs">
                    {testResult.latency_ms} ms
                  </Badge>
                  {testResult.success && (
                    <Badge variant="secondary" className="text-xs">
                      {testResult.tools_count} tools discovered
                    </Badge>
                  )}
                </div>

                {testResult.tools_count > 0 && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-7 text-xs"
                    onClick={() => setShowToolsList(!showToolsList)}
                  >
                    {showToolsList ? (
                      <span className="flex items-center gap-1">
                        Hide Schema <ChevronDown className="h-3 w-3" />
                      </span>
                    ) : (
                      <span className="flex items-center gap-1">
                        View Schema ({testResult.tools_count}){" "}
                        <ChevronRight className="h-3 w-3" />
                      </span>
                    )}
                  </Button>
                )}
              </div>

              {testResult.error && (
                <p className="mt-2 text-xs font-mono text-destructive/90 whitespace-pre-wrap">
                  {testResult.error}
                </p>
              )}

              {showToolsList && testResult.tools.length > 0 && (
                <div className="mt-3 max-h-48 overflow-y-auto space-y-2 rounded border border-border/40 bg-background/50 p-2 font-mono text-xs">
                  {testResult.tools.map((t) => (
                    <div
                      key={t.name}
                      className="border-b border-border/30 pb-1.5 last:border-b-0"
                    >
                      <div className="font-semibold text-foreground">
                        {t.name}
                      </div>
                      {t.description && (
                        <div className="text-muted-foreground text-[11px] font-sans">
                          {t.description}
                        </div>
                      )}
                      {t.input_schema && (
                        <pre className="text-[10px] text-muted-foreground/80 mt-1 max-h-16 overflow-x-auto">
                          {JSON.stringify(t.input_schema, null, 2)}
                        </pre>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}

          <DialogFooter className="flex items-center justify-between sm:justify-between pt-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={handleTestConnection}
              disabled={testing}
              data-testid="mcp-test-connection-btn"
            >
              {testing ? (
                <>
                  <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                  Testing Handshake...
                </>
              ) : (
                <>
                  <Play className="mr-1.5 h-3.5 w-3.5" />
                  Test Connection
                </>
              )}
            </Button>

            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => onOpenChange(false)}
              >
                Cancel
              </Button>
              <Button
                type="submit"
                size="sm"
                disabled={saving}
                data-testid="mcp-save-server-btn"
              >
                {saving && (
                  <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                )}
                {initialServer ? "Save Changes" : "Add Server"}
              </Button>
            </div>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
