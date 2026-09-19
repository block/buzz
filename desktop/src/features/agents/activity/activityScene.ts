// Ported from Berd's agent-activity-panel (branch zmarley/agent-activity-panel).
// Compiles raw activity events + agents into a render-ready scene: file-tree
// rows, per-node heat, agent threads, contention synthesis, and display-time
// mapping.

import type {
  ActivityAgent,
  ActivityEvent,
  ActivityEventKind,
} from "./activityModel";
import { buildTimeMap, type TimeMap } from "./activityTime";
import { basename, binarySearchPrefix, hexToRgb } from "./activityUtils";

export interface ActivityTouch {
  t: number;
  displayT: number;
  kind: ActivityEventKind;
  agentId: string;
  warn: boolean;
  event: ActivitySceneEvent;
}

export interface ActivityNodeHeat {
  hr: number;
  hw: number;
  warn: number;
  cnt: number;
  last: ActivityTouch | null;
}

export interface ActivitySceneNode {
  name: string;
  path: string;
  depth: number;
  isDir: boolean;
  row: number;
  revealAt: number;
  displayRevealAt: number;
  touches: ActivityTouch[];
  prefixTimes: number[];
}

export interface ActivityThreadPoint {
  t: number;
  displayT: number;
  node: ActivitySceneNode;
  event: ActivityEvent | null;
  hold: boolean;
}

export interface ActivityAgentTimeline {
  events: ActivitySceneEvent[];
  times: number[];
  readPrefixCounts: number[];
  writePrefixCounts: number[];
}

export interface ActivitySceneAgent extends ActivityAgent {
  doneT: number;
  displaySpawnT: number;
  displayDoneT: number;
  points: ActivityThreadPoint[];
  pointTimes: number[];
  rgb: Rgb;
  timeline: ActivityAgentTimeline;
}

export interface ActivitySceneEvent extends ActivityEvent {
  id: string;
  displayT: number;
  node?: ActivitySceneNode;
  warn: boolean;
  source?: "contention";
}

export interface ActivityScene {
  events: ActivitySceneEvent[];
  agents: ActivitySceneAgent[];
  agentsById: Map<string, ActivitySceneAgent>;
  primaryAgentId: string;
  nodes: ActivitySceneNode[];
  byPath: Map<string, ActivitySceneNode>;
  t0: number;
  t1: number;
  d0: number;
  d1: number;
  durationMs: number;
  timeMap: TimeMap;
  constants: {
    contentionWindowMs: number;
    transitionMs: number;
  };
  nodeHeat: (node: ActivitySceneNode, t: number) => ActivityNodeHeat;
}

export type Rgb = readonly [number, number, number];

const CONTENTION_WINDOW_MS = 4_000;
const THREAD_TRANSITION_MS = 900;

const READ_DECAY_MS = 6_000;
const WRITE_DECAY_MS = 7_000;
const WARN_DECAY_MS = 5_000;
const DEFAULT_THREAD_PATH = "README.md";

interface SourceTreeNode {
  segment: string;
  path: string;
  isDir: boolean;
  eventTerminal: boolean;
  children: Map<string, SourceTreeNode>;
}

interface DisplayNodeInput {
  name: string;
  path: string;
  depth: number;
  isDir: boolean;
}

function createSourceTreeNode(
  segment: string,
  path: string,
  isDir: boolean,
): SourceTreeNode {
  return {
    segment,
    path,
    isDir,
    eventTerminal: false,
    children: new Map(),
  };
}

function pathSegments(path: string): string[] {
  const segments = path.split("/").filter(Boolean);
  return segments.length > 0 ? segments : [path];
}

function sortedSourceChildren(node: SourceTreeNode): SourceTreeNode[] {
  return Array.from(node.children.values()).sort((a, b) => {
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    return a.segment.localeCompare(b.segment, undefined, {
      sensitivity: "base",
    });
  });
}

function addPathToSourceTree(root: SourceTreeNode, path: string) {
  const segments = pathSegments(path);
  let current = root;

  for (let index = 0; index < segments.length; index += 1) {
    const segment = segments[index];
    const childPath = segments.slice(0, index + 1).join("/");
    const isLeaf = index === segments.length - 1;
    let child = current.children.get(segment);

    if (!child) {
      child = createSourceTreeNode(segment, childPath, !isLeaf);
      current.children.set(segment, child);
    }

    if (!isLeaf) child.isDir = true;
    if (isLeaf) child.eventTerminal = true;
    current = child;
  }
}

function collectDisplayNodeInputs(
  node: SourceTreeNode,
  displayDepth: number,
  inputs: DisplayNodeInput[],
) {
  if (!node.isDir) {
    inputs.push({
      name: basename(node.path),
      path: node.path,
      depth: displayDepth,
      isDir: false,
    });
    return;
  }

  const chain = [node];
  let deepest = node;
  while (!deepest.eventTerminal) {
    const children = sortedSourceChildren(deepest);
    if (children.length !== 1 || !children[0].isDir) break;
    deepest = children[0];
    chain.push(deepest);
  }

  inputs.push({
    name: chain.map((chainNode) => chainNode.segment).join("/"),
    path: deepest.path,
    depth: displayDepth,
    isDir: true,
  });

  for (const child of sortedSourceChildren(deepest)) {
    collectDisplayNodeInputs(child, displayDepth + 1, inputs);
  }
}

function buildNodes(events: readonly ActivityEvent[]): ActivitySceneNode[] {
  const paths = new Set(
    events.flatMap((event) => (event.path ? [event.path] : [])),
  );
  const root = createSourceTreeNode("", "", true);

  for (const path of paths) {
    addPathToSourceTree(root, path);
  }

  const nodeInputs: DisplayNodeInput[] = [];
  for (const child of sortedSourceChildren(root)) {
    collectDisplayNodeInputs(child, 0, nodeInputs);
  }

  return nodeInputs.map((node, row) => ({
    ...node,
    row,
    revealAt: 0,
    displayRevealAt: 0,
    touches: [],
    prefixTimes: [],
  }));
}

type PendingActivitySceneEvent = Omit<ActivitySceneEvent, "id" | "displayT">;

function agentName(
  agentsById: ReadonlyMap<string, ActivityAgent>,
  agentId: string,
): string {
  return agentsById.get(agentId)?.name ?? agentId;
}

function createContentionEvent(
  first: PendingActivitySceneEvent,
  second: PendingActivitySceneEvent,
  agentsById: ReadonlyMap<string, ActivityAgent>,
): PendingActivitySceneEvent | undefined {
  if (!first.path || !second.path || !first.node) return undefined;

  return {
    agentId: second.agentId,
    t: Math.max(first.t, second.t) + 200,
    kind: "x",
    path: first.path,
    label: `write collision: ${basename(first.path)} (${agentName(agentsById, first.agentId)} ↔ ${agentName(agentsById, second.agentId)})`,
    node: first.node,
    warn: true,
    source: "contention",
  };
}

function withContention(
  events: PendingActivitySceneEvent[],
  agentsById: ReadonlyMap<string, ActivityAgent>,
): PendingActivitySceneEvent[] {
  const extra: PendingActivitySceneEvent[] = [];
  const writes = events.filter(
    (event) => event.path && (event.kind === "w" || event.kind === "c"),
  );

  for (let i = 0; i < writes.length; i += 1) {
    for (let j = i + 1; j < writes.length; j += 1) {
      const first = writes[i];
      const second = writes[j];
      if (first.path !== second.path) continue;
      if (first.agentId === second.agentId) continue;
      if (Math.abs(first.t - second.t) > CONTENTION_WINDOW_MS) continue;

      first.warn = true;
      second.warn = true;
      const contentionEvent = createContentionEvent(first, second, agentsById);
      if (contentionEvent) extra.push(contentionEvent);
    }
  }

  return events.concat(extra).sort((a, b) => a.t - b.t);
}

function assignTouches(
  nodes: readonly ActivitySceneNode[],
  events: ActivitySceneEvent[],
) {
  for (const node of nodes) {
    node.touches = [];
  }

  for (const event of events) {
    if (!event.node) continue;
    event.node.touches.push({
      t: event.t,
      displayT: event.displayT,
      kind: event.kind,
      agentId: event.agentId,
      warn: event.warn || event.kind === "x",
      event,
    });
  }

  for (const node of nodes) {
    node.touches.sort((a, b) => a.t - b.t);
    node.prefixTimes = node.touches.map((touch) => touch.displayT);
  }
}

function childNodes(
  node: ActivitySceneNode,
  nodes: readonly ActivitySceneNode[],
): ActivitySceneNode[] {
  return nodes.filter(
    (candidate) =>
      candidate.depth === node.depth + 1 &&
      candidate.path.startsWith(`${node.path}/`),
  );
}

function assignRevealTimes(
  nodes: readonly ActivitySceneNode[],
  sceneStart: number,
  timeMap: TimeMap,
) {
  const displaySceneStart = timeMap.toDisplay(sceneStart);

  for (const node of nodes) {
    if (node.isDir) continue;
    const first = node.touches[0];
    node.revealAt = first?.kind === "c" ? first.t : sceneStart;
    node.displayRevealAt =
      first?.kind === "c" ? first.displayT : displaySceneStart;
  }

  for (let index = nodes.length - 1; index >= 0; index -= 1) {
    const node = nodes[index];
    if (!node.isDir) continue;

    const children = childNodes(node, nodes);
    node.revealAt = children.length
      ? Math.min(...children.map((child) => child.revealAt))
      : sceneStart;
    node.displayRevealAt = children.length
      ? Math.min(...children.map((child) => child.displayRevealAt))
      : displaySceneStart;
  }
}

export function nodeHeat(
  node: ActivitySceneNode,
  displayT: number,
): ActivityNodeHeat {
  const end = binarySearchPrefix(node.prefixTimes, displayT);
  let hr = 0;
  let hw = 0;
  let warn = 0;
  let last: ActivityTouch | null = null;

  for (let index = 0; index < end; index += 1) {
    const touch = node.touches[index];
    last = touch;
    const dt = Math.max(0, displayT - touch.displayT);
    if (touch.kind === "r") {
      hr += Math.exp(-dt / READ_DECAY_MS);
    } else if (touch.kind === "w" || touch.kind === "c" || touch.kind === "x") {
      hw += Math.exp(-dt / WRITE_DECAY_MS);
    }
    if (touch.warn) {
      warn += Math.exp(-dt / WARN_DECAY_MS);
    }
  }

  return { hr, hw, warn, cnt: end, last };
}

function fallbackThreadNode(
  nodes: readonly ActivitySceneNode[],
): ActivitySceneNode | undefined {
  return (
    nodes.find((node) => !node.isDir && node.path === DEFAULT_THREAD_PATH) ??
    nodes.find((node) => !node.isDir) ??
    nodes[0]
  );
}

function nodeForAgentAt(
  agentId: string,
  t: number,
  events: readonly ActivitySceneEvent[],
  fallback: ActivitySceneNode,
): ActivitySceneNode {
  let current = fallback;
  for (const event of events) {
    if (event.t > t) break;
    if (event.agentId === agentId && event.node) current = event.node;
  }
  return current;
}

function buildAgentPoints(
  agent: ActivityAgent,
  events: readonly ActivitySceneEvent[],
  fallback: ActivitySceneNode,
  primaryAgentId: string,
  timeMap: TimeMap,
): ActivityThreadPoint[] {
  const firstAgentWithPath = events.find(
    (event) => event.agentId === agent.id && event.node,
  );
  let current =
    firstAgentWithPath?.node ??
    nodeForAgentAt(primaryAgentId, agent.spawnT, events, fallback);
  const basePoints: ActivityThreadPoint[] = [
    {
      t: agent.spawnT,
      displayT: timeMap.toDisplay(agent.spawnT),
      node: current,
      event: null,
      hold: false,
    },
  ];

  for (const event of events) {
    if (event.agentId !== agent.id) continue;
    if (event.node) current = event.node;
    basePoints.push({
      t: event.t,
      displayT: event.displayT,
      node: current,
      event,
      hold: false,
    });
  }

  const points: ActivityThreadPoint[] = [];
  for (let index = 0; index < basePoints.length; index += 1) {
    const point = basePoints[index];
    const next = basePoints[index + 1];
    points.push(point);
    if (
      next &&
      next.node !== point.node &&
      next.displayT - point.displayT > THREAD_TRANSITION_MS
    ) {
      const displayT = Math.max(
        point.displayT,
        next.displayT - THREAD_TRANSITION_MS,
      );
      points.push({
        t: timeMap.toReal(displayT),
        displayT,
        node: point.node,
        event: null,
        hold: true,
      });
    }
  }

  return points.sort((a, b) => a.displayT - b.displayT || a.t - b.t);
}

function buildAgentTimeline(
  agentId: string,
  events: readonly ActivitySceneEvent[],
): ActivityAgentTimeline {
  const timelineEvents = events.filter((event) => event.agentId === agentId);
  const times = timelineEvents.map((event) => event.displayT);
  const readPrefixCounts = [0];
  const writePrefixCounts = [0];

  for (const event of timelineEvents) {
    const previousReads = readPrefixCounts[readPrefixCounts.length - 1] ?? 0;
    const previousWrites = writePrefixCounts[writePrefixCounts.length - 1] ?? 0;
    readPrefixCounts.push(previousReads + (event.kind === "r" ? 1 : 0));
    writePrefixCounts.push(
      previousWrites + (event.kind === "w" || event.kind === "c" ? 1 : 0),
    );
  }

  return { events: timelineEvents, times, readPrefixCounts, writePrefixCounts };
}

function buildSceneAgents(
  agents: readonly ActivityAgent[],
  events: readonly ActivitySceneEvent[],
  nodes: readonly ActivitySceneNode[],
  primaryAgentId: string,
  timeMap: TimeMap,
): ActivitySceneAgent[] {
  const fallback = fallbackThreadNode(nodes);
  if (!fallback) return [];

  return agents.map((agent) => {
    const doneEvent = events.find(
      (event) => event.agentId === agent.id && event.kind === "d",
    );
    const doneT = agent.doneT ?? doneEvent?.t ?? Number.POSITIVE_INFINITY;
    return {
      ...agent,
      doneT,
      displaySpawnT: timeMap.toDisplay(agent.spawnT),
      displayDoneT: Number.isFinite(doneT)
        ? timeMap.toDisplay(doneT)
        : Number.POSITIVE_INFINITY,
      points: buildAgentPoints(
        agent,
        events,
        fallback,
        primaryAgentId,
        timeMap,
      ),
      pointTimes: [],
      rgb: hexToRgb(agent.color),
      timeline: buildAgentTimeline(agent.id, events),
    };
  });
}

export function compileScene(
  inputEvents: readonly ActivityEvent[],
  inputAgents: readonly ActivityAgent[],
): ActivityScene {
  const nodes = buildNodes(inputEvents);
  const byPath = new Map(nodes.map((node) => [node.path, node]));
  const primaryAgentId = inputAgents[0]?.id ?? inputEvents[0]?.agentId ?? "";
  const inputAgentsById = new Map(
    inputAgents.map((agent) => [agent.id, agent]),
  );
  const sceneStartCandidates = [
    ...inputEvents.map((event) => event.t),
    ...inputAgents.map((agent) => agent.spawnT),
  ];
  const sceneStart = sceneStartCandidates.length
    ? Math.min(...sceneStartCandidates)
    : 0;

  const events = inputEvents
    .map<PendingActivitySceneEvent>((event) => ({
      ...event,
      ...(event.path ? { node: byPath.get(event.path) } : {}),
      warn: event.kind === "x",
    }))
    .sort((a, b) => a.t - b.t);

  const pendingContendedEvents = withContention(events, inputAgentsById);
  const timeMap = buildTimeMap(pendingContendedEvents.map((event) => event.t));
  const contendedEvents: ActivitySceneEvent[] = pendingContendedEvents.map(
    (event, index) => ({
      ...event,
      id: `event-${index}`,
      displayT: timeMap.toDisplay(event.t),
    }),
  );
  assignTouches(nodes, contendedEvents);
  assignRevealTimes(nodes, sceneStart, timeMap);

  const agents = buildSceneAgents(
    inputAgents,
    contendedEvents,
    nodes,
    primaryAgentId,
    timeMap,
  ).map((agent) => ({
    ...agent,
    pointTimes: agent.points.map((point) => point.displayT),
  }));
  const agentsById = new Map(agents.map((agent) => [agent.id, agent]));
  const maxEventT = contendedEvents.at(-1)?.t ?? sceneStart;
  const maxDoneT = Math.max(
    sceneStart,
    ...inputAgents.flatMap((agent) =>
      agent.doneT !== undefined ? [agent.doneT] : [],
    ),
  );
  const t0 = sceneStart;
  const t1 = Math.max(maxEventT, maxDoneT, t0);

  return {
    events: contendedEvents,
    agents,
    agentsById,
    primaryAgentId,
    nodes,
    byPath,
    t0,
    t1,
    d0: 0,
    d1: timeMap.displayDuration,
    durationMs: Math.max(1, t1 - t0),
    timeMap,
    constants: {
      contentionWindowMs: CONTENTION_WINDOW_MS,
      transitionMs: THREAD_TRANSITION_MS,
    },
    nodeHeat,
  };
}
