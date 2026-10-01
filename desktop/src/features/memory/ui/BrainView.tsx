import React, { useRef, useState } from "react";
import { BrainHUD } from "./BrainHUD";
import { SuperRagSearchBar } from "./SuperRagSearchBar";
import { BrainFilterControls } from "./BrainFilterControls";
import { BrainPhysicsControls } from "./BrainPhysicsControls";
import { BrainGraph } from "./BrainGraph";
import { NodeDetailsDrawer } from "./NodeDetailsDrawer";
import { useBrainGraph } from "../lib/useBrainGraph";
import { useBrainStats } from "../lib/useBrainStats";
import { DEFAULT_PHYSICS_CONFIG } from "../lib/physics";
import type { GraphPhysicsConfig } from "../lib/types";

interface BrainViewProps {
  workspacePath?: string;
}

export const BrainView: React.FC<BrainViewProps> = ({ workspacePath }) => {
  const { stats, loading: statsLoading, isSyncing, refreshStats, triggerSync } =
    useBrainStats();

  const {
    rawNodes,
    simNodes,
    simLinks,
    selectedNode,
    searchQuery,
    setSearchQuery,
    activeFilter,
    setActiveFilter,
    temporalCutoff,
    setTemporalCutoff,
    selectNode,
    expandSeed,
    handleInvalidateDecision,
  } = useBrainGraph({ workspacePath });

  const [physicsConfig, setPhysicsConfig] =
    useState<GraphPhysicsConfig>(DEFAULT_PHYSICS_CONFIG);

  const zoomInRef = useRef<(() => void) | null>(null);
  const zoomOutRef = useRef<(() => void) | null>(null);
  const recenterRef = useRef<(() => void) | null>(null);

  // Compute matched nodes count for search bar
  const matchesCount = searchQuery.trim()
    ? simNodes.filter((n) =>
        n.name.toLowerCase().includes(searchQuery.trim().toLowerCase())
      ).length
    : 0;

  // Connected links for selected node
  const connectedLinks = selectedNode
    ? simLinks.filter(
        (l) => l.source === selectedNode.id || l.target === selectedNode.id
      )
    : [];

  return (
    <div
      data-testid="brain-view-page"
      className="relative flex flex-col w-full h-full bg-neutral-950 overflow-hidden select-none"
    >
      {/* 1. Top Ingestion & Health HUD */}
      <BrainHUD
        stats={stats}
        loading={statsLoading}
        isSyncing={isSyncing}
        onRefresh={refreshStats}
        onSync={triggerSync}
      />

      {/* 2. Main Graph Area */}
      <div className="relative flex-1 w-full h-full overflow-hidden">
        {/* Floating Top Controls (Search & Filters) */}
        <div className="absolute top-4 left-4 z-20 flex flex-col gap-2.5 max-w-[calc(100vw-2rem)]">
          <SuperRagSearchBar
            value={searchQuery}
            onChange={setSearchQuery}
            matchesCount={matchesCount}
            totalNodes={simNodes.length}
          />

          <BrainFilterControls
            activeFilter={activeFilter}
            onFilterChange={setActiveFilter}
            temporalCutoff={temporalCutoff}
            onTemporalChange={setTemporalCutoff}
          />
        </div>

        {/* Floating Bottom-Right Controls (Physics & Zoom) */}
        <div className="absolute bottom-5 right-5 z-20">
          <BrainPhysicsControls
            physicsConfig={physicsConfig}
            onPhysicsChange={(newCfg) =>
              setPhysicsConfig((prev) => ({ ...prev, ...newCfg }))
            }
            onZoomIn={() => zoomInRef.current?.()}
            onZoomOut={() => zoomOutRef.current?.()}
            onRecenter={() => recenterRef.current?.()}
          />
        </div>

        {/* D3 Canvas Simulation */}
        <BrainGraph
          nodes={simNodes}
          links={simLinks}
          selectedNode={selectedNode}
          searchQuery={searchQuery}
          physicsConfig={physicsConfig}
          onSelectNode={selectNode}
          onExpandSeed={expandSeed}
          zoomInRef={zoomInRef}
          zoomOutRef={zoomOutRef}
          recenterRef={recenterRef}
        />

        {/* Slide-out Node Inspector Drawer */}
        <NodeDetailsDrawer
          node={selectedNode}
          connectedLinks={connectedLinks}
          allNodes={rawNodes}
          onClose={() => selectNode(null)}
          onSelectNode={(id) => {
            const target = rawNodes.find((n) => n.id === id);
            if (target) selectNode(target);
          }}
          onInvalidateDecision={handleInvalidateDecision}
        />
      </div>
    </div>
  );
};
