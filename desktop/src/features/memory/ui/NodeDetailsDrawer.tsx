import React from "react";
import {
  X,
  Clock,
  AlertTriangle,
  CheckCircle2,
  Tag,
  Link2,
  Ban,
  FileCode,
  MessageSquare,
} from "lucide-react";
import type { GraphNodeDto, GraphLinkDto } from "../lib/types";
import { getNodeVisualMeta } from "../lib/colors";

interface NodeDetailsDrawerProps {
  node: GraphNodeDto | null;
  connectedLinks: GraphLinkDto[];
  allNodes?: GraphNodeDto[];
  onClose: () => void;
  onSelectNode?: (nodeId: string) => void;
  onInvalidateDecision: (linkId: string) => Promise<void>;
}

export const NodeDetailsDrawer: React.FC<NodeDetailsDrawerProps> = ({
  node,
  connectedLinks,
  allNodes = [],
  onClose,
  onSelectNode,
  onInvalidateDecision,
}) => {
  const [loadingLinkId, setLoadingLinkId] = React.useState<string | null>(null);

  if (!node) return null;

  const meta = getNodeVisualMeta(node.entity_type);
  const nodeMap = new Map(allNodes.map((n) => [n.id, n]));

  const handleInvalidate = async (linkId: string) => {
    try {
      setLoadingLinkId(linkId);
      await onInvalidateDecision(linkId);
    } finally {
      setLoadingLinkId(null);
    }
  };

  const isDecision = node.entity_type.toLowerCase() === "decision";
  const isFile = node.entity_type.toLowerCase() === "file";
  const isChat = node.entity_type.toLowerCase() === "chat";

  return (
    <aside
      data-testid="node-details-drawer"
      aria-label="Node details inspection drawer"
      className="absolute right-0 top-0 bottom-0 w-80 sm:w-96 bg-neutral-900/95 backdrop-blur-md border-l border-neutral-800 shadow-2xl p-5 flex flex-col z-30 text-neutral-200 overflow-y-auto animate-in slide-in-from-right duration-200"
    >
      {/* Header */}
      <div className="flex items-center justify-between pb-3 border-b border-neutral-800">
        <div className="flex items-center gap-2">
          <span
            className="px-2.5 py-0.5 text-xs font-medium rounded-full border"
            style={{
              backgroundColor: meta.badgeBg,
              color: meta.badgeText,
              borderColor: meta.badgeBorder,
            }}
          >
            {meta.label}
          </span>

          <span className="text-[11px] text-neutral-400 font-mono">
            {node.id.slice(0, 16)}
          </span>
        </div>

        <button
          onClick={onClose}
          aria-label="Close inspection drawer"
          className="text-neutral-400 hover:text-neutral-100 transition-colors p-1 rounded-md hover:bg-neutral-800"
        >
          <X className="w-4 h-4" />
        </button>
      </div>

      {/* Title & Description */}
      <div className="mt-4 space-y-2">
        <h2 className="text-base font-semibold text-neutral-100 break-words leading-tight">
          {node.name}
        </h2>
        <p className="text-xs text-neutral-300 whitespace-pre-wrap leading-relaxed">
          {node.description || "No description provided."}
        </p>
      </div>

      {/* Quick Action Buttons */}
      <div className="mt-4 flex flex-wrap gap-2">
        {isFile && (
          <button
            onClick={() => {
              // Action: open file
            }}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-neutral-800 hover:bg-neutral-700 text-xs font-medium text-neutral-200 border border-neutral-700 transition-colors"
          >
            <FileCode className="w-3.5 h-3.5 text-amber-400" />
            <span>Open in Editor</span>
          </button>
        )}

        {isChat && (
          <button
            onClick={() => {
              // Action: jump to chat
            }}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-neutral-800 hover:bg-neutral-700 text-xs font-medium text-neutral-200 border border-neutral-700 transition-colors"
          >
            <MessageSquare className="w-3.5 h-3.5 text-cyan-400" />
            <span>Jump to Thread</span>
          </button>
        )}
      </div>

      {/* Metadata Section */}
      {node.metadata && Object.keys(node.metadata).length > 0 && (
        <div className="mt-5 space-y-2">
          <div className="flex items-center gap-1.5 text-neutral-400 text-xs font-medium">
            <Tag className="w-3.5 h-3.5" />
            <span>Metadata Attributes</span>
          </div>

          <div className="bg-neutral-950/70 border border-neutral-800/80 rounded-lg p-2.5 space-y-1.5 text-[11px] font-mono">
            {Object.entries(node.metadata).map(([key, value]) => (
              <div key={key} className="flex justify-between gap-2 overflow-hidden">
                <span className="text-neutral-400 shrink-0">{key}:</span>
                <span className="text-neutral-200 truncate">
                  {typeof value === "object" ? JSON.stringify(value) : String(value)}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Temporal History & Timeline */}
      <div className="mt-5 space-y-2">
        <div className="flex items-center gap-1.5 text-neutral-400 text-xs font-medium">
          <Clock className="w-3.5 h-3.5" />
          <span>Temporal Provenance</span>
        </div>

        <div className="bg-neutral-950/70 border border-neutral-800/80 rounded-lg p-2.5 space-y-1 text-[11px] text-neutral-400">
          <div className="flex justify-between">
            <span>Created At:</span>
            <span className="text-neutral-300">
              {new Date(node.created_at).toLocaleString()}
            </span>
          </div>
          {node.updated_at && (
            <div className="flex justify-between">
              <span>Updated At:</span>
              <span className="text-neutral-300">
                {new Date(node.updated_at).toLocaleString()}
              </span>
            </div>
          )}
        </div>
      </div>

      {/* Connected Graph Relations */}
      <div className="mt-5 space-y-2 flex-1">
        <div className="flex items-center justify-between text-neutral-400 text-xs font-medium">
          <div className="flex items-center gap-1.5">
            <Link2 className="w-3.5 h-3.5" />
            <span>Connected Relations ({connectedLinks.length})</span>
          </div>
        </div>

        {connectedLinks.length === 0 ? (
          <p className="text-xs text-neutral-500 italic p-3 text-center">
            No active connections found for this node.
          </p>
        ) : (
          <div className="space-y-2">
            {connectedLinks.map((link) => {
              const isOutgoing = link.source === node.id;
              const otherNodeId = isOutgoing ? link.target : link.source;
              const otherNode = nodeMap.get(otherNodeId);
              const otherMeta = getNodeVisualMeta(otherNode?.entity_type);
              const isInvalidating = loadingLinkId === link.id;

              return (
                <div
                  key={link.id}
                  className={`p-2.5 rounded-lg border text-xs transition-colors ${
                    link.is_active
                      ? "bg-neutral-800/50 border-neutral-700/60"
                      : "bg-red-950/20 border-red-900/30 opacity-75"
                  }`}
                >
                  <div className="flex items-center justify-between gap-1.5 mb-1">
                    <span className="font-semibold text-neutral-200">
                      {isOutgoing ? "→" : "←"} {link.relation_type}
                    </span>

                    <span
                      className={`text-[10px] px-1.5 py-0.5 rounded font-medium flex items-center gap-1 ${
                        link.is_active
                          ? "bg-emerald-500/15 text-emerald-300"
                          : "bg-red-500/15 text-red-300"
                      }`}
                    >
                      {link.is_active ? (
                        <>
                          <CheckCircle2 className="w-2.5 h-2.5" /> Active
                        </>
                      ) : (
                        <>
                          <AlertTriangle className="w-2.5 h-2.5" /> Superseded
                        </>
                      )}
                    </span>
                  </div>

                  <div className="flex items-center justify-between gap-2 mt-1">
                    <button
                      onClick={() => onSelectNode?.(otherNodeId)}
                      className="flex items-center gap-1.5 text-neutral-300 hover:text-neutral-100 hover:underline truncate"
                    >
                      <span
                        className="w-2 h-2 rounded-full shrink-0"
                        style={{ backgroundColor: otherMeta.color }}
                      />
                      <span className="truncate">
                        {otherNode?.name || otherNodeId}
                      </span>
                    </button>

                    {/* Invalidate Decision Action */}
                    {isDecision && link.is_active && (
                      <button
                        onClick={() => handleInvalidate(link.id)}
                        disabled={isInvalidating}
                        title="Invalidate this relation as superseded / contradictory"
                        className="flex items-center gap-1 px-2 py-0.5 rounded bg-red-600/15 hover:bg-red-600/25 text-red-300 text-[10px] transition-colors border border-red-500/20 disabled:opacity-50 shrink-0"
                      >
                        <Ban className="w-3 h-3" />
                        <span>{isInvalidating ? "Invalidating..." : "Invalidate"}</span>
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </aside>
  );
};
