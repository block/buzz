// Ported from Berd's agent-activity-panel (branch zmarley/agent-activity-panel).
// Canvas 2D drawing passes for the activity scene; layout/theme live in renderSceneLayout.ts.

import type {
  ActivityScene,
  ActivitySceneAgent,
  ActivitySceneEvent,
  Rgb,
} from "./activityScene";
import {
  binarySearchPrefix,
  clamp,
  hexToRgb,
  lerp,
  mixRgb,
  rgba,
  smooth,
} from "./activityUtils";
import type {
  ActivityEventHitTarget,
  ActivityGapHitTarget,
  ActivityPanelLayout,
  ActivityRiderHitTarget,
  ActivityRowHitTarget,
  ActivityTheme,
  FrameHitTargets,
} from "./renderSceneLayout";
import {
  CHART_RIGHT,
  chooseActivityGridStep,
  DEFAULT_ACTIVITY_THEME,
  formatActivityDuration,
  getActivityPanelLayout,
  timeToCanvasX,
} from "./renderSceneLayout";

export interface ActivityHoverView {
  rowPath?: string;
  eventId?: string;
  gapId?: string;
}

export interface ActivityFrameView {
  viewT: number;
  liveT: number;
  width: number;
  height: number;
  dpr: number;
  hover?: ActivityHoverView | null;
}

interface FrameLayout {
  ys: number[];
  hs: number[];
  rowH: number;
  gridBottom: number;
}

const RECENT_SEGMENT_MS = 6_000;
const READ_FADE_MS = 6_000;
const WRITE_FADE_MS = 8_000;
const GRID_LABEL_MIN_SPACING_PX = 40;

function computeRows(
  scene: ActivityScene,
  viewT: number,
  mapLayout: ActivityPanelLayout,
): FrameLayout {
  const rowH = mapLayout.rowH;
  const ys: number[] = [];
  const hs: number[] = [];
  let y = mapLayout.chartTop;

  for (const node of scene.nodes) {
    const born = smooth((viewT - node.displayRevealAt) / 450);
    const h = rowH * born;
    ys.push(y);
    hs.push(h);
    y += h;
  }

  return {
    ys,
    hs,
    rowH,
    gridBottom: y,
  };
}

function rowMidY(layout: FrameLayout, row: number) {
  return layout.ys[row] + layout.hs[row] / 2;
}

function threadY(
  agent: ActivitySceneAgent,
  viewT: number,
  layout: FrameLayout,
): number | null {
  const points = agent.points;
  if (points.length === 0 || viewT < points[0].displayT) return null;

  const index = Math.max(0, binarySearchPrefix(agent.pointTimes, viewT) - 1);

  const current = points[index];
  const next = points[index + 1];
  const y0 = rowMidY(layout, current.node.row);
  if (!next) return y0;

  const progress = smooth(
    (viewT - current.displayT) / Math.max(1, next.displayT - current.displayT),
  );
  return lerp(y0, rowMidY(layout, next.node.row), progress);
}

function drawBackground(
  ctx: CanvasRenderingContext2D,
  view: ActivityFrameView,
  theme: ActivityTheme,
  mapLayout: ActivityPanelLayout,
) {
  ctx.fillStyle = theme.background;
  ctx.fillRect(0, 0, view.width, view.height);
  ctx.fillStyle = theme.panel;
  ctx.fillRect(
    0,
    mapLayout.chartTop,
    mapLayout.treeWidth,
    view.height - mapLayout.chartTop,
  );
}

function drawRows(
  ctx: CanvasRenderingContext2D,
  scene: ActivityScene,
  view: ActivityFrameView,
  theme: ActivityTheme,
  layout: FrameLayout,
  readRgb: Rgb,
  writeRgb: Rgb,
  warnRgb: Rgb,
  coldRgb: Rgb,
): ActivityRowHitTarget[] {
  const rows: ActivityRowHitTarget[] = [];

  for (const node of scene.nodes) {
    const y = layout.ys[node.row];
    const height = layout.hs[node.row];
    if (height <= 0.5) continue;

    const midY = y + height / 2;
    const born = height / layout.rowH;
    const heat = scene.nodeHeat(node, view.viewT);
    const hovered = view.hover?.rowPath === node.path;

    if (hovered) {
      ctx.fillStyle = theme.rowHover;
      ctx.fillRect(0, y, view.width, height);
    } else if (node.isDir) {
      ctx.fillStyle = theme.directoryRow;
      ctx.fillRect(0, y, view.width, height);
    }

    ctx.fillStyle = theme.separator;
    ctx.fillRect(0, y + height, view.width, 1);

    for (let depth = 1; depth <= node.depth; depth += 1) {
      ctx.fillStyle = theme.indentGuide;
      ctx.fillRect(10 + depth * 14, y, 1, height);
    }

    rows.push({
      id: node.path,
      kind: "row",
      x: 0,
      y,
      width: view.width,
      height,
      node,
    });

    if (born <= 0.5) continue;

    const alpha = (born - 0.5) * 2;
    const indent = 16 + node.depth * 14;
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";

    if (node.isDir) {
      ctx.font = `600 11px ${theme.fontMono}`;
      ctx.fillStyle = theme.directoryText;
      ctx.globalAlpha = 0.9 * alpha;
      ctx.fillText(`▾ ${node.name}/`, indent, midY);
      ctx.globalAlpha = 1;
      continue;
    }

    const totalHeat = heat.hr + heat.hw;
    let dotColor = coldRgb;
    let dotAlpha = 0.35;
    if (totalHeat > 0.03) {
      dotColor = mixRgb(readRgb, writeRgb, heat.hw / totalHeat);
      dotAlpha = clamp(0.5 + totalHeat * 0.3, 0.5, 1);
    }
    if (heat.warn > 0.05) {
      dotColor = mixRgb(dotColor, warnRgb, Math.min(1, heat.warn));
    }

    ctx.fillStyle = rgba(dotColor, dotAlpha * alpha);
    ctx.beginPath();
    ctx.arc(indent + 3, midY, 2.5 + Math.min(2, totalHeat), 0, Math.PI * 2);
    ctx.fill();

    if (totalHeat > 0.4) {
      ctx.fillStyle = rgba(dotColor, 0.14 * alpha);
      ctx.beginPath();
      ctx.arc(indent + 3, midY, 7 + totalHeat * 3, 0, Math.PI * 2);
      ctx.fill();
    }

    ctx.font = `11px ${theme.fontMono}`;
    const baseText = hexToRgb(theme.mutedText);
    let nameColor =
      totalHeat > 0.03
        ? mixRgb(baseText, hexToRgb(theme.text), Math.min(1, totalHeat * 0.7))
        : baseText;
    if (heat.warn > 0.1) {
      nameColor = mixRgb(nameColor, warnRgb, Math.min(1, heat.warn * 0.8));
    }
    ctx.fillStyle = rgba(nameColor, (totalHeat > 0.03 ? 1 : 0.62) * alpha);
    ctx.fillText(node.name, indent + 12, midY);
  }

  return rows;
}

function drawTimeGrid(
  ctx: CanvasRenderingContext2D,
  scene: ActivityScene,
  theme: ActivityTheme,
  layout: FrameLayout,
  mapLayout: ActivityPanelLayout,
) {
  const step = chooseActivityGridStep(
    scene.d1,
    mapLayout.chartX1 - mapLayout.chartX0,
  );
  let lastLabelX = Number.NEGATIVE_INFINITY;
  ctx.textAlign = "left";
  ctx.textBaseline = "alphabetic";
  ctx.font = `9px ${theme.fontMono}`;

  for (let displayT = 0; displayT <= scene.d1 + 0.5; displayT += step) {
    const x = timeToCanvasX(scene, displayT, mapLayout);
    ctx.fillStyle = theme.grid;
    ctx.fillRect(
      x,
      mapLayout.chartTop,
      1,
      layout.gridBottom - mapLayout.chartTop,
    );
    if (x - lastLabelX < GRID_LABEL_MIN_SPACING_PX) continue;

    ctx.fillStyle = theme.gridText;
    ctx.fillText(formatActivityDuration(displayT), x + 3, 14);
    lastLabelX = x;
  }
}

function drawGapMarkers(
  ctx: CanvasRenderingContext2D,
  scene: ActivityScene,
  view: ActivityFrameView,
  theme: ActivityTheme,
  mapLayout: ActivityPanelLayout,
): ActivityGapHitTarget[] {
  const targets: ActivityGapHitTarget[] = [];
  const axisTop = 0;
  const axisBottom = mapLayout.axisHeight;
  const tickTop = 4;
  const tickBottom = Math.max(tickTop, axisBottom - 5);

  for (let index = 0; index < scene.timeMap.gaps.length; index += 1) {
    const gap = scene.timeMap.gaps[index];
    const x0 = timeToCanvasX(scene, gap.displayStart, mapLayout);
    const x1 = timeToCanvasX(scene, gap.displayEnd, mapLayout);
    const width = Math.max(1, x1 - x0);
    const hovered = view.hover?.gapId === `gap-${index}`;

    ctx.strokeStyle = hovered ? theme.gapText : theme.gapBand;
    ctx.lineWidth = hovered ? 1.4 : 1;
    ctx.beginPath();
    ctx.moveTo(x0, tickTop);
    ctx.lineTo(x0, tickBottom);
    ctx.moveTo(x1, tickTop);
    ctx.lineTo(x1, tickBottom);
    if (width >= 4) {
      ctx.moveTo(x0, tickBottom);
      ctx.lineTo(x1, tickBottom);
    }
    ctx.stroke();

    if (width >= 12) {
      ctx.fillStyle = theme.gapText;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.font = `11px ${theme.fontMono}`;
      ctx.fillText("⋯", x0 + width / 2, axisBottom / 2);
    }

    const hitWidth = Math.max(8, width);
    targets.push({
      id: `gap-${index}`,
      kind: "gap",
      x: x0 + width / 2 - hitWidth / 2,
      y: axisTop,
      width: hitWidth,
      height: mapLayout.axisHeight,
      realStart: gap.realStart,
      realEnd: gap.realEnd,
      displayStart: gap.displayStart,
      displayEnd: gap.displayEnd,
    });
  }

  return targets;
}

function drawHeatTrails(
  ctx: CanvasRenderingContext2D,
  scene: ActivityScene,
  view: ActivityFrameView,
  layout: FrameLayout,
  mapLayout: ActivityPanelLayout,
  readRgb: Rgb,
  writeRgb: Rgb,
  warnRgb: Rgb,
) {
  for (const event of scene.events) {
    if (event.displayT > view.viewT) break;
    if (!event.node) continue;
    const y = layout.ys[event.node.row];
    const height = layout.hs[event.node.row];
    if (height <= 0.5) continue;

    const warn = event.warn || event.kind === "x";
    const color = event.kind === "r" ? readRgb : warn ? warnRgb : writeRgb;
    const fadeMs = event.kind === "r" ? READ_FADE_MS : WRITE_FADE_MS;
    const x0 = timeToCanvasX(scene, event.displayT, mapLayout);
    const xFade = timeToCanvasX(scene, event.displayT + fadeMs, mapLayout);
    const x1 = Math.min(xFade, timeToCanvasX(scene, view.viewT, mapLayout));
    if (x1 <= x0) continue;

    const gradient = ctx.createLinearGradient(x0, 0, xFade, 0);
    gradient.addColorStop(0, rgba(color, event.kind === "r" ? 0.16 : 0.26));
    gradient.addColorStop(1, rgba(color, 0));
    ctx.fillStyle = gradient;
    ctx.fillRect(x0, y + 1, x1 - x0, Math.max(0, height - 2));
  }
}

function drawAgentPath(
  ctx: CanvasRenderingContext2D,
  points: Array<{ x: number; y: number }>,
) {
  if (points.length === 0) return;
  ctx.beginPath();
  ctx.moveTo(points[0].x, points[0].y);
  for (let index = 1; index < points.length; index += 1) {
    const p0 = points[index - 1];
    const p1 = points[index];
    const dx = (p1.x - p0.x) / 3;
    ctx.bezierCurveTo(p0.x + dx, p0.y, p1.x - dx, p1.y, p1.x, p1.y);
  }
}

function drawThreads(
  ctx: CanvasRenderingContext2D,
  scene: ActivityScene,
  view: ActivityFrameView,
  layout: FrameLayout,
  mapLayout: ActivityPanelLayout,
) {
  ctx.lineCap = "round";
  ctx.lineJoin = "round";

  for (const agent of scene.agents) {
    if (view.viewT < agent.displaySpawnT) continue;

    const points = agent.points
      .filter((point) => point.displayT <= view.viewT)
      .map((point) => ({
        x: timeToCanvasX(scene, point.displayT, mapLayout),
        y: rowMidY(layout, point.node.row),
        t: point.displayT,
      }));
    if (points.length === 0) continue;

    const nowY = threadY(agent, view.viewT, layout);
    const nowX = timeToCanvasX(scene, view.viewT, mapLayout);
    if (nowY === null) continue;

    const fade =
      view.viewT > agent.displayDoneT
        ? Math.max(0.25, 1 - (view.viewT - agent.displayDoneT) / 2_000)
        : 1;

    drawAgentPath(ctx, points);
    const last = points.at(-1);
    if (last && nowX > last.x) {
      const dx = (nowX - last.x) / 3;
      ctx.bezierCurveTo(last.x + dx, last.y, nowX - dx, nowY, nowX, nowY);
    }
    ctx.strokeStyle = rgba(agent.rgb, 0.42 * fade);
    ctx.lineWidth = 1.4;
    ctx.stroke();

    const recent = points.filter(
      (point) => view.viewT - point.t < RECENT_SEGMENT_MS,
    );
    const segment =
      recent.length > 0 ? recent : [{ x: nowX - 1, y: nowY, t: view.viewT }];
    drawAgentPath(ctx, segment);
    const segmentLast = segment.at(-1);
    if (segmentLast && nowX > segmentLast.x) {
      const dx = (nowX - segmentLast.x) / 3;
      ctx.bezierCurveTo(
        segmentLast.x + dx,
        segmentLast.y,
        nowX - dx,
        nowY,
        nowX,
        nowY,
      );
    }
    ctx.strokeStyle = rgba(agent.rgb, 0.85 * fade);
    ctx.lineWidth = 2;
    ctx.stroke();
  }
}

function drawEventMarkers(
  ctx: CanvasRenderingContext2D,
  scene: ActivityScene,
  view: ActivityFrameView,
  theme: ActivityTheme,
  layout: FrameLayout,
  mapLayout: ActivityPanelLayout,
  readRgb: Rgb,
  writeRgb: Rgb,
  warnRgb: Rgb,
): ActivityEventHitTarget[] {
  const targets: ActivityEventHitTarget[] = [];

  // Coalesce runs of pathless status diamonds that land within a few pixels
  // of each other into one marker with a count, so shell-heavy sessions don't
  // dissolve into diamond confetti.
  const CLUSTER_PX = 14;
  let statusRun: {
    agentId: string;
    xStart: number;
    xEnd: number;
    y: number;
    first: ActivitySceneEvent;
    last: ActivitySceneEvent;
    count: number;
    labels: string[];
  } | null = null;

  const flushStatusRun = () => {
    if (!statusRun) return;
    const run = statusRun;
    statusRun = null;
    const agent = scene.agentsById.get(run.agentId);
    if (!agent) return;
    const x = run.count === 1 ? run.xStart : (run.xStart + run.xEnd) / 2;
    const hovered = view.hover?.eventId === run.last.id;
    const size = 2.8;
    ctx.save();
    ctx.translate(x, run.y);
    ctx.rotate(Math.PI / 4);
    ctx.fillStyle = rgba(agent.rgb, hovered ? 1 : 0.9);
    ctx.fillRect(-size, -size, size * 2, size * 2);
    ctx.restore();
    if (run.count > 1) {
      ctx.fillStyle = rgba(agent.rgb, hovered ? 1 : 0.85);
      ctx.font = `9px ${theme.fontMono}`;
      ctx.textAlign = "left";
      ctx.textBaseline = "middle";
      ctx.fillText(`×${run.count}`, x + 6, run.y - 6);
    }
    targets.push({
      id: run.last.id,
      kind: "event",
      x,
      y: run.y,
      radius: Math.max(8, (run.xEnd - run.xStart) / 2 + 6),
      event: run.last,
      ...(run.count > 1
        ? {
            clusterCount: run.count,
            clusterFirst: run.first,
            clusterLabels: run.labels,
          }
        : {}),
    });
  };

  for (const event of scene.events) {
    if (event.displayT > view.viewT) break;
    const agent = scene.agentsById.get(event.agentId);
    if (!agent) continue;

    let y: number | null;
    if (event.node) {
      if (layout.hs[event.node.row] <= 0.5) continue;
      y = rowMidY(layout, event.node.row);
    } else {
      y = threadY(agent, event.displayT, layout);
    }
    if (y === null) continue;

    const x = timeToCanvasX(scene, event.displayT, mapLayout);
    const age = view.viewT - event.displayT;
    const warn = event.warn || event.kind === "x";
    const id = event.id;
    const hovered = view.hover?.eventId === id;

    if (event.kind === "s") {
      if (
        statusRun &&
        statusRun.agentId === event.agentId &&
        Math.abs(statusRun.y - y) < 0.5 &&
        x - statusRun.xEnd <= CLUSTER_PX
      ) {
        statusRun.xEnd = x;
        statusRun.last = event;
        statusRun.count += 1;
        if (event.label && statusRun.labels.at(-1) !== event.label) {
          statusRun.labels.push(event.label);
        }
        continue;
      }
      flushStatusRun();
      statusRun = {
        agentId: event.agentId,
        xStart: x,
        xEnd: x,
        y,
        first: event,
        last: event,
        count: 1,
        labels: event.label ? [event.label] : [],
      };
      continue;
    }

    flushStatusRun();

    if (event.kind === "sp" || event.kind === "d") {
      const size = event.kind === "sp" ? 3.4 : 2.8;
      ctx.save();
      ctx.translate(x, y);
      ctx.rotate(Math.PI / 4);
      ctx.fillStyle = rgba(agent.rgb, hovered ? 1 : 0.9);
      ctx.fillRect(-size, -size, size * 2, size * 2);
      ctx.restore();
      targets.push({ id, kind: "event", x, y, radius: 8, event });
      continue;
    }

    const color = event.kind === "r" ? readRgb : warn ? warnRgb : writeRgb;
    const radius = event.kind === "r" ? 2.2 : 3;
    ctx.fillStyle = rgba(color, event.kind === "r" ? 0.75 : 0.95);
    ctx.beginPath();
    ctx.arc(x, y, radius, 0, Math.PI * 2);
    ctx.fill();

    if (event.kind === "c") {
      ctx.strokeStyle = rgba(color, 0.9);
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.arc(x, y, radius + 2.5, 0, Math.PI * 2);
      ctx.stroke();
    }

    ctx.strokeStyle = rgba(agent.rgb, 0.85);
    ctx.lineWidth = hovered ? 1.7 : 1;
    ctx.beginPath();
    ctx.arc(x, y, radius + 1.2, 0, Math.PI * 2);
    ctx.stroke();

    if (age < 700) {
      const pulse = age / 700;
      ctx.strokeStyle = rgba(color, (1 - pulse) * 0.9);
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.arc(x, y, radius + pulse * 16, 0, Math.PI * 2);
      ctx.stroke();
    }

    if (warn) {
      const phase = ((view.viewT / 1_000) * 1.6) % 1;
      ctx.strokeStyle = rgba(warnRgb, (1 - phase) * 0.55);
      ctx.lineWidth = 1.4;
      ctx.beginPath();
      ctx.arc(x, y, radius + 3 + phase * 10, 0, Math.PI * 2);
      ctx.stroke();
    }

    targets.push({ id, kind: "event", x, y, radius: 8, event });
  }

  flushStatusRun();

  return targets;
}

function drawVeilAndPlayhead(
  ctx: CanvasRenderingContext2D,
  scene: ActivityScene,
  view: ActivityFrameView,
  theme: ActivityTheme,
  layout: FrameLayout,
  mapLayout: ActivityPanelLayout,
) {
  const { chartX1 } = mapLayout;
  const liveX = timeToCanvasX(scene, view.liveT, mapLayout);
  ctx.fillStyle = theme.veil;
  ctx.fillRect(
    liveX,
    mapLayout.chartTop,
    chartX1 - liveX + CHART_RIGHT,
    layout.gridBottom - mapLayout.chartTop,
  );
  ctx.fillStyle = theme.live;
  ctx.globalAlpha = 0.7;
  ctx.fillRect(
    liveX - 0.5,
    mapLayout.chartTop,
    1.5,
    layout.gridBottom - mapLayout.chartTop,
  );
  ctx.globalAlpha = 1;

  const playheadX = timeToCanvasX(scene, view.viewT, mapLayout);
  ctx.fillStyle = theme.playhead;
  ctx.fillRect(
    playheadX - 0.5,
    mapLayout.chartTop,
    1,
    layout.gridBottom - mapLayout.chartTop,
  );
  ctx.beginPath();
  ctx.moveTo(playheadX - 5, mapLayout.chartTop);
  ctx.lineTo(playheadX + 5, mapLayout.chartTop);
  ctx.lineTo(playheadX, mapLayout.chartTop + 7);
  ctx.closePath();
  ctx.fill();
}

const RIDER_HIT_SLOP_PX = 6;

function drawRiders(
  ctx: CanvasRenderingContext2D,
  scene: ActivityScene,
  view: ActivityFrameView,
  theme: ActivityTheme,
  layout: FrameLayout,
  mapLayout: ActivityPanelLayout,
): ActivityRiderHitTarget[] {
  const playheadX = timeToCanvasX(scene, view.viewT, mapLayout);
  const targets: ActivityRiderHitTarget[] = [];
  const riders = scene.agents.flatMap((agent) => {
    if (view.viewT < agent.displaySpawnT) return [];
    const y = threadY(agent, view.viewT, layout);
    if (y === null) return [];
    let alpha = Math.min(1, (view.viewT - agent.displaySpawnT) / 700);
    if (view.viewT > agent.displayDoneT) {
      alpha = Math.max(0.25, 1 - (view.viewT - agent.displayDoneT) / 1_500);
    }
    return [{ agent, y, alpha, labelY: y }];
  });

  riders.sort((a, b) => a.y - b.y);
  for (let index = 1; index < riders.length; index += 1) {
    if (riders[index].labelY - riders[index - 1].labelY < 13) {
      riders[index].labelY = riders[index - 1].labelY + 13;
    }
  }

  for (const rider of riders) {
    const radius = rider.agent.id === scene.primaryAgentId ? 6 : 5;
    targets.push({
      id: `rider:${rider.agent.id}`,
      kind: "rider",
      agentId: rider.agent.id,
      agentName: rider.agent.name,
      x: playheadX,
      y: rider.y,
      radius: radius + RIDER_HIT_SLOP_PX,
    });
    const glow = ctx.createRadialGradient(
      playheadX,
      rider.y,
      0,
      playheadX,
      rider.y,
      radius * 3,
    );
    glow.addColorStop(0, rgba(rider.agent.rgb, 0.45 * rider.alpha));
    glow.addColorStop(1, rgba(rider.agent.rgb, 0));
    ctx.fillStyle = glow;
    ctx.beginPath();
    ctx.arc(playheadX, rider.y, radius * 3, 0, Math.PI * 2);
    ctx.fill();

    ctx.fillStyle = rgba(rider.agent.rgb, rider.alpha);
    ctx.beginPath();
    ctx.arc(playheadX, rider.y, radius, 0, Math.PI * 2);
    ctx.fill();
    ctx.strokeStyle = theme.riderRing;
    ctx.globalAlpha = rider.alpha;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.arc(playheadX, rider.y, radius, 0, Math.PI * 2);
    ctx.stroke();
    ctx.globalAlpha = 1;

    ctx.font = `600 10.5px ${theme.fontSans}`;
    const flip = playheadX > view.width - 110;
    ctx.textAlign = flip ? "right" : "left";
    ctx.textBaseline = "middle";
    ctx.fillStyle = rgba(rider.agent.rgb, Math.min(1, rider.alpha + 0.15));
    ctx.fillText(rider.agent.name, playheadX + (flip ? -10 : 10), rider.labelY);
  }

  return targets;
}

export function drawFrame(
  ctx: CanvasRenderingContext2D,
  scene: ActivityScene,
  view: ActivityFrameView,
  theme: ActivityTheme = DEFAULT_ACTIVITY_THEME,
): FrameHitTargets {
  const dpr = view.dpr || 1;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, view.width, view.height);

  const mapLayout = getActivityPanelLayout(
    view.width,
    view.height,
    scene.nodes.length,
  );
  const layout = computeRows(scene, view.viewT, mapLayout);
  const readRgb = hexToRgb(theme.read);
  const writeRgb = hexToRgb(theme.write);
  const warnRgb = hexToRgb(theme.warn);
  const coldRgb = hexToRgb(theme.cold);
  const { chartX0, chartX1 } = mapLayout;

  drawBackground(ctx, view, theme, mapLayout);
  const rows = drawRows(
    ctx,
    scene,
    view,
    theme,
    layout,
    readRgb,
    writeRgb,
    warnRgb,
    coldRgb,
  );
  drawTimeGrid(ctx, scene, theme, layout, mapLayout);
  const gaps = drawGapMarkers(ctx, scene, view, theme, mapLayout);
  drawHeatTrails(
    ctx,
    scene,
    view,
    layout,
    mapLayout,
    readRgb,
    writeRgb,
    warnRgb,
  );
  drawThreads(ctx, scene, view, layout, mapLayout);
  const events = drawEventMarkers(
    ctx,
    scene,
    view,
    theme,
    layout,
    mapLayout,
    readRgb,
    writeRgb,
    warnRgb,
  );
  drawVeilAndPlayhead(ctx, scene, view, theme, layout, mapLayout);
  const riders = drawRiders(ctx, scene, view, theme, layout, mapLayout);

  ctx.fillStyle = theme.panelEdge;
  ctx.fillRect(
    mapLayout.treeWidth - 6,
    mapLayout.chartTop,
    1,
    layout.gridBottom - mapLayout.chartTop,
  );

  return {
    events,
    rows,
    gaps,
    riders,
    chart: {
      x0: chartX0,
      x1: chartX1,
      y0: mapLayout.chartTop,
      y1: layout.gridBottom,
    },
  };
}
