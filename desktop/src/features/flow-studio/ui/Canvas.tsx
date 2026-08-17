import {
  Background,
  Controls,
  type Node,
  ReactFlow,
  useNodesState,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import * as React from "react";

import type { FlowBlock } from "./BlockPalette";

export type CanvasNodeData = {
  label: string;
  blockType: string;
  status?: string;
};

export type FlowCanvasNode = Node<CanvasNodeData>;

export type FlowCanvasProps = {
  nodes: FlowCanvasNode[];
  onNodesChange: ReturnType<typeof useNodesState<FlowCanvasNode>>[2];
  setNodes: ReturnType<typeof useNodesState<FlowCanvasNode>>[1];
  nodeStatuses?: Record<string, string>;
};

export function useFlowCanvasState(initial: FlowCanvasNode[] = []) {
  return useNodesState<FlowCanvasNode>(initial);
}

export function FlowCanvas({
  nodes,
  onNodesChange,
  setNodes,
  nodeStatuses = {},
}: FlowCanvasProps) {
  const displayNodes = React.useMemo(
    () =>
      nodes.map((node) => ({
        ...node,
        data: {
          ...node.data,
          label: nodeStatuses[node.id]
            ? `${node.data.label} (${nodeStatuses[node.id]})`
            : node.data.label,
        },
        style: nodeStyle(nodeStatuses[node.id]),
      })),
    [nodes, nodeStatuses],
  );

  const onDrop = React.useCallback(
    (event: React.DragEvent) => {
      event.preventDefault();
      const raw = event.dataTransfer.getData("application/buzz-flow-block");
      if (!raw) return;
      const block = JSON.parse(raw) as FlowBlock;
      const id = `step-${nodes.length + 1}`;
      setNodes((prev) => [
        ...prev,
        {
          id,
          data: { label: block.name, blockType: block.block_type },
          position: { x: 80 + prev.length * 40, y: 80 + prev.length * 30 },
        },
      ]);
    },
    [nodes.length, setNodes],
  );

  return (
    <div className="relative mt-4">
      <div
        className="h-96 w-full rounded-md border border-border"
        data-testid="flow-canvas"
        onDragOver={(e) => e.preventDefault()}
        onDrop={onDrop}
        role="application"
      >
        <ReactFlow fitView nodes={displayNodes} onNodesChange={onNodesChange}>
          <Background />
          <Controls />
        </ReactFlow>
      </div>
      {nodes.length === 0 ? (
        <p className="pointer-events-none absolute inset-0 flex items-center justify-center text-sm text-muted-foreground">
          Drag blocks from the palette
        </p>
      ) : null}
    </div>
  );
}

function nodeStyle(status?: string): React.CSSProperties | undefined {
  if (!status) return undefined;
  const colors: Record<string, string> = {
    completed: "#22c55e33",
    failed: "#ef444433",
    error: "#ef444433",
    running: "#3b82f633",
    suspended: "#f59e0b33",
    waiting_approval: "#f59e0b33",
  };
  const border = colors[status];
  return border ? { backgroundColor: border, borderRadius: 8 } : undefined;
}

export function canvasNodesToBlocks(nodes: FlowCanvasNode[]) {
  return nodes.map((node) => ({
    id: node.id,
    block_type: node.data.blockType,
    config_json:
      node.data.blockType === "http"
        ? { url: "https://example.com/hook" }
        : node.data.blockType === "human_approval"
          ? { from: "@anyone", message: "Approve this step?" }
          : {},
  }));
}

export function serializeGraph(nodes: FlowCanvasNode[]) {
  return JSON.stringify({ nodes, edges: [] });
}

export function parseGraph(graphJson: string): FlowCanvasNode[] {
  try {
    const parsed = JSON.parse(graphJson) as { nodes?: FlowCanvasNode[] };
    return Array.isArray(parsed.nodes) ? parsed.nodes : [];
  } catch {
    return [];
  }
}
