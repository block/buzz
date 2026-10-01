import React, { useEffect, useRef, useState, useCallback, useMemo } from "react";
import type { SimNode, SimLink, GraphNodeDto, GraphPhysicsConfig } from "../lib/types";
import { BrainPhysicsSimulation } from "../lib/physics";
import { EDGE_COLORS } from "../lib/colors";

interface BrainGraphProps {
  nodes: SimNode[];
  links: SimLink[];
  selectedNode: GraphNodeDto | null;
  searchQuery: string;
  physicsConfig: GraphPhysicsConfig;
  onSelectNode: (node: GraphNodeDto | null) => void;
  onExpandSeed?: (nodeId: string) => void;
  zoomInRef?: React.MutableRefObject<(() => void) | null>;
  zoomOutRef?: React.MutableRefObject<(() => void) | null>;
  recenterRef?: React.MutableRefObject<(() => void) | null>;
}

export const BrainGraph: React.FC<BrainGraphProps> = ({
  nodes,
  links,
  selectedNode,
  searchQuery,
  physicsConfig,
  onSelectNode,
  onExpandSeed,
  zoomInRef,
  zoomOutRef,
  recenterRef,
}) => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);

  // Camera transform state (pan & zoom)
  const [transform, setTransform] = useState({ x: 0, y: 0, scale: 1 });
  const [hoveredNode, setHoveredNode] = useState<SimNode | null>(null);

  // Dragging and interaction refs
  const isPanning = useRef(false);
  const draggedNode = useRef<SimNode | null>(null);
  const lastMousePos = useRef({ x: 0, y: 0 });

  // Physics simulation instance
  const simRef = useRef<BrainPhysicsSimulation>(new BrainPhysicsSimulation());

  // Connected node IDs for focus highlighting
  const activeFocusIds = useMemo(() => {
    const focusNode = hoveredNode ?? (selectedNode as SimNode | null);
    if (!focusNode) return null;

    const set = new Set<string>();
    set.add(focusNode.id);
    for (const link of links) {
      if (link.source === focusNode.id) set.add(link.target);
      if (link.target === focusNode.id) set.add(link.source);
    }
    return set;
  }, [hoveredNode, selectedNode, links]);

  // Update physics config
  useEffect(() => {
    simRef.current.setConfig(physicsConfig);
  }, [physicsConfig]);

  // Update simulation dimensions & graph
  useEffect(() => {
    const container = containerRef.current;
    if (container) {
      const { clientWidth, clientHeight } = container;
      simRef.current.setDimensions(clientWidth, clientHeight);
    }
    simRef.current.setGraph(nodes, links);
  }, [nodes, links]);

  // Zoom control callbacks
  const handleZoom = useCallback((factor: number) => {
    setTransform((prev) => ({
      ...prev,
      scale: Math.max(0.15, Math.min(prev.scale * factor, 3.5)),
    }));
  }, []);

  const handleRecenter = useCallback(() => {
    setTransform({ x: 0, y: 0, scale: 1 });
  }, []);

  useEffect(() => {
    if (zoomInRef) zoomInRef.current = () => handleZoom(1.2);
    if (zoomOutRef) zoomOutRef.current = () => handleZoom(0.83);
    if (recenterRef) recenterRef.current = handleRecenter;
  }, [zoomInRef, zoomOutRef, recenterRef, handleZoom, handleRecenter]);

  // Main 60 FPS Render Loop
  useEffect(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container) return;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    let animId: number;
    let pulseAngle = 0;

    const render = () => {
      // Step physics
      simRef.current.tick();
      pulseAngle += 0.04;

      const dpr = window.devicePixelRatio || 1;
      const width = container.clientWidth;
      const height = container.clientHeight;

      if (canvas.width !== width * dpr || canvas.height !== height * dpr) {
        canvas.width = width * dpr;
        canvas.height = height * dpr;
        simRef.current.setDimensions(width, height);
      }

      ctx.save();
      ctx.scale(dpr, dpr);
      ctx.clearRect(0, 0, width, height);

      // Deep dark cosmic canvas background
      ctx.fillStyle = "#09090b";
      ctx.fillRect(0, 0, width, height);

      // Apply camera translation & scale
      ctx.save();
      ctx.translate(width / 2 + transform.x, height / 2 + transform.y);
      ctx.scale(transform.scale, transform.scale);
      ctx.translate(-width / 2, -height / 2);

      // 1. Draw Links
      for (let i = 0; i < links.length; i++) {
        const link = links[i];
        const s = link.sourceNode;
        const t = link.targetNode;
        if (!s || !t) continue;

        const isHighlighted =
          activeFocusIds != null &&
          activeFocusIds.has(s.id) &&
          activeFocusIds.has(t.id);
        const isDimmed = activeFocusIds != null && !isHighlighted;

        ctx.beginPath();
        ctx.moveTo(s.x, s.y);
        ctx.lineTo(t.x, t.y);

        if (!link.is_active) {
          ctx.strokeStyle = isDimmed ? "rgba(239, 68, 68, 0.15)" : EDGE_COLORS.supersedes;
          ctx.setLineDash([4, 4]);
          ctx.lineWidth = 1.2;
        } else {
          ctx.strokeStyle = isHighlighted
            ? EDGE_COLORS.highlighted
            : isDimmed
            ? "rgba(148, 163, 184, 0.08)"
            : EDGE_COLORS.active;
          ctx.setLineDash([]);
          ctx.lineWidth = isHighlighted ? 1.8 : 1.0;
        }
        ctx.stroke();

        // Draw edge relation label if highlighted or zoomed in
        if (transform.scale > 0.85 || isHighlighted) {
          const mx = (s.x + t.x) * 0.5;
          const my = (s.y + t.y) * 0.5;
          ctx.fillStyle = isHighlighted
            ? "rgba(255, 255, 255, 0.9)"
            : isDimmed
            ? "rgba(148, 163, 184, 0.2)"
            : EDGE_COLORS.label;
          ctx.font = "9px ui-monospace, SFMono-Regular, monospace";
          ctx.textAlign = "center";
          ctx.fillText(link.relation_type, mx, my - 2);
        }
      }

      // 2. Draw Nodes
      for (let i = 0; i < nodes.length; i++) {
        const node = nodes[i];
        const isSelected = selectedNode?.id === node.id;
        const isHovered = hoveredNode?.id === node.id;
        const isFocused = activeFocusIds != null ? activeFocusIds.has(node.id) : true;
        const matchesQuery =
          searchQuery.trim().length > 0 &&
          node.name.toLowerCase().includes(searchQuery.trim().toLowerCase());

        const radius = isSelected ? node.radius * 1.35 : node.radius;

        // Glowing Halo for Selected or Project or Focused Nodes
        if (isSelected || isHovered || node.entity_type === "project") {
          const pulse = node.entity_type === "project" ? Math.sin(pulseAngle) * 3 : 0;
          ctx.beginPath();
          ctx.arc(node.x, node.y, radius + 6 + pulse, 0, Math.PI * 2);
          ctx.fillStyle = node.glowColor;
          ctx.fill();
        }

        // Draw Node Geometry
        ctx.beginPath();
        if (node.entity_type === "decision") {
          // Diamond shape for architectural decisions
          const d = radius * 1.25;
          ctx.moveTo(node.x, node.y - d);
          ctx.lineTo(node.x + d, node.y);
          ctx.lineTo(node.x, node.y + d);
          ctx.lineTo(node.x - d, node.y);
          ctx.closePath();
        } else {
          // Circular orb
          ctx.arc(node.x, node.y, radius, 0, Math.PI * 2);
        }

        ctx.fillStyle = isFocused ? node.color : "rgba(100, 116, 139, 0.35)";
        ctx.fill();

        // Crisp Border
        if (isSelected || matchesQuery) {
          ctx.strokeStyle = "#ffffff";
          ctx.lineWidth = 2.5;
          ctx.stroke();
        } else {
          ctx.strokeStyle = "rgba(255, 255, 255, 0.3)";
          ctx.lineWidth = 1;
          ctx.stroke();
        }

        // Label Rendering
        const showLabel = isSelected || isHovered || isFocused || transform.scale > 0.7;
        if (showLabel) {
          const fontSize = isSelected ? 12 : 10;
          ctx.font = `${isSelected ? "600" : "400"} ${fontSize}px Inter, sans-serif`;
          ctx.textAlign = "center";

          // Text pill background for legibility
          const text = node.name;
          const metrics = ctx.measureText(text);
          const bgWidth = metrics.width + 6;
          const bgHeight = fontSize + 4;
          const labelY = node.y + radius + 11;

          ctx.fillStyle = "rgba(9, 9, 11, 0.85)";
          ctx.fillRect(node.x - bgWidth / 2, labelY - bgHeight + 3, bgWidth, bgHeight);

          ctx.fillStyle = isSelected
            ? "#ffffff"
            : isFocused
            ? "#e2e8f0"
            : "rgba(148, 163, 184, 0.4)";
          ctx.fillText(text, node.x, labelY);
        }
      }

      ctx.restore();
      ctx.restore();

      animId = requestAnimationFrame(render);
    };

    render();
    return () => cancelAnimationFrame(animId);
  }, [nodes, links, transform, selectedNode, hoveredNode, activeFocusIds, searchQuery]);

  // Coordinate Conversion: Screen to World
  const screenToWorld = useCallback(
    (clientX: number, clientY: number) => {
      const canvas = canvasRef.current;
      const container = containerRef.current;
      if (!canvas || !container) return { x: 0, y: 0 };
      const rect = canvas.getBoundingClientRect();
      const width = container.clientWidth;
      const height = container.clientHeight;

      const px = clientX - rect.left;
      const py = clientY - rect.top;

      const worldX = (px - width / 2 - transform.x) / transform.scale + width / 2;
      const worldY = (py - height / 2 - transform.y) / transform.scale + height / 2;

      return { x: worldX, y: worldY };
    },
    [transform]
  );

  // Mouse Interaction Handlers
  const handleMouseDown = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const { x, y } = screenToWorld(e.clientX, e.clientY);
    lastMousePos.current = { x: e.clientX, y: e.clientY };

    // Check hit on nodes
    const hit = nodes.find((n) => {
      const dx = n.x - x;
      const dy = n.y - y;
      return Math.sqrt(dx * dx + dy * dy) <= n.radius + 6;
    });

    if (hit) {
      draggedNode.current = hit;
      hit.fx = hit.x;
      hit.fy = hit.y;
      onSelectNode(hit);
      if (onExpandSeed) onExpandSeed(hit.id);
    } else {
      isPanning.current = true;
    }
  };

  const handleMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const { x, y } = screenToWorld(e.clientX, e.clientY);

    // If dragging node
    if (draggedNode.current) {
      draggedNode.current.fx = x;
      draggedNode.current.fy = y;
      return;
    }

    // If panning canvas
    if (isPanning.current) {
      const dx = e.clientX - lastMousePos.current.x;
      const dy = e.clientY - lastMousePos.current.y;
      lastMousePos.current = { x: e.clientX, y: e.clientY };
      setTransform((prev) => ({ ...prev, x: prev.x + dx, y: prev.y + dy }));
      return;
    }

    // Hover detection
    const hit = nodes.find((n) => {
      const dx = n.x - x;
      const dy = n.y - y;
      return Math.sqrt(dx * dx + dy * dy) <= n.radius + 5;
    });

    setHoveredNode(hit || null);
  };

  const handleMouseUp = () => {
    if (draggedNode.current) {
      draggedNode.current.fx = null;
      draggedNode.current.fy = null;
      draggedNode.current = null;
    }
    isPanning.current = false;
  };

  const handleWheel = (e: React.WheelEvent<HTMLCanvasElement>) => {
    e.preventDefault();
    const zoomFactor = e.deltaY < 0 ? 1.08 : 0.92;
    handleZoom(zoomFactor);
  };

  return (
    <div
      ref={containerRef}
      data-testid="brain-graph-canvas-container"
      className="relative w-full h-full overflow-hidden select-none bg-neutral-950 cursor-grab active:cursor-grabbing"
    >
      <canvas
        ref={canvasRef}
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onMouseLeave={handleMouseUp}
        onWheel={handleWheel}
        className="w-full h-full block"
      />
    </div>
  );
};
