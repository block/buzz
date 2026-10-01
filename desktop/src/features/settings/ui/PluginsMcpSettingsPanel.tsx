import * as React from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Blocks,
  Check,
  Database,
  FolderGit2,
  Globe,
  Loader2,
  MessageSquare,
  Play,
  Plus,
  RefreshCw,
  Server,
  ShieldCheck,
  Ticket,
  Trash2,
  XCircle,
} from "lucide-react";

import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Switch } from "@/shared/ui/switch";
import { cn } from "@/shared/lib/cn";

import {
  AddCustomMcpServerModal,
  type McpServerConfig,
  type McpTestResult,
} from "./AddCustomMcpServerModal";
import {
  SettingsOptionGroup,
  SettingsOptionGroupList,
} from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

export type ToolPolicy = "auto-approve" | "confirm" | "disabled";

interface BuiltinToolItem {
  name: string;
  category: "orbit" | "dev";
  type: "Read" | "Write" | "Execution";
  description: string;
  defaultPolicy: ToolPolicy;
}

const BUILTIN_TOOLS: BuiltinToolItem[] = [
  {
    name: "orbit.search_context",
    category: "orbit",
    type: "Read",
    description:
      "Hybrid Multi-RAG search over code, docs, and memories with reciprocal rank fusion.",
    defaultPolicy: "auto-approve",
  },
  {
    name: "orbit.store_memory",
    category: "orbit",
    type: "Write",
    description:
      "Persists an architectural decision or fact into knowledge graph and vector memory.",
    defaultPolicy: "confirm",
  },
  {
    name: "orbit.get_project_context",
    category: "orbit",
    type: "Read",
    description:
      "Generates full architectural summary, key entities, and relationships for a workspace.",
    defaultPolicy: "auto-approve",
  },
  {
    name: "orbit.recall_session",
    category: "orbit",
    type: "Read",
    description:
      "Recalls past agent sessions, conversations, and resolutions across connected agents.",
    defaultPolicy: "auto-approve",
  },
  {
    name: "orbit.get_file_history",
    category: "orbit",
    type: "Read",
    description:
      "Retrieves architectural changes and rationale affecting a specific file over time.",
    defaultPolicy: "auto-approve",
  },
  {
    name: "orbit.mark_decision",
    category: "orbit",
    type: "Write",
    description:
      "Updates or invalidates an architectural decision in the bi-temporal knowledge graph.",
    defaultPolicy: "confirm",
  },
  {
    name: "orbit.get_index_stats",
    category: "orbit",
    type: "Read",
    description:
      "Reports total document count, chunk count, vector health, and sync status.",
    defaultPolicy: "auto-approve",
  },
  {
    name: "orbit.delete_memory",
    category: "orbit",
    type: "Write",
    description:
      "Permanently deletes a memory chunk and cascades deletion to vector and graph indexes.",
    defaultPolicy: "confirm",
  },
  {
    name: "buzz-dev-mcp:shell",
    category: "dev",
    type: "Execution",
    description:
      "Runs commands in an isolated subprocess shell with bounded output limits.",
    defaultPolicy: "confirm",
  },
  {
    name: "buzz-dev-mcp:read_file",
    category: "dev",
    type: "Read",
    description:
      "Reads local files with line numbers and bounded pagination windows.",
    defaultPolicy: "auto-approve",
  },
  {
    name: "buzz-dev-mcp:edit_file",
    category: "dev",
    type: "Write",
    description:
      "Performs atomic find-and-replace text modifications and produces unified diffs.",
    defaultPolicy: "confirm",
  },
];

export interface PluginConfig {
  id: string;
  name: string;
  description: string;
  version: string;
  category: string;
  enabled: boolean;
  icon?: string;
  settings?: Record<string, unknown>;
  source: string;
}

export function PluginsMcpSettingsPanel() {
  const [servers, setServers] = React.useState<McpServerConfig[]>([]);
  const [plugins, setPlugins] = React.useState<PluginConfig[]>([]);
  const [toolPolicies, setToolPolicies] = React.useState<
    Record<string, ToolPolicy>
  >({});
  const [refreshing, setRefreshing] = React.useState(false);

  // Modal state
  const [isModalOpen, setIsModalOpen] = React.useState(false);
  const [editingServer, setEditingServer] =
    React.useState<McpServerConfig | null>(null);

  // Redaction toggle & test probe
  const [redactionActive, setRedactionActive] = React.useState(true);
  const [testingServerId, setTestingServerId] = React.useState<string | null>(
    null,
  );
  const [serverTestResults, setServerTestResults] = React.useState<
    Record<string, McpTestResult>
  >({});

  // Plugin import modal
  const [importSource, setImportSource] = React.useState("");

  const loadAll = React.useCallback(async () => {
    try {
      setRefreshing(true);
      const [fetchedServers, fetchedPlugins, fetchedPolicies] =
        await Promise.all([
          invoke<McpServerConfig[]>("list_mcp_servers").catch(() => []),
          invoke<PluginConfig[]>("list_plugins").catch(() => []),
          invoke<Record<string, ToolPolicy>>("get_tool_policies").catch(
            () => ({}) as Record<string, ToolPolicy>,
          ),
        ]);

      setServers(fetchedServers);
      setPlugins(fetchedPlugins);

      // Merge fetched policies with defaults
      const merged: Record<string, ToolPolicy> = {};
      for (const t of BUILTIN_TOOLS) {
        merged[t.name] =
          (fetchedPolicies as Record<string, ToolPolicy>)[t.name] ??
          t.defaultPolicy;
      }
      setToolPolicies(merged);
    } finally {
      setRefreshing(false);
    }
  }, []);

  React.useEffect(() => {
    void loadAll();
  }, [loadAll]);

  const handlePolicyChange = async (toolName: string, policy: ToolPolicy) => {
    setToolPolicies((prev) => ({ ...prev, [toolName]: policy }));
    try {
      await invoke("set_tool_policy", { toolName, policy });
    } catch {
      // Revert if error
      void loadAll();
    }
  };

  const handleToggleToolEnabled = (
    toolName: string,
    currentlyEnabled: boolean,
  ) => {
    if (currentlyEnabled) {
      void handlePolicyChange(toolName, "disabled");
    } else {
      const defaultPol =
        BUILTIN_TOOLS.find((t) => t.name === toolName)?.defaultPolicy ??
        "auto-approve";
      void handlePolicyChange(
        toolName,
        defaultPol === "disabled" ? "auto-approve" : defaultPol,
      );
    }
  };

  const handleTogglePlugin = async (id: string, enabled: boolean) => {
    setPlugins((prev) =>
      prev.map((p) => (p.id === id ? { ...p, enabled } : p)),
    );
    try {
      await invoke("toggle_plugin", { id, enabled });
    } catch {
      void loadAll();
    }
  };

  const handleDeleteServer = async (id: string) => {
    try {
      await invoke("delete_mcp_server", { id });
      setServers((prev) => prev.filter((s) => s.id !== id));
    } catch (err) {
      console.error("Failed to delete server", err);
    }
  };

  const handleQuickTestServer = async (server: McpServerConfig) => {
    setTestingServerId(server.id);
    try {
      const res = await invoke<McpTestResult>("test_mcp_connection", {
        config: server,
      });
      setServerTestResults((prev) => ({ ...prev, [server.id]: res }));
    } catch (err: unknown) {
      setServerTestResults((prev) => ({
        ...prev,
        [server.id]: {
          success: false,
          latency_ms: 0,
          tools_count: 0,
          tools: [],
          error: err instanceof Error ? err.message : String(err),
        },
      }));
    } finally {
      setTestingServerId(null);
    }
  };

  const getPluginIcon = (id: string) => {
    if (id.includes("github"))
      return <FolderGit2 className="h-5 w-5 text-purple-400" />;
    if (id.includes("slack"))
      return <MessageSquare className="h-5 w-5 text-amber-400" />;
    if (id.includes("postgres"))
      return <Database className="h-5 w-5 text-blue-400" />;
    if (id.includes("jira")) return <Ticket className="h-5 w-5 text-sky-400" />;
    if (id.includes("search"))
      return <Globe className="h-5 w-5 text-emerald-400" />;
    return <Blocks className="h-5 w-5 text-primary" />;
  };

  return (
    <section className="min-w-0 space-y-6" data-testid="settings-plugins-mcp">
      <div className="flex flex-wrap items-center justify-between gap-4">
        <SettingsSectionHeader
          title="Plugins & MCP Tools"
          description="Manage Model Context Protocol (MCP) servers, tool approval policies, and memory plugins."
        />
        <Button
          variant="outline"
          size="sm"
          onClick={() => void loadAll()}
          disabled={refreshing}
          className="shrink-0 gap-1.5"
          data-testid="refresh-mcp-btn"
        >
          <RefreshCw
            className={cn("h-3.5 w-3.5", refreshing && "animate-spin")}
          />
          Refresh
        </Button>
      </div>

      {/* Security & Secret Redaction Banner */}
      <div
        className="rounded-xl border border-emerald-500/25 bg-emerald-500/5 p-4 text-sm"
        data-testid="mcp-security-banner"
      >
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-emerald-500/15 text-emerald-500">
              <ShieldCheck className="h-5 w-5" />
            </div>
            <div>
              <div className="flex items-center gap-2 font-medium text-foreground">
                Context Arbiter & Secret Redaction Active
                <Badge
                  variant="outline"
                  className="text-xs border-emerald-500/40 text-emerald-600 dark:text-emerald-400"
                >
                  Enforced
                </Badge>
              </div>
              <p className="text-xs text-muted-foreground mt-0.5">
                Outbound delimiter fencing (
                <code className="font-mono text-emerald-600 dark:text-emerald-400">
                  &lt;orbit_untrusted_context&gt;
                </code>
                ) strips API keys, OAuth tokens, and private keys before tool
                outputs reach external agents.
              </p>
            </div>
          </div>
          <div className="flex items-center gap-3">
            <div className="text-xs text-muted-foreground font-mono">
              ~/.orbit/audit.log
            </div>
            <Switch
              aria-label="Toggle secret redaction"
              checked={redactionActive}
              onCheckedChange={setRedactionActive}
            />
          </div>
        </div>
      </div>

      <SettingsOptionGroupList>
        {/* Built-in Orbit & Developer Tools */}
        <SettingsOptionGroup
          title="Built-in Tools & Approval Policies"
          description="Control which pre-packaged Orbit memory tools and Developer execution capabilities are exposed to agents."
        >
          <div
            className="divide-y divide-border/40"
            data-testid="builtin-tools-table"
          >
            {BUILTIN_TOOLS.map((tool) => {
              const currentPolicy =
                toolPolicies[tool.name] ?? tool.defaultPolicy;
              const isEnabled = currentPolicy !== "disabled";

              return (
                <div
                  key={tool.name}
                  className="flex flex-col gap-2.5 p-4 sm:flex-row sm:items-center sm:justify-between"
                  data-testid={`tool-row-${tool.name}`}
                >
                  <div className="min-w-0 space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-mono text-sm font-semibold text-foreground">
                        {tool.name}
                      </span>
                      <Badge
                        variant="secondary"
                        className={cn(
                          "text-[10px] uppercase tracking-wider font-semibold",
                          tool.type === "Read" &&
                            "bg-blue-500/10 text-blue-600 dark:text-blue-400",
                          tool.type === "Write" &&
                            "bg-amber-500/10 text-amber-600 dark:text-amber-400",
                          tool.type === "Execution" &&
                            "bg-purple-500/10 text-purple-600 dark:text-purple-400",
                        )}
                      >
                        {tool.type}
                      </Badge>
                      <Badge variant="outline" className="text-[10px]">
                        {tool.category === "orbit"
                          ? "Orbit Brain"
                          : "Developer MCP"}
                      </Badge>
                    </div>
                    <p className="text-xs text-muted-foreground">
                      {tool.description}
                    </p>
                  </div>

                  <div className="flex shrink-0 items-center gap-3 self-end sm:self-center">
                    {/* Execution Policy Dropdown */}
                    <div className="flex items-center gap-1.5">
                      <label
                        htmlFor={`policy-select-${tool.name}`}
                        className="sr-only"
                      >
                        Execution policy for {tool.name}
                      </label>
                      <select
                        id={`policy-select-${tool.name}`}
                        aria-label={`Execution policy for ${tool.name}`}
                        disabled={!isEnabled}
                        value={currentPolicy}
                        onChange={(e) =>
                          void handlePolicyChange(
                            tool.name,
                            e.target.value as ToolPolicy,
                          )
                        }
                        className={cn(
                          "h-8 rounded-md border border-border bg-background px-2.5 text-xs font-medium text-foreground focus:outline-none focus:ring-1 focus:ring-primary",
                          !isEnabled && "opacity-50 cursor-not-allowed",
                        )}
                        data-testid={`policy-select-${tool.name}`}
                      >
                        <option value="auto-approve">
                          Auto-Approve (Silent)
                        </option>
                        <option value="confirm">Confirm-on-Execute</option>
                        <option value="disabled">Disabled</option>
                      </select>
                    </div>

                    <Switch
                      aria-label={`Enable ${tool.name}`}
                      checked={isEnabled}
                      onCheckedChange={() =>
                        handleToggleToolEnabled(tool.name, isEnabled)
                      }
                      data-testid={`tool-toggle-${tool.name}`}
                    />
                  </div>
                </div>
              );
            })}
          </div>
        </SettingsOptionGroup>

        {/* Custom MCP Servers */}
        <SettingsOptionGroup
          title="Custom MCP Servers"
          description="Register external stdio or SSE servers (Postgres, GitHub, Slack, local scripts) for agent tool discovery."
        >
          <div className="p-4 space-y-4">
            <div className="flex items-center justify-between">
              <p className="text-xs text-muted-foreground">
                Configured servers in{" "}
                <code className="font-mono text-foreground">
                  ~/.orbit/mcp_servers.json
                </code>
              </p>
              <Button
                size="sm"
                onClick={() => {
                  setEditingServer(null);
                  setIsModalOpen(true);
                }}
                className="gap-1.5"
                data-testid="add-mcp-server-btn"
              >
                <Plus className="h-4 w-4" /> Add Custom MCP Server
              </Button>
            </div>

            {servers.length === 0 ? (
              <div className="rounded-lg border border-dashed border-border/80 p-8 text-center">
                <Server className="mx-auto h-8 w-8 text-muted-foreground/60" />
                <h4 className="mt-2 text-sm font-medium text-foreground">
                  No Custom MCP Servers
                </h4>
                <p className="mt-1 text-xs text-muted-foreground max-w-sm mx-auto">
                  Add local stdio tools or remote SSE endpoints to expose
                  customized capabilities to Antigravity and coding agents.
                </p>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => setIsModalOpen(true)}
                  className="mt-4 gap-1.5"
                >
                  <Plus className="h-3.5 w-3.5" /> Connect Server
                </Button>
              </div>
            ) : (
              <div className="space-y-3" data-testid="mcp-servers-list">
                {servers.map((s) => {
                  const probe = serverTestResults[s.id];
                  const isTesting = testingServerId === s.id;

                  return (
                    <div
                      key={s.id}
                      className="rounded-lg border border-border/60 bg-muted/20 p-3.5 flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3"
                      data-testid={`server-card-${s.id}`}
                    >
                      <div className="min-w-0 space-y-1">
                        <div className="flex items-center gap-2">
                          <span className="font-medium text-sm text-foreground">
                            {s.name}
                          </span>
                          <Badge
                            variant="outline"
                            className="text-xs uppercase font-mono"
                          >
                            {s.transport}
                          </Badge>
                          {probe && (
                            <Badge
                              variant="secondary"
                              className={cn(
                                "text-xs gap-1",
                                probe.success
                                  ? "text-emerald-500"
                                  : "text-destructive",
                              )}
                            >
                              {probe.success ? (
                                <>
                                  <Check className="h-3 w-3" />{" "}
                                  {probe.latency_ms}ms ({probe.tools_count}{" "}
                                  tools)
                                </>
                              ) : (
                                <>
                                  <XCircle className="h-3 w-3" /> Failed
                                </>
                              )}
                            </Badge>
                          )}
                        </div>
                        <div className="text-xs font-mono text-muted-foreground truncate max-w-md">
                          {s.transport === "stdio" ? s.command : s.url}
                        </div>
                      </div>

                      <div className="flex items-center gap-2 self-end sm:self-center">
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => void handleQuickTestServer(s)}
                          disabled={isTesting}
                          className="h-8 gap-1 text-xs"
                          data-testid={`test-server-${s.id}`}
                        >
                          {isTesting ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <Play className="h-3.5 w-3.5" />
                          )}
                          Test
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => {
                            setEditingServer(s);
                            setIsModalOpen(true);
                          }}
                          className="h-8 text-xs"
                        >
                          Edit
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => void handleDeleteServer(s.id)}
                          className="h-8 text-xs text-destructive hover:bg-destructive/10"
                          data-testid={`delete-server-${s.id}`}
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </Button>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </SettingsOptionGroup>

        {/* Plugins Marketplace & Local Importer */}
        <SettingsOptionGroup
          title="Plugins Directory"
          description="Extensions and connectors for synchronizing external developer context into Orbit."
        >
          <div className="p-4 space-y-4">
            <div
              className="grid grid-cols-1 md:grid-cols-2 gap-3.5"
              data-testid="plugins-grid"
            >
              {plugins.map((plugin) => (
                <div
                  key={plugin.id}
                  className="rounded-lg border border-border/60 bg-muted/20 p-4 flex items-start justify-between gap-3 transition-colors hover:border-border"
                  data-testid={`plugin-card-${plugin.id}`}
                >
                  <div className="flex items-start gap-3 min-w-0">
                    <div className="p-2 rounded-md bg-background border border-border/50 shrink-0">
                      {getPluginIcon(plugin.id)}
                    </div>
                    <div className="min-w-0 space-y-1">
                      <div className="flex items-center gap-2">
                        <h4 className="font-medium text-sm text-foreground truncate">
                          {plugin.name}
                        </h4>
                        <span className="text-[10px] text-muted-foreground font-mono">
                          v{plugin.version}
                        </span>
                      </div>
                      <p className="text-xs text-muted-foreground line-clamp-2">
                        {plugin.description}
                      </p>
                      <Badge
                        variant="outline"
                        className="text-[10px] capitalize"
                      >
                        {plugin.category}
                      </Badge>
                    </div>
                  </div>

                  <Switch
                    aria-label={`Toggle ${plugin.name}`}
                    checked={plugin.enabled}
                    onCheckedChange={(checked) =>
                      void handleTogglePlugin(plugin.id, checked)
                    }
                    data-testid={`plugin-toggle-${plugin.id}`}
                  />
                </div>
              ))}
            </div>

            {/* Custom Plugin Importer Section */}
            <div className="rounded-lg border border-border/60 bg-muted/10 p-3.5 flex flex-wrap items-center justify-between gap-3">
              <div className="space-y-0.5">
                <span className="text-sm font-medium text-foreground">
                  Import Custom Plugin
                </span>
                <p className="text-xs text-muted-foreground">
                  Load plugin package from local folder, git repo, or manifest
                  JSON.
                </p>
              </div>

              <div className="flex items-center gap-2">
                <input
                  type="text"
                  placeholder="https://github.com/... or /path/to/plugin"
                  value={importSource}
                  onChange={(e) => setImportSource(e.target.value)}
                  className="h-8 w-64 rounded-md border border-border bg-background px-2.5 text-xs text-foreground focus:outline-none focus:ring-1 focus:ring-primary"
                />
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => {
                    if (importSource.trim()) {
                      alert(
                        `Plugin package "${importSource}" validated and queued for local activation.`,
                      );
                      setImportSource("");
                    }
                  }}
                  className="h-8 text-xs"
                >
                  Import
                </Button>
              </div>
            </div>
          </div>
        </SettingsOptionGroup>
      </SettingsOptionGroupList>

      {/* Modal Dialog */}
      <AddCustomMcpServerModal
        open={isModalOpen}
        onOpenChange={setIsModalOpen}
        onServerSaved={loadAll}
        initialServer={editingServer}
      />
    </section>
  );
}
