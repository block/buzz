"use client";

import { useEffect, useId, useRef, useState } from "react";
import type { CSSProperties, MutableRefObject, PointerEvent as ReactPointerEvent, ReactNode } from "react";
import {
  ArrowUp,
  Archive,
  AtSign,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  CircleCheck,
  ClipboardList,
  GitBranch,
  GitFork,
  FileText,
  Folder,
  Hash,
  Headphones,
  House,
  LockKeyhole,
  MessageSquareText,
  MoreHorizontal,
  MoreVertical,
  Paperclip,
  PanelRight,
  PanelRightOpen,
  Pin,
  Plus,
  Search,
  SlidersHorizontal,
  SmilePlus,
  SquareDashed,
  Trash2,
  Users,
  X,
} from "lucide-react";
import { ProjectArtifactRenderer } from "./berd-project-cube/ProjectArtifactRenderer";
import type {
  ProjectArtifactProjection,
  ProjectArtifactState,
} from "./berd-project-cube/types";

const TOPBAR_HEIGHT = 48;
const TOPBAR_CONTENT_OFFSET = TOPBAR_HEIGHT + 8;

type SegmentedNavigationItem<T extends string> = {
  value: T;
  label: string;
  badge?: number;
  icon?: ReactNode;
};

function SegmentedNavigation<T extends string>({
  activeValue,
  ariaLabel,
  items,
  itemWidth = 88,
  onChange,
  trailing,
  variant = "segmented",
}: {
  activeValue: T;
  ariaLabel: string;
  items: SegmentedNavigationItem<T>[];
  itemWidth?: number;
  onChange: (value: T) => void;
  trailing?: ReactNode;
  variant?: "segmented" | "tabs";
}) {
  const activeIndex = Math.max(0, items.findIndex((item) => item.value === activeValue));
  const style = {
    "--segmented-item-width": `${itemWidth}px`,
    "--segmented-active-offset": `${activeIndex * (itemWidth + 2)}px`,
  } as CSSProperties;

  return (
    <nav className={`segmented${variant === "tabs" ? " primary-tabs" : ""}`} aria-label={ariaLabel} style={style}>
      <span className="segmented-slider" aria-hidden="true" />
      {items.map((item) => (
        <button key={item.value} type="button" className={activeValue === item.value ? "selected" : ""} onClick={() => onChange(item.value)}>
          {item.icon}
          {item.label}
          {item.badge !== undefined ? <span className="segmented-badge">{item.badge}</span> : null}
        </button>
      ))}
      {trailing}
    </nav>
  );
}

const PRIMARY_PROJECT_STATE: ProjectArtifactState = {
  seed: 381,
  name: "Buzz navigation",
  accentColor: "#b8d9ca",
  accentCssColor: "#b8d9ca",
  mood: "active",
  moodIntensity: 0.72,
  contentMode: "cube",
};

const SECONDARY_PROJECT_STATE: ProjectArtifactState = {
  seed: 734,
  name: "Agent workspace",
  accentColor: "#d8ca94",
  accentCssColor: "#d8ca94",
  mood: "energetic",
  moodIntensity: 0.68,
  contentMode: "cube",
};

const TERTIARY_PROJECT_STATE: ProjectArtifactState = {
  seed: 912,
  name: "Blue sky prototype",
  accentColor: "#27a9eb",
  accentCssColor: "#27a9eb",
  mood: "active",
  moodIntensity: 0.76,
  contentMode: "cube",
};

const RELAY_PROJECT_STATE: ProjectArtifactState = {
  seed: 144,
  name: "Relay handoffs",
  accentColor: "#e38b6e",
  accentCssColor: "#e38b6e",
  mood: "energetic",
  moodIntensity: 0.7,
  contentMode: "cube",
};

const ATLAS_PROJECT_STATE: ProjectArtifactState = {
  seed: 626,
  name: "Atlas research",
  accentColor: "#9d8bea",
  accentCssColor: "#9d8bea",
  mood: "active",
  moodIntensity: 0.66,
  contentMode: "cube",
};

const LANTERN_PROJECT_STATE: ProjectArtifactState = {
  seed: 508,
  name: "Lantern launch",
  accentColor: "#78b98c",
  accentCssColor: "#78b98c",
  mood: "active",
  moodIntensity: 0.74,
  contentMode: "cube",
};

const PRIMARY_IMAGE_URLS = ["/berd-project-assets/memory-03.webp"];
const SECONDARY_IMAGE_URLS = ["/berd-project-assets/memory-17.webp"];
const TERTIARY_IMAGE_URLS = ["/berd-project-assets/memory-35.webp"];
const USER_PROFILES = {
  caroline: "/user-profiles/caroline.png",
  cynthia: "/user-profiles/cynthia.png",
  morgan: "/user-profiles/morgan.png",
  arj: "/user-profiles/arj.png",
  simon: "/user-profiles/simon.png",
} as const;

type TaskNoodle = {
  avatar: string;
  label: string;
  id: string;
  context: string;
  created: string;
  assignee: string;
};
type ProjectTasks = readonly [TaskNoodle, TaskNoodle, TaskNoodle];

type SelectedTask = {
  projectKey: string;
  projectName: string;
  accentColor: string;
  task: TaskNoodle;
  taskIndex: number;
};

const COMMON_TASK_DETAILS = {
  created: "Aug 26, 2026",
  assignee: "Cynthia Chen",
};

const PRIMARY_TASKS: ProjectTasks = [
  { avatar: "/berd-agent-avatars/pushback-gloopies-5.png", label: "GIF support", id: "BUZZ-1892", context: "Add dependable animated GIF playback and previews throughout project conversations, including clear loading and failure states.", ...COMMON_TASK_DETAILS },
  { avatar: "/berd-agent-avatars/builderbot-gloopies-20.png", label: "Nav polish", id: "BUZZ-1904", context: "Refine the primary navigation spacing, selected states, and responsive behavior so it stays quiet and legible.", ...COMMON_TASK_DETAILS },
  { avatar: "/berd-agent-avatars/berdy-gloopies-22.png", label: "Review PR-381", id: "BUZZ-381", context: "Review the project workspace changes, check the interaction details, and leave actionable feedback before merge.", ...COMMON_TASK_DETAILS },
];

const SECONDARY_TASKS: ProjectTasks = [
  { avatar: "/berd-agent-avatars/berdy-gloopies-22.png", label: "Agent CLI", id: "AGENT-214", context: "Prototype a compact command-line workflow for launching and inspecting workspace agents.", ...COMMON_TASK_DETAILS },
  { avatar: "/berd-agent-avatars/pushback-gloopies-5.png", label: "Team astrology", id: "AGENT-227", context: "Explore a playful team activity that turns project signals into a lightweight weekly reading.", ...COMMON_TASK_DETAILS },
  { avatar: "/berd-agent-avatars/builderbot-gloopies-20.png", label: "Run QA", id: "AGENT-231", context: "Exercise the key workspace flows and record any interaction or rendering regressions.", ...COMMON_TASK_DETAILS },
];

const TERTIARY_TASKS: ProjectTasks = [
  { avatar: "/berd-agent-avatars/builderbot-gloopies-20.png", label: "Prototype CLI", id: "BLUE-104", context: "Build the smallest useful prototype for the new project command-line experience.", ...COMMON_TASK_DETAILS },
  { avatar: "/berd-agent-avatars/berdy-gloopies-22.png", label: "Motion study", id: "BLUE-118", context: "Tune the hover, focus, and transition motion so the project objects feel physical but controlled.", ...COMMON_TASK_DETAILS },
  { avatar: "/berd-agent-avatars/pushback-gloopies-5.png", label: "Ship demo", id: "BLUE-125", context: "Prepare the prototype for a focused internal demonstration and collect follow-up questions.", ...COMMON_TASK_DETAILS },
];

const RELAY_TASKS: readonly TaskNoodle[] = [
  { avatar: "/berd-agent-avatars/pushback-gloopies-5.png", label: "Handoff timeline", id: "RELAY-42", context: "Make agent-to-agent handoffs readable as a single chronological thread with clear ownership changes.", ...COMMON_TASK_DETAILS },
  { avatar: "/berd-agent-avatars/berdy-gloopies-22.png", label: "Retry states", id: "RELAY-57", context: "Design recoverable retry states for interrupted handoffs without duplicating completed work.", ...COMMON_TASK_DETAILS },
  { avatar: "/berd-agent-avatars/builderbot-gloopies-20.png", label: "Connection audit", id: "RELAY-63", context: "Audit connection health signals and surface the smallest useful status summary for each relay.", ...COMMON_TASK_DETAILS },
  { avatar: "/berd-agent-avatars/pushback-gloopies-5.png", label: "Latency budget", id: "RELAY-71", context: "Set a practical latency budget for each handoff stage and identify the slowest transitions.", ...COMMON_TASK_DETAILS },
  { avatar: "/berd-agent-avatars/berdy-gloopies-22.png", label: "Owner fallback", id: "RELAY-79", context: "Define who takes over when the assigned agent becomes unavailable during an active relay.", ...COMMON_TASK_DETAILS },
];

const ATLAS_TASKS: readonly TaskNoodle[] = [
  { avatar: "/berd-agent-avatars/builderbot-gloopies-20.png", label: "Source map", id: "ATLAS-88", context: "Map the research sources behind project decisions and show where each conclusion originated.", ...COMMON_TASK_DETAILS },
  { avatar: "/berd-agent-avatars/pushback-gloopies-5.png", label: "Insight clusters", id: "ATLAS-96", context: "Group related findings into concise themes while preserving links back to the original evidence.", ...COMMON_TASK_DETAILS },
  { avatar: "/berd-agent-avatars/berdy-gloopies-22.png", label: "Weekly digest", id: "ATLAS-103", context: "Generate a lightweight weekly digest of new findings, open questions, and changed assumptions.", ...COMMON_TASK_DETAILS },
  { avatar: "/berd-agent-avatars/builderbot-gloopies-20.png", label: "Archive sources", id: "ATLAS-111", context: "Archive stale references while retaining a clear trail for decisions that still depend on them.", ...COMMON_TASK_DETAILS },
];

const LANTERN_TASKS: readonly TaskNoodle[] = [
  { avatar: "/berd-agent-avatars/berdy-gloopies-22.png", label: "Launch checklist", id: "LANTERN-12", context: "Turn the launch plan into a shared checklist with owners, dependencies, and confidence signals.", ...COMMON_TASK_DETAILS },
  { avatar: "/berd-agent-avatars/builderbot-gloopies-20.png", label: "Audience notes", id: "LANTERN-19", context: "Collect audience-specific talking points and keep the latest approved narrative easy to find.", ...COMMON_TASK_DETAILS },
];

const GOOSE_PROJECT_TASKS: readonly TaskNoodle[] = [
  ...SECONDARY_TASKS,
  { avatar: "/berd-agent-avatars/pushback-gloopies-5.png", label: "Prompt presets", id: "AGENT-244", context: "Create reusable prompt presets for the most common workspace agent workflows.", ...COMMON_TASK_DETAILS },
];

const BUZZ_PROJECT_TASKS: readonly TaskNoodle[] = TERTIARY_TASKS.slice(0, 2);

type ProjectDefinition = {
  key: string;
  name: string;
  state: ProjectArtifactState;
  imageUrls: string[];
  tasks: readonly TaskNoodle[];
};

type ProjectWorkspaceTab = "overview" | "tasks" | "activity";
type ProjectUtility = "remote" | "repository" | "terminal" | "files" | "contributors";

const PROJECT_WORKSPACE_DETAILS: Record<string, {
  description: string;
  milestone: string;
  channel: string;
  progress: number;
  blockers: number;
}> = {
  berd: { description: "A composable workspace for projects, messages, agents, and the work between them.", milestone: "Berd Strategy v2", channel: "#buzz-interface-squad", progress: 68, blockers: 2 },
  goose: { description: "A focused environment for creating, coordinating, and evaluating a new generation of workspace agents.", milestone: "Agent workspace beta", channel: "#agent-experience", progress: 54, blockers: 3 },
  buzz: { description: "Explorations for making project work spatial, expressive, and easier to move through at a glance.", milestone: "Prototype review", channel: "#blue-sky", progress: 81, blockers: 1 },
  relay: { description: "Reliable handoffs between people and agents with clear ownership, history, and recovery paths.", milestone: "Handoff pilot", channel: "#relay", progress: 46, blockers: 4 },
  atlas: { description: "A living map of research evidence, decisions, open questions, and the assumptions behind them.", milestone: "Research digest", channel: "#atlas-research", progress: 72, blockers: 1 },
  lantern: { description: "A shared launch room that keeps owners, dependencies, narrative, and readiness in one place.", milestone: "Launch readiness", channel: "#lantern-launch", progress: 62, blockers: 2 },
};

const PROJECTS: readonly ProjectDefinition[] = [
  { key: "berd", name: "Berd", state: PRIMARY_PROJECT_STATE, imageUrls: PRIMARY_IMAGE_URLS, tasks: PRIMARY_TASKS },
  { key: "goose", name: "Goose", state: SECONDARY_PROJECT_STATE, imageUrls: SECONDARY_IMAGE_URLS, tasks: GOOSE_PROJECT_TASKS },
  { key: "buzz", name: "Buzz", state: TERTIARY_PROJECT_STATE, imageUrls: TERTIARY_IMAGE_URLS, tasks: BUZZ_PROJECT_TASKS },
  { key: "relay", name: "Relay", state: RELAY_PROJECT_STATE, imageUrls: SECONDARY_IMAGE_URLS, tasks: RELAY_TASKS },
  { key: "atlas", name: "Atlas", state: ATLAS_PROJECT_STATE, imageUrls: TERTIARY_IMAGE_URLS, tasks: ATLAS_TASKS },
  { key: "lantern", name: "Lantern", state: LANTERN_PROJECT_STATE, imageUrls: PRIMARY_IMAGE_URLS, tasks: LANTERN_TASKS },
];

type LabelMotion = {
  projection: ProjectArtifactProjection;
  reduceMotion: boolean;
};

type NoodlePoint = { x: number; y: number };
const NOODLE_STROKE_WIDTH = 60;
const MIN_NOODLE_PADDING = 1;
const NOODLE_CUBE_GAP = 26;
const NOODLE_TEXT_OFFSET = 62;
const NOODLE_TRAILING_PADDING = 4;

function signedArea(points: NoodlePoint[]) {
  return points.reduce((area, point, index) => {
    const next = points[(index + 1) % points.length];
    return area + point.x * next.y - next.x * point.y;
  }, 0) / 2;
}

function resampleClosedContour(points: NoodlePoint[], count: number) {
  if (points.length < 2) return points;
  const ordered = signedArea(points) >= 0 ? points : [...points].reverse();
  const lengths = [0];
  for (let index = 0; index < ordered.length; index += 1) {
    const point = ordered[index];
    const next = ordered[(index + 1) % ordered.length];
    lengths.push(lengths[lengths.length - 1] + Math.hypot(next.x - point.x, next.y - point.y));
  }
  const perimeter = lengths[lengths.length - 1];
  const samples: NoodlePoint[] = [];
  let edge = 0;
  for (let sample = 0; sample < count; sample += 1) {
    const distance = (sample / count) * perimeter;
    while (edge < ordered.length - 1 && lengths[edge + 1] < distance) edge += 1;
    const start = ordered[edge];
    const end = ordered[(edge + 1) % ordered.length];
    const edgeLength = lengths[edge + 1] - lengths[edge] || 1;
    const progress = (distance - lengths[edge]) / edgeLength;
    samples.push({
      x: start.x + (end.x - start.x) * progress,
      y: start.y + (end.y - start.y) * progress,
    });
  }

  const minY = Math.min(...samples.map((point) => point.y));
  const minX = Math.min(...samples.map((point) => point.x));
  const maxX = Math.max(...samples.map((point) => point.x));
  const topCenter = { x: (minX + maxX) / 2, y: minY };
  const startIndex = samples.reduce((best, point, index) =>
    Math.hypot(point.x - topCenter.x, point.y - topCenter.y) <
    Math.hypot(samples[best].x - topCenter.x, samples[best].y - topCenter.y)
      ? index
      : best,
  0);
  return samples.slice(startIndex).concat(samples.slice(0, startIndex));
}

function offsetContour(points: NoodlePoint[], distance: number) {
  const clockwise = signedArea(points) >= 0;
  const edgeNormal = (from: NoodlePoint, to: NoodlePoint) => {
    const dx = to.x - from.x;
    const dy = to.y - from.y;
    const length = Math.hypot(dx, dy) || 1;
    return clockwise
      ? { x: dy / length, y: -dx / length }
      : { x: -dy / length, y: dx / length };
  };
  return points.map((point, index) => {
    const previous = points[(index - 1 + points.length) % points.length];
    const next = points[(index + 1) % points.length];
    const beforeNormal = edgeNormal(previous, point);
    const afterNormal = edgeNormal(point, next);
    const sumX = beforeNormal.x + afterNormal.x;
    const sumY = beforeNormal.y + afterNormal.y;
    const sumLength = Math.hypot(sumX, sumY) || 1;
    const normal = { x: sumX / sumLength, y: sumY / sumLength };
    const projection = Math.max(
      0.67,
      normal.x * afterNormal.x + normal.y * afterNormal.y,
    );
    const miter = Math.min(distance * 1.45, distance / projection);
    return { x: point.x + normal.x * miter, y: point.y + normal.y * miter };
  });
}

function contourRangeByLength(
  points: NoodlePoint[],
  start: number,
  targetLength: number,
  direction: 1 | -1,
) {
  const startIndex = Math.round(start * points.length) % points.length;
  const range = [points[startIndex]];
  let index = startIndex;
  let traversed = 0;
  while (range.length <= points.length && traversed < targetLength) {
    const nextIndex = (index + direction + points.length) % points.length;
    const point = points[index];
    const next = points[nextIndex];
    const segmentLength = Math.hypot(next.x - point.x, next.y - point.y) || 1;
    if (traversed + segmentLength >= targetLength) {
      const progress = (targetLength - traversed) / segmentLength;
      range.push({
        x: point.x + (next.x - point.x) * progress,
        y: point.y + (next.y - point.y) * progress,
      });
      break;
    }
    traversed += segmentLength;
    index = nextIndex;
    range.push(next);
  }
  return range;
}

function closedContourLength(points: NoodlePoint[]) {
  return points.reduce((length, point, index) => {
    const next = points[(index + 1) % points.length];
    return length + Math.hypot(next.x - point.x, next.y - point.y);
  }, 0);
}

function smoothPath(points: NoodlePoint[]) {
  if (points.length < 2) return "";
  const commands = [`M ${points[0].x} ${points[0].y}`];
  for (let index = 0; index < points.length - 1; index += 1) {
    const before = points[Math.max(0, index - 1)];
    const current = points[index];
    const next = points[index + 1];
    const after = points[Math.min(points.length - 1, index + 2)];
    const controlOne = {
      x: current.x + (next.x - before.x) / 6,
      y: current.y + (next.y - before.y) / 6,
    };
    const controlTwo = {
      x: next.x - (after.x - current.x) / 6,
      y: next.y - (after.y - current.y) / 6,
    };
    commands.push(
      `C ${controlOne.x} ${controlOne.y}, ${controlTwo.x} ${controlTwo.y}, ${next.x} ${next.y}`,
    );
  }
  return commands.join(" ");
}

function WrappedTaskLabels({
  motion,
  tasks,
  accentColor,
  selectedIndex,
  onSelect,
}: {
  motion: MutableRefObject<LabelMotion>;
  tasks: ProjectTasks;
  accentColor: string;
  selectedIndex: number | null;
  onSelect: (taskIndex: number) => void;
}) {
  const pathId = useId().replace(/:/g, "");
  const supportPathId = `${pathId}-support`;
  const navPathId = `${pathId}-nav`;
  const reviewPathId = `${pathId}-review`;
  const supportPathRef = useRef<SVGPathElement>(null);
  const navPathRef = useRef<SVGPathElement>(null);
  const reviewPathRef = useRef<SVGPathElement>(null);
  const supportAvatarRef = useRef<SVGImageElement>(null);
  const navAvatarRef = useRef<SVGImageElement>(null);
  const reviewAvatarRef = useRef<SVGImageElement>(null);
  const supportTextRef = useRef<SVGTextElement>(null);
  const navTextRef = useRef<SVGTextElement>(null);
  const reviewTextRef = useRef<SVGTextElement>(null);
  const smoothedTraceRef = useRef<NoodlePoint[] | null>(null);
  const liveTraceRef = useRef<NoodlePoint[]>([]);
  const taskStartOverridesRef = useRef<Array<number | null>>([null, null, null]);
  const taskCurrentStartsRef = useRef([0.82, 0, 0.82]);
  const dragRef = useRef<{
    index: number;
    startX: number;
    startY: number;
    grabOffset: number;
    moved: boolean;
  } | null>(null);
  const [draggingIndex, setDraggingIndex] = useState<number | null>(null);

  useEffect(() => {
    let frame = 0;
    const update = () => {
      const projection = motion.current.projection;
      const left = 55 + projection.x;
      const top = 35 + projection.y;
      const right = left + projection.width;
      const bottom = top + projection.height;
      const width = projection.width;
      const height = projection.height;
      const contour = projection.contour?.map((point) => ({
        x: point.x + 55,
        y: point.y + 35,
      }));

      if (contour && contour.length >= 4) {
        const targetTrace = offsetContour(
          resampleClosedContour(contour, 160),
          NOODLE_STROKE_WIDTH / 2 + NOODLE_CUBE_GAP,
        );
        if (!smoothedTraceRef.current || smoothedTraceRef.current.length !== targetTrace.length) {
          smoothedTraceRef.current = targetTrace.map((point) => ({ ...point }));
        } else {
          const blend = motion.current.reduceMotion ? 1 : 0.52;
          smoothedTraceRef.current.forEach((point, index) => {
            point.x += (targetTrace[index].x - point.x) * blend;
            point.y += (targetTrace[index].y - point.y) * blend;
          });
        }
        const trace = smoothedTraceRef.current;
        liveTraceRef.current = trace;
        const perimeter = closedContourLength(trace);
        // Rounded stroke caps each consume half the stroke width. Keep a small
        // sampling cushion while allowing the task noodles to sit more tightly.
        const minimumGapFraction =
          (NOODLE_STROKE_WIDTH + MIN_NOODLE_PADDING) / perimeter;
        const topGap = Math.max(0.02, minimumGapFraction);
        // The two left-side noodles meet near a broad projected corner, so the
        // same perimeter distance reads much larger than the top seam.
        const leftGap = Math.max(0.012, minimumGapFraction * 0.58);
        const defaultSupportStart = 0.82;
        const supportStart = taskStartOverridesRef.current[0] ?? defaultSupportStart;
        const navStart = taskStartOverridesRef.current[1] ?? topGap / 2;
        const reviewStart =
          taskStartOverridesRef.current[2] ?? defaultSupportStart - leftGap;
        taskCurrentStartsRef.current = [supportStart, navStart, reviewStart];
        const noodleLength = (text: SVGTextElement | null, fallback: number) =>
          NOODLE_TEXT_OFFSET +
          (text?.getComputedTextLength() || fallback) +
          NOODLE_TRAILING_PADDING;

        supportPathRef.current?.setAttribute(
          "d",
          smoothPath(
            contourRangeByLength(
              trace,
              supportStart,
              noodleLength(supportTextRef.current, 118),
              1,
            ),
          ),
        );
        navPathRef.current?.setAttribute(
          "d",
          smoothPath(
            contourRangeByLength(
              trace,
              navStart,
              noodleLength(navTextRef.current, 112),
              1,
            ),
          ),
        );
        reviewPathRef.current?.setAttribute(
          "d",
          smoothPath(
            contourRangeByLength(
              trace,
              reviewStart,
              noodleLength(reviewTextRef.current, 174),
              -1,
            ),
          ),
        );
      } else {
        const margin = Math.max(28, Math.min(width, height) * 0.085);
        const topY = top - margin;
        const leftX = left - margin;
        const rightX = right + margin;
        const bottomY = bottom + margin;
        supportPathRef.current?.setAttribute(
          "d",
          `M ${left + width * 0.43} ${topY} C ${left + width * 0.1} ${topY}, ${leftX} ${top + height * 0.08}, ${leftX} ${top + height * 0.48}`,
        );
        navPathRef.current?.setAttribute(
          "d",
          `M ${left + width * 0.55} ${topY} C ${right - width * 0.08} ${topY}, ${rightX} ${top + height * 0.04}, ${rightX} ${top + height * 0.4}`,
        );
        reviewPathRef.current?.setAttribute(
          "d",
          `M ${leftX} ${top + height * 0.56} C ${leftX} ${bottom - height * 0.08}, ${left + width * 0.06} ${bottomY}, ${left + width * 0.58} ${bottomY}`,
        );
      }

      const placeAvatar = (
        path: SVGPathElement | null,
        avatar: SVGImageElement | null,
        distanceFromStart: number,
      ) => {
        if (!path || !avatar) return;
        const length = path.getTotalLength();
        const point = path.getPointAtLength(
          Math.min(length, distanceFromStart),
        );
        const nextPoint = path.getPointAtLength(
          Math.min(length, distanceFromStart + 2),
        );
        const angle =
          (Math.atan2(nextPoint.y - point.y, nextPoint.x - point.x) * 180) /
          Math.PI;
        const size = 46;
        avatar.setAttribute("x", String(point.x - size / 2));
        avatar.setAttribute("y", String(point.y - size / 2));
        avatar.setAttribute("width", String(size));
        avatar.setAttribute("height", String(size));
        avatar.setAttribute(
          "transform",
          `rotate(${angle} ${point.x} ${point.y})`,
        );
      };

      placeAvatar(supportPathRef.current, supportAvatarRef.current, 22);
      placeAvatar(navPathRef.current, navAvatarRef.current, 22);
      placeAvatar(reviewPathRef.current, reviewAvatarRef.current, 22);
      frame = requestAnimationFrame(update);
    };
    frame = requestAnimationFrame(update);
    return () => cancelAnimationFrame(frame);
  }, [motion]);

  const pointerTraceFraction = (event: ReactPointerEvent<SVGGElement>) => {
    const svg = event.currentTarget.ownerSVGElement;
    const trace = liveTraceRef.current;
    if (!svg || trace.length === 0) return null;
    const rect = svg.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return null;
    const x = ((event.clientX - rect.left) / rect.width) * 760;
    const y = ((event.clientY - rect.top) / rect.height) * 720;
    let nearestIndex = 0;
    let nearestDistance = Number.POSITIVE_INFINITY;
    trace.forEach((point, index) => {
      const distance = (point.x - x) ** 2 + (point.y - y) ** 2;
      if (distance < nearestDistance) {
        nearestDistance = distance;
        nearestIndex = index;
      }
    });
    return nearestIndex / trace.length;
  };

  const beginTaskDrag = (
    event: ReactPointerEvent<SVGGElement>,
    taskIndex: number,
  ) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    const pointerFraction = pointerTraceFraction(event);
    const currentStart = taskCurrentStartsRef.current[taskIndex];
    const grabOffset =
      pointerFraction === null
        ? 0
        : ((currentStart - pointerFraction + 1.5) % 1) - 0.5;
    dragRef.current = {
      index: taskIndex,
      startX: event.clientX,
      startY: event.clientY,
      grabOffset,
      moved: false,
    };
    setDraggingIndex(taskIndex);
  };

  const updateTaskDrag = (
    event: ReactPointerEvent<SVGGElement>,
    taskIndex: number,
  ) => {
    const drag = dragRef.current;
    if (!drag || drag.index !== taskIndex) return;
    if (Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) > 4) {
      drag.moved = true;
    }
    if (drag.moved) {
      const pointerFraction = pointerTraceFraction(event);
      if (pointerFraction !== null) {
        taskStartOverridesRef.current[taskIndex] =
          (pointerFraction + drag.grabOffset + 1) % 1;
      }
    }
  };

  const endTaskDrag = (
    event: ReactPointerEvent<SVGGElement>,
    taskIndex: number,
  ) => {
    const drag = dragRef.current;
    if (!drag || drag.index !== taskIndex) return;
    if (event.currentTarget instanceof Element && event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    dragRef.current = null;
    setDraggingIndex(null);
    if (!drag.moved) onSelect(taskIndex);
  };

  return (
    <svg className="wrapped-labels" viewBox="0 0 760 720" role="group" aria-label="Tasks wrapped around the project cube">
        <defs>
          <path ref={supportPathRef} id={supportPathId} d="M 270 119 C 148 108, 91 184, 79 310" />
          <path ref={navPathRef} id={navPathId} d="M 354 111 C 500 91, 604 132, 608 251" />
          <path ref={reviewPathRef} id={reviewPathId} d="M 89 440 C 96 575, 191 646, 352 641" />
        </defs>
        {[
          { task: tasks[0], pathId: supportPathId, avatarRef: supportAvatarRef, textRef: supportTextRef, className: "support-ribbon" },
          { task: tasks[1], pathId: navPathId, avatarRef: navAvatarRef, textRef: navTextRef, className: "nav-ribbon" },
          { task: tasks[2], pathId: reviewPathId, avatarRef: reviewAvatarRef, textRef: reviewTextRef, className: "review-ribbon" },
        ].map(({ task, pathId: taskPathId, avatarRef, textRef, className }, index) => (
          <g
            key={task.id}
            className={`label-ribbon ${className}${selectedIndex === index ? " is-selected" : ""}${draggingIndex === index ? " is-dragging" : ""}`}
            style={{ "--task-accent": accentColor } as CSSProperties}
            role="button"
            tabIndex={0}
            aria-label={`Open ${task.label}`}
            aria-pressed={selectedIndex === index}
            onPointerDown={(event) => beginTaskDrag(event, index)}
            onPointerMove={(event) => updateTaskDrag(event, index)}
            onPointerUp={(event) => endTaskDrag(event, index)}
            onPointerCancel={(event) => endTaskDrag(event, index)}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                onSelect(index);
              }
            }}
          >
            <use href={`#${taskPathId}`} className="noodle-hit-area" />
            <use href={`#${taskPathId}`} className="ribbon-stroke" />
            <image ref={avatarRef} className="noodle-avatar" href={task.avatar} preserveAspectRatio="xMidYMid meet" />
            <text ref={textRef} className="ribbon-text"><textPath href={`#${taskPathId}`} startOffset={NOODLE_TEXT_OFFSET}>{task.label}</textPath></text>
          </g>
        ))}
    </svg>
  );
}

function ProjectOrbit({
  projectKey,
  displayName,
  className,
  imageUrls,
  state,
  tasks,
  selectedTask,
  onSelectTask,
}: {
  projectKey: string;
  displayName: string;
  className: string;
  imageUrls: string[];
  state: ProjectArtifactState;
  tasks: ProjectTasks;
  selectedTask: SelectedTask | null;
  onSelectTask: (selection: SelectedTask) => void;
}) {
  const motion = useRef<LabelMotion>({
    projection: { x: 145, y: 135, width: 280, height: 300 },
    reduceMotion: false,
  });
  const projectNamePillRef = useRef<HTMLDivElement>(null);
  const [cubeHovered, setCubeHovered] = useState(false);
  const isSelectedProject = selectedTask?.projectKey === projectKey;
  const isDimmed = Boolean(selectedTask && !isSelectedProject);

  useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => { motion.current.reduceMotion = query.matches; };
    update();
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);

  return (
    <div
      className={`orbit-system ${className}${isSelectedProject ? " is-selected-project" : ""}${isDimmed ? " is-dimmed" : ""}`}
      aria-label={`${state.name} project`}
    >
      <WrappedTaskLabels
        motion={motion}
        tasks={tasks}
        accentColor={state.accentCssColor}
        selectedIndex={isSelectedProject ? selectedTask.taskIndex : null}
        onSelect={(taskIndex) => {
          onSelectTask({
            projectKey,
            projectName: displayName,
            accentColor: state.accentCssColor,
            task: tasks[taskIndex],
            taskIndex,
          });
        }}
      />
      <div
        className="canvas-wrap"
        onPointerMove={(event) => {
          const rect = event.currentTarget.getBoundingClientRect();
          const projection = motion.current.projection;
          const x = ((event.clientX - rect.left) / rect.width) * 650;
          const y = ((event.clientY - rect.top) / rect.height) * 650;
          setCubeHovered(
            x >= projection.x &&
            x <= projection.x + projection.width &&
            y >= projection.y &&
            y <= projection.y + projection.height,
          );
        }}
        onPointerLeave={() => setCubeHovered(false)}
      >
        <ProjectArtifactRenderer
          state={state}
          imageUrls={imageUrls}
          environmentUrl="/berd-project-assets/studio_soft.exr"
          variant="tile"
          cameraDistanceScale={1.35}
          focusSide={isSelectedProject}
          onCubeProjection={(projection) => {
            motion.current.projection = projection;
            if (projectNamePillRef.current) {
              projectNamePillRef.current.style.left = `${55 + projection.x + projection.width / 2}px`;
              projectNamePillRef.current.style.top = `${35 + projection.y + projection.height - 42}px`;
            }
          }}
        />
      </div>
      <div
        ref={projectNamePillRef}
        className={`project-name-pill${cubeHovered ? " is-visible" : ""}`}
        aria-hidden="true"
      >
        {displayName}
      </div>
    </div>
  );
}

const PROJECT_TASK_ASSIGNEES = [
  { name: "Cynthia Chen", avatar: USER_PROFILES.cynthia },
  { name: "Morgan Martin", avatar: USER_PROFILES.morgan },
  { name: "Arj", avatar: USER_PROFILES.arj },
  { name: "Caroline McKenzie", avatar: USER_PROFILES.caroline },
  { name: "Simon", avatar: USER_PROFILES.simon },
] as const;

function projectTaskAssignees(task: TaskNoodle) {
  const offset = Array.from(task.id).reduce((total, character) => total + character.charCodeAt(0), 0) % PROJECT_TASK_ASSIGNEES.length;
  return [PROJECT_TASK_ASSIGNEES[offset], PROJECT_TASK_ASSIGNEES[(offset + 1) % PROJECT_TASK_ASSIGNEES.length]] as const;
}

function ProjectTaskRowContent({ task }: { task: TaskNoodle }) {
  const assignees = projectTaskAssignees(task);
  return <>
    <span className="me-task-type-icon" aria-hidden="true"><ClipboardList size={17} strokeWidth={1.6} /></span>
    <span className="me-task-copy"><small>{task.id}</small><strong>{task.label}</strong></span>
    <span className="me-task-agents projects-task-people" aria-label={`Assigned to ${assignees.map(({ name }) => name).join(" and ")}`}>{assignees.map(({ name, avatar }) => <img key={name} src={avatar} alt="" />)}</span>
  </>;
}

function ProjectTaskFocusCard({
  task,
  projectName,
  contextId,
  connected = false,
  closing = false,
  bestieChat = null,
  onClose,
}: {
  task: TaskNoodle;
  projectName: string;
  contextId: string;
  connected?: boolean;
  closing?: boolean;
  bestieChat?: ReactNode;
  onClose: () => void;
}) {
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  return (
    <div className={`me-focus-layer project-task-focus-layer${closing ? " is-closing" : ""}`} role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div className={`me-focus-composition project-task-focus-composition${bestieChat ? " has-bestie-chat" : ""}`}>
        <article className={`content-surface me-focus-card project-task-focus-card projects-task-context-target${connected ? " is-bestie-connected" : ""}`} data-bestie-context={contextId} role="dialog" aria-modal="false" aria-label={task.label}>
          <header>
            <ProjectTaskRowContent task={task} />
          </header>
          <div className="project-task-focus-body">
            <div className="project-task-focus-meta">
              <p><span>Project</span><strong>{projectName}</strong></p>
              <p><span>Priority</span><strong>P0</strong></p>
              <p><span>Due date</span><strong>September 1, 2026</strong></p>
            </div>
            <section><span>Description</span><p>{task.context}</p></section>
            <section className="project-task-agents me-task-focus-agents">
              <span>Agents</span>
              <div aria-label="Agents working on this task">
                <img src="/berd-agent-avatars/berdy-gloopies-22.png" alt="Berdy" />
                <img src="/berd-agent-avatars/builderbot-gloopies-20.png" alt="Builderbot" />
                <img src="/berd-agent-avatars/pushback-gloopies-5.png" alt="Pushback" />
              </div>
            </section>
          </div>
          <footer>
            <div><button type="button" aria-label="Task action one"><SquareDashed aria-hidden="true" size={18} strokeWidth={1.5} /></button><button type="button" aria-label="Task action two"><SquareDashed aria-hidden="true" size={18} strokeWidth={1.5} /></button></div>
            <button type="button">Review comments</button>
          </footer>
        </article>
        {bestieChat}
      </div>
    </div>
  );
}

function ProjectWorkspace({
  project,
  connectedTaskId,
  selectedTaskId,
  taskPanelClosing,
  utility,
  onBack,
  onSelectTask,
  onSelectUtility,
}: {
  project: ProjectDefinition;
  activeTab: ProjectWorkspaceTab;
  connectedTaskId: string | null;
  selectedTaskId: string | null;
  taskPanelClosing: boolean;
  utility: ProjectUtility;
  onBack: () => void;
  onSelectTab: (tab: ProjectWorkspaceTab) => void;
  onSelectTask: (taskId: string | null) => void;
  onSelectUtility: (utility: ProjectUtility) => void;
}) {
  const detail = PROJECT_WORKSPACE_DETAILS[project.key] ?? PROJECT_WORKSPACE_DETAILS.berd;
  const selectedTask = project.tasks.find((task) => task.id === selectedTaskId) ?? null;
  const contextIdFor = (taskId: string) => `project:${project.key}:${taskId}`;
  const connectedClassFor = (taskId: string) => connectedTaskId === taskId ? " is-bestie-connected" : "";
  const utilityItems: Array<{ id: ProjectUtility; label: string }> = [
    { id: "remote", label: "Remote" },
    { id: "repository", label: "Main" },
    { id: "terminal", label: "Terminal" },
    { id: "files", label: "Files" },
    { id: "contributors", label: "Contributors" },
  ];
  const forYou = [
    { text: `Arj merged a PR to update ${project.name} agent avatars`, time: "30min" },
    { text: `Tulsi requested review on ${project.tasks[0]?.label ?? "the lead task"}`, time: "3h" },
    { text: `Morgan, Taylor, and Jude are discussing ${detail.milestone}`, time: "Yesterday" },
    { text: `Kenny merged a fix mentioned in ${detail.channel}`, time: "Aug 28, 2026" },
  ];
  const projectChannel = `${project.name.toLowerCase().replaceAll(" ", "-")}-main`;

  return (
    <div className={`project-workspace project-detail-workspace${connectedTaskId ? " has-bestie-context" : ""}${selectedTask ? " has-task-focus" : ""}`}>
      <div className="project-detail-layout">
        <section className="content-surface project-detail-primary project-detail-panel">
          <aside className="project-detail-summary" aria-label={`${project.name} project overview`}>
            <header>
              <button type="button" aria-label="Back to projects" onClick={onBack}><ChevronLeft aria-hidden="true" size={20} strokeWidth={1.7} /></button>
              <h1>{project.name}</h1>
              <button type="button" aria-label="Project options"><MoreHorizontal aria-hidden="true" size={20} strokeWidth={1.8} /></button>
            </header>
            <div className="project-detail-summary-body">
              <section className="project-detail-team">
                <h2>Team</h2>
                <div aria-label="Six project contributors">{Array.from({ length: 6 }, (_, index) => <i key={index} />)}</div>
              </section>
              <section className="project-detail-fact"><h2>Progress</h2><p>13 of 54 tasks</p></section>
              <section className="project-detail-fact"><h2>Description</h2><p>{detail.description}</p></section>
              <section className="project-detail-attention">
                <h2>For you</h2>
                {forYou.map((item, index) => {
                  const task = project.tasks[index % project.tasks.length];
                  return <button key={item.text} type="button" className={`projects-task-context-target${connectedClassFor(task.id)}`} data-bestie-context={contextIdFor(task.id)} onClick={() => onSelectTask(task.id)}><SquareDashed aria-hidden="true" size={18} strokeWidth={1.45} /><span>{item.text}</span><time>{item.time}</time></button>;
                })}
              </section>
            </div>
          </aside>

          <article className="project-detail-chat bestie-content-drop-target" data-bestie-context={`project:${project.key}:chat`}>
            <header className="chat-header">
              <div><LockKeyhole aria-hidden="true" size={18} strokeWidth={1.7} /><strong>{projectChannel}</strong></div>
              <div className="chat-header-actions"><button type="button" aria-label="Project members"><Users aria-hidden="true" size={18} strokeWidth={1.7} /><span>13</span></button></div>
            </header>
            <div className="chat-feed">
              <div className="chat-message"><img className="human-avatar" src={USER_PROFILES.caroline} alt="" /><div><p><strong>Caroline McKenzie</strong><span>Yesterday at 7:05PM</span></p><div>Brought copy link out of the context menu and into the main message rail. Also tried simplifying by removing quick reactions, but can put them back later if people really miss them. Testing if we take something super loud away and people care, then we know it is worth its place.</div></div></div>
              <div className="chat-message"><img className="human-avatar" src={USER_PROFILES.arj} alt="" /><div><p><strong>Jude</strong><span>5:03AM</span></p><div>Outside of the code review process, do you have any feedback or concerns about {project.name} development that you can share with engineers?</div><button type="button" className="reply-link"><span className="mini-avatars"><img src={USER_PROFILES.morgan} alt="" /><img src={USER_PROFILES.simon} alt="" /></span><strong>4 replies</strong><span>last reply 5 hours ago</span></button></div></div>
            </div>
            <MessageComposer channel={projectChannel} />
          </article>
        </section>

        <aside className="content-surface project-utility-rail project-detail-panel" aria-label="Project utilities">
          <div className="project-utility-list">
            {utilityItems.map((item) => <button key={item.id} type="button" className={utility === item.id ? "is-active" : ""} onClick={() => onSelectUtility(item.id)}><SquareDashed size={18} strokeWidth={1.45} /><span>{item.label}</span></button>)}
          </div>
        </aside>
      </div>

      {selectedTask ? <ProjectTaskFocusCard task={selectedTask} projectName={project.name} contextId={contextIdFor(selectedTask.id)} connected={connectedTaskId === selectedTask.id} closing={taskPanelClosing} onClose={() => onSelectTask(null)} /> : null}
    </div>
  );
}

function ProjectsView({ bestieContext, onClearBestie }: { bestieContext: string | null; onClearBestie: () => void }) {
  const [activeProjectIndex, setActiveProjectIndex] = useState(0);
  const [workspaceOpen, setWorkspaceOpen] = useState(false);
  const [workspaceTab, setWorkspaceTab] = useState<ProjectWorkspaceTab>("overview");
  const [workspaceTaskId, setWorkspaceTaskId] = useState<string | null>(null);
  const [workspaceTaskClosing, setWorkspaceTaskClosing] = useState(false);
  const [workspaceUtility, setWorkspaceUtility] = useState<ProjectUtility>("remote");
  const [expandedTaskId, setExpandedTaskId] = useState<string | null>(null);
  const [collapsingTaskId, setCollapsingTaskId] = useState<string | null>(null);
  const [taskScope, setTaskScope] = useState<"all" | "mine">("mine");
  const [projectChatPosition, setProjectChatPosition] = useState<{ left: number; top: number } | null>(null);
  const carouselRowRef = useRef<HTMLDivElement>(null);
  const collapseTimerRef = useRef<number | null>(null);
  const workspaceTaskTimerRef = useRef<number | null>(null);
  const carouselDragRef = useRef<{
    pointerId: number;
    startX: number;
    lastX: number;
    moved: boolean;
  } | null>(null);
  const activeProject = PROJECTS[activeProjectIndex];
  const expandedTask = activeProject.tasks.find((task) => task.id === expandedTaskId) ?? null;
  const projectContextPrefix = `project:${activeProject.key}:`;
  const connectedTaskId = bestieContext?.startsWith(projectContextPrefix) ? bestieContext.slice(projectContextPrefix.length) : null;
  const connectedTask = connectedTaskId === "chat"
    ? { label: `${activeProject.name} main chat` }
    : activeProject.tasks.find((task) => task.id === connectedTaskId) ?? null;
  const hasDefaultTaskContext = !workspaceOpen && Boolean(connectedTaskId && connectedTaskId !== "chat" && connectedTask);
  const contextIdFor = (taskId: string) => `${projectContextPrefix}${taskId}`;
  const connectedClassFor = (taskId: string) => connectedTaskId === taskId ? " is-bestie-connected" : "";
  const scopedProjectTasks = taskScope === "all"
    ? activeProject.tasks
    : activeProject.tasks.filter((_, index) => index % 2 === 0);

  useEffect(() => {
    if (collapseTimerRef.current !== null) window.clearTimeout(collapseTimerRef.current);
    setCollapsingTaskId(null);
    setExpandedTaskId(null);
    setWorkspaceTaskId(null);
    setWorkspaceTaskClosing(false);
  }, [activeProjectIndex]);

  useEffect(() => () => {
    if (collapseTimerRef.current !== null) window.clearTimeout(collapseTimerRef.current);
    if (workspaceTaskTimerRef.current !== null) window.clearTimeout(workspaceTaskTimerRef.current);
  }, []);

  useEffect(() => {
    if (!workspaceOpen) return;
    const frame = window.requestAnimationFrame(() => {
      document.querySelector<HTMLElement>(".projects-stage")?.scrollTo({ top: 0, behavior: "auto" });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [workspaceOpen]);

  useEffect(() => {
    if (!bestieContext || !connectedTask || hasDefaultTaskContext) { setProjectChatPosition(null); return; }
    let settleTimer = 0;
    const stage = document.querySelector<HTMLElement>(".projects-stage");
    const positionChat = () => {
      const matchingTargets = document.querySelectorAll<HTMLElement>(`[data-bestie-context="${CSS.escape(bestieContext)}"]`);
      const target = matchingTargets.item(matchingTargets.length - 1);
      if (!target) return;
      const rect = target.getBoundingClientRect();
      const chatWidth = Math.min(390, window.innerWidth - 32);
      const rightSpace = window.innerWidth - rect.right - 16;
      const left = rightSpace >= chatWidth ? rect.right + 12 : Math.max(16, rect.left - chatWidth - 12);
      const top = Math.max(82, rect.top);
      setProjectChatPosition({ left, top });
    };
    positionChat();
    settleTimer = window.setTimeout(positionChat, 440);
    window.addEventListener("resize", positionChat);
    stage?.addEventListener("scroll", positionChat, { passive: true });
    return () => {
      window.clearTimeout(settleTimer);
      window.removeEventListener("resize", positionChat);
      stage?.removeEventListener("scroll", positionChat);
    };
  }, [bestieContext, connectedTask, expandedTaskId, hasDefaultTaskContext, workspaceOpen, workspaceTaskId, workspaceTab]);

  const offsetForProject = (projectIndex: number) => {
    let offset = projectIndex - activeProjectIndex;
    const midpoint = PROJECTS.length / 2;
    if (offset > midpoint) offset -= PROJECTS.length;
    if (offset < -midpoint) offset += PROJECTS.length;
    return offset;
  };

  const selectPreviousProject = () => {
    setActiveProjectIndex((index) => (index - 1 + PROJECTS.length) % PROJECTS.length);
  };

  const selectNextProject = () => {
    setActiveProjectIndex((index) => (index + 1) % PROJECTS.length);
  };

  const collapseExpandedTask = () => {
    if (!expandedTaskId) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      setExpandedTaskId(null);
      setCollapsingTaskId(null);
      return;
    }
    setCollapsingTaskId(expandedTaskId);
    if (collapseTimerRef.current !== null) window.clearTimeout(collapseTimerRef.current);
    collapseTimerRef.current = window.setTimeout(() => {
      setExpandedTaskId(null);
      setCollapsingTaskId(null);
      collapseTimerRef.current = null;
    }, 280);
  };

  const selectWorkspaceTask = (taskId: string | null) => {
    if (workspaceTaskTimerRef.current !== null) {
      window.clearTimeout(workspaceTaskTimerRef.current);
      workspaceTaskTimerRef.current = null;
    }
    if (taskId) {
      setWorkspaceTaskId(taskId);
      setWorkspaceTaskClosing(false);
      return;
    }
    if (!workspaceTaskId) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      setWorkspaceTaskId(null);
      setWorkspaceTaskClosing(false);
      return;
    }
    setWorkspaceTaskClosing(true);
    workspaceTaskTimerRef.current = window.setTimeout(() => {
      setWorkspaceTaskId(null);
      setWorkspaceTaskClosing(false);
      workspaceTaskTimerRef.current = null;
    }, 280);
  };

  useEffect(() => {
    if (!expandedTaskId || collapsingTaskId) return;
    const handleClickAway = (event: PointerEvent) => {
      const target = event.target as Element | null;
      if (target?.closest(".project-task-focus-card, .projects-task-list .me-task-row, .projects-context-chat, .snake-button, .bestie-drag-preview")) return;
      collapseExpandedTask();
    };
    document.addEventListener("pointerdown", handleClickAway);
    return () => document.removeEventListener("pointerdown", handleClickAway);
  }, [expandedTaskId, collapsingTaskId]);

  const beginCarouselDrag = (event: ReactPointerEvent<HTMLDivElement>) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    carouselDragRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      lastX: event.clientX,
      moved: false,
    };
    carouselRowRef.current?.classList.add("is-dragging");
  };

  const updateCarouselDrag = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = carouselDragRef.current;
    const row = carouselRowRef.current;
    if (!drag || !row || drag.pointerId !== event.pointerId) return;
    drag.lastX = event.clientX;
    const rawDelta = drag.lastX - drag.startX;
    if (Math.abs(rawDelta) > 5) drag.moved = true;
    const delta = Math.max(-row.clientWidth * 0.34, Math.min(row.clientWidth * 0.34, rawDelta));
    row.style.setProperty("--carousel-drag-x", `${delta}px`);
  };

  const endCarouselDrag = (
    event: ReactPointerEvent<HTMLDivElement>,
    cancelled = false,
  ) => {
    const drag = carouselDragRef.current;
    const row = carouselRowRef.current;
    if (!drag || !row || drag.pointerId !== event.pointerId) return;
    const delta = drag.lastX - drag.startX;
    if (event.currentTarget instanceof Element && event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    carouselDragRef.current = null;
    row.classList.remove("is-dragging");

    if (!cancelled && Math.abs(delta) >= 64) {
      if (delta < 0) selectNextProject();
      else selectPreviousProject();
    } else if (!cancelled && !drag.moved) {
      const rect = event.currentTarget.getBoundingClientRect();
      const position = (event.clientX - rect.left) / rect.width;
      if (position < 0.34) selectPreviousProject();
      if (position > 0.66) selectNextProject();
      if (position >= 0.34 && position <= 0.66) {
        setWorkspaceOpen(true);
        setWorkspaceTab("overview");
        setWorkspaceTaskId(null);
      }
    }

    requestAnimationFrame(() => {
      row.style.setProperty("--carousel-drag-x", "0px");
    });
  };

  return (
    <section className={`projects-stage${expandedTask ? " has-task-focus" : ""}${hasDefaultTaskContext ? " has-bestie-task-context" : ""}`} aria-label="Projects" data-bestie-context={`${projectContextPrefix}chat`} style={{ "--project-accent": workspaceOpen && activeProject.key === "berd" ? "#efb8b2" : activeProject.state.accentCssColor } as CSSProperties}>
      {workspaceOpen ? (
        <ProjectWorkspace
          project={activeProject}
          activeTab={workspaceTab}
          connectedTaskId={connectedTaskId}
          selectedTaskId={workspaceTaskId}
          taskPanelClosing={workspaceTaskClosing}
          utility={workspaceUtility}
          onBack={() => { setWorkspaceOpen(false); setWorkspaceTaskId(null); setWorkspaceTaskClosing(false); onClearBestie(); }}
          onSelectTab={(tab) => { setWorkspaceTab(tab); selectWorkspaceTask(null); }}
          onSelectTask={selectWorkspaceTask}
          onSelectUtility={setWorkspaceUtility}
        />
      ) : <>
      <div ref={carouselRowRef} className="projects-cube-row" aria-label="Project carousel">
        {PROJECTS.map((project, projectIndex) => {
          const offset = offsetForProject(projectIndex);
          const slot = offset === 0 ? "active" : Math.abs(offset) === 1 ? "neighbor" : "far";
          return (
            <div
              key={project.key}
              className={`projects-gallery-item slot-${slot}`}
              style={{ left: `${50 + offset * 50}%` }}
              aria-hidden="true"
            >
              <ProjectArtifactRenderer
                state={project.state}
                imageUrls={project.imageUrls}
                environmentUrl="/berd-project-assets/studio_soft.exr"
                variant="tile"
                cameraDistanceScale={1.35}
              />
            </div>
          );
        })}
        <div
          className="projects-carousel-drag-surface"
          role="slider"
          tabIndex={0}
          aria-label="Select project"
          aria-valuemin={1}
          aria-valuemax={PROJECTS.length}
          aria-valuenow={activeProjectIndex + 1}
          aria-valuetext={activeProject.name}
          onPointerDown={beginCarouselDrag}
          onPointerMove={updateCarouselDrag}
          onPointerUp={(event) => endCarouselDrag(event)}
          onPointerCancel={(event) => endCarouselDrag(event, true)}
          onKeyDown={(event) => {
            if (event.key === "ArrowLeft") {
              event.preventDefault();
              selectPreviousProject();
            }
            if (event.key === "ArrowRight") {
              event.preventDefault();
              selectNextProject();
            }
          }}
        />
      </div>

      <div className={`projects-default-composition${hasDefaultTaskContext ? " has-bestie-context" : ""}`}>
      <section className={`projects-info${expandedTask ? " has-task-focus" : ""}${connectedTask ? " has-bestie-context" : ""}`} aria-live="polite">
        <div className="projects-info-heading">
          <button type="button" className="projects-active-name" aria-label={`Open ${activeProject.name} project`} onClick={() => { setWorkspaceOpen(true); setWorkspaceTab("overview"); setWorkspaceTaskId(null); }}>{activeProject.name}</button>
          <div className={`projects-task-mode${taskScope === "mine" ? " is-tasks" : ""}`} aria-label="Project task filter">
            <button type="button" className={taskScope === "all" ? "is-active" : ""} onClick={() => { setTaskScope("all"); setExpandedTaskId(null); }}>All</button>
            <button type="button" className={taskScope === "mine" ? "is-active" : ""} onClick={() => { setTaskScope("mine"); setExpandedTaskId(null); }}>My tasks</button>
          </div>
        </div>

        <div className="projects-task-focus-layout">
          <div className="projects-task-containers">
            <article className="content-surface me-glass-card me-assigned-card task-list-surface projects-task-list">
              {scopedProjectTasks.map((task) => (
              <div key={task.id} className={`me-task-list-item projects-task-context-target${expandedTaskId === task.id ? " is-selected" : ""}${connectedClassFor(task.id)}`} data-bestie-context={contextIdFor(task.id)}>
                <button type="button" className="me-task-row projects-task-summary" aria-expanded={expandedTaskId === task.id} onClick={() => expandedTaskId === task.id ? collapseExpandedTask() : setExpandedTaskId(task.id)}>
                  <ProjectTaskRowContent task={task} />
                </button>
              </div>
              ))}
            </article>
          </div>

        </div>
      </section>
      {hasDefaultTaskContext && bestieContext && !expandedTask && connectedTask ? <MeContextChat className="projects-context-chat" context={connectedTask.label} position={null} onClose={onClearBestie} /> : null}
      </div>
      </>}
      {!workspaceOpen && expandedTask ? <ProjectTaskFocusCard task={expandedTask} projectName={activeProject.name} contextId={contextIdFor(expandedTask.id)} connected={connectedTaskId === expandedTask.id} closing={collapsingTaskId === expandedTask.id} bestieChat={hasDefaultTaskContext && bestieContext && connectedTask ? <MeContextChat className="projects-context-chat" context={connectedTask.label} position={null} onClose={onClearBestie} /> : null} onClose={collapseExpandedTask} /> : null}
      {connectedTask && bestieContext && !hasDefaultTaskContext ? <MeContextChat className="projects-context-chat" context={connectedTask.label} position={projectChatPosition} onClose={onClearBestie} /> : null}
    </section>
  );
}

function TaskDetailPanel({
  selection,
  onClose,
}: {
  selection: SelectedTask;
  onClose: () => void;
}) {
  const { task, projectName } = selection;
  const [responseOpen, setResponseOpen] = useState(false);
  const [response, setResponse] = useState("");
  const responseRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    setResponseOpen(false);
    setResponse("");
  }, [task.id]);

  useEffect(() => {
    if (!responseOpen) return;
    const focusTimer = window.setTimeout(() => responseRef.current?.focus(), 100);
    return () => window.clearTimeout(focusTimer);
  }, [responseOpen]);

  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (responseOpen) setResponseOpen(false);
      else onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose, responseOpen]);

  return (
    <aside className="content-surface task-detail-panel" aria-label={`${task.label} task details`}>
      <header className="task-detail-header">
        <h2>{task.label}</h2>
        <button type="button" className="task-detail-close" onClick={onClose} aria-label="Close task details"><X aria-hidden="true" size={18} strokeWidth={1.75} /></button>
      </header>
      <div className="task-detail-divider" />
      <div className="task-meta-grid">
        <div className="task-meta task-meta-path"><span>Task</span><strong>{projectName} / {task.id} / {task.label}</strong></div>
        <div className="task-meta"><span>Created</span><strong>{task.created}</strong></div>
        <div className="task-meta"><span>Assigned to</span><strong>{task.assignee}</strong></div>
      </div>
      <div className="task-context">
        <span>Context</span>
        <p>{task.context}</p>
      </div>
      <div className="task-actions">
        <span>What would you like to do?</span>
        <div className={`task-action-list${responseOpen ? " is-composing" : ""}`}>
          {!responseOpen ? <button type="button">Approve</button> : null}
          {!responseOpen ? <button type="button">Deny</button> : null}
          <div className={`response-composer${responseOpen ? " is-open" : ""}`}>
            {responseOpen ? (
              <textarea
                ref={responseRef}
                value={response}
                onChange={(event) => setResponse(event.target.value)}
                placeholder="Send a response"
                aria-label="Response"
              />
            ) : (
              <button type="button" className="response-composer-trigger" onClick={() => setResponseOpen(true)}>Send a response</button>
            )}
          </div>
        </div>
      </div>
    </aside>
  );
}

type MessagePanelId = "sidebar" | "chat" | "thread";
type MessagePanelLayout = { x: number; y: number };
type MessagePanelEdge = "left" | "right" | "top" | "bottom";
type MessagePanelConnection = {
  a: MessagePanelId;
  b: MessagePanelId;
  edge: MessagePanelEdge;
};

const INITIAL_MESSAGE_LAYOUT: Record<MessagePanelId, MessagePanelLayout> = {
  sidebar: { x: 0, y: 0 },
  chat: { x: 0, y: 0 },
  thread: { x: 0, y: 0 },
};

const MESSAGE_PANEL_IDS: MessagePanelId[] = ["sidebar", "chat", "thread"];
const INITIAL_MESSAGE_CONNECTIONS: MessagePanelConnection[] = [
  { a: "sidebar", b: "chat", edge: "left" },
  { a: "chat", b: "thread", edge: "left" },
];

function MessageComposer({ compact = false, channel = "buzz-design" }: { compact?: boolean; channel?: string }) {
  return (
    <div className={`message-composer${compact ? " is-compact" : ""}`}>
      <span>Message #{channel}</span>
      <div className="message-composer-actions">
        <AtSign aria-hidden="true" size={18} strokeWidth={1.7} />
        <Paperclip aria-hidden="true" size={18} strokeWidth={1.7} />
        <SmilePlus aria-hidden="true" size={18} strokeWidth={1.7} />
        <span className="message-composer-format" aria-hidden="true">Aᵃ</span>
        <button type="button" aria-label="Send message"><ArrowUp aria-hidden="true" size={18} strokeWidth={1.8} /></button>
      </div>
    </div>
  );
}

type MessageSidePanelHeaderProps = {
  icon: ReactNode;
  title: string;
  closeLabel: string;
  onClose: () => void;
  dragHandlers?: {
    onPointerDown?: (event: ReactPointerEvent<HTMLElement>) => void;
    onPointerMove?: (event: ReactPointerEvent<HTMLElement>) => void;
    onPointerUp?: (event: ReactPointerEvent<HTMLElement>) => void;
    onPointerCancel?: (event: ReactPointerEvent<HTMLElement>) => void;
  };
};

function MessageSidePanelHeader({ icon, title, closeLabel, onClose, dragHandlers }: MessageSidePanelHeaderProps) {
  return (
    <header className="panel-drag-handle message-side-panel-header" {...dragHandlers}>
      <div>{icon}<strong>{title}</strong></div>
      <button type="button" aria-label={closeLabel} onClick={onClose}><X aria-hidden="true" size={18} strokeWidth={1.7} /></button>
    </header>
  );
}

function bestieContextTag(context: string | null) {
  if (!context) return null;
  if (context === "messages:chat") return "@buzz-interface-squad";
  if (context === "messages:thread") return "@Thread";
  if (context.startsWith("messages:task:")) return `@${context.slice("messages:task:".length)}`;
  if (context === "messages:content:morgan-jude-arjun") return "@Group-chat";
  if (context.startsWith("messages:content:")) return `@#${context.slice("messages:content:".length)}`;
  if (context.startsWith("messages:message:")) return context === "messages:message:bestie-task-created" ? "@Task-message" : "@Message";
  if (context.startsWith("messages:catchup:")) return context.includes(":message:") ? "@Catch-up-message" : "@Catch-up";
  if (context.startsWith("project:")) return `@${context.split(":").at(-1)}`;
  if (context.startsWith("PR-123:issue:")) return "@PR-123";
  if (context === "attention") return "@For-you";
  if (context === "assigned") return "@My-tasks";
  return `@${context}`;
}

const BESTIE_CONTEXT_PANEL_WIDTH = 360;
const BESTIE_CONTEXT_PANEL_GAP = 8;

function BestieChat({ onClose, modularMode = false, context = null, initialPosition = null, contextPanel = false, onContextPanelWidthChange, onMessagesPanelDockChange }: { onClose: () => void; modularMode?: boolean; context?: string | null; initialPosition?: { x: number; y: number } | null; contextPanel?: boolean; onContextPanelWidthChange?: (width: number) => void; onMessagesPanelDockChange?: (docked: boolean) => void }) {
  const contextualStandalone = modularMode && Boolean(context?.startsWith("messages:"));
  const [detached, setDetached] = useState(contextualStandalone);
  const [position, setPosition] = useState(initialPosition ?? { x: 0, y: 0 });
  const [mergedEdge, setMergedEdge] = useState<MessagePanelEdge | null>(null);
  const [mergedToStickyPanel, setMergedToStickyPanel] = useState(false);
  const [slotSize, setSlotSize] = useState<{ width: number; height: number } | null>(null);
  const [mergedRadius, setMergedRadius] = useState<string | null>(null);
  const [freeSize, setFreeSize] = useState<{ width: number; height: number } | null>(null);
  const panelRef = useRef<HTMLElement>(null);
  const mergedTargetRef = useRef<{ cell: HTMLElement; edge: MessagePanelEdge; slotted: boolean } | null>(null);
  const resizeRef = useRef<{
    pointerId: number;
    edge: "top" | "right" | "bottom";
    connected: boolean;
    startX: number;
    startY: number;
    startWidth: number;
    startHeight: number;
    width: number;
    height: number;
    startYPosition: number;
    yPosition: number;
    xPosition: number;
    rightAnchored: boolean;
  } | null>(null);
  const dragRef = useRef<{
    pointerId: number;
    startX: number;
    startY: number;
    originX: number;
    originY: number;
    x: number;
    y: number;
    beganDetached: boolean;
    connectedAtStart: boolean;
    hasBroken: boolean;
    panX: number;
    panY: number;
    shellPanX: number;
    shellPanY: number;
    previewCell: HTMLElement | null;
    previewEdge: MessagePanelEdge | null;
  }>({ pointerId: -1, startX: 0, startY: 0, originX: 0, originY: 0, x: 0, y: 0, beganDetached: false, connectedAtStart: false, hasBroken: false, panX: 0, panY: 0, shellPanX: 0, shellPanY: 0, previewCell: null, previewEdge: null });
  const contextLabel = context === "messages:chat"
    ? "buzz-interface-squad"
    : context === "messages:thread"
      ? "this thread"
      : context?.startsWith("messages:message:")
        ? "this message"
      : context?.startsWith("messages:task:")
        ? context.slice("messages:task:".length)
        : context?.startsWith("messages:content:")
          ? "this content"
        : context?.startsWith("messages:catchup:")
          ? context.includes(":message:") ? "this Catch up message" : "this Catch up item"
          : null;
  const contextTag = bestieContextTag(context);

  const clearPreview = () => {
    const drag = dragRef.current;
    if (drag.previewCell && drag.previewEdge) drag.previewCell.classList.remove(`merge-preview-${drag.previewEdge}`);
    drag.previewCell = null;
    drag.previewEdge = null;
  };

  const removeMergedTarget = () => {
    const merged = mergedTargetRef.current;
    if (!merged) return;
    merged.cell.classList.remove(`bestie-merge-target-${merged.edge}`);
    merged.cell.classList.remove("bestie-is-slotted");
    merged.cell.style.removeProperty("--bestie-slot-width");
    merged.cell.style.removeProperty("--bestie-slot-height");
    merged.cell.style.removeProperty("--bestie-stack-width");
    mergedTargetRef.current = null;
  };

  const isStandaloneBestieTarget = (cell: HTMLElement) =>
    modularMode || cell.matches(".message-task-detail-panel, .catch-up-card");

  useEffect(() => () => {
    removeMergedTarget();
    clearPreview();
  }, []);

  useEffect(() => {
    if (!modularMode || !contextPanel) return;
    const stage = document.querySelector<HTMLElement>(".messages-stage");
    if (!stage) return;
    let observedNeighbor: HTMLElement | null = null;
    const resolveNeighbor = () =>
      stage.querySelector<HTMLElement>(".message-task-detail-panel.is-docked:not(.is-awaiting-dock)")
      ?? stage.querySelector<HTMLElement>(".thread-cell .thread-panel")
      ?? stage.querySelector<HTMLElement>(".chat-cell .chat-panel");
    const syncToNeighbor = () => {
      const neighbor = resolveNeighbor();
      if (!neighbor) return;
      const neighborRect = neighbor.getBoundingClientRect();
      const panelWidth = panelRef.current?.getBoundingClientRect().width || BESTIE_CONTEXT_PANEL_WIDTH;
      setFreeSize((current) => {
        const width = current?.width ?? panelWidth;
        return current && Math.abs(current.height - neighborRect.height) < .5 ? current : { width, height: neighborRect.height };
      });
      setPosition((current) => {
        const nextX = Math.max(20, window.innerWidth - panelWidth - 20);
        const nextY = neighborRect.top;
        return Math.abs(current.x - nextX) < .5 && Math.abs(current.y - nextY) < .5 ? current : { x: nextX, y: nextY };
      });
    };
    const resizeObserver = new ResizeObserver(syncToNeighbor);
    const observeNeighbor = () => {
      const neighbor = resolveNeighbor();
      if (neighbor !== observedNeighbor) {
        resizeObserver.disconnect();
        observedNeighbor = neighbor;
        if (neighbor) resizeObserver.observe(neighbor);
      }
      syncToNeighbor();
    };
    const mutationObserver = new MutationObserver(observeNeighbor);
    observeNeighbor();
    mutationObserver.observe(stage, { childList: true, subtree: true, attributes: true, attributeFilter: ["class", "style"] });
    window.addEventListener("resize", observeNeighbor);
    return () => {
      resizeObserver.disconnect();
      mutationObserver.disconnect();
      window.removeEventListener("resize", observeNeighbor);
    };
  }, [contextPanel, modularMode]);

  const findSnapTarget = (pointerX?: number, pointerY?: number) => {
    const panel = panelRef.current;
    if (!panel) return null;
    const dragged = panel.getBoundingClientRect();
    const candidates: Array<{ cell: HTMLElement; edge: MessagePanelEdge; distance: number; x: number; y: number; targetRect: { left: number; right: number; top: number; bottom: number; width: number; height: number } }> = [];
    const targetSelector = modularMode ? '[data-bestie-module-target="true"]' : ".message-panel-cell[data-panel-id]";
    const hoveredTarget = modularMode && pointerX !== undefined && pointerY !== undefined
      ? document.elementsFromPoint(pointerX, pointerY).map((element) => element.closest<HTMLElement>(targetSelector)).find((element): element is HTMLElement => Boolean(element)) ?? null
      : null;
    document.querySelectorAll<HTMLElement>(targetSelector).forEach((cell) => {
      if (modularMode && cell.dataset.panelId === "sidebar") return;
      const target = cell.matches(".message-task-detail-panel, .catch-up-card") ? cell : cell.querySelector<HTMLElement>(".message-panel");
      if (!target) return;
      const rect = target.getBoundingClientRect();
      const targetRect = { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom, width: rect.width, height: rect.height };
      if (hoveredTarget === cell && pointerX !== undefined && pointerY !== undefined) {
        const edgeDistances: Array<{ edge: MessagePanelEdge; distance: number }> = [
          { edge: "left", distance: Math.abs(pointerX - rect.left) },
          { edge: "right", distance: Math.abs(rect.right - pointerX) },
          { edge: "top", distance: Math.abs(pointerY - rect.top) },
          { edge: "bottom", distance: Math.abs(rect.bottom - pointerY) },
        ];
        const standaloneTarget = isStandaloneBestieTarget(cell);
        const availableEdges = standaloneTarget && !modularMode
          ? edgeDistances.filter(({ edge }) => edge === "left"
              ? rect.left >= 236
              : edge === "right"
                ? window.innerWidth - rect.right >= 236
                : edge === "top"
                  ? rect.top - TOPBAR_CONTENT_OFFSET >= 236
                  : window.innerHeight - rect.bottom >= 236)
          : edgeDistances;
        const preferred = (availableEdges.length ? availableEdges : edgeDistances).sort((a, b) => a.distance - b.distance)[0];
        const placement = preferred.edge === "left"
          ? { x: rect.left - dragged.width, y: rect.top }
          : preferred.edge === "right"
            ? { x: rect.right, y: rect.top }
            : preferred.edge === "top"
              ? { x: rect.left, y: rect.top - dragged.height }
              : { x: rect.left, y: rect.bottom };
        candidates.push({ cell, edge: preferred.edge, distance: -1000, ...placement, targetRect });
        return;
      }
      const rowAlignment = Math.min(Math.abs(dragged.top - rect.top), Math.abs(dragged.bottom - rect.bottom));
      const columnAlignment = Math.min(Math.abs(dragged.left - rect.left), Math.abs(dragged.right - rect.right));
      const canSlotInside = cell.dataset.panelId !== "sidebar";
      if (rowAlignment <= 68) {
        candidates.push(
          { cell, edge: "left", distance: Math.min(Math.abs(dragged.right - rect.left), canSlotInside ? Math.abs(dragged.left - rect.left) : Number.POSITIVE_INFINITY), x: rect.left - dragged.width, y: rect.top, targetRect },
          { cell, edge: "right", distance: Math.min(Math.abs(dragged.left - rect.right), canSlotInside ? Math.abs(dragged.right - rect.right) : Number.POSITIVE_INFINITY), x: rect.right, y: rect.top, targetRect },
        );
      }
      if (columnAlignment <= 68) {
        candidates.push(
          { cell, edge: "top", distance: Math.min(Math.abs(dragged.bottom - rect.top), canSlotInside ? Math.abs(dragged.top - rect.top) : Number.POSITIVE_INFINITY), x: rect.left, y: rect.top - dragged.height, targetRect },
          { cell, edge: "bottom", distance: Math.min(Math.abs(dragged.top - rect.bottom), canSlotInside ? Math.abs(dragged.bottom - rect.bottom) : Number.POSITIVE_INFINITY), x: rect.left, y: rect.bottom, targetRect },
        );
      }
    });
    const best = candidates.sort((a, b) => a.distance - b.distance)[0];
    return best && best.distance <= 58 ? best : null;
  };

  const updatePreview = (snap: ReturnType<typeof findSnapTarget>) => {
    const drag = dragRef.current;
    if (snap && drag.previewCell === snap.cell && drag.previewEdge === snap.edge) return;
    clearPreview();
    if (!snap) return;
    snap.cell.classList.add(`merge-preview-${snap.edge}`);
    drag.previewCell = snap.cell;
    drag.previewEdge = snap.edge;
  };

  const beginResize = (event: ReactPointerEvent<HTMLButtonElement>, edge: "top" | "right" | "bottom") => {
    event.preventDefault();
    event.stopPropagation();
    const panel = panelRef.current;
    if (!panel) return;
    const rect = panel.getBoundingClientRect();
    event.currentTarget.setPointerCapture(event.pointerId);
    resizeRef.current = {
      pointerId: event.pointerId,
      edge,
      connected: mergedTargetRef.current !== null,
      startX: event.clientX,
      startY: event.clientY,
      startWidth: rect.width,
      startHeight: rect.height,
      width: rect.width,
      height: rect.height,
      startYPosition: position.y,
      yPosition: position.y,
      xPosition: position.x,
      rightAnchored: contextPanel && Math.abs(window.innerWidth - rect.right - 20) <= 8,
    };
    panel.classList.add("is-resizing");
  };

  const updateResize = (event: ReactPointerEvent<HTMLButtonElement>) => {
    const resize = resizeRef.current;
    const panel = panelRef.current;
    if (!resize || resize.pointerId !== event.pointerId || !panel) return;
    if (event.buttons === 0) {
      endResize(event);
      return;
    }
    const deltaX = event.clientX - resize.startX;
    const deltaY = event.clientY - resize.startY;
    if (resize.edge === "right") resize.width = Math.max(220, Math.min(contextPanel ? 560 : 920, resize.startWidth + deltaX));
    if (resize.connected) {
      const merged = mergedTargetRef.current;
      if (!merged) return;
      if (merged.edge === "left" || merged.edge === "right") {
        const maximumSlotWidth = Math.max(220, merged.cell.getBoundingClientRect().width - 260);
        resize.width = Math.min(maximumSlotWidth, resize.width);
      }
      panel.style.setProperty("--bestie-slot-width", `${resize.width}px`);
      merged.cell.style.setProperty("--bestie-slot-width", `${resize.width}px`);
      if (merged.edge === "top" || merged.edge === "bottom") {
        merged.cell.style.setProperty("--bestie-stack-width", `${resize.width}px`);
      }
    } else {
      if (resize.edge === "bottom") resize.height = Math.max(280, Math.min(window.innerHeight - 90, resize.startHeight + deltaY));
      if (resize.edge === "top") {
        const appliedDelta = Math.max(resize.startHeight - (window.innerHeight - 90), Math.min(resize.startHeight - 280, deltaY));
        resize.height = resize.startHeight - appliedDelta;
        resize.yPosition = resize.startYPosition + appliedDelta;
        panel.style.setProperty("--bestie-y", `${resize.yPosition}px`);
      }
      panel.style.setProperty("--bestie-free-width", `${resize.width}px`);
      panel.style.setProperty("--bestie-free-height", `${resize.height}px`);
      if (contextPanel && resize.edge === "right") {
        if (resize.rightAnchored) {
          resize.xPosition = Math.max(20, window.innerWidth - resize.width - 20);
          panel.style.setProperty("--bestie-x", `${resize.xPosition}px`);
          setPosition((current) => ({ ...current, x: resize.xPosition }));
        }
        setFreeSize({ width: resize.width, height: resize.height });
        onContextPanelWidthChange?.(resize.width);
      }
    }
  };

  const endResize = (event: ReactPointerEvent<HTMLButtonElement>) => {
    const resize = resizeRef.current;
    const panel = panelRef.current;
    if (!resize || resize.pointerId !== event.pointerId || !panel) return;
    if (event.currentTarget instanceof Element && event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    panel.classList.remove("is-resizing");
    if (resize.connected) {
      setSlotSize((current) => current ? { ...current, width: resize.width } : current);
    } else {
      setFreeSize({ width: resize.width, height: resize.height });
      if (resize.edge === "top") setPosition((current) => ({ ...current, y: resize.yPosition }));
      if (contextPanel && resize.edge === "right" && resize.rightAnchored) setPosition((current) => ({ ...current, x: resize.xPosition }));
    }
    resizeRef.current = null;
  };

  const beginDrag = (event: ReactPointerEvent<HTMLElement>) => {
    if ((event.target as Element).closest("button, input")) return;
    const panel = panelRef.current;
    if (!panel) return;
    const rect = panel.getBoundingClientRect();
    const shell = panel.closest<HTMLElement>(".app-shell");
    const shellStyle = shell ? getComputedStyle(shell) : null;
    const shellPanX = Number.parseFloat(shellStyle?.getPropertyValue("--messages-pan-x") || "0") || 0;
    const shellPanY = Number.parseFloat(shellStyle?.getPropertyValue("--messages-pan-y") || "0") || 0;
    const panX = mergedToStickyPanel ? 0 : shellPanX;
    const panY = mergedToStickyPanel ? 0 : shellPanY;
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      originX: rect.left - panX,
      originY: rect.top - panY,
      x: rect.left - panX,
      y: rect.top - panY,
      beganDetached: detached,
      connectedAtStart: mergedTargetRef.current !== null,
      hasBroken: false,
      panX,
      panY,
      shellPanX,
      shellPanY,
      previewCell: null,
      previewEdge: null,
    };
    panel.style.setProperty("--bestie-x", `${rect.left - panX}px`);
    panel.style.setProperty("--bestie-y", `${rect.top - panY}px`);
    panel.classList.add("is-dragging");
  };

  const updateDrag = (event: ReactPointerEvent<HTMLElement>) => {
    const drag = dragRef.current;
    const panel = panelRef.current;
    if (!panel || drag.pointerId !== event.pointerId) return;
    if (event.buttons === 0) {
      endDrag(event);
      return;
    }
    const pullDistance = Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY);
    if (drag.connectedAtStart && pullDistance > 112) {
      drag.hasBroken = true;
      if (!modularMode) {
        const detachedRect = panel.getBoundingClientRect();
        setFreeSize({ width: detachedRect.width, height: detachedRect.height });
        removeMergedTarget();
        setMergedEdge(null);
        setSlotSize(null);
        setMergedRadius(null);
      }
      if (mergedToStickyPanel) {
        drag.originX -= drag.shellPanX;
        drag.originY -= drag.shellPanY;
        drag.x -= drag.shellPanX;
        drag.y -= drag.shellPanY;
        drag.panX = drag.shellPanX;
        drag.panY = drag.shellPanY;
        panel.classList.remove("is-merged-sticky");
        setMergedToStickyPanel(false);
      }
    }
    const resistance = drag.connectedAtStart && !drag.hasBroken
      ? 0.76 + Math.min(pullDistance / 112, 1) * 0.12
      : 1;
    const nextX = drag.originX + (event.clientX - drag.startX) * resistance;
    const nextY = drag.originY + (event.clientY - drag.startY) * resistance;
    drag.x = Math.max(16 - drag.panX, Math.min(window.innerWidth - panel.offsetWidth - 16 - drag.panX, nextX));
    drag.y = Math.max(TOPBAR_CONTENT_OFFSET - drag.panY, Math.min(window.innerHeight - panel.offsetHeight - 16 - drag.panY, nextY));
    panel.style.setProperty("--bestie-x", `${drag.x}px`);
    panel.style.setProperty("--bestie-y", `${drag.y}px`);
    panel.classList.toggle("is-tearing", !drag.beganDetached && pullDistance > 48);
    updatePreview(drag.connectedAtStart && !drag.hasBroken ? null : findSnapTarget(event.clientX, event.clientY));
  };

  const endDrag = (event: ReactPointerEvent<HTMLElement>) => {
    const drag = dragRef.current;
    const panel = panelRef.current;
    if (!panel || drag.pointerId !== event.pointerId) return;
    if (event.currentTarget instanceof Element && event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    const stayedConnected = drag.connectedAtStart && !drag.hasBroken;
    const snap = stayedConnected ? null : findSnapTarget(event.clientX, event.clientY);
    const snapIsStandalone = Boolean(snap && isStandaloneBestieTarget(snap.cell));
    clearPreview();
    if (modularMode && !snap) {
      panel.classList.remove("is-dragging", "is-tearing");
      if (drag.connectedAtStart) {
        panel.style.setProperty("--bestie-x", `${drag.originX}px`);
        panel.style.setProperty("--bestie-y", `${drag.originY}px`);
        setPosition({ x: drag.originX, y: drag.originY });
      } else {
        panel.classList.remove("is-detached");
        panel.style.removeProperty("--bestie-x");
        panel.style.removeProperty("--bestie-y");
        setDetached(false);
        onMessagesPanelDockChange?.(false);
      }
      drag.pointerId = -1;
      return;
    }
    const shouldDetach = stayedConnected || snap || drag.beganDetached || Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) > 84;
    panel.classList.remove("is-dragging", "is-tearing");
    if (shouldDetach) {
      panel.classList.add("is-detached");
      setDetached(true);
      if (snap) {
        removeMergedTarget();
        if (snapIsStandalone) {
          const standaloneWidth = panel.offsetWidth;
          onMessagesPanelDockChange?.(true);
          onContextPanelWidthChange?.(standaloneWidth);
          setMergedEdge(null);
          setMergedToStickyPanel(false);
          setSlotSize(null);
          setMergedRadius(null);
          setFreeSize({ width: standaloneWidth, height: snap.targetRect.height });
        } else {
          const targetIsSidebar = snap.cell.dataset.panelId === "sidebar";
          const horizontal = snap.edge === "left" || snap.edge === "right";
          const slotted = !targetIsSidebar && snap.cell.classList.contains("message-panel-cell");
          const targetPanel = snap.cell.querySelector<HTMLElement>(".message-panel");
          const targetStyle = targetPanel ? getComputedStyle(targetPanel) : null;
          const targetRadii = {
            topLeft: targetStyle?.borderTopLeftRadius || "28px",
            topRight: targetStyle?.borderTopRightRadius || "28px",
            bottomRight: targetStyle?.borderBottomRightRadius || "28px",
            bottomLeft: targetStyle?.borderBottomLeftRadius || "28px",
          };
          const nextMergedRadius = snap.edge === "top"
            ? `${targetRadii.topLeft} ${targetRadii.topRight} 0px 0px`
            : snap.edge === "bottom"
              ? `0px 0px ${targetRadii.bottomRight} ${targetRadii.bottomLeft}`
              : snap.edge === "left"
                ? `${targetRadii.topLeft} 0px 0px ${targetRadii.bottomLeft}`
                : `0px ${targetRadii.topRight} ${targetRadii.bottomRight} 0px`;
          const nextSlotSize = slotted
            ? horizontal
              ? { width: Math.max(220, Math.min(300, snap.targetRect.width * .42)), height: snap.targetRect.height }
              : { width: snap.targetRect.width, height: Math.max(220, Math.min(300, snap.targetRect.height * .36)) }
            : null;
          if (nextSlotSize) {
            snap.cell.classList.add("bestie-is-slotted");
            snap.cell.style.setProperty("--bestie-slot-width", `${nextSlotSize.width}px`);
            snap.cell.style.setProperty("--bestie-slot-height", `${nextSlotSize.height}px`);
          }
          snap.cell.classList.add(`bestie-merge-target-${snap.edge}`);
          mergedTargetRef.current = { cell: snap.cell, edge: snap.edge, slotted };
          setMergedEdge(snap.edge);
          setMergedToStickyPanel(targetIsSidebar);
          setSlotSize(nextSlotSize);
          setMergedRadius(nextMergedRadius);
          setFreeSize(null);
        }
      } else if (!stayedConnected) {
        setMergedRadius(null);
      }
      const standaloneNext = snap && snapIsStandalone
        ? {
            x: Math.max(20, Math.min(window.innerWidth - panel.offsetWidth - 20, snap.edge === "left" ? snap.targetRect.left - panel.offsetWidth - BESTIE_CONTEXT_PANEL_GAP : snap.edge === "right" ? snap.targetRect.right + BESTIE_CONTEXT_PANEL_GAP : snap.targetRect.left)),
            y: Math.max(TOPBAR_CONTENT_OFFSET, Math.min(window.innerHeight - panel.offsetHeight - 20, snap.edge === "top" ? snap.targetRect.top - panel.offsetHeight - BESTIE_CONTEXT_PANEL_GAP : snap.edge === "bottom" ? snap.targetRect.bottom + BESTIE_CONTEXT_PANEL_GAP : snap.targetRect.top)),
          }
        : null;
      const next = stayedConnected
        ? { x: drag.originX, y: drag.originY }
        : standaloneNext
          ? standaloneNext
        : snap
          ? snap.cell.dataset.panelId === "sidebar"
            ? { x: snap.x, y: snap.y }
            : snap.cell.classList.contains("bestie-is-slotted")
              ? snap.edge === "left"
                ? { x: snap.targetRect.left - drag.shellPanX, y: snap.targetRect.top - drag.shellPanY }
                : snap.edge === "right"
                  ? { x: snap.targetRect.right - (Number.parseFloat(snap.cell.style.getPropertyValue("--bestie-slot-width")) || panel.offsetWidth) - drag.shellPanX, y: snap.targetRect.top - drag.shellPanY }
                  : snap.edge === "top"
                    ? { x: snap.targetRect.left - drag.shellPanX, y: snap.targetRect.top - drag.shellPanY }
                    : { x: snap.targetRect.left - drag.shellPanX, y: snap.targetRect.bottom - (Number.parseFloat(snap.cell.style.getPropertyValue("--bestie-slot-height")) || panel.offsetHeight) - drag.shellPanY }
              : { x: snap.x - drag.shellPanX, y: snap.y - drag.shellPanY }
          : { x: drag.x, y: drag.y };
      panel.style.setProperty("--bestie-x", `${next.x}px`);
      panel.style.setProperty("--bestie-y", `${next.y}px`);
      setPosition(next);
    } else {
      panel.style.removeProperty("--bestie-x");
      panel.style.removeProperty("--bestie-y");
    }
    drag.pointerId = -1;
  };

  useEffect(() => {
    const move = (event: PointerEvent) => updateDrag(event as unknown as ReactPointerEvent<HTMLElement>);
    const finish = (event: PointerEvent) => endDrag(event as unknown as ReactPointerEvent<HTMLElement>);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
    };
  });

  const closeChat = () => {
    removeMergedTarget();
    clearPreview();
    setMergedEdge(null);
    setMergedToStickyPanel(false);
    setSlotSize(null);
    setMergedRadius(null);
    onClose();
  };

  return (
    <aside
      ref={panelRef}
      className={`content-surface bestie-chat${detached ? " is-detached" : ""}${contextPanel ? " is-context-panel" : ""}${mergedEdge ? ` is-connected is-merged-${mergedEdge}` : ""}${mergedToStickyPanel ? " is-merged-sticky" : ""}${slotSize ? " is-slotted" : ""}`}
      role="dialog"
      aria-label="Chat with Bestie"
      style={detached ? ({ "--bestie-x": `${position.x}px`, "--bestie-y": `${position.y}px`, ...(slotSize ? { "--bestie-slot-width": `${slotSize.width}px`, "--bestie-slot-height": `${slotSize.height}px` } : {}), ...(mergedRadius ? { "--bestie-merged-radius": mergedRadius } : {}), ...(freeSize ? { "--bestie-free-width": `${freeSize.width}px`, "--bestie-free-height": `${freeSize.height}px` } : {}) } as CSSProperties) : undefined}
    >
      <header
        className="bestie-chat-header"
        onPointerDown={beginDrag}
        onPointerMove={updateDrag}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onLostPointerCapture={endDrag}
      >
        <div><img src="/snek.png" alt="" /><strong>Bestie</strong></div>
        <button type="button" aria-label="Close Bestie chat" onClick={closeChat}><X aria-hidden="true" size={18} strokeWidth={1.7} /></button>
      </header>
      <div className="bestie-chat-content">
        <div className="bestie-message is-bestie"><img src="/snek.png" alt="" /><p>{contextTag ? <span className="bestie-context-tag">{contextTag}</span> : null}{contextLabel ? `How can I help with ${contextLabel}?` : "Hi! What are you working through?"}</p></div>
        <div className="bestie-message is-user"><p>I want to make these panels feel more composable.</p></div>
        <div className="bestie-message is-bestie"><img src="/snek.png" alt="" /><p>I can help with that. Drag me into the workspace when you want to keep this chat around.</p></div>
      </div>
      <div className="bestie-composer"><input aria-label="Message Bestie" placeholder="How can Bestie help?" /><button type="button" aria-label="Send to Bestie"><ArrowUp aria-hidden="true" size={18} strokeWidth={1.8} /></button></div>
      {detached ? (mergedEdge || contextPanel ? <button type="button" className="panel-resize-handle is-width-only is-right-edge" aria-label="Resize Bestie width" onPointerDown={(event) => beginResize(event, "right")} onPointerMove={updateResize} onPointerUp={endResize} onPointerCancel={endResize} onLostPointerCapture={endResize} /> : <><button type="button" className="panel-resize-handle is-height-only is-top-edge" aria-label="Resize Bestie from the top edge" onPointerDown={(event) => beginResize(event, "top")} onPointerMove={updateResize} onPointerUp={endResize} onPointerCancel={endResize} onLostPointerCapture={endResize} /><button type="button" className="panel-resize-handle is-width-only is-right-edge" aria-label="Resize Bestie from the right edge" onPointerDown={(event) => beginResize(event, "right")} onPointerMove={updateResize} onPointerUp={endResize} onPointerCancel={endResize} onLostPointerCapture={endResize} /><button type="button" className="panel-resize-handle is-height-only is-bottom-edge" aria-label="Resize Bestie from the bottom edge" onPointerDown={(event) => beginResize(event, "bottom")} onPointerMove={updateResize} onPointerUp={endResize} onPointerCancel={endResize} onLostPointerCapture={endResize} /></>) : null}
    </aside>
  );
}

type CatchUpItem = {
  id: string;
  time: string;
  avatar: string;
  author: string;
  title: string;
  messages: readonly string[];
  channel: string;
};

const CATCH_UP_ITEMS: readonly CatchUpItem[] = [
  {
    id: "dessa-updates",
    time: "Today, 10:32 AM",
    avatar: USER_PROFILES.simon,
    author: "Simon",
    title: "asked for the latest updates on the Dessa prototype.",
    messages: ["btw did you push the multi voice changes to the repo?", "did you have a few alts on the multi voice selection?"],
    channel: "#buzz-interface",
  },
  {
    id: "cube-motion",
    time: "Today, 9:18 AM",
    avatar: USER_PROFILES.morgan,
    author: "Morgan",
    title: "flagged a motion detail on the project cube prototype.",
    messages: ["the hover state still feels a little abrupt", "could the noodles settle with the same easing as the cube?"],
    channel: "#project-cubes",
  },
  {
    id: "launch-copy",
    time: "Yesterday, 4:46 PM",
    avatar: USER_PROFILES.arj,
    author: "Arj",
    title: "needs a decision on the launch announcement copy.",
    messages: ["I added two shorter headline options", "which direction should I take into the final review?"],
    channel: "#launch-planning",
  },
  {
    id: "agent-handoff",
    time: "Yesterday, 2:05 PM",
    avatar: USER_PROFILES.caroline,
    author: "Tulsi",
    title: "shared a question about the agent handoff flow.",
    messages: ["should the receiving agent acknowledge before work moves over?", "I can prototype either state today"],
    channel: "#agent-tools",
  },
];

function CatchUpView({ bestiePanelOpen = false, bestieContext = null }: { bestiePanelOpen?: boolean; bestieContext?: string | null }) {
  const [activeIndex, setActiveIndex] = useState(0);
  const itemAt = (offset: number) => CATCH_UP_ITEMS[(activeIndex + offset + CATCH_UP_ITEMS.length) % CATCH_UP_ITEMS.length];
  const selectOffset = (offset: number) => setActiveIndex((current) => (current + offset + CATCH_UP_ITEMS.length) % CATCH_UP_ITEMS.length);
  const referenceClass = (context: string) => bestiePanelOpen && bestieContext === context ? " is-bestie-reference" : "";

  return (
    <section className="catch-up-focus" aria-label="Catch up" aria-live="polite">
      <button type="button" className="catch-up-arrow is-previous" aria-label="Previous catch up" onClick={() => selectOffset(-1)}><ChevronLeft aria-hidden="true" size={20} strokeWidth={1.8} /></button>
      <div className="catch-up-carousel">
        {([-1, 0, 1] as const).map((offset) => {
          const item = itemAt(offset);
          const position = offset === 0 ? "current" : offset < 0 ? "previous" : "next";
          return (
            <article
              key={item.id}
              data-bestie-module-target={offset === 0 ? "true" : undefined}
              data-bestie-context={offset === 0 ? `messages:catchup:${item.id}` : undefined}
              className={`content-surface catch-up-card is-${position}${offset === 0 ? referenceClass(`messages:catchup:${item.id}`) : ""}`}
              aria-label={offset === 0 ? undefined : `${offset < 0 ? "Previous" : "Next"} catch up: ${item.author} ${item.title}`}
              role={offset === 0 ? undefined : "button"}
              tabIndex={offset === 0 ? undefined : 0}
              onClick={offset === 0 ? undefined : () => selectOffset(offset)}
              onKeyDown={offset === 0 ? undefined : (event) => {
                if (event.key !== "Enter" && event.key !== " ") return;
                event.preventDefault();
                selectOffset(offset);
              }}
            >
              <span className="catch-up-time">{item.time}</span>
              <div className="catch-up-card-body">
                <img className="catch-up-avatar" src={item.avatar} alt="" />
                <h2><strong>{item.author}</strong> {item.title}</h2>
                <div className="catch-up-message-stack">
                  {item.messages.map((message, messageIndex) => {
                    const messageContext = `messages:catchup:${item.id}:message:${messageIndex}`;
                    return <p key={message} className={`bestie-content-drop-target${referenceClass(messageContext)}`} data-bestie-context={messageContext}>{message}</p>;
                  })}
                </div>
              </div>
              {offset === 0 ? <MessageComposer /> : null}
            </article>
          );
        })}
      </div>
      <button type="button" className="catch-up-arrow is-next" aria-label="Next catch up" onClick={() => selectOffset(1)}><ChevronRight aria-hidden="true" size={20} strokeWidth={1.8} /></button>
      <span className="catch-up-position">{activeIndex + 1} of {CATCH_UP_ITEMS.length}</span>
    </section>
  );
}

type MeContextId = "attention" | "assigned" | string;

type MeTask = {
  id: string;
  title: string;
  project: string;
  summary: string;
  assignee: keyof typeof USER_PROFILES;
  assigneeName: string;
};

const ME_TASKS: readonly MeTask[] = [
  { id: "PR-123", title: "PR-123 code review is complete", project: "Berd", summary: "Bestie used /code-review-loop to review PR-123 for any issues, and identified 5 potential issues: 3 are blocking and 2 are recommended to fix.", assignee: "cynthia", assigneeName: "Cynthia" },
  { id: "BERD-204", title: "Berd Strategy v2 doc is ready to review", project: "Berd", summary: "The updated strategy document is ready for your final review before it is shared with the broader project team.", assignee: "morgan", assigneeName: "Morgan Martin" },
  { id: "MOBILE-88", title: "Agent-error-fix worktree is set up", project: "Mobile", summary: "A clean worktree is ready with the error state isolated and the relevant test fixtures already running.", assignee: "arj", assigneeName: "Arj" },
  { id: "MSG-41", title: "Morgan is asking about offsite meal plans", project: "Mobile", summary: "Morgan needs a quick preference check before the offsite meal order is finalized this afternoon.", assignee: "caroline", assigneeName: "Caroline McKenzie" },
  { id: "GOOSE-72", title: "Arjun, Kenny, and Taylor asked about UI patterns", project: "Goose", summary: "Three related questions about the new composable container patterns are waiting for a single shared response.", assignee: "simon", assigneeName: "Simon" },
  { id: "GOOSE-81", title: "Agent handoff copy needs a decision", project: "Goose", summary: "Choose between the concise and descriptive handoff states before the next prototype review.", assignee: "cynthia", assigneeName: "Cynthia" },
];

type CodeReviewIssue = {
  id: string;
  title: string;
  severity: "P0" | "P1";
  status: string;
  summary: string;
  location: string;
  impact: string;
  recommendation: string;
};

const CODE_REVIEW_ISSUES: readonly CodeReviewIssue[] = [
  { id: "#1249", title: "Missing Spanish localization support", severity: "P0", status: "Open", summary: "Spanish locale requests fall back to an incomplete English review state.", location: "packages/interface/src/i18n/locale-resolver.ts · line 124", impact: "Spanish-language users see mixed-language copy and cannot confidently complete the review flow.", recommendation: "Add the missing locale mapping and translated fallback copy, then cover supported and unknown locales with a regression test." },
  { id: "#1253", title: "Token refresh can discard an active session", severity: "P0", status: "Open", summary: "Two simultaneous refresh requests can overwrite the newest authentication token.", location: "packages/auth/src/session/refresh-session.ts · line 88", impact: "A signed-in user can be unexpectedly returned to the login screen while submitting review feedback.", recommendation: "Serialize refresh attempts per session and ignore stale responses using the token generation timestamp." },
  { id: "#1258", title: "Review submission bypasses permission check", severity: "P0", status: "Needs fix", summary: "The direct submit path does not verify that the reviewer still has write access.", location: "apps/web/src/reviews/actions/submit-review.ts · line 61", impact: "A user whose role changed after loading the page could submit a review they are no longer authorized to modify.", recommendation: "Re-check workspace and repository permissions server-side immediately before persisting the review." },
  { id: "#1264", title: "Empty-state retry silently does nothing", severity: "P1", status: "Open", summary: "The retry action remains disabled after the first failed request resolves.", location: "apps/web/src/reviews/components/review-empty-state.tsx · line 47", impact: "Users can get trapped in the empty state and must refresh the entire page to try loading comments again.", recommendation: "Reset the request state after failure and add an interaction test for repeated retry attempts." },
  { id: "#1271", title: "Analytics event omits workspace ID", severity: "P1", status: "Open", summary: "The review_completed event is emitted without its workspace identifier.", location: "packages/analytics/src/events/review-events.ts · line 203", impact: "Completion metrics cannot be reliably grouped by workspace, weakening rollout and adoption reporting.", recommendation: "Pass the active workspace ID into the event payload and validate the required field in the analytics schema." },
];

function AnalogClock() {
  const [time, setTime] = useState(() => new Date());
  useEffect(() => {
    const interval = window.setInterval(() => setTime(new Date()), 1000);
    return () => window.clearInterval(interval);
  }, []);
  const secondAngle = time.getSeconds() * 6;
  const minuteAngle = time.getMinutes() * 6 + time.getSeconds() * 0.1;
  const hourAngle = ((time.getHours() % 12) + time.getMinutes() / 60) * 30;
  return (
    <div className="me-clock" role="timer" aria-label={time.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}>
      {Array.from({ length: 60 }, (_, index) => <span key={index} className={index % 5 === 0 ? "hour-tick" : "minute-tick"} style={{ transform: `rotate(${index * 6}deg)` }} />)}
      <i className="clock-hand hour-hand" style={{ transform: `translateX(-50%) rotate(${hourAngle}deg)` }} />
      <i className="clock-hand minute-hand" style={{ transform: `translateX(-50%) rotate(${minuteAngle}deg)` }} />
      <i className="clock-hand second-hand" style={{ transform: `translateX(-50%) rotate(${secondAngle}deg)` }} />
      <b className="clock-hub" />
    </div>
  );
}

function MeTaskRow({ task, onSelect }: { task: MeTask; onSelect: (task: MeTask) => void }) {
  return (
    <button type="button" className="me-task-row" onClick={() => onSelect(task)}>
      <span className="me-task-type-icon" aria-hidden="true"><ClipboardList size={17} strokeWidth={1.6} /></span>
      <span className="me-task-copy"><small>{task.id}</small><strong>{task.title}</strong></span>
      <span className="me-task-assignee" aria-label={`Assigned to ${task.assigneeName}`}><img src={USER_PROFILES[task.assignee]} alt="" /></span>
    </button>
  );
}

function MeContextChat({ context, onClose, position, className = "" }: { context: string; onClose: () => void; position: { left: number; top: number } | null; className?: string }) {
  return (
    <aside className={`content-surface me-context-chat${className ? ` ${className}` : ""}`} aria-label={`Bestie chat for ${context}`} style={position ? { left: position.left, top: position.top } : undefined}>
      <header><span><img src="/snek.png" alt="" />Bestie</span><button type="button" aria-label="Close Bestie chat" onClick={onClose}><X aria-hidden="true" size={17} /></button></header>
      <div className="me-context-messages">
        <p><span className="bestie-context-tag">{bestieContextTag(context)}</span>How can I help with {context}?</p>
        <p>Go through the details and help me make a plan for the next steps.</p>
      </div>
      <div className="me-context-input">How can Bestie help?<button type="button" aria-label="Send"><ArrowUp aria-hidden="true" size={18} /></button></div>
    </aside>
  );
}

function WorkspaceCreateView({ onClose }: { onClose: () => void }) {
  const [draft, setDraft] = useState("");
  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);

  return (
    <div className="workspace-create-layer" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <article className="workspace-create-card" role="dialog" aria-modal="true" aria-label="Create a workspace">
        <p>What workspaces are you<br />looking to build?</p>
        <form onSubmit={(event) => { event.preventDefault(); if (!draft.trim()) return; }}>
          <input autoFocus aria-label="Workspace description" value={draft} onChange={(event) => setDraft(event.target.value)} placeholder="A task workspace..." />
          <button type="submit" aria-label="Create workspace"><ArrowUp aria-hidden="true" size={20} strokeWidth={1.8} /></button>
        </form>
      </article>
    </div>
  );
}

function MeFocusView({ task, cardIndex, view, bestieContext, onClose, onCloseBestie }: { task: MeTask; cardIndex: number; view: "feed" | "task"; bestieContext: string | null; onClose: () => void; onCloseBestie: () => void }) {
  const [detailItem, setDetailItem] = useState<number | null>(null);
  const [focusChatOpen, setFocusChatOpen] = useState(false);
  const [focusChatDraft, setFocusChatDraft] = useState("");
  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (detailItem !== null) setDetailItem(null);
      else onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [detailItem, onClose]);
  const isTaskFocus = view === "task";
  const isReview = !isTaskFocus && cardIndex === 0;
  const reviewIssue = detailItem !== null && isReview ? CODE_REVIEW_ISSUES[detailItem] : null;
  const itemCopy = (item: number) => isReview
    ? CODE_REVIEW_ISSUES[item]?.summary ?? "A code review issue needs attention."
    : `${item === 0 ? "Morgan" : item === 1 ? "Taylor" : "Arjun"} asked for an update on the interaction patterns and next prototype review.`;
  const issueContext = (item: number) => `${task.id}:issue:${item}`;
  const itemIndexes = isReview ? CODE_REVIEW_ISSUES.map((_, item) => item) : [0, 1, 2];
  const pairedIssueMatch = bestieContext?.match(new RegExp(`^${task.id}:issue:(\\d+)$`));
  const pairedIssue = pairedIssueMatch ? CODE_REVIEW_ISSUES[Number(pairedIssueMatch[1])] : null;
  const hasPairedBestie = bestieContext === task.id || Boolean(pairedIssueMatch);
  const pairedBestieLabel = pairedIssue
    ? `code review issue ${pairedIssue.id}: ${pairedIssue.title.toLowerCase()}`
    : isTaskFocus || isReview ? task.title : "#interface-squad";
  const focusChatSubject = reviewIssue ? `${reviewIssue.id} ${reviewIssue.title}` : isTaskFocus || isReview ? task.title : "#interface-squad";
  const focusFooter = (primaryLabel: string) => (
    <footer className={focusChatOpen ? "is-chatting" : undefined}>
      {focusChatOpen ? (
        <form className="me-focus-inline-chat" onSubmit={(event) => { event.preventDefault(); if (!focusChatDraft.trim()) return; setFocusChatOpen(false); setFocusChatDraft(""); }}>
          <input autoFocus aria-label={`Chat about ${focusChatSubject}`} value={focusChatDraft} onChange={(event) => setFocusChatDraft(event.target.value)} placeholder={`Ask about ${focusChatSubject}`} />
          <button type="submit" aria-label="Send chat"><ArrowUp aria-hidden="true" size={18} /></button>
        </form>
      ) : <><div><button type="button" aria-label={`Mark done: ${focusChatSubject}`}><Check aria-hidden="true" size={17} /></button><button type="button" aria-label={`Start chat about ${focusChatSubject}`} onClick={() => { setFocusChatOpen(true); setFocusChatDraft(""); }}><MessageSquareText aria-hidden="true" size={17} /></button></div><button type="button">{primaryLabel}<ChevronDown aria-hidden="true" size={16} /></button></>}
    </footer>
  );
  return (
    <div className="me-focus-layer" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div className={`me-focus-composition${hasPairedBestie ? " has-home-bestie-chat" : ""}`}>
        <article className={`content-surface me-focus-card ${isReview ? "is-review-focus" : "is-thread-focus"}${isTaskFocus ? " is-home-task-focus" : ""}${detailItem !== null ? " is-detail-view" : ""}${bestieContext === task.id ? " is-bestie-connected" : ""}`} data-bestie-context={task.id} role="dialog" aria-modal="false" aria-label={detailItem !== null ? `Details for issue ${detailItem + 1}` : task.title}>
          {isTaskFocus ? <>
            <header><span className="me-source-swatch"><ClipboardList aria-hidden="true" size={17} /></span><span>{task.title}</span><button type="button" aria-label="Close task focus view" onClick={onClose}><X aria-hidden="true" size={17} /></button></header>
            <div className="me-task-focus-content">
              <div className="me-task-bubble-meta">
                <p><span>Project</span><strong>{task.project}</strong></p>
                <p><span>Priority</span><strong>{task.id === "PR-123" ? "P0" : "P1"}</strong></p>
                <p><span>Due date</span><strong>September 1, 2026</strong></p>
              </div>
              <div className="me-task-bubble-description"><span>Description</span><p>{task.summary}</p></div>
              <section className="project-task-agents me-task-focus-agents">
                <span>Agents</span>
                <div aria-label="Agents working on this task">
                  <img src="/berd-agent-avatars/berdy-gloopies-22.png" alt="Berdy" />
                  <img src="/berd-agent-avatars/builderbot-gloopies-20.png" alt="Builderbot" />
                  <img src="/berd-agent-avatars/pushback-gloopies-5.png" alt="Pushback" />
                </div>
              </section>
            </div>
            {focusFooter("Review comments")}
          </> : detailItem !== null ? <>
            <header className="me-focus-secondary-header"><button type="button" aria-label="Back to code review" onClick={() => setDetailItem(null)}><ChevronLeft aria-hidden="true" size={19} /></button><span>{reviewIssue ? `${reviewIssue.id} ${reviewIssue.title}` : "UI pattern update"}</span><span className="me-focus-detail-actions" aria-hidden="true"><SquareDashed size={18} strokeWidth={1.5} /><SquareDashed size={18} strokeWidth={1.5} /></span></header>
            <div className="me-focus-issue-detail">
              <div className="me-focus-issue-status"><span>{reviewIssue ? `${reviewIssue.severity} · ${reviewIssue.severity === "P0" ? "Blocking" : "Recommended"}` : "Needs attention"}</span><span>{reviewIssue?.status ?? "Open"}</span></div>
              <section><span>Issue</span><p>{itemCopy(detailItem)}</p></section>
              <section><span>Location</span><p>{reviewIssue?.location ?? "packages/interface/src/components/review.tsx · line 42"}</p></section>
              <section><span>Why it matters</span><p>{reviewIssue?.impact ?? "The current behavior makes the review state difficult to understand and complete."}</p></section>
              <section><span>Recommended fix</span><p>{reviewIssue?.recommendation ?? "Update the interaction and add coverage for the affected state."}</p></section>
            </div>
            {focusFooter("Fix issue")}
          </> : <>
            <header><span className={`me-source-swatch swatch-${cardIndex}`}>{isReview ? <GitBranch aria-hidden="true" size={17} /> : <MessageSquareText aria-hidden="true" size={17} />}</span><span>{isReview ? task.title : "#interface-squad"}</span><button type="button" aria-label="Close focus view" onClick={onClose}><X aria-hidden="true" size={17} /></button></header>
            <div className="me-focus-summary">{isReview ? <>Bestie used <code>/code-review-loop</code> and identified 3 blocking P0s and 2 P1s.</> : "Morgan and Taylor asked about the latest updates to UI system"}</div>
            <div className="me-focus-items">
              {itemIndexes.map((item) => <section key={item} className={bestieContext === issueContext(item) ? "is-bestie-connected" : ""} data-bestie-context={issueContext(item)} role="button" tabIndex={0} aria-label={`View details for ${CODE_REVIEW_ISSUES[item]?.id ?? `issue ${item + 1}`}`} onClick={() => setDetailItem(item)} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); setDetailItem(item); } }}><span>{CODE_REVIEW_ISSUES[item]?.id ?? "#1249"}</span><p>{itemCopy(item)}</p><span className="me-focus-item-actions" aria-hidden="true"><SquareDashed size={18} strokeWidth={1.5} /><SquareDashed size={18} strokeWidth={1.5} /></span></section>)}
            </div>
            {focusFooter(isReview ? "Fix comments" : "Respond to thread")}
          </>}
        </article>
        {hasPairedBestie ? <MeContextChat className="me-focus-context-chat" context={pairedBestieLabel} position={null} onClose={onCloseBestie} /> : null}
      </div>
    </div>
  );
}

function MeView({ bestieContext, onClearBestie }: { bestieContext: string | null; onClearBestie: () => void }) {
  const [mode, setMode] = useState<"attention" | "tasks">("attention");
  const [contentScrolledUnderToolbar, setContentScrolledUnderToolbar] = useState(false);
  const [chatPosition, setChatPosition] = useState<{ left: number; top: number } | null>(null);
  const [focusedCard, setFocusedCard] = useState<{ task: MeTask; index: number; view: "feed" | "task" } | null>(null);
  const [inlineChatTask, setInlineChatTask] = useState<string | null>(null);
  const [inlineChatDraft, setInlineChatDraft] = useState("");
  const reviewIssueMatch = bestieContext?.match(/^PR-123:issue:(\d+)$/);
  const bestieReviewIssue = reviewIssueMatch ? CODE_REVIEW_ISSUES[Number(reviewIssueMatch[1])] : null;
  const contextLabel = bestieContext === "attention"
    ? "what needs your attention"
    : bestieContext === "assigned"
      ? "your assigned tasks"
      : bestieReviewIssue
        ? `code review issue ${bestieReviewIssue.id}: ${bestieReviewIssue.title.toLowerCase()}`
      : bestieContext === "PR-123"
        ? "PR-123"
      : bestieContext === "GOOSE-72"
        ? "the #interface-squad update"
        : ME_TASKS.find((task) => task.id === bestieContext)?.title ?? bestieContext;
  const connectedClass = (context: string) => bestieContext === context ? " is-bestie-connected" : "";

  useEffect(() => {
    if (!bestieContext) { setChatPosition(null); return; }
    let settleTimer = 0;
    const positionChat = () => {
      const matchingTargets = document.querySelectorAll<HTMLElement>(`[data-bestie-context="${CSS.escape(bestieContext)}"]`);
      const target = matchingTargets.item(matchingTargets.length - 1);
      const dashboard = document.querySelector<HTMLElement>(".me-dashboard");
      if (!target) return;
      const rect = target.getBoundingClientRect();
      const matrix = dashboard ? new DOMMatrixReadOnly(getComputedStyle(dashboard).transform) : null;
      const remainingDashboardShift = dashboard?.contains(target) ? -190 - (matrix?.m41 ?? 0) : 0;
      const projectedLeft = rect.left + remainingDashboardShift;
      const projectedRight = rect.right + remainingDashboardShift;
      const chatWidth = Math.min(390, window.innerWidth - 32);
      const availableOnRight = window.innerWidth - projectedRight - 16;
      const left = availableOnRight >= chatWidth ? projectedRight + 12 : Math.max(16, projectedLeft - chatWidth - 12);
      const top = Math.max(82, rect.top);
      setChatPosition({ left, top });
    };
    positionChat();
    settleTimer = window.setTimeout(positionChat, 440);
    window.addEventListener("resize", positionChat);
    return () => { window.clearTimeout(settleTimer); window.removeEventListener("resize", positionChat); };
  }, [bestieContext, mode]);

  return (
    <section className="me-stage" aria-label="Me dashboard" data-bestie-context={mode === "tasks" ? "assigned" : "attention"}>
      <div className="me-floating me-cow"><img src="/me-cow.png" alt="" /></div>
      <div className="me-floating me-dog"><img src="/me-dog.png" alt="" /></div>
      <div className="me-floating me-cube me-cube-berd"><ProjectArtifactRenderer state={PRIMARY_PROJECT_STATE} imageUrls={PRIMARY_IMAGE_URLS} environmentUrl="/berd-project-assets/studio_soft.exr" variant="tile" cameraDistanceScale={1.35} /></div>
      <div className="me-floating me-cube me-cube-goose"><ProjectArtifactRenderer state={SECONDARY_PROJECT_STATE} imageUrls={SECONDARY_IMAGE_URLS} environmentUrl="/berd-project-assets/studio_soft.exr" variant="tile" cameraDistanceScale={1.35} /></div>
      <div className="me-floating me-cube me-cube-buzz"><ProjectArtifactRenderer state={TERTIARY_PROJECT_STATE} imageUrls={TERTIARY_IMAGE_URLS} environmentUrl="/berd-project-assets/studio_soft.exr" variant="tile" cameraDistanceScale={1.35} /></div>
      <img className="me-floating me-agent" src="/berd-agent-avatars/pushback-gloopies-5.png" alt="" />
      <AnalogClock />

      <div className={`me-dashboard${bestieContext ? " has-context-chat" : ""}`}>
        <div className="me-toolbar">
          <SegmentedNavigation activeValue={mode} ariaLabel="Home content" itemWidth={116} items={[{ value: "attention", label: "For you", badge: 4 }, { value: "tasks", label: "My tasks", badge: ME_TASKS.length }]} onChange={(nextMode) => { setMode(nextMode); setContentScrolledUnderToolbar(false); setInlineChatTask(null); setInlineChatDraft(""); }} />
          <button type="button" className="me-filter-button" aria-label="Filter"><SlidersHorizontal aria-hidden="true" size={14} strokeWidth={1.6} /></button>
        </div>

        {mode === "attention" ? (
          <div className={`me-card-stack me-for-you-stack${contentScrolledUnderToolbar ? " is-scrolled-under-toolbar" : ""}`} onScroll={(event) => setContentScrolledUnderToolbar(event.currentTarget.scrollTop > 8)}>
            {[ME_TASKS[0], ME_TASKS[4], ME_TASKS[2], ME_TASKS[1]].map((task, index) => (
              <article key={task.id} className={`content-surface me-glass-card me-for-you-card${inlineChatTask === task.id ? " is-chatting" : ""}${connectedClass(task.id)}`} data-bestie-context={task.id} role="button" tabIndex={0} aria-label={`Focus ${task.title}`} onClick={(event) => { if (!(event.target as Element).closest("button, input, form")) setFocusedCard({ task, index, view: "feed" }); }} onKeyDown={(event) => { if ((event.target as Element).closest("button, input, form")) return; if (event.key === "Enter" || event.key === " ") { event.preventDefault(); setFocusedCard({ task, index, view: "feed" }); } }}>
                <header><span className={`me-source-swatch swatch-${index}`}>{index === 0 ? <GitBranch aria-hidden="true" size={17} /> : <MessageSquareText aria-hidden="true" size={17} />}</span><span>{index === 1 ? "#interface-squad" : task.title}</span></header>
                <div className="me-for-you-copy">{index === 0 ? <>Bestie used <code>/code-review-loop</code> and identified 3 blocking P0s and 2 P1s.</> : index === 1 ? <>Morgan and Taylor asked about the latest updates to UI system<div className="me-card-contributors"><p><img src={USER_PROFILES.morgan} alt="" /><span><strong>Morgan Martin</strong><small>Yesterday at 7:05PM</small><em>Could we tighten the empty state before tomorrow’s review?</em></span></p><p><img src={USER_PROFILES.simon} alt="" /><span><strong>Taylor Ho</strong><small>Yesterday at 7:05PM</small><em>I added updated spacing notes to the latest prototype.</em></span></p></div></> : task.summary}</div>
                <footer>{inlineChatTask === task.id ? <form className="me-card-inline-chat" onClick={(event) => event.stopPropagation()} onSubmit={(event) => { event.preventDefault(); if (!inlineChatDraft.trim()) return; setInlineChatTask(null); setInlineChatDraft(""); }}><input autoFocus aria-label={`Chat about ${task.title}`} value={inlineChatDraft} onChange={(event) => setInlineChatDraft(event.target.value)} placeholder={`Ask about ${index === 1 ? "#interface-squad" : task.id}`} /><button type="submit" aria-label="Send chat"><ArrowUp aria-hidden="true" size={18} /></button></form> : <><div><button type="button" aria-label={`Mark complete: ${task.title}`}><Check aria-hidden="true" size={17} /></button><button type="button" aria-label={`Start chat about ${task.title}`} onClick={(event) => { event.stopPropagation(); setInlineChatTask(task.id); setInlineChatDraft(""); }}><MessageSquareText aria-hidden="true" size={17} /></button></div><button type="button" onClick={() => setFocusedCard({ task, index, view: "feed" })}>{index === 0 ? "Review 5 issues" : index === 1 ? "Reply to Morgan & Taylor" : index === 2 ? "Open worktree" : "Review strategy"}</button></>}</footer>
              </article>
            ))}
          </div>
        ) : (
          <div className={`me-task-containers${contentScrolledUnderToolbar ? " is-scrolled-under-toolbar" : ""}`} onScroll={(event) => setContentScrolledUnderToolbar(event.currentTarget.scrollTop > 8)}>
            <article className={`content-surface me-glass-card me-assigned-card task-list-surface${connectedClass("assigned")}`} data-bestie-context="assigned">
              {ME_TASKS.map((task, index) => <div key={task.id} className={`me-task-list-item${connectedClass(task.id)}`} data-bestie-context={task.id}><MeTaskRow task={task} onSelect={() => setFocusedCard({ task, index, view: "task" })} /></div>)}
            </article>
          </div>
        )}
      </div>
      {bestieContext && contextLabel && !focusedCard ? <MeContextChat context={contextLabel} position={chatPosition} onClose={onClearBestie} /> : null}
      {focusedCard ? <MeFocusView task={focusedCard.task} cardIndex={focusedCard.index} view={focusedCard.view} bestieContext={bestieContext} onClose={() => setFocusedCard(null)} onCloseBestie={onClearBestie} /> : null}
    </section>
  );
}

function BestieDragButton({ open, onToggle, onContextDrop }: { open: boolean; onToggle: () => void; onContextDrop: (context: MeContextId, targetRect: DOMRect) => void }) {
  const [dragPoint, setDragPoint] = useState<{ x: number; y: number } | null>(null);
  const dragRef = useRef({ pointerId: -1, startX: 0, startY: 0, moved: false, target: null as HTMLElement | null, source: null as HTMLButtonElement | null });
  const findTarget = (x: number, y: number) => {
    const elements = document.elementsFromPoint(x, y);
    const foreground = elements.find((element): element is HTMLElement => element instanceof HTMLElement);
    if (foreground?.closest(".sidebar-cell, .channel-panel")) return undefined;
    return elements.find((element) => element instanceof HTMLElement && element.dataset.bestieContext) as HTMLElement | undefined;
  };
  const clearRightEdgePreview = () => document.querySelector<HTMLElement>(".messages-stage")?.classList.remove("is-bestie-right-edge-preview");
  const updateRightEdgePreview = (x: number, y: number) => {
    const stage = document.querySelector<HTMLElement>(".messages-stage");
    if (!stage) return;
    const rect = stage.getBoundingClientRect();
    const zoneWidth = Math.max(120, Math.min(220, rect.width * .16));
    const insideRightDropZone = x >= rect.right - zoneWidth && x <= rect.right && y >= rect.top && y <= rect.bottom;
    stage.classList.toggle("is-bestie-right-edge-preview", insideRightDropZone);
  };
  const clearTarget = () => { dragRef.current.target?.classList.remove("is-bestie-drop-target"); dragRef.current.target = null; };
  useEffect(() => {
    const moveDrag = (event: PointerEvent) => {
      if (dragRef.current.pointerId !== event.pointerId) return;
      if (!dragRef.current.moved && Math.hypot(event.clientX - dragRef.current.startX, event.clientY - dragRef.current.startY) < 6) return;
      dragRef.current.moved = true;
      setDragPoint({ x: event.clientX, y: event.clientY });
      updateRightEdgePreview(event.clientX, event.clientY);
      const target = findTarget(event.clientX, event.clientY);
      if (target === dragRef.current.target) return;
      clearTarget();
      if (target) { target.classList.add("is-bestie-drop-target"); dragRef.current.target = target; }
    };
    const finishDrag = (event: PointerEvent) => {
      if (dragRef.current.pointerId !== event.pointerId) return;
      const target = dragRef.current.target;
      const moved = dragRef.current.moved;
      const source = dragRef.current.source;
      if (source?.hasPointerCapture(event.pointerId)) source.releasePointerCapture(event.pointerId);
      clearTarget();
      clearRightEdgePreview();
      setDragPoint(null);
      dragRef.current = { pointerId: -1, startX: 0, startY: 0, moved: false, target: null, source: null };
      if (event.type === "pointercancel") return;
      if (moved && target?.dataset.bestieContext) onContextDrop(target.dataset.bestieContext, target.getBoundingClientRect());
      else if (!moved) onToggle();
    };
    window.addEventListener("pointermove", moveDrag);
    window.addEventListener("pointerup", finishDrag);
    window.addEventListener("pointercancel", finishDrag);
    return () => {
      clearRightEdgePreview();
      window.removeEventListener("pointermove", moveDrag);
      window.removeEventListener("pointerup", finishDrag);
      window.removeEventListener("pointercancel", finishDrag);
    };
  }, [onContextDrop, onToggle]);
  const onPointerDown = (event: ReactPointerEvent<HTMLButtonElement>) => {
    event.preventDefault();
    try { event.currentTarget.setPointerCapture(event.pointerId); } catch { /* Synthetic and assistive pointer sources may not support capture. */ }
    dragRef.current = { pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, moved: false, target: null, source: event.currentTarget };
  };
  return <><button type="button" className={`round-button snake-button${open ? " is-active" : ""}${dragPoint ? " is-dragging" : ""}`} aria-label="Chat with Bestie or drag Bestie onto a card" aria-expanded={open} onPointerDown={onPointerDown}><img src="/snek.png" alt="" /></button>{dragPoint ? <span className="round-button snake-button bestie-drag-preview" style={{ left: dragPoint.x, top: dragPoint.y }}><img src="/snek.png" alt="" /></span> : null}</>;
}

type ChatRowMenu = {
  id: string;
  label: string;
  left: number;
  top: number;
};

const PROJECT_CHANNEL_TASKS = [
  { id: "TSK-123", label: "Fix agent creation error", progress: "35% complete", assignedTo: "Morgan Martin", description: "Agent creation is triggering an error state on the description input field by default when it should wait for submission.", sessions: ["Bug report", "Validation fix", "QA handoff"] },
  { id: "TSK-128", label: "Review localization gaps", progress: "60% complete", assignedTo: "Tulsi Patel", description: "Review the remaining untranslated interface strings and confirm fallback behavior for Spanish and French locales.", sessions: ["String audit", "Copy review", "Locale rollout"] },
  { id: "TSK-134", label: "Polish task panel states", progress: "20% complete", assignedTo: "Caroline McKenzie", description: "Refine docked, standalone, resizing, and contextual Bestie states for the task detail panel.", sessions: ["Panel behavior", "Interaction review", "Polish pass"] },
] as const;

const DIRECT_MESSAGE_PROFILES = [
  { name: "morganmartin", image: USER_PROFILES.morgan },
  { name: "tho", image: USER_PROFILES.simon },
  { name: "tulsi", image: USER_PROFILES.arj },
  { name: "laurenkenny", image: USER_PROFILES.caroline },
] as const;

type SidebarConversation = {
  label: string;
  messages: readonly { author: string; time: string; image: string; body: string }[];
  sessionNotes: readonly [string, string];
  followUpSessions?: readonly [
    readonly { author: string; time: string; image: string; body: string }[],
    readonly { author: string; time: string; image: string; body: string }[],
  ];
};

const SIDEBAR_CONVERSATIONS: Record<string, SidebarConversation> = {
  "task:TSK-123": {
    label: "Fix agent creation error",
    messages: [
      { author: "Morgan Martin", time: "9:08AM", image: USER_PROFILES.morgan, body: "I reproduced the early validation error and isolated it to the description field blur handler." },
      { author: "Caroline McKenzie", time: "9:17AM", image: USER_PROFILES.caroline, body: "The expected behavior is to validate on submit, then clear the error as soon as the value becomes valid." },
    ],
    sessionNotes: ["Validation fix: the handler now waits for submit and preserves helper-text spacing.", "QA handoff: test first submit, correction, resubmit, and keyboard-only flows."],
    followUpSessions: [[
      { author: "Morgan Martin", time: "1:24PM", image: USER_PROFILES.morgan, body: "The fix now gates validation behind the first submit attempt and clears the error while typing." },
      { author: "Jude", time: "1:39PM", image: USER_PROFILES.arj, body: "I checked the transition states. Helper copy keeps its space, so the field no longer jumps." },
    ], [
      { author: "Simon", time: "3:06PM", image: USER_PROFILES.simon, body: "QA passed the initial submit, correction, resubmit, and keyboard-only scenarios." },
      { author: "Caroline McKenzie", time: "3:18PM", image: USER_PROFILES.caroline, body: "Great. I’m marking the visual review complete and handing this back for merge." },
    ]],
  },
  "task:TSK-128": {
    label: "Review localization gaps",
    messages: [
      { author: "Tulsi Patel", time: "10:02AM", image: USER_PROFILES.arj, body: "The audit found twelve hard-coded strings across task creation and agent handoff." },
      { author: "Simon", time: "10:20AM", image: USER_PROFILES.simon, body: "I grouped them by surface and flagged the three strings that need product-copy review." },
    ],
    sessionNotes: ["Copy review: confirm terminology for task ownership, handoff, and completion.", "Locale rollout: land Spanish first, verify fallback metrics, then enable French."],
    followUpSessions: [[
      { author: "Caroline McKenzie", time: "12:11PM", image: USER_PROFILES.caroline, body: "I rewrote the ownership and handoff strings so they stay clear without relying on English word order." },
      { author: "Tulsi Patel", time: "12:28PM", image: USER_PROFILES.arj, body: "The Spanish review is clean. French needs one shorter label in the compact task header." },
    ], [
      { author: "Simon", time: "2:02PM", image: USER_PROFILES.simon, body: "Spanish is enabled for the internal cohort and fallback events are reporting correctly." },
      { author: "Tulsi Patel", time: "2:17PM", image: USER_PROFILES.arj, body: "I’ll watch the metrics today, then enable French after the compact-label update lands." },
    ]],
  },
  "task:TSK-134": {
    label: "Polish task panel states",
    messages: [
      { author: "Caroline McKenzie", time: "11:14AM", image: USER_PROFILES.caroline, body: "The panel now matches its neighbor’s height, but the detached state still needs a clearer edge treatment." },
      { author: "Morgan Martin", time: "11:31AM", image: USER_PROFILES.morgan, body: "I’ll test the corner-radius transitions with chat, thread, task, and Bestie open together." },
    ],
    sessionNotes: ["Interaction review: verify edge handles, docking previews, and eight-pixel panel gaps.", "Polish pass: tune radius transitions and remove any layout jump during docking."],
    followUpSessions: [[
      { author: "Morgan Martin", time: "1:08PM", image: USER_PROFILES.morgan, body: "Right-edge resizing works with chat, thread, and Bestie visible. The eight-pixel gaps stay stable." },
      { author: "Jude", time: "1:22PM", image: USER_PROFILES.arj, body: "Dock previews are clear now. I only noticed a small radius jump when detaching the task panel." },
    ], [
      { author: "Caroline McKenzie", time: "4:03PM", image: USER_PROFILES.caroline, body: "I eased the radius transition and aligned the detached panel header with the connected state." },
      { author: "Morgan Martin", time: "4:19PM", image: USER_PROFILES.morgan, body: "The full composition feels stable now, including four-panel resizing at the narrow breakpoint." },
    ]],
  },
  "project:goose-main": {
    label: "goose-main",
    messages: [
      { author: "Caroline McKenzie", time: "9:12AM", image: USER_PROFILES.caroline, body: "The new handoff state is in the prototype. I kept the receiving agent visible while the transfer finishes." },
      { author: "Simon", time: "9:26AM", image: USER_PROFILES.simon, body: "Nice. Can we make the acknowledgement explicit before ownership changes?" },
    ],
    sessionNotes: ["Review follow-up: confirm the acknowledgement copy and loading state.", "Implementation plan: add the acknowledgement event, then test interrupted handoffs."],
  },
  "project:buzz-main": {
    label: "buzz-main",
    messages: [
      { author: "Morgan Martin", time: "10:04AM", image: USER_PROFILES.morgan, body: "I tightened the panel spacing and moved the resize target onto the full edge." },
      { author: "Caroline McKenzie", time: "10:18AM", image: USER_PROFILES.caroline, body: "The eight-pixel rhythm feels much clearer. I’ll check it with three panels open." },
    ],
    sessionNotes: ["Review follow-up: verify panel spacing at narrow and wide viewport sizes.", "Implementation plan: centralize the panel gap and edge-handle tokens."],
  },
  "personal:design-system": {
    label: "design-system",
    messages: [
      { author: "Caroline McKenzie", time: "Yesterday at 4:40PM", image: USER_PROFILES.caroline, body: "I added the compact navigation row and reused it for the overflow menu." },
      { author: "Jude", time: "8:15AM", image: USER_PROFILES.arj, body: "Let’s keep the labels regular weight and reserve stronger emphasis for state changes." },
    ],
    sessionNotes: ["Review follow-up: compare the compact row across navigation and menus.", "Implementation plan: publish the row height, gap, and hover fill as shared tokens."],
  },
  "personal:launch-planning": {
    label: "launch-planning",
    messages: [
      { author: "Simon", time: "Yesterday at 3:28PM", image: USER_PROFILES.simon, body: "The onboarding walkthrough is ready for another internal pass on Friday." },
      { author: "Morgan Martin", time: "8:47AM", image: USER_PROFILES.morgan, body: "I’ll collect the remaining rough edges and group them by moment in the flow." },
    ],
    sessionNotes: ["Review follow-up: collect feedback on setup, first task, and Bestie discovery.", "Implementation plan: fix blockers first, then record a clean walkthrough."],
  },
  "personal:agent-tools": {
    label: "agent-tools",
    messages: [
      { author: "Jude", time: "11:02AM", image: USER_PROFILES.arj, body: "The tool picker now remembers the last category used in each project." },
      { author: "Caroline McKenzie", time: "11:11AM", image: USER_PROFILES.caroline, body: "That’s helpful. Could search still cover tools outside the selected category?" },
    ],
    sessionNotes: ["Review follow-up: test category memory without narrowing global search.", "Implementation plan: persist the category locally and keep search results unscoped."],
  },
  "message:morganmartin": {
    label: "Morgan Martin",
    messages: [
      { author: "Morgan Martin", time: "9:36AM", image: USER_PROFILES.morgan, body: "I left a few notes on the task panel hierarchy. The stacked metadata is much easier to scan." },
      { author: "Cynthia", time: "9:42AM", image: USER_PROFILES.cynthia, body: "Great—I’m simplifying the actions next and keeping the detail panel focused." },
    ],
    sessionNotes: ["Review follow-up: resolve the remaining hierarchy notes together.", "Implementation plan: simplify the actions and validate the narrow panel layout."],
  },
  "message:tho": {
    label: "Taylor Ho",
    messages: [
      { author: "Taylor Ho", time: "Yesterday at 6:18PM", image: USER_PROFILES.simon, body: "Do you want the session tabs to preserve their selection when changing channels?" },
      { author: "Cynthia", time: "8:08AM", image: USER_PROFILES.cynthia, body: "For now I’m resetting to the first session so each channel opens predictably." },
    ],
    sessionNotes: ["Review follow-up: revisit whether session state should persist per channel.", "Implementation plan: store the active session by conversation if testing supports it."],
  },
  "message:tulsi": {
    label: "Tulsi Patel",
    messages: [
      { author: "Tulsi Patel", time: "10:22AM", image: USER_PROFILES.arj, body: "I tried dragging Bestie onto a task chip and the context highlight reads really well." },
      { author: "Cynthia", time: "10:29AM", image: USER_PROFILES.cynthia, body: "Perfect. I’m keeping that reference highlighted while the Bestie panel is open." },
    ],
    sessionNotes: ["Review follow-up: test Bestie context on messages, chips, and focus cards.", "Implementation plan: use the same context contract across every page state."],
  },
  "message:laurenkenny": {
    label: "Lauren Kenny",
    messages: [
      { author: "Lauren Kenny", time: "Yesterday at 2:54PM", image: USER_PROFILES.caroline, body: "The smaller Catch Up cards feel faster to review. The composer just needs to stay pinned to the bottom." },
      { author: "Cynthia", time: "3:01PM", image: USER_PROFILES.cynthia, body: "Fixed—the card now owns the equal inset and the composer fills the available width." },
    ],
    sessionNotes: ["Review follow-up: check the composer inset on every Catch Up card.", "Implementation plan: reuse the same inset token for compact card composers."],
  },
};

function ChatNavRow({ id, label, icon, current = false, onSelect, menuOpen, onOpenMenu }: { id: string; label: string; icon: ReactNode; current?: boolean; onSelect: () => void; menuOpen: boolean; onOpenMenu: (menu: ChatRowMenu) => void }) {
  return (
    <div className={`channel-nav-row${menuOpen ? " is-menu-open" : ""}`}>
      <button type="button" className={`channel-nav-primary${current ? " is-current" : ""}`} onClick={onSelect}>{icon}<span>{label}</span></button>
      <button
        type="button"
        className="channel-row-more"
        aria-label={`More options for ${label}`}
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        aria-controls={menuOpen ? "chat-row-menu" : undefined}
        onClick={(event) => {
          event.stopPropagation();
          const rect = event.currentTarget.getBoundingClientRect();
          const menuHeight = 140;
          onOpenMenu({
            id,
            label,
            left: Math.min(window.innerWidth - 208, Math.max(12, rect.left - 8)),
            top: rect.bottom + menuHeight + 8 > window.innerHeight ? Math.max(12, rect.top - menuHeight - 6) : rect.bottom + 6,
          });
        }}
      ><MoreHorizontal aria-hidden="true" size={18} strokeWidth={1.8} /></button>
    </div>
  );
}

function MessagesView({ onEnterCatchUp, bestiePanelOpen = false, bestiePanelWidth = BESTIE_CONTEXT_PANEL_WIDTH, bestieContext = null }: { onEnterCatchUp: () => void; bestiePanelOpen?: boolean; bestiePanelWidth?: number; bestieContext?: string | null }) {
  const [layouts, setLayouts] = useState(INITIAL_MESSAGE_LAYOUT);
  const [connections, setConnections] = useState(INITIAL_MESSAGE_CONNECTIONS);
  const [catchUpOpen, setCatchUpOpen] = useState(false);
  const [threadOpen, setThreadOpen] = useState(false);
  const [moduleLayout, setModuleLayout] = useState<"side" | "thread-top" | "thread-bottom">("side");
  const [stackSplit, setStackSplit] = useState(.5);
  const [chatVerticalSize, setChatVerticalSize] = useState<{ offsetY: number; height: number } | null>(null);
  const [messageTaskOpen, setMessageTaskOpen] = useState(false);
  const [selectedProjectTask, setSelectedProjectTask] = useState<(typeof PROJECT_CHANNEL_TASKS)[number]["id"]>(PROJECT_CHANNEL_TASKS[0].id);
  const [activeSession, setActiveSession] = useState(0);
  const [sessionSlider, setSessionSlider] = useState({ x: 0, width: 0 });
  const [activeChatRow, setActiveChatRow] = useState("project:berd-main");
  const [chatRowMenu, setChatRowMenu] = useState<ChatRowMenu | null>(null);
  const [messageTaskDocked, setMessageTaskDocked] = useState(false);
  const [messageTaskPosition, setMessageTaskPosition] = useState({ x: 0, y: 0 });
  const [messageTaskDockRect, setMessageTaskDockRect] = useState<{ left: number; top: number; width: number; height: number } | null>(null);
  const [messageTaskDockWidth, setMessageTaskDockWidth] = useState(360);
  const [messageTaskSize, setMessageTaskSize] = useState<{ width: number; height: number } | null>(null);
  const [columnWidths, setColumnWidths] = useState<Record<MessagePanelId, number> | null>(null);
  const [detachedSizes, setDetachedSizes] = useState<Partial<Record<MessagePanelId, { width: number; height: number }>>>({});
  const boardRef = useRef<HTMLDivElement>(null);
  const sessionTabsRef = useRef<HTMLElement>(null);
  const panRef = useRef({ pointerId: -1, startX: 0, startY: 0, originX: 0, originY: 0, x: 0, y: 0 });
  const moduleDragRef = useRef<{ pointerId: number; startX: number; startY: number; edge: "right" | "top" | "bottom" | null; moved: boolean; cell: HTMLElement | null }>({ pointerId: -1, startX: 0, startY: 0, edge: null, moved: false, cell: null });
  const stackResizeRef = useRef<{ pointerId: number; startY: number; startFirst: number; total: number; first: number; cell: HTMLElement | null }>({ pointerId: -1, startY: 0, startFirst: 0, total: 0, first: 0, cell: null });
  const chatVerticalResizeRef = useRef<{ pointerId: number; edge: "top" | "bottom"; startY: number; startHeight: number; startOffset: number; height: number; offset: number; cell: HTMLElement | null; pairedCell: HTMLElement | null }>({ pointerId: -1, edge: "bottom", startY: 0, startHeight: 0, startOffset: 0, height: 0, offset: 0, cell: null, pairedCell: null });
  const messageTaskDragRef = useRef({ pointerId: -1, startX: 0, startY: 0, originX: 0, originY: 0, x: 0, y: 0, preview: false, panel: null as HTMLElement | null });
  const messageTaskResizeRef = useRef<{
    pointerId: number;
    edge: "top" | "right" | "bottom";
    docked: boolean;
    startX: number;
    startY: number;
    startWidth: number;
    startHeight: number;
    width: number;
    height: number;
    startYPosition: number;
    yPosition: number;
    panel: HTMLElement | null;
  }>({ pointerId: -1, edge: "right", docked: false, startX: 0, startY: 0, startWidth: 0, startHeight: 0, width: 0, height: 0, startYPosition: 0, yPosition: 0, panel: null });
  const resizeRef = useRef<{
    id: MessagePanelId;
    pointerId: number;
    connected: boolean;
    edge: "top" | "right" | "bottom";
    neighborId: MessagePanelId | null;
    startX: number;
    startY: number;
    startWidth: number;
    startHeight: number;
    neighborWidth: number;
    width: number;
    height: number;
    startOffsetY: number;
    offsetY: number;
    columns: Record<MessagePanelId, number>;
    cell: HTMLElement;
  } | null>(null);
  const bestieReferenceClass = (context: string) => bestiePanelOpen && bestieContext === context ? " is-bestie-reference" : "";
  const selectedProjectTaskDetails = PROJECT_CHANNEL_TASKS.find((task) => task.id === selectedProjectTask) ?? PROJECT_CHANNEL_TASKS[0];

  useEffect(() => {
    if (!bestiePanelOpen) return;
    setModuleLayout("side");
    setColumnWidths(null);
  }, [bestiePanelOpen]);

  useEffect(() => {
    if (!chatRowMenu) return;
    const closeMenu = (event?: Event) => {
      if (event?.target instanceof Element && event.target.closest(".channel-row-more, .chat-row-menu")) return;
      setChatRowMenu(null);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setChatRowMenu(null);
    };
    document.addEventListener("pointerdown", closeMenu);
    window.addEventListener("keydown", closeOnEscape);
    window.addEventListener("resize", closeMenu);
    document.querySelector<HTMLElement>(".channel-list")?.addEventListener("scroll", closeMenu, { passive: true });
    return () => {
      document.removeEventListener("pointerdown", closeMenu);
      window.removeEventListener("keydown", closeOnEscape);
      window.removeEventListener("resize", closeMenu);
      document.querySelector<HTMLElement>(".channel-list")?.removeEventListener("scroll", closeMenu);
    };
  }, [chatRowMenu]);

  const measureMessageTaskDock = () => {
    const stage = boardRef.current?.closest<HTMLElement>(".messages-stage");
    const board = boardRef.current;
    const neighborPanel = threadOpen
      ? board?.querySelector<HTMLElement>('[data-panel-id="thread"] .message-panel')
      : board?.querySelector<HTMLElement>('[data-panel-id="chat"] .message-panel');
    if (!stage || !board || !neighborPanel) return null;
    const stageRect = stage.getBoundingClientRect();
    const boardRect = board.getBoundingClientRect();
    const neighborRect = neighborPanel.getBoundingClientRect();
    return {
      left: boardRect.right - stageRect.left + 8,
      top: neighborRect.top - stageRect.top,
      width: messageTaskDockWidth,
      height: neighborRect.height,
    };
  };

  const closeMessageTask = () => {
    setMessageTaskOpen(false);
    setMessageTaskDocked(false);
    setMessageTaskDockRect(null);
    setMessageTaskPosition({ x: 0, y: 0 });
    setMessageTaskSize(null);
  };

  const openMessageTask = () => {
    setMessageTaskDockWidth(Math.max(310, Math.min(430, window.innerWidth * .23)));
    setMessageTaskDockRect(null);
    setMessageTaskSize(null);
    setColumnWidths(null);
    setMessageTaskDocked(true);
    setMessageTaskOpen(true);
  };

  useEffect(() => {
    if (!messageTaskOpen) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        closeMessageTask();
      }
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [messageTaskOpen]);

  useEffect(() => {
    if (!messageTaskDocked) return;
    const measureDock = () => {
      const rect = measureMessageTaskDock();
      if (rect) setMessageTaskDockRect(rect);
    };
    measureDock();
    const observer = new ResizeObserver(measureDock);
    if (boardRef.current) observer.observe(boardRef.current);
    const neighborPanel = threadOpen
      ? boardRef.current?.querySelector<HTMLElement>('[data-panel-id="thread"] .message-panel')
      : boardRef.current?.querySelector<HTMLElement>('[data-panel-id="chat"] .message-panel');
    if (neighborPanel) observer.observe(neighborPanel);
    window.addEventListener("resize", measureDock);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", measureDock);
    };
  }, [messageTaskDocked, messageTaskDockWidth, columnWidths, layouts, chatVerticalSize, threadOpen, moduleLayout, bestiePanelOpen]);
  const dragRef = useRef<{
    id: MessagePanelId;
    pointerId: number;
    startX: number;
    startY: number;
    originX: number;
    originY: number;
    cell: HTMLElement;
    x: number;
    y: number;
    connectedAtStart: boolean;
    hasBroken: boolean;
    previewCell: HTMLElement | null;
    previewEdge: MessagePanelEdge | null;
  } | null>(null);

  const clearMergePreview = (drag: NonNullable<typeof dragRef.current>) => {
    if (drag.previewCell && drag.previewEdge) {
      drag.previewCell.classList.remove(`merge-preview-${drag.previewEdge}`);
    }
    drag.previewCell = null;
    drag.previewEdge = null;
  };

  const updateMergePreview = (
    drag: NonNullable<typeof dragRef.current>,
    snap: ReturnType<typeof findSnapTarget>,
  ) => {
    if (snap && drag.previewCell?.dataset.panelId === snap.target && drag.previewEdge === snap.edge) return;
    clearMergePreview(drag);
    if (!snap) return;
    const targetCell = boardRef.current?.querySelector<HTMLElement>(`[data-panel-id="${snap.target}"]`);
    if (!targetCell) return;
    targetCell.classList.add(`merge-preview-${snap.edge}`);
    drag.previewCell = targetCell;
    drag.previewEdge = snap.edge;
  };

  const isPanelConnected = (id: MessagePanelId) =>
    connections.some((connection) => connection.a === id || connection.b === id);

  const arePanelsConnected = (first: MessagePanelId, second: MessagePanelId) =>
    connections.some((connection) =>
      (connection.a === first && connection.b === second)
      || (connection.a === second && connection.b === first));

  const rightNeighborFor = (id: MessagePanelId) => {
    for (const connection of connections) {
      if (connection.a === id && connection.edge === "left") return connection.b;
      if (connection.b === id && connection.edge === "right") return connection.a;
    }
    return null;
  };

  const leftNeighborFor = (id: MessagePanelId) => {
    for (const connection of connections) {
      if (connection.a === id && connection.edge === "right") return connection.b;
      if (connection.b === id && connection.edge === "left") return connection.a;
    }
    return null;
  };

  const beginPanelResize = (event: ReactPointerEvent<HTMLButtonElement>, id: MessagePanelId, edge: "top" | "right" | "bottom") => {
    event.preventDefault();
    event.stopPropagation();
    const board = boardRef.current;
    const cell = event.currentTarget.closest<HTMLElement>(".message-panel-cell");
    const panel = cell?.querySelector<HTMLElement>(".message-panel");
    if (!board || !cell || !panel) return;
    const connected = isPanelConnected(id);
    const panelRect = panel.getBoundingClientRect();
    const neighborId = connected ? (rightNeighborFor(id) ?? leftNeighborFor(id)) : null;
    const measuredColumns = Object.fromEntries(MESSAGE_PANEL_IDS.map((panelId) => {
      const panelCell = board.querySelector<HTMLElement>(`[data-panel-id="${panelId}"]`);
      return [panelId, columnWidths?.[panelId] || panelCell?.getBoundingClientRect().width || panelRect.width];
    })) as Record<MessagePanelId, number>;
    event.currentTarget.setPointerCapture(event.pointerId);
    resizeRef.current = {
      id,
      pointerId: event.pointerId,
      connected,
      edge,
      neighborId,
      startX: event.clientX,
      startY: event.clientY,
      startWidth: connected ? measuredColumns[id] : panelRect.width,
      startHeight: panelRect.height,
      neighborWidth: neighborId ? measuredColumns[neighborId] : 0,
      width: connected ? measuredColumns[id] : panelRect.width,
      height: panelRect.height,
      startOffsetY: layouts[id].y,
      offsetY: layouts[id].y,
      columns: measuredColumns,
      cell,
    };
    cell.classList.add("is-resizing");
    if (connected) {
      board.style.gridTemplateColumns = MESSAGE_PANEL_IDS.map((panelId) => `${measuredColumns[panelId]}px`).join(" ");
    }
  };

  const updatePanelResize = (event: ReactPointerEvent<HTMLButtonElement>) => {
    const resize = resizeRef.current;
    const board = boardRef.current;
    if (!resize || resize.pointerId !== event.pointerId || !board) return;
    if (event.buttons === 0) {
      endPanelResize(event);
      return;
    }
    const deltaX = event.clientX - resize.startX;
    const deltaY = event.clientY - resize.startY;
    const minimumWidth = resize.id === "sidebar" ? 190 : 280;
    if (resize.connected) {
      const neighborMinimum = resize.neighborId === "sidebar" ? 190 : 280;
      const requestedWidthDelta = deltaX;
      const boundedDelta = resize.neighborId
        ? Math.max(minimumWidth - resize.startWidth, Math.min(resize.neighborWidth - neighborMinimum, requestedWidthDelta))
        : Math.max(minimumWidth - resize.startWidth, Math.min(420, requestedWidthDelta));
      resize.width = resize.startWidth + boundedDelta;
      resize.columns[resize.id] = resize.width;
      if (resize.neighborId) resize.columns[resize.neighborId] = resize.neighborWidth - boundedDelta;
      board.style.gridTemplateColumns = MESSAGE_PANEL_IDS.map((panelId) => `${resize.columns[panelId]}px`).join(" ");
    } else {
      if (resize.edge === "right") resize.width = Math.max(minimumWidth, Math.min(920, resize.startWidth + deltaX));
      if (resize.edge === "bottom") resize.height = Math.max(280, Math.min(window.innerHeight - 90, resize.startHeight + deltaY));
      if (resize.edge === "top") {
        const appliedDelta = Math.max(resize.startHeight - (window.innerHeight - 90), Math.min(resize.startHeight - 280, deltaY));
        resize.height = resize.startHeight - appliedDelta;
        resize.offsetY = resize.startOffsetY + appliedDelta;
        resize.cell.style.setProperty("--panel-y", `${resize.offsetY}px`);
      }
      resize.cell.style.setProperty("--free-panel-width", `${resize.width}px`);
      resize.cell.style.setProperty("--free-panel-height", `${resize.height}px`);
    }
  };

  const endPanelResize = (event: ReactPointerEvent<HTMLButtonElement>) => {
    const resize = resizeRef.current;
    if (!resize || resize.pointerId !== event.pointerId) return;
    if (event.currentTarget instanceof Element && event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    resize.cell.classList.remove("is-resizing");
    if (resize.connected) setColumnWidths({ ...resize.columns });
    else {
      setDetachedSizes((current) => ({ ...current, [resize.id]: { width: resize.width, height: resize.height } }));
      if (resize.edge === "top") setLayouts((current) => ({ ...current, [resize.id]: { ...current[resize.id], y: resize.offsetY } }));
    }
    resizeRef.current = null;
  };

  const panelResizeHandle = (id: MessagePanelId) => {
    const connected = isPanelConnected(id);
    const rightNeighbor = connected ? rightNeighborFor(id) : null;
    const leftNeighbor = connected ? leftNeighborFor(id) : null;
    if (connected && !rightNeighbor && !leftNeighbor) return null;
    const resizeNeighbor = rightNeighbor ?? leftNeighbor;
    const handle = (edge: "top" | "right" | "bottom") => (
      <button
        key={edge}
        type="button"
        className={`panel-resize-handle ${edge === "right" ? "is-width-only" : "is-height-only"} is-${edge}-edge`}
        aria-label={connected && resizeNeighbor ? `Resize ${id} and ${resizeNeighbor} widths` : `Resize ${id} from the ${edge} edge`}
        onPointerDown={(event) => beginPanelResize(event, id, edge)}
        onPointerMove={updatePanelResize}
        onPointerUp={endPanelResize}
        onPointerCancel={endPanelResize}
        onLostPointerCapture={endPanelResize}
      />
    );
    return connected ? handle("right") : <>{handle("top")}{handle("right")}{handle("bottom")}</>;
  };

  const panelRadius = (id: MessagePanelId) => {
    const corners = [28, 28, 28, 28];
    const flatten = (left: boolean, top: boolean, right: boolean, bottom: boolean) => {
      if (left && top) corners[0] = 0;
      if (right && top) corners[1] = 0;
      if (right && bottom) corners[2] = 0;
      if (left && bottom) corners[3] = 0;
    };
    connections.forEach((connection) => {
      const isA = connection.a === id;
      const isB = connection.b === id;
      if (!isA && !isB) return;
      if ((isA && connection.edge === "left") || (isB && connection.edge === "right")) flatten(false, true, true, true);
      if ((isA && connection.edge === "right") || (isB && connection.edge === "left")) flatten(true, true, false, true);
      if ((isA && connection.edge === "top") || (isB && connection.edge === "bottom")) flatten(true, false, true, true);
      if ((isA && connection.edge === "bottom") || (isB && connection.edge === "top")) flatten(true, true, true, false);
    });
    return `${corners[0]}px ${corners[1]}px ${corners[2]}px ${corners[3]}px`;
  };

  const findSnapTarget = (
    id: MessagePanelId,
    x: number,
    y: number,
  ) => {
    const cell = boardRef.current?.querySelector<HTMLElement>(`[data-panel-id="${id}"]`);
    if (!cell) return null;
    const base = cell.getBoundingClientRect();
    const dragged = {
      left: base.left + x,
      right: base.right + x,
      top: base.top + y,
      bottom: base.bottom + y,
      width: base.width,
      height: base.height,
    };
    const candidates: Array<{
      target: MessagePanelId;
      edge: MessagePanelEdge;
      distance: number;
      x: number;
      y: number;
    }> = [];
    const remainingConnections = connections.filter(
      (connection) => connection.a !== id && connection.b !== id,
    );
    const targetSideOccupied = (target: MessagePanelId, side: MessagePanelEdge) =>
      remainingConnections.some((connection) => {
        if (connection.a === target) {
          const usedSide: MessagePanelEdge = {
            left: "right",
            right: "left",
            top: "bottom",
            bottom: "top",
          }[connection.edge] as MessagePanelEdge;
          return usedSide === side;
        }
        return connection.b === target && connection.edge === side;
      });
    MESSAGE_PANEL_IDS.filter((target) => target !== id).forEach((target) => {
      const targetCell = boardRef.current?.querySelector<HTMLElement>(`[data-panel-id="${target}"]`);
      if (!targetCell) return;
      const targetBase = targetCell.getBoundingClientRect();
      const targetLayout = layouts[target];
      const targetRect = {
        left: targetBase.left + targetLayout.x,
        right: targetBase.right + targetLayout.x,
        top: targetBase.top + targetLayout.y,
        bottom: targetBase.bottom + targetLayout.y,
      };
      const rowAlignment = Math.abs(dragged.top - targetRect.top);
      const columnAlignment = Math.abs(dragged.left - targetRect.left);
      const draggedCenterX = (dragged.left + dragged.right) / 2;
      const draggedCenterY = (dragged.top + dragged.bottom) / 2;
      const targetCenterX = (targetRect.left + targetRect.right) / 2;
      const targetCenterY = (targetRect.top + targetRect.bottom) / 2;
      if (rowAlignment <= 58) {
        const edge: MessagePanelEdge = draggedCenterX < targetCenterX ? "left" : "right";
        if (!targetSideOccupied(target, edge)) {
          candidates.push(edge === "left"
            ? { target, edge, distance: Math.abs(dragged.right - targetRect.left), x: targetRect.left - base.left - dragged.width, y: targetRect.top - base.top }
            : { target, edge, distance: Math.abs(dragged.left - targetRect.right), x: targetRect.right - base.left, y: targetRect.top - base.top });
        }
      }
      if (columnAlignment <= 58) {
        const edge: MessagePanelEdge = draggedCenterY < targetCenterY ? "top" : "bottom";
        if (!targetSideOccupied(target, edge)) {
          candidates.push(edge === "top"
            ? { target, edge, distance: Math.abs(dragged.bottom - targetRect.top), x: targetRect.left - base.left, y: targetRect.top - base.top - dragged.height }
            : { target, edge, distance: Math.abs(dragged.top - targetRect.bottom), x: targetRect.left - base.left, y: targetRect.bottom - base.top });
        }
      }
    });
    const best = candidates.sort((a, b) => a.distance - b.distance)[0];
    return best && best.distance <= 58 ? best : null;
  };

  const beginPanelDrag = (
    event: ReactPointerEvent<HTMLElement>,
    id: MessagePanelId,
  ) => {
    if ((event.target as Element).closest("button, input")) return;
    const cell = event.currentTarget.closest(".message-panel-cell") as HTMLElement | null;
    if (!cell) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    const current = layouts[id];
    const connectedAtStart = isPanelConnected(id);
    dragRef.current = {
      id,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      originX: current.x,
      originY: current.y,
      cell,
      x: current.x,
      y: current.y,
      connectedAtStart,
      hasBroken: false,
      previewCell: null,
      previewEdge: null,
    };
    cell.classList.add("is-pulling", "is-active-drag");
  };

  const updatePanelDrag = (event: ReactPointerEvent<HTMLElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    if (event.buttons === 0) {
      endPanelDrag(event);
      return;
    }
    const deltaX = event.clientX - drag.startX;
    const deltaY = event.clientY - drag.startY;
    const pullDistance = Math.hypot(deltaX, deltaY);
    if (drag.connectedAtStart && pullDistance > 112) drag.hasBroken = true;
    const resistance = !drag.connectedAtStart || drag.hasBroken
      ? 1
      : 0.76 + Math.min(pullDistance / 112, 1) * 0.12;
    drag.x = Math.max(-900, Math.min(900, drag.originX + deltaX * resistance));
    drag.y = Math.max(-560, Math.min(560, drag.originY + deltaY * resistance));
    drag.cell.style.setProperty("--panel-x", `${drag.x}px`);
    drag.cell.style.setProperty("--panel-y", `${drag.y}px`);
    drag.cell.style.setProperty("--panel-tilt", `${Math.max(-1.2, Math.min(1.2, (event.clientX - drag.startX) * 0.012))}deg`);
    drag.cell.classList.toggle("is-breaking", drag.hasBroken);
    updateMergePreview(
      drag,
      drag.connectedAtStart && !drag.hasBroken ? null : findSnapTarget(drag.id, drag.x, drag.y),
    );
  };

  const endPanelDrag = (event: ReactPointerEvent<HTMLElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    const stayedConnected = drag.connectedAtStart && !drag.hasBroken;
    const baseConnections = stayedConnected
      ? connections
      : connections.filter((connection) => connection.a !== drag.id && connection.b !== drag.id);
    const snap = stayedConnected ? null : findSnapTarget(drag.id, drag.x, drag.y);
    clearMergePreview(drag);
    const next = stayedConnected
      ? { x: drag.originX, y: drag.originY }
      : snap
        ? { x: snap.x, y: snap.y }
        : { x: drag.x, y: drag.y };
    drag.cell.style.setProperty("--panel-x", `${next.x}px`);
    drag.cell.style.setProperty("--panel-y", `${next.y}px`);
    drag.cell.style.setProperty("--panel-tilt", "0deg");
    drag.cell.classList.remove("is-pulling", "is-breaking", "is-active-drag");
    setLayouts((current) => ({ ...current, [drag.id]: next }));
    if (!stayedConnected) {
      setConnections(snap
        ? [...baseConnections, { a: drag.id, b: snap.target, edge: snap.edge }]
        : baseConnections);
    }
    dragRef.current = null;
  };

  const panelStyle = (id: MessagePanelId) => {
    const detachedSize = detachedSizes[id];
    return {
      "--panel-x": `${layouts[id].x}px`,
      "--panel-y": `${layouts[id].y}px`,
      "--panel-tilt": "0deg",
      "--panel-radius": panelRadius(id),
      ...(detachedSize ? { "--free-panel-width": `${detachedSize.width}px`, "--free-panel-height": `${detachedSize.height}px` } : {}),
    } as CSSProperties;
  };

  const dragHandlers = (id: MessagePanelId) => ({
    onPointerDown: (event: ReactPointerEvent<HTMLElement>) => beginPanelDrag(event, id),
    onPointerMove: updatePanelDrag,
    onPointerUp: endPanelDrag,
    onPointerCancel: endPanelDrag,
  });

  const beginStackResize = (event: ReactPointerEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();
    const board = boardRef.current;
    const cell = event.currentTarget.closest<HTMLElement>('[data-panel-id="thread"]');
    if (!board || !cell) return;
    const rows = getComputedStyle(board).gridTemplateRows.split(" ").map(Number.parseFloat);
    const total = rows.reduce((sum, value) => sum + value, 0);
    event.currentTarget.setPointerCapture(event.pointerId);
    stackResizeRef.current = { pointerId: event.pointerId, startY: event.clientY, startFirst: rows[0] || total / 2, total, first: rows[0] || total / 2, cell };
    cell.classList.add("is-resizing");
  };

  const updateStackResize = (event: ReactPointerEvent<HTMLButtonElement>) => {
    const resize = stackResizeRef.current;
    const board = boardRef.current;
    if (resize.pointerId !== event.pointerId || !resize.cell || !board) return;
    resize.first = Math.max(220, Math.min(resize.total - 220, resize.startFirst + event.clientY - resize.startY));
    board.style.gridTemplateRows = `${resize.first}px ${resize.total - resize.first}px`;
  };

  const endStackResize = (event: ReactPointerEvent<HTMLButtonElement>) => {
    const resize = stackResizeRef.current;
    if (resize.pointerId !== event.pointerId || !resize.cell) return;
    if (event.currentTarget instanceof Element && event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    resize.cell.classList.remove("is-resizing");
    setStackSplit(resize.total ? resize.first / resize.total : .5);
    stackResizeRef.current = { pointerId: -1, startY: 0, startFirst: 0, total: 0, first: 0, cell: null };
  };

  useEffect(() => {
    const move = (event: PointerEvent) => updateStackResize(event as unknown as ReactPointerEvent<HTMLButtonElement>);
    const finish = (event: PointerEvent) => endStackResize(event as unknown as ReactPointerEvent<HTMLButtonElement>);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
    };
  });

  const clearModulePreview = () => {
    const chat = boardRef.current?.querySelector<HTMLElement>('[data-panel-id="chat"]');
    chat?.classList.remove("merge-preview-right", "merge-preview-top", "merge-preview-bottom");
  };

  const moduleDragHandlers = {
    onPointerDown: (event: ReactPointerEvent<HTMLElement>) => {
      if ((event.target as Element).closest("button, input")) return;
      const cell = event.currentTarget.closest<HTMLElement>('[data-panel-id="thread"]');
      if (!cell) return;
      cell.setPointerCapture(event.pointerId);
      moduleDragRef.current = { pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, edge: null, moved: false, cell };
      cell.classList.add("is-module-dragging");
    },
    onPointerMove: (event: ReactPointerEvent<HTMLElement>) => {
      const drag = moduleDragRef.current;
      const chat = boardRef.current?.querySelector<HTMLElement>('[data-panel-id="chat"]');
      if (drag.pointerId !== event.pointerId || !drag.cell || !chat) return;
      if (Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) < 8) return;
      drag.moved = true;
      const rect = chat.getBoundingClientRect();
      const boardRect = boardRef.current?.getBoundingClientRect() ?? rect;
      const verticalThreshold = Math.min(150, rect.height * .3);
      const edge: "right" | "top" | "bottom" = event.clientX >= boardRect.right - Math.min(150, boardRect.width * .14)
        ? "right"
        : event.clientY <= rect.top + verticalThreshold
          ? "top"
          : event.clientY >= rect.bottom - verticalThreshold
            ? "bottom"
            : "right";
      if (edge === drag.edge) return;
      clearModulePreview();
      chat.classList.add(`merge-preview-${edge}`);
      drag.edge = edge;
    },
    onPointerUp: (event: ReactPointerEvent<HTMLElement>) => {
      const drag = moduleDragRef.current;
      if (drag.pointerId !== event.pointerId) return;
      if (drag.cell?.hasPointerCapture(event.pointerId)) drag.cell.releasePointerCapture(event.pointerId);
      clearModulePreview();
      drag.cell?.classList.remove("is-module-dragging");
      if (drag.moved && drag.edge) {
        setModuleLayout(drag.edge === "top" ? "thread-top" : drag.edge === "bottom" ? "thread-bottom" : "side");
      }
      moduleDragRef.current = { pointerId: -1, startX: 0, startY: 0, edge: null, moved: false, cell: null };
    },
    onPointerCancel: (event: ReactPointerEvent<HTMLElement>) => {
      const drag = moduleDragRef.current;
      if (drag.pointerId !== event.pointerId) return;
      clearModulePreview();
      drag.cell?.classList.remove("is-module-dragging");
      moduleDragRef.current = { pointerId: -1, startX: 0, startY: 0, edge: null, moved: false, cell: null };
    },
  };

  useEffect(() => {
    const move = (event: PointerEvent) => moduleDragHandlers.onPointerMove(event as unknown as ReactPointerEvent<HTMLElement>);
    const finish = (event: PointerEvent) => moduleDragHandlers.onPointerUp(event as unknown as ReactPointerEvent<HTMLElement>);
    const cancel = (event: PointerEvent) => moduleDragHandlers.onPointerCancel(event as unknown as ReactPointerEvent<HTMLElement>);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", cancel);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", cancel);
    };
  });

  const beginChatVerticalResize = (event: ReactPointerEvent<HTMLButtonElement>, edge: "top" | "bottom") => {
    event.preventDefault();
    event.stopPropagation();
    const cell = boardRef.current?.querySelector<HTMLElement>('[data-panel-id="chat"]') ?? null;
    const panel = cell?.querySelector<HTMLElement>(".chat-panel");
    if (!cell || !panel) return;
    const pairedCell = moduleLayout === "side" && threadOpen && arePanelsConnected("chat", "thread")
      ? boardRef.current?.querySelector<HTMLElement>('[data-panel-id="thread"]') ?? null
      : null;
    const cellRect = cell.getBoundingClientRect();
    const panelRect = panel.getBoundingClientRect();
    event.currentTarget.setPointerCapture(event.pointerId);
    chatVerticalResizeRef.current = { pointerId: event.pointerId, edge, startY: event.clientY, startHeight: panelRect.height, startOffset: panelRect.top - cellRect.top, height: panelRect.height, offset: panelRect.top - cellRect.top, cell, pairedCell };
    cell.classList.add("is-resizing");
    pairedCell?.classList.add("is-resizing");
  };

  const updateChatVerticalResize = (event: ReactPointerEvent<HTMLButtonElement>) => {
    const resize = chatVerticalResizeRef.current;
    if (resize.pointerId !== event.pointerId || !resize.cell) return;
    const cellHeight = resize.cell.getBoundingClientRect().height;
    const deltaY = event.clientY - resize.startY;
    if (resize.edge === "bottom") {
      resize.height = Math.max(280, Math.min(cellHeight - resize.startOffset, resize.startHeight + deltaY));
      resize.offset = resize.startOffset;
    } else {
      resize.offset = Math.max(0, Math.min(resize.startOffset + resize.startHeight - 280, resize.startOffset + deltaY));
      resize.height = resize.startHeight - (resize.offset - resize.startOffset);
    }
    resize.cell.style.setProperty("--chat-panel-height", `${resize.height}px`);
    resize.cell.style.setProperty("--chat-resize-y", `${resize.offset}px`);
    resize.pairedCell?.style.setProperty("--thread-panel-height", `${resize.height}px`);
    resize.pairedCell?.style.setProperty("--thread-resize-y", `${resize.offset}px`);
  };

  const endChatVerticalResize = (event: ReactPointerEvent<HTMLButtonElement>) => {
    const resize = chatVerticalResizeRef.current;
    if (resize.pointerId !== event.pointerId || !resize.cell) return;
    if (event.currentTarget instanceof Element && event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    resize.cell.classList.remove("is-resizing");
    resize.pairedCell?.classList.remove("is-resizing");
    setChatVerticalSize({ offsetY: resize.offset, height: resize.height });
    chatVerticalResizeRef.current = { pointerId: -1, edge: "bottom", startY: 0, startHeight: 0, startOffset: 0, height: 0, offset: 0, cell: null, pairedCell: null };
  };

  useEffect(() => {
    const move = (event: PointerEvent) => updateChatVerticalResize(event as unknown as ReactPointerEvent<HTMLButtonElement>);
    const finish = (event: PointerEvent) => endChatVerticalResize(event as unknown as ReactPointerEvent<HTMLButtonElement>);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
    };
  });

  const closeThread = () => {
    setThreadOpen(false);
    setModuleLayout("side");
    setConnections((current) => current.filter((connection) => connection.a !== "thread" && connection.b !== "thread"));
  };

  const openThread = () => {
    setThreadOpen(true);
    setModuleLayout("side");
    setConnections((current) => {
      const withoutThread = current.filter((connection) => connection.a !== "thread" && connection.b !== "thread");
      return [...withoutThread, { a: "chat", b: "thread", edge: "left" }];
    });
  };

  const clearMessageTaskDockPreview = () => {
    boardRef.current?.querySelector<HTMLElement>('[data-panel-id="chat"]')?.classList.remove("is-task-dock-preview");
    boardRef.current?.querySelector<HTMLElement>('[data-panel-id="thread"]')?.classList.remove("is-task-dock-preview");
  };

  const isOverTaskDock = (x: number, y: number) => {
    const board = boardRef.current;
    const chat = board?.querySelector<HTMLElement>('[data-panel-id="chat"]');
    if (!board || !chat) return false;
    const boardRect = board.getBoundingClientRect();
    const chatRect = chat.getBoundingClientRect();
    const dockZoneLeft = Math.max(chatRect.left + 260, chatRect.right - 120);
    return x >= dockZoneLeft && x <= boardRect.right + 24 && y >= chatRect.top - 32 && y <= chatRect.bottom + 32;
  };

  const beginMessageTaskDrag = (event: ReactPointerEvent<HTMLElement>) => {
    if ((event.target as Element).closest("button")) return;
    const panel = event.currentTarget.closest<HTMLElement>(".message-task-detail-panel");
    if (!panel || messageTaskDocked) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    messageTaskDragRef.current = { pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, originX: messageTaskPosition.x, originY: messageTaskPosition.y, x: messageTaskPosition.x, y: messageTaskPosition.y, preview: false, panel };
    panel.classList.add("is-dragging");
  };

  const updateMessageTaskDrag = (event: ReactPointerEvent<HTMLElement>) => {
    const drag = messageTaskDragRef.current;
    if (drag.pointerId !== event.pointerId || !drag.panel) return;
    drag.x = drag.originX + event.clientX - drag.startX;
    drag.y = drag.originY + event.clientY - drag.startY;
    drag.panel.style.setProperty("--task-panel-x", `${drag.x}px`);
    drag.panel.style.setProperty("--task-panel-y", `${drag.y}px`);
    drag.preview = isOverTaskDock(event.clientX, event.clientY);
    const chatCell = boardRef.current?.querySelector<HTMLElement>('[data-panel-id="chat"]');
    const threadCell = boardRef.current?.querySelector<HTMLElement>('[data-panel-id="thread"]');
    chatCell?.classList.toggle("is-task-dock-preview", drag.preview);
    threadCell?.classList.toggle("is-task-dock-preview", drag.preview);
    drag.panel.classList.toggle("is-dock-preview", drag.preview);
  };

  const endMessageTaskDrag = (event: ReactPointerEvent<HTMLElement>) => {
    const drag = messageTaskDragRef.current;
    if (drag.pointerId !== event.pointerId || !drag.panel) return;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    clearMessageTaskDockPreview();
    drag.panel.classList.remove("is-dragging", "is-dock-preview");
    if (drag.preview) {
      const board = boardRef.current;
      if (board) {
        setModuleLayout("side");
        setMessageTaskDockWidth(Math.max(310, Math.min(430, drag.panel.getBoundingClientRect().width)));
        setMessageTaskDockRect(null);
        setColumnWidths(null);
      }
      setMessageTaskPosition({ x: 0, y: 0 });
      setMessageTaskDocked(true);
    } else {
      setMessageTaskPosition({ x: drag.x, y: drag.y });
    }
    messageTaskDragRef.current = { ...messageTaskDragRef.current, pointerId: -1, panel: null, preview: false };
  };

  const beginMessageTaskResize = (event: ReactPointerEvent<HTMLButtonElement>, edge: "top" | "right" | "bottom") => {
    event.preventDefault();
    event.stopPropagation();
    const panel = event.currentTarget.closest<HTMLElement>(".message-task-detail-panel");
    if (!panel) return;
    const panelRect = panel.getBoundingClientRect();
    event.currentTarget.setPointerCapture(event.pointerId);
    messageTaskResizeRef.current = { pointerId: event.pointerId, edge, docked: messageTaskDocked, startX: event.clientX, startY: event.clientY, startWidth: panelRect.width, startHeight: panelRect.height, width: panelRect.width, height: panelRect.height, startYPosition: messageTaskPosition.y, yPosition: messageTaskPosition.y, panel };
    panel.classList.add("is-resizing");
  };

  const updateMessageTaskResize = (event: ReactPointerEvent<HTMLButtonElement>) => {
    const resize = messageTaskResizeRef.current;
    const board = boardRef.current;
    if (resize.pointerId !== event.pointerId || !resize.panel || !board) return;
    const deltaX = event.clientX - resize.startX;
    const deltaY = event.clientY - resize.startY;
    if (resize.docked) {
      resize.width = Math.max(280, Math.min(460, resize.startWidth + deltaX));
      board.style.width = `calc(100% - ${resize.width + 8 + (bestiePanelOpen ? bestiePanelWidth + BESTIE_CONTEXT_PANEL_GAP : 0)}px)`;
      const stage = board.closest<HTMLElement>(".messages-stage");
      const neighborPanel = threadOpen
        ? board.querySelector<HTMLElement>('[data-panel-id="thread"] .message-panel')
        : board.querySelector<HTMLElement>('[data-panel-id="chat"] .message-panel');
      if (stage && neighborPanel) {
        const stageRect = stage.getBoundingClientRect();
        const boardRect = board.getBoundingClientRect();
        const neighborRect = neighborPanel.getBoundingClientRect();
        resize.panel.style.left = `${boardRect.right - stageRect.left + 8}px`;
        resize.panel.style.top = `${neighborRect.top - stageRect.top}px`;
        resize.panel.style.width = `${resize.width}px`;
        resize.panel.style.height = `${neighborRect.height}px`;
      }
    } else {
      if (resize.edge === "right") resize.width = Math.max(320, Math.min(720, resize.startWidth + deltaX));
      if (resize.edge === "bottom") resize.height = Math.max(360, Math.min(window.innerHeight - 96, resize.startHeight + deltaY));
      if (resize.edge === "top") {
        const appliedDelta = Math.max(resize.startHeight - (window.innerHeight - 96), Math.min(resize.startHeight - 360, deltaY));
        resize.height = resize.startHeight - appliedDelta;
        resize.yPosition = resize.startYPosition + appliedDelta;
        resize.panel.style.setProperty("--task-panel-y", `${resize.yPosition}px`);
      }
      resize.panel.style.width = `${resize.width}px`;
      resize.panel.style.height = `${resize.height}px`;
    }
  };

  const endMessageTaskResize = (event: ReactPointerEvent<HTMLButtonElement>) => {
    const resize = messageTaskResizeRef.current;
    if (resize.pointerId !== event.pointerId || !resize.panel) return;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    resize.panel.classList.remove("is-resizing");
    if (resize.docked) {
      setMessageTaskDockWidth(resize.width);
      setMessageTaskDockRect(null);
      setColumnWidths(null);
    } else {
      setMessageTaskSize({ width: resize.width, height: resize.height });
      if (resize.edge === "top") setMessageTaskPosition((current) => ({ ...current, y: resize.yPosition }));
    }
    messageTaskResizeRef.current = { ...messageTaskResizeRef.current, pointerId: -1, panel: null };
  };

  const beginCanvasPan = (event: ReactPointerEvent<HTMLElement>) => {
    if ((event.target as Element).closest(".message-panel, .message-task-detail-panel, .catch-up-focus, button, input")) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    panRef.current = {
      ...panRef.current,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      originX: panRef.current.x,
      originY: panRef.current.y,
    };
    event.currentTarget.classList.add("is-panning");
  };

  const updateCanvasPan = (event: ReactPointerEvent<HTMLElement>) => {
    const pan = panRef.current;
    if (pan.pointerId !== event.pointerId) return;
    pan.x = Math.max(-1200, Math.min(1200, pan.originX + event.clientX - pan.startX));
    pan.y = Math.max(-800, Math.min(800, pan.originY + event.clientY - pan.startY));
    const shell = event.currentTarget.closest<HTMLElement>(".app-shell");
    shell?.style.setProperty("--messages-pan-x", `${pan.x}px`);
    shell?.style.setProperty("--messages-pan-y", `${pan.y}px`);
  };

  const endCanvasPan = (event: ReactPointerEvent<HTMLElement>) => {
    if (panRef.current.pointerId !== event.pointerId) return;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    panRef.current.pointerId = -1;
    event.currentTarget.classList.remove("is-panning");
  };

  const openCatchUp = () => {
    clearMessageTaskDockPreview();
    closeMessageTask();
    setCatchUpOpen(true);
    onEnterCatchUp();
  };

  const openSidebarConversation = (id: string) => {
    setActiveChatRow(id);
    setActiveSession(0);
    setThreadOpen(false);
    setCatchUpOpen(false);
    setChatRowMenu(null);
  };

  const activeSidebarConversation = SIDEBAR_CONVERSATIONS[activeChatRow];
  const activeTaskConversation = activeChatRow.startsWith("task:");
  const activeConversationMessages = activeSidebarConversation
    ? activeTaskConversation && activeSession > 0
      ? activeSidebarConversation.followUpSessions?.[activeSession - 1] ?? activeSidebarConversation.messages
      : activeSidebarConversation.messages
    : [];

  useEffect(() => {
    const tabs = sessionTabsRef.current;
    if (!activeTaskConversation || !tabs) {
      setSessionSlider({ x: 0, width: 0 });
      return;
    }
    const updateSlider = () => {
      const activeTab = tabs.querySelector<HTMLElement>('button[aria-current="page"]');
      if (!activeTab) return;
      const next = { x: activeTab.offsetLeft, width: activeTab.offsetWidth };
      setSessionSlider((current) => current.x === next.x && current.width === next.width ? current : next);
    };
    updateSlider();
    const observer = new ResizeObserver(updateSlider);
    observer.observe(tabs);
    tabs.querySelectorAll("button").forEach((button) => observer.observe(button));
    return () => observer.disconnect();
  }, [activeSession, activeTaskConversation, selectedProjectTask]);

  return (
    <section
      className="messages-stage is-modular"
      aria-label="Composable messages workspace"
      data-bestie-context="messages:chat"
    >
      <div
        ref={boardRef}
        className={`messages-board is-modular-layout is-${moduleLayout}${threadOpen ? "" : " is-thread-closed"}${bestiePanelOpen ? " has-bestie-panel" : ""}${messageTaskDocked ? " has-task-panel" : ""}`}
        style={{
          ...(moduleLayout === "side"
            ? (threadOpen
                ? (columnWidths
                    ? { gridTemplateColumns: MESSAGE_PANEL_IDS.map((id) => `${columnWidths[id]}px`).join(" ") }
                    : bestiePanelOpen && messageTaskDocked
                      ? { gridTemplateColumns: "minmax(145px, 180px) minmax(200px, 1fr) minmax(180px, 240px)" }
                      : bestiePanelOpen
                        ? { gridTemplateColumns: "minmax(190px, 220px) minmax(280px, 1fr) minmax(240px, 300px)" }
                      : {})
                : { gridTemplateColumns: `${columnWidths?.sidebar ? `${columnWidths.sidebar}px` : bestiePanelOpen && messageTaskDocked ? "minmax(145px, 180px)" : bestiePanelOpen ? "minmax(190px, 220px)" : "minmax(220px, 255px)"} ${bestiePanelOpen && messageTaskDocked ? "minmax(200px, 1fr)" : bestiePanelOpen ? "minmax(280px, 1fr)" : "minmax(440px, 1fr)"}` })
            : { gridTemplateColumns: `${columnWidths?.sidebar ? `${columnWidths.sidebar}px` : "minmax(220px, 255px)"} minmax(440px, 1fr)`, gridTemplateRows: `minmax(220px, ${stackSplit}fr) minmax(220px, ${1 - stackSplit}fr)` }),
          ...(messageTaskDocked || bestiePanelOpen
            ? { width: `calc(100% - ${(messageTaskDocked ? messageTaskDockWidth + 8 : 0) + (bestiePanelOpen ? bestiePanelWidth + BESTIE_CONTEXT_PANEL_GAP : 0)}px)` }
            : {}),
        }}
      >
        <div data-panel-id="sidebar" className={`message-panel-cell sidebar-cell${catchUpOpen ? " catch-up-active" : ""}${isPanelConnected("sidebar") ? "" : " is-detached"}`} style={panelStyle("sidebar")}>
          <span className="merge-edge-indicator" aria-hidden="true" />
          <aside className="content-surface message-panel channel-panel">
            <div className="channel-search-wrap">
              <Search aria-hidden="true" size={18} strokeWidth={1.7} />
              <input aria-label="Search channels" placeholder="Search" />
            </div>
            <nav className="channel-list" aria-label="Channels">
              <button type="button" className={`catch-up-row${catchUpOpen ? " is-current" : ""}`} aria-current={catchUpOpen ? "page" : undefined} onClick={openCatchUp}><span className="catch-up-count">{CATCH_UP_ITEMS.length}</span><span>Catch up</span></button>
              <p>Projects</p>
              <ChatNavRow id="project:berd-main" label="berd-main" icon={<Hash aria-hidden="true" size={18} strokeWidth={1.7} />} current={!catchUpOpen && activeChatRow === "project:berd-main"} onSelect={() => openSidebarConversation("project:berd-main")} menuOpen={chatRowMenu?.id === "project:berd-main"} onOpenMenu={(menu) => setChatRowMenu((current) => current?.id === menu.id ? null : menu)} />
              <div className="project-channel-task-list" aria-label="Tasks in berd-main">
                {PROJECT_CHANNEL_TASKS.map((task) => <button key={task.id} type="button" className={`project-task-nav-row${activeChatRow === `task:${task.id}` ? " is-current" : ""}`} onClick={() => { setSelectedProjectTask(task.id); setActiveChatRow(`task:${task.id}`); setActiveSession(0); setThreadOpen(false); setCatchUpOpen(false); setChatRowMenu(null); }}><CircleCheck aria-hidden="true" size={17} strokeWidth={1.65} /><span>{task.label}</span></button>)}
              </div>
              {['goose-main', 'buzz-main'].map((name) => { const id = `project:${name}`; return <ChatNavRow key={name} id={id} label={name} icon={<Hash aria-hidden="true" size={18} strokeWidth={1.7} />} current={!catchUpOpen && activeChatRow === id} onSelect={() => openSidebarConversation(id)} menuOpen={chatRowMenu?.id === id} onOpenMenu={(menu) => setChatRowMenu((current) => current?.id === menu.id ? null : menu)} />; })}
              <p>Personal</p>
              {['design-system', 'launch-planning', 'agent-tools'].map((name) => { const id = `personal:${name}`; return <ChatNavRow key={name} id={id} label={name} icon={<Hash aria-hidden="true" size={18} strokeWidth={1.7} />} current={!catchUpOpen && activeChatRow === id} onSelect={() => openSidebarConversation(id)} menuOpen={chatRowMenu?.id === id} onOpenMenu={(menu) => setChatRowMenu((current) => current?.id === menu.id ? null : menu)} />; })}
              <p>Messages</p>
              {DIRECT_MESSAGE_PROFILES.map(({ name, image }) => { const id = `message:${name}`; return <ChatNavRow key={name} id={id} label={name} icon={<img className="direct-message-avatar" src={image} alt="" />} current={!catchUpOpen && activeChatRow === id} onSelect={() => openSidebarConversation(id)} menuOpen={chatRowMenu?.id === id} onOpenMenu={(menu) => setChatRowMenu((current) => current?.id === menu.id ? null : menu)} />; })}
            </nav>
            {catchUpOpen ? null : panelResizeHandle("sidebar")}
          </aside>
        </div>

        {catchUpOpen ? <CatchUpView bestiePanelOpen={bestiePanelOpen} bestieContext={bestieContext} /> : <>
        <div data-panel-id="chat" data-bestie-module-target="true" className={`message-panel-cell chat-cell${isPanelConnected("chat") ? "" : " is-detached"}`} style={{ ...panelStyle("chat"), ...(chatVerticalSize ? { "--chat-panel-height": `${chatVerticalSize.height}px`, "--chat-resize-y": `${chatVerticalSize.offsetY}px` } : {}) } as CSSProperties}>
          <span className="merge-edge-indicator" aria-hidden="true" />
          <article className="content-surface message-panel chat-panel">
            <header className="chat-header session-chat-header">
              {activeTaskConversation ? <nav ref={sessionTabsRef} className="chat-session-tabs" aria-label={`${selectedProjectTaskDetails.label} sessions`}>
                <span className={`chat-session-slider${sessionSlider.width ? " is-ready" : ""}`} aria-hidden="true" style={{ width: sessionSlider.width, transform: `translate3d(${sessionSlider.x}px, 0, 0)` }} />
                {selectedProjectTaskDetails.sessions.map((session, index) => <button key={session} type="button" className={activeSession === index ? "is-active" : ""} aria-current={activeSession === index ? "page" : undefined} onClick={() => setActiveSession(index)}><MessageSquareText aria-hidden="true" size={17} strokeWidth={1.65} /><span>{session}</span></button>)}
              </nav> : <div className="chat-channel-heading"><Hash aria-hidden="true" size={18} strokeWidth={1.7} /><strong>{activeSidebarConversation?.label ?? "berd-main"}</strong></div>}
              <div className="chat-header-actions session-header-actions">{activeTaskConversation ? <button type="button" className={messageTaskOpen ? "is-active" : ""} aria-label={messageTaskOpen ? "Collapse task details" : "Expand task details"} aria-pressed={messageTaskOpen} onClick={messageTaskOpen ? closeMessageTask : openMessageTask}><PanelRight aria-hidden="true" size={18} strokeWidth={1.7} /></button> : null}<button type="button" aria-label="More conversation actions"><MoreHorizontal aria-hidden="true" size={18} strokeWidth={1.8} /></button></div>
            </header>
            <div className={`chat-feed bestie-content-drop-target${bestieReferenceClass("messages:chat")}`} data-bestie-context="messages:chat">
              {activeChatRow === "project:berd-main" && activeSession === 0 ? <>
                <div className={`chat-message bestie-content-drop-target${bestieReferenceClass("messages:message:caroline-update")}`} data-bestie-context="messages:message:caroline-update"><img className="human-avatar" src={USER_PROFILES.caroline} alt="" /><div><p><strong>Caroline McKenzie</strong><span>Yesterday at 7:05PM</span></p><div>I reproduced the agent creation error. It appears after focusing the description field, before anyone submits the form.</div></div></div>
                <div className={`chat-message bestie-content-drop-target${bestieReferenceClass("messages:message:jude-feedback")}`} data-bestie-context="messages:message:jude-feedback"><img className="human-avatar" src={USER_PROFILES.arj} alt="" /><div><p><strong>Jude</strong><span>5:03AM</span></p><div>Could we wait to validate until submit, while still clearing the error as soon as someone starts typing?</div><button type="button" className="reply-link" aria-controls="message-thread-panel" aria-expanded={threadOpen} onClick={openThread}><span className="mini-avatars"><img src={USER_PROFILES.morgan} alt="" /><img src={USER_PROFILES.simon} alt="" /></span><strong>4 replies</strong><span>last reply 5 hours ago</span></button></div></div>
                <div className={`chat-message message-task-post bestie-content-drop-target${bestieReferenceClass("messages:message:bestie-task-created")}`} data-bestie-context="messages:message:bestie-task-created"><img src="/snek.png" alt="" /><div><p><strong>Bestie</strong><span>5:12AM</span></p><div>I created a task for the error state work.</div><button type="button" className={`inline-message-task bestie-content-drop-target${bestieReferenceClass("messages:task:TSK-123")}`} data-bestie-context="messages:task:TSK-123" aria-label="Open task TSK-123" aria-expanded={messageTaskOpen && selectedProjectTask === "TSK-123"} onClick={() => { setSelectedProjectTask("TSK-123"); openMessageTask(); }}><span className="inline-task-icon"><ClipboardList aria-hidden="true" size={18} strokeWidth={1.7} /></span><span className="inline-task-copy"><strong>TSK-123</strong><small>Fix error state on agent creation</small></span><span className="inline-task-preview" aria-hidden="true"><span className="inline-task-preview-heading"><i><ClipboardList size={16} strokeWidth={1.7} /></i><strong>TSK-123</strong></span><b>Fix error state on agent creation</b><span className="inline-task-preview-meta"><span><small>Progress</small>35% complete</span><span><small>Assigned to</small>Morgan Martin</span></span><span className="inline-task-preview-description"><small>Description</small>Wait to show the description error state until someone submits.</span></span></button></div></div>
              </> : null}
              {activeChatRow === "project:berd-main" && activeSession === 1 ? <>
                <div className={`chat-message bestie-content-drop-target${bestieReferenceClass("messages:message:morgan-review")}`} data-bestie-context="messages:message:morgan-review"><img className="human-avatar" src={USER_PROFILES.morgan} alt="" /><div><p><strong>Morgan Martin</strong><span>Yesterday at 8:42PM</span></p><div>I pushed the validation update and added coverage for empty, partially completed, and resubmitted forms.</div></div></div>
                <div className={`chat-message bestie-content-drop-target${bestieReferenceClass("messages:message:simon-review")}`} data-bestie-context="messages:message:simon-review"><img className="human-avatar" src={USER_PROFILES.simon} alt="" /><div><p><strong>Simon</strong><span>9:18AM</span></p><div>The behavior looks right now. I left one comment about keeping the helper copy stable when the error disappears.</div></div></div>
                <div className={`chat-message bestie-content-drop-target${bestieReferenceClass("messages:message:caroline-review")}`} data-bestie-context="messages:message:caroline-review"><img className="human-avatar" src={USER_PROFILES.caroline} alt="" /><div><p><strong>Caroline McKenzie</strong><span>9:31AM</span></p><div>Agreed. That will prevent the layout from jumping while someone corrects the field.</div></div></div>
              </> : null}
              {activeChatRow === "project:berd-main" && activeSession === 2 ? <>
                <div className={`chat-message bestie-content-drop-target${bestieReferenceClass("messages:message:caroline-plan")}`} data-bestie-context="messages:message:caroline-plan"><img className="human-avatar" src={USER_PROFILES.caroline} alt="" /><div><p><strong>Caroline McKenzie</strong><span>Today at 10:06AM</span></p><div>Let’s ship the validation fix first, then reuse the same field state in the edit-agent flow.</div></div></div>
                <div className={`chat-message bestie-content-drop-target${bestieReferenceClass("messages:message:jude-plan")}`} data-bestie-context="messages:message:jude-plan"><img className="human-avatar" src={USER_PROFILES.arj} alt="" /><div><p><strong>Jude</strong><span>10:14AM</span></p><div>I can take the shared field state. I’ll keep the API unchanged so the existing creation screen only needs a small update.</div></div></div>
                <div className={`chat-message message-task-post bestie-content-drop-target${bestieReferenceClass("messages:message:bestie-plan")}`} data-bestie-context="messages:message:bestie-plan"><img src="/snek.png" alt="" /><div><p><strong>Bestie</strong><span>10:16AM</span></p><div>I summarized the plan: update validation timing, preserve helper-copy space, and reuse the field state in the edit flow.</div></div></div>
              </> : null}
              {activeSidebarConversation ? <>
                {activeConversationMessages.map((message, index) => {
                  const context = `messages:conversation:${activeChatRow}:message:${index}`;
                  return <div key={`${activeChatRow}-${message.author}`} className={`chat-message bestie-content-drop-target${bestieReferenceClass(context)}`} data-bestie-context={context}><img className="human-avatar" src={message.image} alt="" /><div><p><strong>{message.author}</strong><span>{message.time}</span></p><div>{message.body}</div></div></div>;
                })}
              </> : null}
            </div>
            <MessageComposer />
            <button type="button" className="panel-resize-handle chat-edge-resize-handle is-height-only is-top-edge" aria-label="Resize main chat from top" onPointerDown={(event) => beginChatVerticalResize(event, "top")} onPointerMove={updateChatVerticalResize} onPointerUp={endChatVerticalResize} onPointerCancel={endChatVerticalResize} onLostPointerCapture={endChatVerticalResize} />
            <button type="button" className="panel-resize-handle chat-edge-resize-handle is-height-only is-bottom-edge" aria-label="Resize main chat from bottom" onPointerDown={(event) => beginChatVerticalResize(event, "bottom")} onPointerMove={updateChatVerticalResize} onPointerUp={endChatVerticalResize} onPointerCancel={endChatVerticalResize} onLostPointerCapture={endChatVerticalResize} />
            {messageTaskDocked ? null : panelResizeHandle("chat")}
          </article>
        </div>

        <div
          data-panel-id="thread"
          data-bestie-module-target="true"
          className={`message-panel-cell thread-cell${isPanelConnected("thread") ? "" : " is-detached"}`}
          style={{
            ...panelStyle("thread"),
            ...(moduleLayout === "side" && arePanelsConnected("chat", "thread") && chatVerticalSize
              ? { "--thread-panel-height": `${chatVerticalSize.height}px`, "--thread-resize-y": `${chatVerticalSize.offsetY}px` }
              : {}),
          } as CSSProperties}
          onPointerMove={moduleDragHandlers.onPointerMove}
          onPointerUp={moduleDragHandlers.onPointerUp}
          onPointerCancel={moduleDragHandlers.onPointerCancel}
        >
          <span className="merge-edge-indicator" aria-hidden="true" />
          {threadOpen ? (
          <aside id="message-thread-panel" className="content-surface message-panel thread-panel">
            <MessageSidePanelHeader icon={<PanelRightOpen aria-hidden="true" size={18} strokeWidth={1.7} />} title="Thread" closeLabel="Close thread" onClose={closeThread} dragHandlers={{ onPointerDown: moduleDragHandlers.onPointerDown }} />
            <div className={`thread-content bestie-content-drop-target${bestieReferenceClass("messages:thread")}`} data-bestie-context="messages:thread">
              <div className={`thread-message bestie-content-drop-target${bestieReferenceClass("messages:message:thread-caroline-reply")}`} data-bestie-context="messages:message:thread-caroline-reply"><img className="human-avatar" src={USER_PROFILES.caroline} alt="" /><div><p><strong>Caroline McKenzie</strong><span>Yesterday at 7:05PM</span></p><div>Brought copy link out of the context menu and into the main message rail. Also tried simplifying by removing quick reactions but can put them back later if people really miss them.</div></div></div>
            </div>
            <MessageComposer compact />
            {moduleLayout === "side" && arePanelsConnected("chat", "thread") ? <>
              <button type="button" className="panel-resize-handle is-height-only is-top-edge" aria-label="Resize connected messages and thread panels from top" onPointerDown={(event) => beginChatVerticalResize(event, "top")} onPointerMove={updateChatVerticalResize} onPointerUp={endChatVerticalResize} onPointerCancel={endChatVerticalResize} onLostPointerCapture={endChatVerticalResize} />
              <button type="button" className="panel-resize-handle is-height-only is-bottom-edge" aria-label="Resize connected messages and thread panels from bottom" onPointerDown={(event) => beginChatVerticalResize(event, "bottom")} onPointerMove={updateChatVerticalResize} onPointerUp={endChatVerticalResize} onPointerCancel={endChatVerticalResize} onLostPointerCapture={endChatVerticalResize} />
            </> : null}
            {moduleLayout === "side" ? panelResizeHandle("thread") : (
              <button
                type="button"
                className={`panel-resize-handle is-height-only ${moduleLayout === "thread-top" ? "is-bottom-edge" : "is-top-edge"}`}
                aria-label="Resize stacked message panels"
                onPointerDown={beginStackResize}
                onPointerMove={updateStackResize}
                onPointerUp={endStackResize}
                onPointerCancel={endStackResize}
                onLostPointerCapture={endStackResize}
              />
            )}
          </aside>
          ) : null}
        </div>
        </>}
      </div>
      {chatRowMenu ? (
        <div
          id="chat-row-menu"
          className="chat-row-menu"
          role="menu"
          aria-label={`Options for ${chatRowMenu.label}`}
          style={{ left: chatRowMenu.left, top: chatRowMenu.top }}
        >
          <button type="button" role="menuitem" autoFocus onClick={() => setChatRowMenu(null)}><Pin aria-hidden="true" size={18} strokeWidth={1.7} /><span>Pin chat</span></button>
          <button type="button" role="menuitem" onClick={() => setChatRowMenu(null)}><GitFork aria-hidden="true" size={18} strokeWidth={1.7} /><span>Fork</span></button>
          <button type="button" role="menuitem" onClick={() => setChatRowMenu(null)}><Archive aria-hidden="true" size={18} strokeWidth={1.7} /><span>Archive</span></button>
          <button type="button" role="menuitem" className="is-destructive" onClick={() => setChatRowMenu(null)}><Trash2 aria-hidden="true" size={18} strokeWidth={1.7} /><span>Delete</span></button>
        </div>
      ) : null}
      {messageTaskOpen ? (
        <aside
          data-bestie-module-target="true"
          className={`content-surface message-task-detail-panel${messageTaskDocked ? " is-docked" : ""}${messageTaskDocked && !messageTaskDockRect ? " is-awaiting-dock" : ""}`}
          aria-label={`Task ${selectedProjectTaskDetails.id} details`}
          onPointerDown={beginMessageTaskDrag}
          onPointerMove={updateMessageTaskDrag}
          onPointerUp={endMessageTaskDrag}
          onPointerCancel={endMessageTaskDrag}
          style={messageTaskDocked && messageTaskDockRect
            ? { left: messageTaskDockRect.left, top: messageTaskDockRect.top, width: messageTaskDockRect.width, height: messageTaskDockRect.height }
            : { "--task-panel-x": `${messageTaskPosition.x}px`, "--task-panel-y": `${messageTaskPosition.y}px`, ...(messageTaskSize ? { width: messageTaskSize.width, height: messageTaskSize.height } : {}) } as CSSProperties}
        >
          <header className="message-side-panel-header message-task-panel-header">
            <div><span className="message-task-icon"><ClipboardList aria-hidden="true" size={18} strokeWidth={1.7} /></span><strong>{selectedProjectTaskDetails.id}</strong></div>
            <div className="message-task-panel-actions">
              <button type="button" aria-label="More task actions"><MoreHorizontal aria-hidden="true" size={20} strokeWidth={1.8} /></button>
            </div>
          </header>
          <div className={`message-task-detail-body bestie-content-drop-target${bestieReferenceClass(`messages:task:${selectedProjectTaskDetails.id}`)}`} data-bestie-context={`messages:task:${selectedProjectTaskDetails.id}`}>
            <h2>{selectedProjectTaskDetails.label}</h2>
            <div className="message-task-detail-meta"><p><span>Progress</span><strong>{selectedProjectTaskDetails.progress}</strong></p><p><span>Assigned to</span><strong>{selectedProjectTaskDetails.assignedTo}</strong></p></div>
            <section><span>Description</span><p>{selectedProjectTaskDetails.description}</p></section>
            <section className="message-task-assignees"><span>Agents</span><div><img src="/berd-agent-avatars/berdy-gloopies-22.png" alt="" /><img src="/berd-agent-avatars/builderbot-gloopies-20.png" alt="" /><img src="/berd-agent-avatars/pushback-gloopies-5.png" alt="" /></div></section>
          </div>
          {messageTaskDocked ? <button type="button" className="panel-resize-handle task-panel-resize-handle is-width-only is-right-edge" aria-label="Resize task panel from the right edge" onPointerDown={(event) => beginMessageTaskResize(event, "right")} onPointerMove={updateMessageTaskResize} onPointerUp={endMessageTaskResize} onPointerCancel={endMessageTaskResize} onLostPointerCapture={endMessageTaskResize} /> : <><button type="button" className="panel-resize-handle task-panel-resize-handle is-height-only is-top-edge" aria-label="Resize task panel from the top edge" onPointerDown={(event) => beginMessageTaskResize(event, "top")} onPointerMove={updateMessageTaskResize} onPointerUp={endMessageTaskResize} onPointerCancel={endMessageTaskResize} onLostPointerCapture={endMessageTaskResize} /><button type="button" className="panel-resize-handle task-panel-resize-handle is-width-only is-right-edge" aria-label="Resize task panel from the right edge" onPointerDown={(event) => beginMessageTaskResize(event, "right")} onPointerMove={updateMessageTaskResize} onPointerUp={endMessageTaskResize} onPointerCancel={endMessageTaskResize} onLostPointerCapture={endMessageTaskResize} /><button type="button" className="panel-resize-handle task-panel-resize-handle is-height-only is-bottom-edge" aria-label="Resize task panel from the bottom edge" onPointerDown={(event) => beginMessageTaskResize(event, "bottom")} onPointerMove={updateMessageTaskResize} onPointerUp={endMessageTaskResize} onPointerCancel={endMessageTaskResize} onLostPointerCapture={endMessageTaskResize} /></>}
        </aside>
      ) : null}
    </section>
  );
}

export function BuzzProjectPrototype() {
  const [activeTab, setActiveTab] = useState<"me" | "messages" | "projects">("me");
  const [workspaceCreateOpen, setWorkspaceCreateOpen] = useState(false);
  const [bestieOpen, setBestieOpen] = useState(false);
  const [bestieContext, setBestieContext] = useState<string | null>(null);
  const [bestieInitialPosition, setBestieInitialPosition] = useState<{ x: number; y: number } | null>(null);
  const [bestiePanelWidth, setBestiePanelWidth] = useState(BESTIE_CONTEXT_PANEL_WIDTH);
  const [bestieDockedInMessages, setBestieDockedInMessages] = useState(false);
  const appShellRef = useRef<HTMLElement>(null);
  const bestieMessagePanelOpen = activeTab === "messages" && bestieOpen && (bestieDockedInMessages || Boolean(bestieContext?.startsWith("messages:")));

  const showTab = (tab: "me" | "messages" | "projects") => {
    if (bestieDockedInMessages) {
      setBestieOpen(false);
      setBestieInitialPosition(null);
    }
    setBestieContext(null);
    setBestieDockedInMessages(false);
    setWorkspaceCreateOpen(false);
    setActiveTab(tab);
  };

  const openWorkspaceCreator = () => {
    setBestieOpen(false);
    setBestieContext(null);
    setBestieInitialPosition(null);
    setBestieDockedInMessages(false);
    setActiveTab("me");
    setWorkspaceCreateOpen(true);
  };

  return (
    <main ref={appShellRef} className="app-shell">
      <header className="topbar">
        <div className="header-spacer" aria-hidden="true" />
        <SegmentedNavigation
          activeValue={activeTab}
          ariaLabel="Primary navigation"
          variant="tabs"
          items={[
            { value: "me", label: "Home", icon: <House aria-hidden="true" size={17} strokeWidth={1.8} /> },
            { value: "messages", label: "Messages", icon: <MessageSquareText aria-hidden="true" size={17} strokeWidth={1.8} /> },
            { value: "projects", label: "Projects", icon: <Folder aria-hidden="true" size={17} strokeWidth={1.8} /> },
          ]}
          onChange={showTab}
          trailing={<button type="button" className={`add-button${workspaceCreateOpen ? " is-active" : ""}`} aria-label="Create new" aria-expanded={workspaceCreateOpen} onClick={openWorkspaceCreator}><Plus aria-hidden="true" size={17} strokeWidth={1.8} /></button>}
        />
        <div className="utilities"><BestieDragButton open={bestieOpen || Boolean(bestieContext)} onToggle={() => { setBestieContext(null); setBestieInitialPosition(null); setBestieDockedInMessages(false); setBestieOpen((open) => !open); }} onContextDrop={(context, targetRect) => {
          setBestieContext(context);
          if (activeTab === "messages") {
            const panelWidth = BESTIE_CONTEXT_PANEL_WIDTH;
            setBestiePanelWidth(panelWidth);
            setBestieDockedInMessages(true);
            const x = Math.max(20, window.innerWidth - panelWidth - 20);
            const y = TOPBAR_CONTENT_OFFSET;
            setBestieInitialPosition({ x, y });
            setBestieOpen(true);
          } else {
            setBestieInitialPosition(null);
            setBestieDockedInMessages(false);
            setBestieOpen(false);
          }
        }} /><button type="button" className="round-button" aria-label="Search"><Search className="search-icon" aria-hidden="true" size={18} strokeWidth={1.75} /></button><div className="avatar" aria-label="Cynthia"><img src={USER_PROFILES.cynthia} alt="" /></div></div>
      </header>

      {bestieOpen ? <BestieChat onClose={() => { setBestieOpen(false); setBestieDockedInMessages(false); }} modularMode={activeTab === "messages"} context={bestieContext} initialPosition={bestieInitialPosition} contextPanel={bestieMessagePanelOpen} onContextPanelWidthChange={setBestiePanelWidth} onMessagesPanelDockChange={setBestieDockedInMessages} /> : null}

      <div key={activeTab} className="page-content-transition">
        {activeTab === "projects" ? (
          <ProjectsView bestieContext={bestieContext} onClearBestie={() => setBestieContext(null)} />
        ) : activeTab === "messages" ? (
          <MessagesView onEnterCatchUp={() => { setBestieOpen(false); setBestieContext(null); setBestieInitialPosition(null); setBestieDockedInMessages(false); }} bestiePanelOpen={bestieMessagePanelOpen} bestiePanelWidth={bestiePanelWidth} bestieContext={bestieContext} />
        ) : (
          <MeView bestieContext={bestieContext} onClearBestie={() => setBestieContext(null)} />
        )}
      </div>
      {workspaceCreateOpen ? <WorkspaceCreateView onClose={() => setWorkspaceCreateOpen(false)} /> : null}
    </main>
  );
}
