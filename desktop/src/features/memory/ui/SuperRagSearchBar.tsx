import React from "react";
import { Search, Sparkles, X } from "lucide-react";

interface SuperRagSearchBarProps {
  value: string;
  onChange: (val: string) => void;
  matchesCount: number;
  totalNodes: number;
  confidenceScore?: number;
}

export const SuperRagSearchBar: React.FC<SuperRagSearchBarProps> = ({
  value,
  onChange,
  matchesCount,
  totalNodes,
  confidenceScore = 0.94,
}) => {
  return (
    <div
      data-testid="superrag-search-bar"
      className="relative flex items-center gap-2 px-3 py-1.5 bg-neutral-900/90 backdrop-blur-md border border-neutral-800 rounded-lg shadow-xl text-neutral-200"
    >
      <Search className="w-4 h-4 text-purple-400 shrink-0" />

      <input
        type="text"
        placeholder="SuperRAG search memory, decisions, files, concepts..."
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="w-64 sm:w-80 bg-transparent text-xs text-neutral-100 placeholder-neutral-500 focus:outline-none"
      />

      {value.length > 0 && (
        <button
          onClick={() => onChange("")}
          aria-label="Clear search"
          className="p-0.5 rounded text-neutral-400 hover:text-neutral-200 hover:bg-neutral-800 transition-colors"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      )}

      {/* Match indicator pill */}
      {value.length > 0 && (
        <div className="flex items-center gap-1.5 pl-2 border-l border-neutral-800 text-[11px]">
          <span className="text-neutral-400">
            {matchesCount} / {totalNodes}
          </span>

          <div
            title="SuperRAG Dense + Lexical Fusion Confidence"
            className="flex items-center gap-1 px-1.5 py-0.5 rounded bg-emerald-500/15 text-emerald-400 font-medium"
          >
            <Sparkles className="w-3 h-3" />
            <span>{Math.round(confidenceScore * 100)}%</span>
          </div>
        </div>
      )}
    </div>
  );
};
