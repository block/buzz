import * as React from "react";
import { Bot, Download, Network } from "lucide-react";

import { useCommunities } from "@/features/communities/useCommunities";
import { Button } from "@/shared/ui/button";

import { AgentGraph } from "./AgentGraph";
import { SkillImportModal } from "./SkillImportModal";
import { UnifiedCostMonitor } from "./UnifiedCostMonitor";

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

export function AgentStudioView() {
  const { activeCommunity } = useCommunities();
  const [nodes, setNodes] = React.useState<GraphNode[]>([]);
  const [edges, setEdges] = React.useState<GraphEdge[]>([]);
  const [error, setError] = React.useState<string | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [importOpen, setImportOpen] = React.useState(false);

  React.useEffect(() => {
    let cancelled = false;
    const relayHttp = activeCommunity?.relayUrl?.replace(/^ws/i, "http");
    if (!relayHttp) {
      setLoading(false);
      return;
    }

    void (async () => {
      try {
        const res = await fetch(`${relayHttp}/agent-studio/graph`);
        if (!res.ok) {
          throw new Error(`HTTP ${res.status}`);
        }
        const data = (await res.json()) as {
          nodes?: GraphNode[];
          edges?: GraphEdge[];
        };
        if (!cancelled) {
          setNodes(data.nodes ?? []);
          setEdges(data.edges ?? []);
          setError(null);
        }
      } catch (e) {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : "Failed to load graph");
          setNodes([]);
          setEdges([]);
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [activeCommunity?.relayUrl]);

  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto px-4 pb-4 pt-4 sm:px-6"
      data-testid="agent-studio-view"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <Bot className="h-5 w-5 text-primary" />
          <h1 className="text-base font-medium">Agent Studio</h1>
        </div>
        <Button onClick={() => setImportOpen(true)} size="sm" type="button">
          <Download className="mr-2 h-4 w-4" />
          Import skill
        </Button>
      </div>
      <p className="mt-2 max-w-2xl text-sm text-muted-foreground">
        Dependency graph for personas, commands, and skills — ported from
        claude-code-cli-ui. Events persist via Nostr kinds 47200–47399.
      </p>

      <section className="mt-6 rounded-lg border border-border bg-card p-4">
        <div className="mb-3 flex items-center gap-2 text-sm font-medium">
          <Network className="h-4 w-4" />
          Dependency graph
        </div>
        {loading ? (
          <p className="text-sm text-muted-foreground">Loading graph…</p>
        ) : null}
        {error ? (
          <p className="text-sm text-red-400">
            Could not load graph from relay: {error}
          </p>
        ) : null}
        {!loading && !error && nodes.length === 0 && edges.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No agents scanned yet. Import skills or create personas to populate
            the graph.
          </p>
        ) : null}
        {nodes.length > 0 ? (
          <p className="mb-3 text-sm text-muted-foreground">
            {nodes.length} nodes · {edges.length} edges
          </p>
        ) : null}
        <AgentGraph edges={edges} nodes={nodes} />
      </section>

      <UnifiedCostMonitor />

      <SkillImportModal onOpenChange={setImportOpen} open={importOpen} />
    </div>
  );
}
