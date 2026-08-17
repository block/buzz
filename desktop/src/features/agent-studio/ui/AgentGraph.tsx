import {
  Background,
  Controls,
  type Edge,
  type Node,
  ReactFlow,
  useEdgesState,
  useNodesState,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import * as React from "react";

type GraphNode = {
  id: string;
  kind: string;
  slug: string;
};

type GraphEdge = {
  source_type: string;
  source_slug: string;
  target_type: string;
  target_slug: string;
  relationship_type: string;
  evidence: string;
};

function toFlowNodes(nodes: GraphNode[]): Node[] {
  return nodes.map((node, index) => ({
    id: node.id,
    data: { label: `${node.kind}: ${node.slug}` },
    position: { x: (index % 4) * 180, y: Math.floor(index / 4) * 100 },
  }));
}

function toFlowEdges(edges: GraphEdge[]): Edge[] {
  return edges.map((edge, index) => ({
    id: `e-${index}-${edge.source_slug}-${edge.target_slug}`,
    source: `${edge.source_type}:${edge.source_slug}`,
    target: `${edge.target_type}:${edge.target_slug}`,
    label: edge.relationship_type,
  }));
}

type AgentGraphProps = {
  nodes: GraphNode[];
  edges: GraphEdge[];
};

export function AgentGraph({ nodes, edges }: AgentGraphProps) {
  const [flowNodes, setNodes, onNodesChange] = useNodesState(
    toFlowNodes(nodes),
  );
  const [flowEdges, setEdges, onEdgesChange] = useEdgesState(
    toFlowEdges(edges),
  );

  React.useEffect(() => {
    setNodes(toFlowNodes(nodes));
    setEdges(toFlowEdges(edges));
  }, [nodes, edges, setNodes, setEdges]);

  if (nodes.length === 0) {
    return null;
  }

  return (
    <div className="h-80 w-full rounded-md border border-border">
      <ReactFlow
        edges={flowEdges}
        fitView
        nodes={flowNodes}
        onEdgesChange={onEdgesChange}
        onNodesChange={onNodesChange}
      >
        <Background />
        <Controls />
      </ReactFlow>
    </div>
  );
}
