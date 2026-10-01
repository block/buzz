import * as React from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  Bot,
  Brain,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Cpu,
  ExternalLink,
  Feather,
  FileCode,
  Layers,
  Loader2,
  MessageSquare,
  Plug,
  RefreshCw,
  ShieldCheck,
  Sparkles,
  Terminal,
  Unplug,
} from "lucide-react";

import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";

export type HarnessStatus = "connected" | "detected" | "not_installed";

export interface HarnessInfo {
  id: string;
  name: string;
  status: HarnessStatus;
  config_path: string;
  skill_path: string;
  description: string;
  icon: string;
  docs_url?: string;
}

const HARNESS_ICONS: Record<
  string,
  React.ComponentType<{ className?: string }>
> = {
  sparkles: Sparkles,
  terminal: Terminal,
  code: FileCode,
  "file-code": FileCode,
  feather: Feather,
  cpu: Cpu,
  layers: Layers,
  command: Terminal,
  "message-square": MessageSquare,
  bot: Bot,
};

export function AgentHarnessHub() {
  const [harnesses, setHarnesses] = React.useState<HarnessInfo[]>([]);
  const [scanning, setScanning] = React.useState(true);
  const [connectingId, setConnectingId] = React.useState<string | null>(null);
  const [disconnectingId, setDisconnectingId] = React.useState<string | null>(
    null,
  );
  const [showSkillSpec, setShowSkillSpec] = React.useState(false);
  const [lastActionSuccess, setLastActionSuccess] = React.useState<
    string | null
  >(null);

  const scanHarnesses = React.useCallback(async () => {
    try {
      setScanning(true);
      const res = await invoke<HarnessInfo[]>("detect_agent_harnesses").catch(
        () => [],
      );
      setHarnesses(res);
    } finally {
      setScanning(false);
    }
  }, []);

  React.useEffect(() => {
    void scanHarnesses();
  }, [scanHarnesses]);

  const handleConnect = async (id: string) => {
    setConnectingId(id);
    try {
      await invoke("connect_harness", { harnessId: id });
      setLastActionSuccess(`Successfully wired ${id} to Central Orbit Brain!`);
      setTimeout(() => setLastActionSuccess(null), 4000);
      await scanHarnesses();
    } catch (err) {
      console.error(`Failed to connect harness ${id}:`, err);
    } finally {
      setConnectingId(null);
    }
  };

  const handleDisconnect = async (id: string) => {
    setDisconnectingId(id);
    try {
      await invoke("disconnect_harness", { harnessId: id });
      await scanHarnesses();
    } catch (err) {
      console.error(`Failed to disconnect harness ${id}:`, err);
    } finally {
      setDisconnectingId(null);
    }
  };

  const connectedCount = harnesses.filter(
    (h) => h.status === "connected",
  ).length;
  const detectedCount = harnesses.filter((h) => h.status === "detected").length;

  return (
    <div className="space-y-6" data-testid="agent-harness-hub">
      {/* Header and Live Status */}
      <div className="rounded-xl border border-border/70 bg-card/60 p-5 shadow-sm backdrop-blur-sm">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div className="space-y-1">
            <div className="flex items-center gap-2.5">
              <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-primary/10 text-primary">
                <Brain className="h-5 w-5" />
              </div>
              <h3 className="text-base font-semibold text-foreground">
                Centralized Brain — Agent Harness Hub
              </h3>
            </div>
            <p className="text-xs text-muted-foreground">
              Auto-wire local coding agents to central persistent memory (
              <code className="font-mono text-[11px] text-foreground/80">
                ~/.orbit/brain/
              </code>
              ) with 1 click. Zero CLI required.
            </p>
          </div>

          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => void scanHarnesses()}
              disabled={scanning}
              className="h-8 gap-1.5 text-xs"
              data-testid="scan-harnesses-btn"
            >
              <RefreshCw
                className={cn("h-3.5 w-3.5", scanning && "animate-spin")}
              />
              {scanning ? "Scanning..." : "Scan System"}
            </Button>
          </div>
        </div>

        {/* Central Brain Status Strip */}
        <div className="mt-4 grid grid-cols-1 sm:grid-cols-3 gap-2.5 pt-3 border-t border-border/50 text-xs">
          <div className="flex items-center gap-2 bg-muted/20 px-3 py-2 rounded-lg">
            <div className="h-2 w-2 rounded-full bg-emerald-500 animate-pulse" />
            <span className="text-muted-foreground">Brain Engine:</span>
            <span className="font-medium text-foreground">Local-First V1</span>
          </div>

          <div className="flex items-center gap-2 bg-muted/20 px-3 py-2 rounded-lg">
            <ShieldCheck className="h-4 w-4 text-emerald-500" />
            <span className="text-muted-foreground">Context Arbiter:</span>
            <span className="font-medium text-foreground">
              Delimiter Fencing Active
            </span>
          </div>

          <div className="flex items-center gap-2 bg-muted/20 px-3 py-2 rounded-lg">
            <Plug className="h-4 w-4 text-primary" />
            <span className="text-muted-foreground">Connected Agents:</span>
            <span className="font-medium text-foreground">
              {connectedCount} Connected ({detectedCount} Ready)
            </span>
          </div>
        </div>

        {lastActionSuccess && (
          <div className="mt-3 flex items-center gap-2 rounded-md bg-emerald-500/10 px-3 py-2 text-xs text-emerald-600 dark:text-emerald-400">
            <CheckCircle2 className="h-4 w-4 shrink-0" />
            <span>{lastActionSuccess}</span>
          </div>
        )}
      </div>

      {/* Cloud Account & Local Separation Banner */}
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border/50 bg-muted/10 p-3.5 text-xs text-muted-foreground">
        <div className="flex items-center gap-2.5">
          <Badge
            variant="outline"
            className="text-[10px] uppercase font-mono tracking-wider"
          >
            Local Scope
          </Badge>
          <span>
            Harness auto-wiring grants agents local{" "}
            <code className="font-mono text-foreground/80">orbit.*</code> MCP
            access. Cloud sync is a separate explicit opt-in.
          </span>
        </div>
        <div className="flex items-center gap-2">
          <Badge variant="secondary" className="text-[11px]">
            Use Local Brain (No Account Required)
          </Badge>
        </div>
      </div>

      {/* Harnesses Grid */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3.5">
        {harnesses.map((harness) => {
          const Icon = HARNESS_ICONS[harness.icon] || Bot;
          const isConnected = harness.status === "connected";
          const isDetected = harness.status === "detected";
          const isNotInstalled = harness.status === "not_installed";

          return (
            <div
              key={harness.id}
              className={cn(
                "flex flex-col justify-between rounded-xl border p-4 transition-all duration-150",
                isConnected
                  ? "border-emerald-500/40 bg-emerald-500/5 shadow-sm"
                  : isDetected
                    ? "border-primary/40 bg-card hover:border-primary/60"
                    : "border-border/50 bg-muted/10 opacity-75 hover:opacity-100",
              )}
              data-testid={`harness-card-${harness.id}`}
            >
              <div className="space-y-3">
                <div className="flex items-start justify-between gap-2">
                  <div className="flex items-center gap-2.5">
                    <div
                      className={cn(
                        "flex h-9 w-9 items-center justify-center rounded-lg text-sm font-semibold",
                        isConnected
                          ? "bg-emerald-500/15 text-emerald-500"
                          : isDetected
                            ? "bg-primary/10 text-primary"
                            : "bg-muted text-muted-foreground",
                      )}
                    >
                      <Icon className="h-5 w-5" />
                    </div>
                    <div>
                      <h4 className="font-medium text-sm text-foreground">
                        {harness.name}
                      </h4>
                      <p className="text-[11px] text-muted-foreground line-clamp-1">
                        {harness.id}
                      </p>
                    </div>
                  </div>

                  <div>
                    {isConnected && (
                      <Badge className="bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 hover:bg-emerald-500/20 border-emerald-500/30 text-[10px]">
                        ✓ Connected
                      </Badge>
                    )}
                    {isDetected && (
                      <Badge
                        variant="outline"
                        className="border-primary/40 text-primary text-[10px]"
                      >
                        Detected
                      </Badge>
                    )}
                    {isNotInstalled && (
                      <Badge
                        variant="secondary"
                        className="text-[10px] text-muted-foreground"
                      >
                        Not Found
                      </Badge>
                    )}
                  </div>
                </div>

                <p className="text-xs text-muted-foreground/90 leading-relaxed line-clamp-2">
                  {harness.description}
                </p>

                <div className="space-y-1 rounded-md bg-background/50 p-2 text-[10px] font-mono text-muted-foreground border border-border/40">
                  <div className="truncate" title={harness.config_path}>
                    <span className="text-foreground/60 select-none">
                      cfg:{" "}
                    </span>
                    {harness.config_path.split(/[/\\]/).slice(-3).join("/")}
                  </div>
                  <div className="truncate" title={harness.skill_path}>
                    <span className="text-foreground/60 select-none">
                      skill:{" "}
                    </span>
                    {harness.skill_path.split(/[/\\]/).slice(-3).join("/")}
                  </div>
                </div>
              </div>

              {/* Action Buttons */}
              <div className="mt-4 pt-3 border-t border-border/40 flex items-center justify-between gap-2">
                {harness.docs_url ? (
                  <button
                    type="button"
                    onClick={() => {
                      const url = harness.docs_url;
                      if (url) {
                        void openUrl(url);
                      }
                    }}
                    className="inline-flex items-center gap-1 text-[11px] text-muted-foreground hover:text-foreground transition-colors"
                  >
                    <ExternalLink className="h-3 w-3" /> Docs
                  </button>
                ) : null}

                <div className="ml-auto">
                  {isConnected ? (
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => void handleDisconnect(harness.id)}
                      disabled={disconnectingId === harness.id}
                      className="h-7 text-xs text-rose-500 hover:text-rose-600 hover:bg-rose-500/10 border-rose-500/20"
                      data-testid={`disconnect-btn-${harness.id}`}
                    >
                      {disconnectingId === harness.id ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        <Unplug className="h-3 w-3 mr-1" />
                      )}
                      Disconnect
                    </Button>
                  ) : (
                    <Button
                      size="sm"
                      variant={isDetected ? "default" : "outline"}
                      onClick={() => void handleConnect(harness.id)}
                      disabled={connectingId === harness.id}
                      className="h-7 text-xs"
                      data-testid={`connect-btn-${harness.id}`}
                    >
                      {connectingId === harness.id ? (
                        <Loader2 className="h-3 w-3 animate-spin mr-1" />
                      ) : (
                        <Plug className="h-3 w-3 mr-1" />
                      )}
                      Connect to Brain
                    </Button>
                  )}
                </div>
              </div>
            </div>
          );
        })}
      </div>

      {/* Universal Skill Definition Drawer / Collapsible */}
      <div className="rounded-xl border border-border/60 bg-muted/5 overflow-hidden">
        <button
          type="button"
          onClick={() => setShowSkillSpec(!showSkillSpec)}
          className="w-full flex items-center justify-between p-4 text-xs font-medium text-foreground hover:bg-muted/10 transition-colors"
        >
          <div className="flex items-center gap-2">
            <FileCode className="h-4 w-4 text-primary" />
            <span>
              Universal Orbit Skill Injected into Connected Harnesses (
              <code className="font-mono text-[11px]">SKILL.md</code>)
            </span>
          </div>
          {showSkillSpec ? (
            <ChevronUp className="h-4 w-4" />
          ) : (
            <ChevronDown className="h-4 w-4" />
          )}
        </button>

        {showSkillSpec && (
          <div className="p-4 pt-0 border-t border-border/40 text-xs text-muted-foreground space-y-2.5 font-mono">
            <p className="text-foreground font-sans font-medium">
              When you connect an agent harness, Orbit injects the canonical
              cross-agent skill instructions:
            </p>
            <ol className="list-decimal pl-5 space-y-1.5 text-[11px] leading-relaxed">
              <li>
                <strong className="text-foreground font-mono">
                  At Session Start:
                </strong>{" "}
                Calls{" "}
                <code className="text-primary">
                  orbit.get_project_context(path=".")
                </code>{" "}
                to load architectural decisions and conventions.
              </li>
              <li>
                <strong className="text-foreground font-mono">
                  Before Modifying Code:
                </strong>{" "}
                Calls{" "}
                <code className="text-primary">
                  orbit.get_file_history(file_path="...")
                </code>{" "}
                to inspect prior rationale.
              </li>
              <li>
                <strong className="text-foreground font-mono">
                  Before Decisions:
                </strong>{" "}
                Calls{" "}
                <code className="text-primary">
                  orbit.search_context(query="...")
                </code>{" "}
                to verify past architectural decisions.
              </li>
              <li>
                <strong className="text-foreground font-mono">
                  After Tasks:
                </strong>{" "}
                Calls{" "}
                <code className="text-primary">
                  orbit.store_memory(content="...")
                </code>{" "}
                to synchronize other peer agents.
              </li>
              <li>
                <strong className="text-foreground font-mono">
                  When Obsoleting Decisions:
                </strong>{" "}
                Calls{" "}
                <code className="text-primary">
                  orbit.mark_decision(id="...", state="superseded")
                </code>{" "}
                to update the knowledge graph.
              </li>
            </ol>
          </div>
        )}
      </div>
    </div>
  );
}
