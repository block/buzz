import * as React from "react";
import { GitBranch, Layers, Play, Save } from "lucide-react";
import { useQuery } from "@tanstack/react-query";

import { useChannelsQuery } from "@/features/channels/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { WorkflowApprovalCard } from "@/features/workflows/ui/WorkflowApprovalCard";
import { getFlowGraph, publishFlowGraph } from "@/shared/api/tauriHiveStudio";
import {
  createWorkflow,
  getRunApprovals,
  getWorkflowRuns,
  triggerWorkflow,
} from "@/shared/api/tauriWorkflows";
import { Button } from "@/shared/ui/button";

import { BlockPalette, type FlowBlock } from "./BlockPalette";
import {
  canvasNodesToBlocks,
  FlowCanvas,
  parseGraph,
  serializeGraph,
  useFlowCanvasState,
} from "./Canvas";
import { FilesPanel } from "./FilesPanel";
import { KnowledgeBasePanel } from "./KnowledgeBasePanel";
import { TablesPanel } from "./TablesPanel";

export function FlowStudioView() {
  const { activeCommunity } = useCommunities();
  const channelsQuery = useChannelsQuery();
  const [blocks, setBlocks] = React.useState<FlowBlock[]>([]);
  const [error, setError] = React.useState<string | null>(null);
  const [loading, setLoading] = React.useState(true);
  const [statusMessage, setStatusMessage] = React.useState<string | null>(null);
  const [workflowId, setWorkflowId] = React.useState<string | null>(null);
  const flowId = workflowId ?? "flow-studio-draft";
  const [activeRunId, setActiveRunId] = React.useState<string | null>(null);
  const [nodes, setNodes, onNodesChange] = useFlowCanvasState();

  const channelId =
    channelsQuery.data?.find((c) => c.name === "general")?.id ??
    channelsQuery.data?.[0]?.id ??
    null;

  React.useEffect(() => {
    let cancelled = false;
    const relayHttp = activeCommunity?.relayUrl?.replace(/^ws/i, "http");
    if (!relayHttp) {
      setLoading(false);
      setBlocks([]);
      return;
    }

    void (async () => {
      try {
        const res = await fetch(`${relayHttp}/flow-studio/blocks`);
        if (!res.ok) {
          throw new Error(`HTTP ${res.status}`);
        }
        const data = (await res.json()) as { blocks?: FlowBlock[] };
        if (!cancelled) {
          setBlocks(data.blocks ?? []);
          setError(null);
        }
      } catch (e) {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : "Failed to load blocks");
          setBlocks([]);
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

  React.useEffect(() => {
    let cancelled = false;
    void getFlowGraph(flowId)
      .then((saved) => {
        if (cancelled || !saved.found || !saved.graph_json) {
          return;
        }
        const restored = parseGraph(saved.graph_json);
        if (restored.length > 0) {
          setNodes(restored);
        }
      })
      .catch(() => {
        // No saved graph yet — start from empty canvas.
      });
    return () => {
      cancelled = true;
    };
  }, [flowId, setNodes]);

  const runsQuery = useQuery({
    enabled: Boolean(workflowId),
    queryFn: () => getWorkflowRuns(workflowId as string, 5),
    queryKey: ["workflow-runs", workflowId],
    refetchInterval: (query) => {
      const runs = query.state.data;
      const active = runs?.some(
        (run) =>
          run.status === "pending" ||
          run.status === "running" ||
          run.status === "waiting_approval",
      );
      return active ? 1000 : false;
    },
  });

  const approvalsQuery = useQuery({
    enabled: Boolean(workflowId && activeRunId),
    queryFn: () => getRunApprovals(workflowId as string, activeRunId as string),
    queryKey: ["run-approvals", workflowId, activeRunId],
    refetchInterval: 5000,
  });

  const activeRun = runsQuery.data?.[0];
  React.useEffect(() => {
    if (activeRun?.id) {
      setActiveRunId(activeRun.id);
    }
  }, [activeRun?.id]);

  const nodeStatuses = React.useMemo(() => {
    const map: Record<string, string> = {};
    for (const step of activeRun?.executionTrace ?? []) {
      map[step.stepId] = step.status;
    }
    return map;
  }, [activeRun?.executionTrace]);

  const relayHttp = activeCommunity?.relayUrl?.replace(/^ws/i, "http");

  const saveGraph = () => {
    if (nodes.length === 0) return;
    void publishFlowGraph(flowId, serializeGraph(nodes))
      .then((data) => {
        setStatusMessage(data.message ?? "Graph saved");
      })
      .catch((e: unknown) => {
        setStatusMessage(e instanceof Error ? e.message : "Save failed");
      });
  };

  const runFlow = async () => {
    if (!relayHttp || !channelId || nodes.length === 0) {
      setStatusMessage("Add canvas blocks and connect to a channel first");
      return;
    }
    const canvasBlocks = canvasNodesToBlocks(nodes);
    const yamlRes = await fetch(`${relayHttp}/flow-studio/yaml/from-canvas`, {
      body: JSON.stringify({ blocks: canvasBlocks, flow_id: flowId }),
      headers: { "Content-Type": "application/json" },
      method: "POST",
    });
    const yamlData = (await yamlRes.json()) as {
      yaml?: string;
      error?: string;
    };
    if (yamlData.error || !yamlData.yaml) {
      setStatusMessage(yamlData.error ?? "YAML export failed");
      return;
    }
    try {
      const saved = await createWorkflow(channelId, yamlData.yaml);
      setWorkflowId(saved.workflow.id);
      const triggered = await triggerWorkflow(saved.workflow.id);
      setActiveRunId(triggered.runId);
      setStatusMessage(`Run started: ${triggered.runId.slice(0, 8)}…`);
      void runsQuery.refetch();
    } catch (e) {
      setStatusMessage(e instanceof Error ? e.message : "Run failed");
    }
  };

  const pendingApproval = approvalsQuery.data?.find(
    (a) => a.status === "pending",
  );

  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto px-4 pb-4 pt-4 sm:px-6"
      data-testid="flow-studio-view"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <GitBranch className="h-5 w-5 text-primary" />
          <h1 className="text-base font-medium">Flow Studio</h1>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            disabled={nodes.length === 0}
            onClick={saveGraph}
            size="sm"
            type="button"
            variant="outline"
          >
            <Save className="mr-2 h-4 w-4" />
            Save graph
          </Button>
          <Button
            disabled={nodes.length === 0 || !channelId}
            onClick={() => void runFlow()}
            size="sm"
            type="button"
          >
            <Play className="mr-2 h-4 w-4" />
            Run flow
          </Button>
        </div>
      </div>
      <p className="mt-2 max-w-2xl text-sm text-muted-foreground">
        Visual workflow builder (Buzz Hive). Drag blocks onto the canvas, save
        as kind 46200, run via `buzz-workflow`.
      </p>
      {statusMessage ? (
        <p className="mt-2 text-sm text-muted-foreground">{statusMessage}</p>
      ) : null}
      {pendingApproval ? (
        <div className="mt-4 max-w-lg">
          <WorkflowApprovalCard approval={pendingApproval} />
        </div>
      ) : null}

      <section className="mt-6">
        <div className="mb-3 flex items-center gap-2 text-sm font-medium">
          <Layers className="h-4 w-4" />
          Block palette
        </div>
        {loading ? (
          <p className="text-sm text-muted-foreground">Loading blocks…</p>
        ) : null}
        {error ? (
          <p className="text-sm text-red-400">
            Could not load blocks from relay: {error}
          </p>
        ) : null}
        {!loading && !error && blocks.length === 0 ? (
          <p className="text-sm text-muted-foreground">No blocks registered.</p>
        ) : null}
        <BlockPalette blocks={blocks} />
      </section>

      <FlowCanvas
        nodeStatuses={nodeStatuses}
        nodes={nodes}
        onNodesChange={onNodesChange}
        setNodes={setNodes}
      />

      <KnowledgeBasePanel />
      <TablesPanel />
      <FilesPanel />
    </div>
  );
}
