// Ported from Berd's agent-activity-panel (branch zmarley/agent-activity-panel).
// Layout math, grid-step selection, time↔x mapping, duration formatting, the
// default canvas theme, and hit-target types for the activity renderer.

import type {
  ActivityScene,
  ActivitySceneEvent,
  ActivitySceneNode,
} from "./activityScene";
import { clamp } from "./activityUtils";

export interface ActivityTheme {
  background: string;
  panel: string;
  panelEdge: string;
  rowHover: string;
  directoryRow: string;
  separator: string;
  grid: string;
  gridText: string;
  indentGuide: string;
  text: string;
  mutedText: string;
  directoryText: string;
  gapBand: string;
  gapText: string;
  cold: string;
  read: string;
  write: string;
  warn: string;
  live: string;
  playhead: string;
  veil: string;
  riderRing: string;
  fontMono: string;
  fontSans: string;
}

export interface ActivityEventHitTarget {
  id: string;
  kind: "event";
  x: number;
  y: number;
  radius: number;
  event: ActivitySceneEvent;
  /** Set when this marker represents a run of coalesced status events. */
  clusterCount?: number;
  clusterFirst?: ActivitySceneEvent;
  clusterLabels?: string[];
}

export interface ActivityRowHitTarget {
  id: string;
  kind: "row";
  x: number;
  y: number;
  width: number;
  height: number;
  node: ActivitySceneNode;
}

export interface ActivityGapHitTarget {
  id: string;
  kind: "gap";
  x: number;
  y: number;
  width: number;
  height: number;
  realStart: number;
  realEnd: number;
  displayStart: number;
  displayEnd: number;
}

export interface ActivityRiderHitTarget {
  id: string;
  kind: "rider";
  agentId: string;
  agentName: string;
  x: number;
  y: number;
  radius: number;
}

export interface FrameHitTargets {
  events: ActivityEventHitTarget[];
  rows: ActivityRowHitTarget[];
  gaps: ActivityGapHitTarget[];
  riders: ActivityRiderHitTarget[];
  chart: {
    x0: number;
    x1: number;
    y0: number;
    y1: number;
  };
}

export interface ActivityPanelLayout {
  treeWidth: number;
  chartX0: number;
  chartX1: number;
  chartTop: number;
  axisHeight: number;
  contentHeight: number;
  rowH: number;
  viewportHeight: number;
}

const AXIS_STRIP_HEIGHT = 24;
const BOTTOM_PADDING = 12;
const DEFAULT_TREE_WIDTH = 252;
export const CHART_RIGHT = 16;
const MIN_ROW_HEIGHT = 13;
const MAX_ROW_HEIGHT = 24;
const MIN_GRID_TICKS = 2;
const MAX_GRID_TICKS = 12;
const GRID_TICK_STEPS_MS = [
  5_000,
  10_000,
  30_000,
  60_000,
  5 * 60_000,
  15 * 60_000,
  60 * 60_000,
  2 * 60 * 60_000,
  4 * 60 * 60_000,
  8 * 60 * 60_000,
] as const;

export const DEFAULT_ACTIVITY_THEME: ActivityTheme = {
  background: "#070b13",
  panel: "rgba(10,15,26,0.94)",
  panelEdge: "rgba(27,36,54,0.9)",
  rowHover: "rgba(140,165,210,0.07)",
  directoryRow: "rgba(140,165,210,0.035)",
  separator: "rgba(255,255,255,0.03)",
  grid: "rgba(120,140,175,0.10)",
  gridText: "rgba(120,140,175,0.50)",
  indentGuide: "rgba(120,140,175,0.12)",
  text: "#dfe7f5",
  mutedText: "#9aa8c2",
  directoryText: "#8fa3c8",
  gapBand: "rgba(120,140,175,0.08)",
  gapText: "rgba(154,168,194,0.72)",
  cold: "#4a5a7d",
  read: "#38b6ff",
  write: "#ff9e3d",
  warn: "#ff5d73",
  live: "#7bd88f",
  playhead: "rgba(223,231,245,0.85)",
  veil: "rgba(3,5,9,0.6)",
  riderRing: "rgba(255,255,255,0.65)",
  fontMono: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
  fontSans: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
};

export function formatActivityDuration(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1_000));
  const hours = Math.floor(totalSeconds / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  const seconds = totalSeconds % 60;

  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }

  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

export function chooseActivityGridStep(
  displayDurationMs: number,
  chartWidthPx: number,
): number {
  const safeDuration = Math.max(1, displayDurationMs);
  const safeWidth = Number.isFinite(chartWidthPx)
    ? Math.max(0, chartWidthPx)
    : 0;
  const maxTicks = clamp(
    Math.floor(safeWidth / 72),
    MIN_GRID_TICKS,
    MAX_GRID_TICKS,
  );
  const targetStep = safeDuration / maxTicks;
  return (
    GRID_TICK_STEPS_MS.find((step) => step >= targetStep) ??
    GRID_TICK_STEPS_MS[GRID_TICK_STEPS_MS.length - 1]
  );
}

const ROW_HEIGHT_BUDGET_PX = 560;

/**
 * Comfortable natural content height for a scene: rows render at full
 * MAX_ROW_HEIGHT for small trees and compress toward MIN_ROW_HEIGHT as the
 * tree grows past the row budget, after which content height keeps growing
 * linearly (callers wrap the canvas in a scrollable viewport).
 */
export function preferredActivityContentHeight(nodeCount: number): number {
  const safeCount = Math.max(1, nodeCount);
  const rowH = clamp(
    Math.round(ROW_HEIGHT_BUDGET_PX / safeCount),
    MIN_ROW_HEIGHT,
    MAX_ROW_HEIGHT,
  );
  return AXIS_STRIP_HEIGHT + safeCount * rowH + BOTTOM_PADDING;
}

export function getActivityPanelLayout(
  width: number,
  height = 0,
  nodeCount = 0,
): ActivityPanelLayout {
  const treeWidth = Math.min(
    DEFAULT_TREE_WIDTH,
    Math.max(96, width - CHART_RIGHT - 24),
  );
  const chartX0 = treeWidth;
  const chartX1 = Math.max(chartX0 + 1, width - CHART_RIGHT);
  const availableHeight = Math.max(
    1,
    height - AXIS_STRIP_HEIGHT - BOTTOM_PADDING,
  );
  const minContentHeight =
    AXIS_STRIP_HEIGHT +
    Math.max(1, nodeCount) * MIN_ROW_HEIGHT +
    BOTTOM_PADDING;
  const contentHeight = Math.max(height || minContentHeight, minContentHeight);
  const contentAvailableHeight = Math.max(
    1,
    contentHeight - AXIS_STRIP_HEIGHT - BOTTOM_PADDING,
  );
  const rowH =
    nodeCount > 0 && nodeCount * MIN_ROW_HEIGHT > availableHeight
      ? MIN_ROW_HEIGHT
      : clamp(
          contentAvailableHeight / Math.max(1, nodeCount),
          MIN_ROW_HEIGHT,
          MAX_ROW_HEIGHT,
        );

  return {
    treeWidth,
    chartX0,
    chartX1,
    chartTop: AXIS_STRIP_HEIGHT,
    axisHeight: AXIS_STRIP_HEIGHT,
    contentHeight,
    rowH,
    viewportHeight: height,
  };
}

export function timeToCanvasX(
  scene: ActivityScene,
  displayT: number,
  layout: ActivityPanelLayout,
): number {
  const { chartX0, chartX1 } = layout;
  const duration = Math.max(1, scene.d1 - scene.d0);
  const ratio = clamp((displayT - scene.d0) / duration, 0, 1);
  return chartX0 + ratio * (chartX1 - chartX0);
}

export function canvasXToTime(
  scene: ActivityScene,
  x: number,
  layout: ActivityPanelLayout,
): number {
  const { chartX0, chartX1 } = layout;
  const ratio = clamp((x - chartX0) / Math.max(1, chartX1 - chartX0), 0, 1);
  return scene.d0 + ratio * Math.max(1, scene.d1 - scene.d0);
}
