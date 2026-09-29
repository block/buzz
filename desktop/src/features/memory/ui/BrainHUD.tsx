import React from "react";
import {
  Brain,
  FolderGit2,
  FileCode2,
  Layers,
  Cpu,
  Bot,
  CloudCheck,
  RefreshCw,
} from "lucide-react";
import type { BrainStats } from "../lib/types";

interface BrainHUDProps {
  stats: BrainStats;
  loading?: boolean;
  isSyncing?: boolean;
  onRefresh?: () => void;
  onSync?: () => void;
}

export const BrainHUD: React.FC<BrainHUDProps> = ({
  stats,
  loading = false,
  isSyncing = false,
  onRefresh,
  onSync,
}) => {
  return (
    <header
      data-testid="brain-hud-header"
      className="flex flex-wrap items-center justify-between gap-3 px-4 py-2.5 bg-neutral-900/80 backdrop-blur-md border-b border-neutral-800 text-neutral-200 select-none z-20"
    >
      {/* Brand & Pulsing Brain Orb */}
      <div className="flex items-center gap-3">
        <div className="relative flex items-center justify-center w-8 h-8 rounded-lg bg-purple-500/10 border border-purple-500/30 text-purple-400">
          <Brain className="w-4 h-4 animate-pulse" />
          <span className="absolute -top-1 -right-1 flex h-2 w-2">
            <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75" />
            <span className="relative inline-flex rounded-full h-2 w-2 bg-emerald-500" />
          </span>
        </div>

        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-sm font-semibold text-neutral-100 tracking-tight">
              Orbit Brain
            </h1>
            <span className="text-[10px] uppercase font-bold tracking-widest px-1.5 py-0.5 rounded bg-purple-500/15 text-purple-300 border border-purple-500/25">
              Obsidian View
            </span>
          </div>
          <p className="text-[11px] text-neutral-400">
            Real-time multi-agent context universe
          </p>
        </div>
      </div>

      {/* Metric Status Pills */}
      <div className="flex flex-wrap items-center gap-2 text-xs">
        {/* Workspaces */}
        <div
          title="Active monitored workspaces"
          className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-neutral-800/60 border border-neutral-700/60 text-neutral-300"
        >
          <FolderGit2 className="w-3.5 h-3.5 text-emerald-400" />
          <span>{stats.monitored_workspaces} Workspaces</span>
        </div>

        {/* Files Indexed */}
        <div
          title="Source files indexed across workspaces"
          className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-neutral-800/60 border border-neutral-700/60 text-neutral-300"
        >
          <FileCode2 className="w-3.5 h-3.5 text-amber-400" />
          <span>{stats.total_files_indexed} Files</span>
        </div>

        {/* Chunks */}
        <div
          title="Total AST syntax chunks in SQLite"
          className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-neutral-800/60 border border-neutral-700/60 text-neutral-300"
        >
          <Layers className="w-3.5 h-3.5 text-cyan-400" />
          <span>{stats.total_chunks.toLocaleString()} Chunks</span>
        </div>

        {/* Vector Engine Health */}
        <div
          title="Embedded LanceDB + BGE-Small ONNX vector health"
          className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-neutral-800/60 border border-neutral-700/60 text-neutral-300"
        >
          <Cpu className="w-3.5 h-3.5 text-purple-400" />
          <span className="capitalize">{stats.vector_index_status}</span>
          <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-pulse" />
        </div>

        {/* Connected Agents */}
        <div
          title="Active connected agent harnesses (Antigravity, Claude, Cursor, etc.)"
          className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-neutral-800/60 border border-neutral-700/60 text-neutral-300"
        >
          <Bot className="w-3.5 h-3.5 text-violet-400" />
          <span>{stats.connected_agents_count} Agents</span>
        </div>

        {/* Cloud Sync State */}
        <div
          title={
            stats.last_synced_at
              ? `Last synced: ${new Date(stats.last_synced_at).toLocaleTimeString()}`
              : "Operating local-first on device"
          }
          className="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-neutral-800/60 border border-neutral-700/60 text-neutral-300"
        >
          <CloudCheck className="w-3.5 h-3.5 text-sky-400" />
          <span className="capitalize">
            {stats.cloud_sync_state === "local" ? "Local (On Device)" : "Synced (Encrypted)"}
          </span>
        </div>

        {/* Sync Now Action */}
        {onSync && (
          <button
            onClick={onSync}
            disabled={isSyncing}
            className="flex items-center gap-1 px-2 py-1 text-[11px] rounded bg-purple-600/20 hover:bg-purple-600/30 text-purple-300 border border-purple-500/30 transition-colors disabled:opacity-50"
          >
            <RefreshCw className={`w-3 h-3 ${isSyncing ? "animate-spin" : ""}`} />
            <span>{isSyncing ? "Syncing..." : "Sync"}</span>
          </button>
        )}

        {/* Refresh button */}
        {onRefresh && (
          <button
            onClick={onRefresh}
            aria-label="Refresh telemetry"
            className="p-1 rounded hover:bg-neutral-800 text-neutral-400 hover:text-neutral-200 transition-colors"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin" : ""}`} />
          </button>
        )}
      </div>
    </header>
  );
};
