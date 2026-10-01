import { useState, useEffect, useCallback, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { GraphNodeDto, GraphLinkDto, SimNode, SimLink } from "./types";
import { getNodeVisualMeta } from "./colors";
import { getSampleBrainGraph } from "./mockBrainData";

interface UseBrainGraphOptions {
  workspacePath?: string;
  initialHops?: number;
}

export function useBrainGraph({ workspacePath, initialHops = 2 }: UseBrainGraphOptions = {}) {
  const [rawNodes, setRawNodes] = useState<GraphNodeDto[]>([]);
  const [rawLinks, setRawLinks] = useState<GraphLinkDto[]>([]);
  const [selectedNode, setSelectedNode] = useState<GraphNodeDto | null>(null);
  const [selectedSeedId, setSelectedSeedId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [activeFilter, setActiveFilter] = useState<string>("ALL");
  const [hops, setHops] = useState<number>(initialHops);
  const [loading, setLoading] = useState(false);
  const [temporalCutoff, setTemporalCutoff] = useState<number | null>(null);

  const fetchGraph = useCallback(
    async (seedId?: string | null) => {
      try {
        setLoading(true);
        const res = await invoke<{
          nodes: GraphNodeDto[];
          links: GraphLinkDto[];
        }>("fetch_brain_graph", {
          workspacePath: workspacePath || null,
          seedId: seedId || null,
          hops,
        });

        if (res && Array.isArray(res.nodes) && res.nodes.length > 0) {
          setRawNodes(res.nodes);
          setRawLinks(res.links || []);
        } else {
          // Use sample universe if backend returns empty
          const sample = getSampleBrainGraph();
          setRawNodes(sample.nodes);
          setRawLinks(sample.links);
        }
      } catch {
        // Fallback for UI preview when backend command is not yet compiled
        const sample = getSampleBrainGraph();
        setRawNodes(sample.nodes);
        setRawLinks(sample.links);
      } finally {
        setLoading(false);
      }
    },
    [workspacePath, hops]
  );

  useEffect(() => {
    fetchGraph(selectedSeedId);
  }, [fetchGraph, selectedSeedId]);

  const handleInvalidateDecision = useCallback(
    async (linkId: string) => {
      try {
        await invoke("invalidate_brain_decision", { relationId: linkId }).catch(
          () => undefined
        );
        // Mark link as inactive in local state
        setRawLinks((prev) =>
          prev.map((l) =>
            l.id === linkId
              ? {
                  ...l,
                  is_active: false,
                  invalid_at: new Date().toISOString(),
                }
              : l
          )
        );
      } catch (err) {
        console.error("Failed to invalidate decision:", err);
      }
    },
    []
  );

  // Filter nodes & links based on activeFilter, search query, and temporal cutoff
  const { filteredSimNodes, filteredSimLinks } = useMemo(() => {
    const cutoffTime = temporalCutoff ?? Date.now();

    // 1. Filter Nodes
    const matchingNodes = rawNodes.filter((node) => {
      // Temporal filter
      const nodeTime = new Date(node.created_at).getTime();
      if (nodeTime > cutoffTime) return false;

      // Type & Agent identity filter
      if (activeFilter !== "ALL") {
        const filterUpper = activeFilter.toUpperCase();
        const entityUpper = node.entity_type.toUpperCase();

        if (filterUpper === "AGENT") {
          if (entityUpper !== "AGENT" && entityUpper !== "CHAT") return false;
        } else if (filterUpper.startsWith("AGENT:")) {
          const targetAgent = filterUpper.slice(6).toLowerCase();
          const nodeAgent = (
            (node.metadata?.agent_name as string) || ""
          ).toLowerCase();
          const nodeName = node.name.toLowerCase();

          if (entityUpper !== "AGENT" && entityUpper !== "CHAT") return false;
          if (nodeAgent !== targetAgent && !nodeName.includes(targetAgent)) {
            return false;
          }
        } else if (entityUpper !== filterUpper) {
          return false;
        }
      }

      return true;
    });

    const matchingNodeIds = new Set(matchingNodes.map((n) => n.id));

    // 2. Filter Links
    const matchingLinks = rawLinks.filter((link) => {
      const linkTime = new Date(link.valid_at).getTime();
      if (linkTime > cutoffTime) return false;
      return matchingNodeIds.has(link.source) && matchingNodeIds.has(link.target);
    });

    // 3. Prepare SimNodes
    const simNodes: SimNode[] = matchingNodes.map((n) => {
      const meta = getNodeVisualMeta(n.entity_type);
      return {
        ...n,
        x: 600 + (Math.random() - 0.5) * 400,
        y: 400 + (Math.random() - 0.5) * 300,
        vx: 0,
        vy: 0,
        radius: meta.defaultRadius,
        color: meta.color,
        glowColor: meta.glowColor,
      };
    });

    const nodeMap = new Map(simNodes.map((n) => [n.id, n]));
    const simLinks: SimLink[] = matchingLinks.map((l) => ({
      ...l,
      sourceNode: nodeMap.get(l.source),
      targetNode: nodeMap.get(l.target),
    }));

    return { filteredSimNodes: simNodes, filteredSimLinks: simLinks };
  }, [rawNodes, rawLinks, activeFilter, temporalCutoff]);

  const selectNode = useCallback((node: GraphNodeDto | null) => {
    setSelectedNode(node);
  }, []);

  const expandSeed = useCallback((nodeId: string) => {
    setSelectedSeedId(nodeId);
  }, []);

  const resetGraph = useCallback(() => {
    setSelectedSeedId(null);
    setSelectedNode(null);
    setSearchQuery("");
    setActiveFilter("ALL");
    fetchGraph(null);
  }, [fetchGraph]);

  return {
    rawNodes,
    rawLinks,
    simNodes: filteredSimNodes,
    simLinks: filteredSimLinks,
    selectedNode,
    selectedSeedId,
    searchQuery,
    setSearchQuery,
    activeFilter,
    setActiveFilter,
    hops,
    setHops,
    temporalCutoff,
    setTemporalCutoff,
    loading,
    selectNode,
    expandSeed,
    resetGraph,
    handleInvalidateDecision,
  };
}
