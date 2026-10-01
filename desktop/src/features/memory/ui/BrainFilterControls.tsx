import React from "react";
import { Filter, Calendar } from "lucide-react";
import { OBSIDIAN_NODE_PALETTE } from "../lib/colors";

interface BrainFilterControlsProps {
  activeFilter: string;
  onFilterChange: (filter: string) => void;
  temporalCutoff: number | null;
  onTemporalChange: (timestamp: number | null) => void;
  minTimestamp?: number;
  maxTimestamp?: number;
}

const FILTER_ITEMS = [
  { key: "ALL", label: "All Nodes", color: "#94A3B8" },
  { key: "PROJECT", label: "Projects", color: OBSIDIAN_NODE_PALETTE.project.color },
  { key: "AGENT", label: "Agents", color: OBSIDIAN_NODE_PALETTE.agent.color },
  { key: "CHAT", label: "Chats", color: OBSIDIAN_NODE_PALETTE.chat.color },
  { key: "FILE", label: "Files", color: OBSIDIAN_NODE_PALETTE.file.color },
  { key: "DECISION", label: "Decisions", color: OBSIDIAN_NODE_PALETTE.decision.color },
  { key: "CONCEPT", label: "Concepts", color: OBSIDIAN_NODE_PALETTE.concept.color },
];

export const BrainFilterControls: React.FC<BrainFilterControlsProps> = ({
  activeFilter,
  onFilterChange,
  temporalCutoff,
  onTemporalChange,
  minTimestamp = Date.now() - 86400000 * 30, // 30 days ago
  maxTimestamp = Date.now(),
}) => {
  const currentVal = temporalCutoff ?? maxTimestamp;
  const isScrubbing = temporalCutoff !== null && temporalCutoff < maxTimestamp;

  return (
    <div
      data-testid="brain-filter-controls"
      className="flex flex-wrap items-center gap-2 p-1.5 bg-neutral-900/90 backdrop-blur-md border border-neutral-800 rounded-lg shadow-lg text-neutral-300 text-xs"
    >
      <div className="flex items-center gap-1 pl-1 pr-1.5 border-r border-neutral-800 text-neutral-400">
        <Filter className="w-3.5 h-3.5" />
        <span className="font-medium text-[11px]">Filters</span>
      </div>

      {/* Filter Category Buttons */}
      <div className="flex items-center gap-1 overflow-x-auto">
        {FILTER_ITEMS.map((item) => {
          const isActive =
            activeFilter === item.key ||
            (item.key === "AGENT" && activeFilter.startsWith("AGENT:"));
          return (
            <button
              key={item.key}
              onClick={() => onFilterChange(item.key)}
              className={`flex items-center gap-1.5 px-2 py-1 rounded text-[11px] font-medium transition-colors ${
                isActive
                  ? "bg-neutral-800 text-neutral-100 shadow-sm border border-neutral-700"
                  : "text-neutral-400 hover:text-neutral-200 hover:bg-neutral-800/50"
              }`}
            >
              <span
                className="w-2 h-2 rounded-full"
                style={{ backgroundColor: item.color }}
              />
              <span>{item.label}</span>
            </button>
          );
        })}
      </div>

      {/* Specific Agent Sub-Filter (Antigravity, Claude Code, Cursor, Goose, etc.) */}
      {(activeFilter === "AGENT" || activeFilter.startsWith("AGENT:")) && (
        <div className="flex items-center gap-1 pl-2 border-l border-neutral-800">
          <select
            aria-label="Filter by agent identity"
            value={activeFilter.startsWith("AGENT:") ? activeFilter.slice(6) : "ALL"}
            onChange={(e) => {
              const val = e.target.value;
              onFilterChange(val === "ALL" ? "AGENT" : `AGENT:${val}`);
            }}
            className="bg-neutral-800/80 border border-neutral-700 rounded px-1.5 py-0.5 text-[11px] text-neutral-200 focus:outline-none focus:border-purple-500"
          >
            <option value="ALL">All Agent Harnesses</option>
            <option value="antigravity">Antigravity</option>
            <option value="claude_code">Claude Code</option>
            <option value="cursor">Cursor</option>
            <option value="goose">Goose</option>
            <option value="codex">Codex</option>
            <option value="opencode">OpenCode</option>
            <option value="zcode">ZCode</option>
            <option value="agy_cli">AGY CLI</option>
            <option value="kimi">Kimi</option>
          </select>
        </div>
      )}

      {/* Temporal Timeline Scrubber */}
      <div className="flex items-center gap-2 pl-2 border-l border-neutral-800">
        <Calendar className="w-3.5 h-3.5 text-neutral-400 shrink-0" />
        <span className="text-[11px] text-neutral-400 whitespace-nowrap">
          {isScrubbing ? new Date(currentVal).toLocaleDateString() : "Present"}
        </span>

        <input
          type="range"
          aria-label="Temporal cutoff filter"
          min={minTimestamp}
          max={maxTimestamp}
          value={currentVal}
          onChange={(e) => {
            const val = Number(e.target.value);
            onTemporalChange(val >= maxTimestamp - 1000 ? null : val);
          }}
          className="w-24 accent-purple-500 h-1 bg-neutral-800 rounded-lg cursor-pointer"
        />

        {isScrubbing && (
          <button
            onClick={() => onTemporalChange(null)}
            className="text-[10px] text-purple-400 hover:text-purple-300 hover:underline"
          >
            Reset
          </button>
        )}
      </div>
    </div>
  );
};
