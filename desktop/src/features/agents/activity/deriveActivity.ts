// Ported/adapted from Berd's agent-activity-panel (branch
// zmarley/agent-activity-panel). Derives activity events from decrypted
// NIP-AO observer frames: acp_read session/update tool calls are merged by
// toolCallId, then mapped to read/write/create/status events using Berd's
// battle-tested path and tool-kind heuristics.

import type { ObserverEvent } from "../ui/agentSessionTypes";
import {
  type ActivityAgent,
  type ActivityEvent,
  colorForAgent,
} from "./activityModel";
import { basename } from "./activityUtils";

export interface DeriveActivityOptions {
  agentId: string;
  agentName?: string;
  agentIndex?: number;
  roots?: readonly string[];
}

interface CandidateEvent extends ActivityEvent {
  createEligible?: boolean;
}

/** Merged view of one tool call across its tool_call/tool_call_update frames. */
interface ToolCallRecord {
  toolCallId: string;
  /** Epoch ms of the first frame that mentioned this tool call. */
  t: number;
  sourceSeq: number;
  sourceTurnId?: string;
  intent?: string;
  /** First-known-wins: the initial tool_call names the tool. */
  title?: string;
  /** First-known-wins ACP ToolKind string (read/edit/execute/other/…). */
  acpKind?: string;
  /** Last-known-wins: goose populates locations on completion updates. */
  locations: string[];
  /** Last-known-wins tool arguments. */
  rawInput?: Record<string, unknown>;
}

// Mirrored from Berd (which mirrored chat/lib/toolCallPresentation.ts) so
// activity extraction agrees with tool-card summaries about the common
// argument names that carry paths.
const PATH_ARGUMENT_KEYS = [
  "path",
  "file",
  "filePath",
  "filepath",
  "targetPath",
  "directory",
  "dir",
  "cwd",
  "folder",
] as const;

const SEARCH_PATH_ARGUMENT_KEYS = [
  "path",
  "directory",
  "dir",
  "cwd",
  "folder",
] as const;

function asRecord(value: unknown): Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function asString(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function cleanPathValue(path: string): string | undefined {
  const normalized = path.trim();
  if (normalized.length === 0) return undefined;
  return stripTrailingSlash(normalized);
}

function stripTrailingSlash(path: string): string {
  if (path === "/") return path;
  return path.replace(/\/+$/, "");
}

function isAbsolutePath(path: string): boolean {
  return path.startsWith("/");
}

function isSameOrDescendant(path: string, root: string): boolean {
  return path === root || path.startsWith(`${root}/`);
}

function abbreviateOutOfRootPath(path: string): string {
  if (path.startsWith("~/")) return path;

  const segments = path.split("/").filter(Boolean);
  if (segments.length === 0) return "~/…";

  return `~/…/${segments.slice(-Math.min(3, segments.length)).join("/")}`;
}

export function normalizeActivityPath(
  rawPath: string,
  roots: readonly string[],
): string | undefined {
  const path = cleanPathValue(rawPath);
  if (!path) return undefined;

  if (!isAbsolutePath(path)) {
    return path.replace(/^\.\/+/, "");
  }

  const normalizedRoots = roots
    .map(cleanPathValue)
    .filter((root): root is string => Boolean(root))
    .sort((a, b) => b.length - a.length);

  const matchingRoot = normalizedRoots.find((root) =>
    isSameOrDescendant(path, root),
  );

  if (!matchingRoot) {
    return abbreviateOutOfRootPath(path);
  }

  const relative = path.slice(matchingRoot.length).replace(/^\/+/, "");
  return relative.length > 0 ? relative : basename(matchingRoot);
}

function collectStringValues(value: unknown): string[] {
  if (typeof value === "string") {
    const trimmed = value.trim();
    return trimmed.length > 0 ? [trimmed] : [];
  }

  if (Array.isArray(value)) {
    return value.flatMap(collectStringValues);
  }

  return [];
}

function pathArguments(
  args: Record<string, unknown>,
  keys: readonly string[],
): string[] {
  return keys.flatMap((key) => collectStringValues(args[key]));
}

function uniqueNormalizedPaths(
  rawPaths: readonly string[],
  roots: readonly string[],
): string[] {
  const seen = new Set<string>();
  const out: string[] = [];

  for (const rawPath of rawPaths) {
    const path = normalizeActivityPath(rawPath, roots);
    if (!path || seen.has(path)) continue;
    seen.add(path);
    out.push(path);
  }

  return out;
}

/**
 * Berd rendered labels as "extension.tool". Over raw ACP we only have the
 * title (goose and buzz-agent set it to the tool name, e.g.
 * "developer__shell"), so translate the first "__" into the same dot form.
 */
function labelForTool(record: ToolCallRecord): string {
  const raw = record.title ?? record.toolCallId;
  const separator = raw.indexOf("__");
  if (separator > 0) {
    return `${raw.slice(0, separator)}.${raw.slice(separator + 2)}`;
  }
  return raw;
}

function resolvedToolName(record: ToolCallRecord): string {
  const raw = record.title ?? "";
  const separator = raw.indexOf("__");
  return separator > 0 ? raw.slice(separator + 2) : raw;
}

function toolHasExtension(
  record: ToolCallRecord,
  extensionName: string,
): boolean {
  return (record.title ?? "").startsWith(`${extensionName}__`);
}

function candidateForPath(
  record: ToolCallRecord,
  agentId: string,
  kind: "r" | "w",
  path: string,
  createEligible = false,
): CandidateEvent {
  return {
    agentId,
    t: record.t,
    kind,
    path,
    label: labelForTool(record),
    ...(record.intent ? { intent: record.intent } : {}),
    sourceSeq: record.sourceSeq,
    sourceToolCallId: record.toolCallId,
    ...(record.sourceTurnId ? { sourceTurnId: record.sourceTurnId } : {}),
    createEligible,
  };
}

function statusCandidate(
  record: ToolCallRecord,
  agentId: string,
): CandidateEvent {
  return {
    agentId,
    t: record.t,
    kind: "s",
    label: labelForTool(record),
    ...(record.intent ? { intent: record.intent } : {}),
    sourceSeq: record.sourceSeq,
    sourceToolCallId: record.toolCallId,
    ...(record.sourceTurnId ? { sourceTurnId: record.sourceTurnId } : {}),
  };
}

function pushPathCandidates(
  candidates: CandidateEvent[],
  record: ToolCallRecord,
  agentId: string,
  paths: readonly string[],
  roots: readonly string[],
  kind: "r" | "w",
  createEligible = false,
) {
  const normalizedPaths = uniqueNormalizedPaths(paths, roots);

  if (normalizedPaths.length === 0) {
    candidates.push(statusCandidate(record, agentId));
    return;
  }

  for (const path of normalizedPaths) {
    candidates.push(
      candidateForPath(record, agentId, kind, path, createEligible),
    );
  }
}

function textEditorCommandKind(command: unknown): "r" | "w" {
  if (typeof command !== "string") return "r";

  switch (command) {
    case "write":
    case "create":
    case "str_replace":
    case "insert":
    case "edit":
    case "undo_edit":
      return "w";
    default:
      return "r";
  }
}

/**
 * Berd's flat-name mapping for tool calls without a usable ACP kind. In Berd
 * this path handled replayed messages that lost toolKind; in Buzz it also
 * handles adapters that classify everything as "other" (buzz-agent does) —
 * the tool name in the title is then the best signal we have.
 */
function flatNameCandidates(
  record: ToolCallRecord,
  agentId: string,
  roots: readonly string[],
): CandidateEvent[] {
  const candidates: CandidateEvent[] = [];
  const toolName = resolvedToolName(record);
  const args = record.rawInput ?? {};
  const paths =
    record.locations.length > 0
      ? record.locations
      : pathArguments(args, PATH_ARGUMENT_KEYS);

  if (toolName === "text_editor" && toolHasExtension(record, "developer")) {
    const kind = textEditorCommandKind(args.command);
    pushPathCandidates(
      candidates,
      record,
      agentId,
      paths,
      roots,
      kind,
      kind === "w",
    );
    return candidates;
  }

  // Flat developer-extension tool names in current goose builds:
  // write { path, content }, edit { path, before, after }, tree { path, depth }.
  // Not gated on extension metadata since titles may omit the prefix.
  if (toolName === "write") {
    pushPathCandidates(candidates, record, agentId, paths, roots, "w", true);
    return candidates;
  }

  if (toolName === "edit") {
    // Edits require existing content, so they are never create-eligible.
    pushPathCandidates(candidates, record, agentId, paths, roots, "w", false);
    return candidates;
  }

  if (toolName === "tree") {
    pushPathCandidates(candidates, record, agentId, paths, roots, "r");
    return candidates;
  }

  if (toolName === "shell") {
    if (record.locations.length > 0) {
      pushPathCandidates(
        candidates,
        record,
        agentId,
        record.locations,
        roots,
        "r",
      );
    } else {
      candidates.push(statusCandidate(record, agentId));
    }
    return candidates;
  }

  if (paths.length > 0) {
    pushPathCandidates(candidates, record, agentId, paths, roots, "r");
  } else {
    candidates.push(statusCandidate(record, agentId));
  }
  return candidates;
}

function toolCandidates(
  record: ToolCallRecord,
  agentId: string,
  roots: readonly string[],
): CandidateEvent[] {
  const candidates: CandidateEvent[] = [];
  const argPaths = (keys: readonly string[]) =>
    record.locations.length > 0
      ? record.locations
      : pathArguments(record.rawInput ?? {}, keys);

  switch (record.acpKind) {
    case "read":
      pushPathCandidates(
        candidates,
        record,
        agentId,
        argPaths(PATH_ARGUMENT_KEYS),
        roots,
        "r",
      );
      return candidates;

    case "edit":
      pushPathCandidates(
        candidates,
        record,
        agentId,
        argPaths(PATH_ARGUMENT_KEYS),
        roots,
        "w",
        true,
      );
      return candidates;

    case "delete":
    case "move":
      pushPathCandidates(
        candidates,
        record,
        agentId,
        argPaths(PATH_ARGUMENT_KEYS),
        roots,
        "w",
      );
      return candidates;

    case "search":
      pushPathCandidates(
        candidates,
        record,
        agentId,
        argPaths(SEARCH_PATH_ARGUMENT_KEYS),
        roots,
        "r",
      );
      return candidates;

    case "execute":
      // ACP execute locations currently expose touched paths, not access
      // kind. Treat each location as a read-like touch (Berd behavior).
      pushPathCandidates(
        candidates,
        record,
        agentId,
        record.locations,
        roots,
        "r",
      );
      return candidates;

    case "fetch":
    case "think":
    case "switch_mode":
      candidates.push(statusCandidate(record, agentId));
      return candidates;

    // "other" and missing kinds fall through to the flat-name mapping.
    // Deviation from Berd (which sent "other" straight to status): adapters
    // like buzz-agent classify every tool as "other" while still naming it
    // in the title, so the name is the better signal here. Tools without
    // path arguments still end up as status events, matching Berd.
    case "other":
    case undefined:
      return flatNameCandidates(record, agentId, roots);

    default:
      candidates.push(statusCandidate(record, agentId));
      return candidates;
  }
}

const INTENT_MAX_LENGTH = 240;

/**
 * Compact long narration by keeping its trailing sentences — the text
 * closest to the tool call is the most intent-like ("… Now let me patch the
 * store:"). Complete sentences render without an ellipsis; the hard cut
 * (leading …) only fires when a single sentence exceeds the cap.
 */
function compactIntent(text: string): string {
  if (text.length <= INTENT_MAX_LENGTH) return text;

  const sentences = text.split(/(?<=[.!?…])\s+/);
  let kept = "";
  for (let index = sentences.length - 1; index >= 0; index -= 1) {
    const candidate = kept ? `${sentences[index]} ${kept}` : sentences[index];
    if (candidate.length > INTENT_MAX_LENGTH) break;
    kept = candidate;
  }
  if (kept) return kept;

  return `…${text.slice(1 - INTENT_MAX_LENGTH).trimStart()}`;
}

/** Extract plain text from an ACP content block (or array of them). */
function textFromContent(content: unknown): string {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content.map(textFromContent).filter(Boolean).join(" ");
  }
  const record = asRecord(content);
  const text = asString(record.text);
  return text ?? "";
}

function frameTime(frame: ObserverEvent): number {
  const parsed = Date.parse(frame.timestamp);
  return Number.isFinite(parsed) ? parsed : 0;
}

function locationPathsFromUpdate(update: Record<string, unknown>): string[] {
  const locations = update.locations;
  if (!Array.isArray(locations)) return [];
  return locations.flatMap((location) => {
    const path = asString(asRecord(location).path);
    return path ? [path] : [];
  });
}

function parentDir(path: string): string {
  const index = path.lastIndexOf("/");
  return index <= 0 ? "/" : path.slice(0, index);
}

function segmentDepth(path: string): number {
  return path.split("/").filter(Boolean).length;
}

/**
 * Infer workspace roots from the absolute paths in a frame window. Buzz
 * observer frames carry no workspace metadata (Berd read roots from its
 * workspace model), so we use the longest common directory prefix of every
 * absolute path the agent touched. Mixed streams (repo + /etc) collapse the
 * prefix below the depth floor and we fall back to no roots, which renders
 * abbreviated paths — Berd's out-of-root behavior.
 */
export function inferActivityRoots(frames: readonly ObserverEvent[]): string[] {
  const dirs: string[] = [];

  for (const frame of frames) {
    if (frame.kind !== "acp_read") continue;
    const payload = asRecord(frame.payload);
    if (asString(payload.method) !== "session/update") continue;
    const update = asRecord(asRecord(payload.params).update);
    const updateType = asString(update.sessionUpdate);
    if (updateType !== "tool_call" && updateType !== "tool_call_update") {
      continue;
    }

    const rawPaths = [
      ...locationPathsFromUpdate(update),
      ...pathArguments(asRecord(update.rawInput), PATH_ARGUMENT_KEYS),
    ];
    for (const rawPath of rawPaths) {
      const path = cleanPathValue(rawPath);
      if (path && isAbsolutePath(path)) dirs.push(parentDir(path));
    }
  }

  if (dirs.length === 0) return [];

  let prefix = dirs[0];
  for (const dir of dirs) {
    while (prefix !== "/" && !isSameOrDescendant(dir, prefix)) {
      prefix = parentDir(prefix);
    }
    if (prefix === "/") break;
  }

  return segmentDepth(prefix) >= 3 ? [prefix] : [];
}

/**
 * Derive activity events for one agent from its decrypted observer frames.
 * Frames are expected in seq order (the observer store keeps them sorted and
 * batch-unwrapped). Multi-agent scenes compose by concatenating results.
 */
export function deriveActivityFromObserverFrames(
  frames: readonly ObserverEvent[],
  opts: DeriveActivityOptions,
): { events: ActivityEvent[]; agents: ActivityAgent[] } {
  const roots = opts.roots ?? [];
  const records = new Map<string, ToolCallRecord>();
  const recordOrder: ToolCallRecord[] = [];

  let firstFrameT: number | undefined;
  let currentTurnKey: string | null | undefined;
  let narration = "";

  const resetNarrationOnTurnChange = (frame: ObserverEvent) => {
    const turnKey = frame.turnId ?? null;
    if (turnKey !== currentTurnKey) {
      currentTurnKey = turnKey;
      narration = "";
    }
  };

  for (const frame of frames) {
    if (firstFrameT === undefined) firstFrameT = frameTime(frame);
    resetNarrationOnTurnChange(frame);

    if (frame.kind !== "acp_read") continue;

    const payload = asRecord(frame.payload);
    if (asString(payload.method) !== "session/update") continue;

    const params = asRecord(payload.params);
    const update = asRecord(params.update);
    const updateType = asString(update.sessionUpdate);

    if (updateType === "agent_message_chunk") {
      const text = textFromContent(update.content);
      if (text) narration = narration ? `${narration} ${text}` : text;
      continue;
    }

    if (updateType !== "tool_call" && updateType !== "tool_call_update") {
      continue;
    }

    const toolCallId = asString(update.toolCallId);
    if (!toolCallId) continue;

    const sessionKey =
      asString(params.sessionId) ?? frame.sessionId ?? "unknown-session";
    const recordKey = `${sessionKey}:${toolCallId}`;

    let record = records.get(recordKey);
    if (!record) {
      record = {
        toolCallId,
        t: frameTime(frame),
        sourceSeq: frame.seq,
        ...(frame.turnId ? { sourceTurnId: frame.turnId } : {}),
        locations: [],
      };
      if (updateType === "tool_call") {
        const compactedNarration = narration.replace(/\s+/g, " ").trim();
        if (compactedNarration) {
          record.intent = compactIntent(compactedNarration);
        }
        narration = "";
      }
      records.set(recordKey, record);
      recordOrder.push(record);
    }

    const title = asString(update.title);
    if (title && record.title === undefined) record.title = title;

    const kind = asString(update.kind);
    if (kind && record.acpKind === undefined) record.acpKind = kind;

    const locations = locationPathsFromUpdate(update);
    if (locations.length > 0) record.locations = locations;

    if (
      typeof update.rawInput === "object" &&
      update.rawInput !== null &&
      !Array.isArray(update.rawInput)
    ) {
      record.rawInput = update.rawInput as Record<string, unknown>;
    }
  }

  const candidates = recordOrder.flatMap((record) =>
    toolCandidates(record, opts.agentId, roots),
  );

  const sortedCandidates = candidates.sort(
    (a, b) => a.t - b.t || (a.sourceSeq ?? 0) - (b.sourceSeq ?? 0),
  );
  const seenPaths = new Set<string>();
  const events: ActivityEvent[] = [];

  for (const candidate of sortedCandidates) {
    const { createEligible, ...event } = candidate;
    if (event.path) {
      const firstTouch = !seenPaths.has(event.path);
      if (createEligible && firstTouch) {
        event.kind = "c";
      }
      seenPaths.add(event.path);
    }
    events.push(event);
  }

  const spawnT = firstFrameT ?? events[0]?.t ?? 0;
  return {
    events,
    agents: [
      {
        id: opts.agentId,
        name: opts.agentName ?? opts.agentId.slice(0, 8),
        color: colorForAgent(opts.agentId, opts.agentIndex),
        spawnT,
      },
    ],
  };
}
