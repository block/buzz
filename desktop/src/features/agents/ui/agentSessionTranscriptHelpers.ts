import type { ObserverEvent, PromptSection } from "./agentSessionTypes";
import {
  findBuzzToolName,
  isGenericToolTitle,
  normalizeToolName,
} from "./agentSessionToolCatalog";
import { asRecord, asString, titleCase } from "./agentSessionUtils";

export function extractPromptText(payload: Record<string, unknown>): string {
  const params = asRecord(payload.params);
  const prompt = params.prompt;
  if (!Array.isArray(prompt)) return "";
  return prompt.map(extractBlockText).filter(Boolean).join("\n");
}

export function parsePromptText(text: string): {
  sections: PromptSection[];
  userText: string;
  userTitle: string;
  userPubkey: string | null;
  userEventId: string | null;
} {
  const sections = parsePromptSections(text).filter(
    (s) => s.body.trim().length > 0,
  );
  if (sections.length === 0) {
    return {
      sections: [],
      userText: text.trim(),
      userTitle: "Prompt",
      userPubkey: null,
      userEventId: null,
    };
  }

  const eventSection = sections.find((section) => {
    const title = section.title.toLowerCase();
    return title.startsWith("buzz event");
  });
  const eventContent = eventSection
    ? extractEventContent(eventSection.body)
    : "";
  const eventAuthorPubkey = eventSection
    ? extractEventAuthorPubkey(eventSection.body)
    : null;
  const eventId = eventSection ? extractEventId(eventSection.body) : null;
  const eventKind = eventSection?.title.split(":").slice(1).join(":").trim();

  return {
    sections,
    userText: eventContent,
    userTitle: eventKind ? titleCase(eventKind) : "Buzz event",
    userPubkey: eventAuthorPubkey,
    userEventId: eventId,
  };
}

/**
 * Split the framed `session/new` `systemPrompt` into its `Base`/`System`/
 * `Team Instructions`/`Core Memory`/`Channel Canvas` sub-sections
 * deterministically.
 *
 * The harness composes the value in order:
 *   `[Base]\n{base}\n\n[System]\n{persona}\n\n[Team Instructions]\n{team}\n\n[Agent Memory — core]\n{core}\n\n[Channel Canvas]\n{canvas}`
 * with any section omitted when absent. Extraction runs in reverse producer
 * order so that each `lastIndexOf` search operates on the full input and each
 * extraction boundary is unambiguous.
 *
 * Five extraction passes:
 *
 * 1. **Canvas** (`[Channel Canvas]`): appended last by `with_canvas()`.
 *    - Start-of-string: canvas-only input.
 *    - Appended frame (`\n\n[Channel Canvas]\n`): blank-line separator used by
 *      `with_canvas()`; LAST occurrence guards against an embedded header in a
 *      persona body (single preceding newline only).
 *
 * 2. **Core** (`[Agent Memory — core]`): appended before canvas by `with_core()`.
 *    Same two cases, same last-occurrence guard.
 *
 * 3. **Team Instructions** (`[Team Instructions]`): appended before core by
 *    `with_team()` in `buzz-acp/src/pool.rs`. Same two cases (start-of-string
 *    or `\n\n[Team Instructions]\n` inline), same last-occurrence guard. Output
 *    position: after System, before Core Memory.
 *
 * 4. **Base/System**: remainder after the three top-level section extractions.
 *    Split on the first `\n[System]\n` boundary; no embedded `[...]` line
 *    inside a body can start a new section.
 *
 * 5. **Legacy Team Instructions** (backward compat): if the `System` body
 *    contains the exact canonical delimiter `\n\n---\n# Team Instructions\n`
 *    (produced by the now-removed `compose_prompt()` in buzz-persona), the body
 *    is split at the **last** occurrence of that boundary. The text before
 *    becomes the `System` body; the text after becomes a `Team Instructions`
 *    section inserted immediately after `System`. Non-canonical lookalikes
 *    (bare `---` without the heading, a `# Team Instructions` on a different
 *    line, or only a single preceding newline) are kept literal inside `System`.
 */
export function parseSystemPromptSections(
  systemPrompt: string,
): PromptSection[] {
  const sections: PromptSection[] = [];

  // ── 1. Extract [Channel Canvas] ───────────────────────────────────────────
  const CANVAS_HEADER = "[Channel Canvas]";
  const CANVAS_MARKER_INLINE = `\n\n${CANVAS_HEADER}\n`;
  let canvasBody: string | null = null;
  let remainder = systemPrompt;

  if (remainder.startsWith(`${CANVAS_HEADER}\n`)) {
    canvasBody = remainder.slice(`${CANVAS_HEADER}\n`.length).trim();
    remainder = "";
  } else {
    const lastCanvas = remainder.lastIndexOf(CANVAS_MARKER_INLINE);
    if (lastCanvas !== -1) {
      canvasBody = remainder
        .slice(lastCanvas + CANVAS_MARKER_INLINE.length)
        .trim();
      remainder = remainder.slice(0, lastCanvas);
    }
  }

  // ── 2. Extract [Agent Memory — core] ──────────────────────────────────────
  const CORE_HEADER = "[Agent Memory — core]";
  const CORE_MARKER_INLINE = `\n\n${CORE_HEADER}\n`;
  let coreBody: string | null = null;

  if (remainder.startsWith(`${CORE_HEADER}\n`)) {
    coreBody = remainder.slice(`${CORE_HEADER}\n`.length).trim();
    remainder = "";
  } else {
    const lastCore = remainder.lastIndexOf(CORE_MARKER_INLINE);
    if (lastCore !== -1) {
      coreBody = remainder.slice(lastCore + CORE_MARKER_INLINE.length).trim();
      remainder = remainder.slice(0, lastCore);
    }
  }

  // ── 3. Extract [Team Instructions] (modern runtime framing) ─────────────
  // with_team() in buzz-acp/src/pool.rs appends "\n\n[Team Instructions]\n{instructions}"
  // after [System] and before core/canvas. Same two cases as canvas/core:
  // start-of-string (team-only input) or the inline double-newline marker
  // (last occurrence guards against embedded lookalikes preceded by a single \n).
  const TEAM_HEADER = "[Team Instructions]";
  const TEAM_MARKER_INLINE = `\n\n${TEAM_HEADER}\n`;
  let modernTeamBody: string | null = null;

  if (remainder.startsWith(`${TEAM_HEADER}\n`)) {
    modernTeamBody = remainder.slice(`${TEAM_HEADER}\n`.length).trim();
    remainder = "";
  } else {
    const lastTeam = remainder.lastIndexOf(TEAM_MARKER_INLINE);
    if (lastTeam !== -1) {
      modernTeamBody = remainder
        .slice(lastTeam + TEAM_MARKER_INLINE.length)
        .trim();
      remainder = remainder.slice(0, lastTeam);
    }
  }

  // ── 4. Parse Base/System from the remaining prefix ────────────────────────
  // The canonical team-instructions delimiter produced by compose_prompt() in
  // buzz-persona/src/resolve.rs:
  //   format!("{persona_prompt}\n\n---\n# Team Instructions\n{instructions}")
  const TEAM_DELIMITER = "\n\n---\n# Team Instructions\n";

  // splitSystemBody: split a raw [System] body string at the last occurrence
  // of the canonical team delimiter, returning { systemBody, teamBody | null }.
  // Using lastIndexOf mirrors the canvas/core last-occurrence guard: a persona
  // author can embed an exact delimiter-like passage inside the persona body;
  // only the final occurrence is the producer boundary appended by compose_prompt().
  function splitSystemBody(raw: string): {
    systemBody: string;
    teamBody: string | null;
  } {
    const at = raw.lastIndexOf(TEAM_DELIMITER);
    if (at === -1) return { systemBody: raw.trim(), teamBody: null };
    return {
      systemBody: raw.slice(0, at).trim(),
      teamBody: raw.slice(at + TEAM_DELIMITER.length).trim() || null,
    };
  }

  const baseAndSystem = remainder;
  if (baseAndSystem) {
    if (baseAndSystem.startsWith("[System]\n")) {
      const raw = baseAndSystem.slice("[System]\n".length);
      const { systemBody, teamBody } = splitSystemBody(raw);
      if (systemBody) sections.push({ title: "System", body: systemBody });
      if (teamBody)
        sections.push({ title: "Team Instructions", body: teamBody });
    } else {
      const marker = "\n[System]\n";
      const at = baseAndSystem.indexOf(marker);
      const head = at === -1 ? baseAndSystem : baseAndSystem.slice(0, at);
      const baseBody = head.replace(/^\[Base]\n/, "").trim();
      if (baseBody) sections.push({ title: "Base", body: baseBody });

      if (at !== -1) {
        const raw = baseAndSystem.slice(at + marker.length);
        const { systemBody, teamBody } = splitSystemBody(raw);
        if (systemBody) sections.push({ title: "System", body: systemBody });
        if (teamBody)
          sections.push({ title: "Team Instructions", body: teamBody });
      }
    }
  }

  // ── 5. Append team (modern), core, and canvas sections in producer order ──
  if (modernTeamBody)
    sections.push({ title: "Team Instructions", body: modernTeamBody });
  if (coreBody) sections.push({ title: "Core Memory", body: coreBody });
  if (canvasBody) sections.push({ title: "Channel Canvas", body: canvasBody });

  return sections;
}

function parsePromptSections(text: string): PromptSection[] {
  const sections: PromptSection[] = [];
  let current: PromptSection | null = null;
  const preamble: string[] = [];

  for (const line of text.split(/\r?\n/)) {
    const header = line.match(/^\[([^\]]+)]\s*$/);
    if (header) {
      if (current) {
        sections.push({
          title: current.title,
          body: current.body.trim(),
        });
      } else if (preamble.join("\n").trim()) {
        sections.push({ title: "Prompt", body: preamble.join("\n").trim() });
      }
      current = { title: header[1], body: "" };
      continue;
    }

    if (current) {
      current.body += current.body ? `\n${line}` : line;
    } else {
      preamble.push(line);
    }
  }

  if (current) {
    sections.push({ title: current.title, body: current.body.trim() });
  } else if (preamble.join("\n").trim()) {
    sections.push({ title: "Prompt", body: preamble.join("\n").trim() });
  }

  return sections;
}

const EVENT_CONTENT_BOUNDARY_RE =
  /^(?:Event ID|Channel|Kind|From|Time|Tags|Parsed):\s*/;
const EVENT_BLOCK_BOUNDARY_RE = /^--- Event \d+\b/;

function extractEventContent(body: string): string {
  const lines = body.split(/\r?\n/);
  const chunks: string[] = [];

  for (let i = 0; i < lines.length; i++) {
    const match = lines[i].match(/^Content:\s?(.*)$/);
    if (!match) {
      continue;
    }

    const contentLines = [match[1] ?? ""];
    for (let j = i + 1; j < lines.length; j++) {
      const line = lines[j];
      if (
        EVENT_CONTENT_BOUNDARY_RE.test(line) ||
        EVENT_BLOCK_BOUNDARY_RE.test(line)
      ) {
        break;
      }
      contentLines.push(line);
    }

    const content = contentLines.join("\n").trim();
    if (content) {
      chunks.push(content);
    }
  }

  return chunks.join("\n\n");
}

function extractEventAuthorPubkey(body: string): string | null {
  const fromMatch = body.match(/^From:.*\bhex:\s*([0-9a-fA-F]{64})/m);
  return fromMatch?.[1]?.toLowerCase() ?? null;
}

function extractEventId(body: string): string | null {
  const eventIdMatch = body.match(/^Event ID:\s*([0-9a-fA-F]{64})\b/m);
  return eventIdMatch?.[1]?.toLowerCase() ?? null;
}

export function extractContentText(value: unknown): string {
  if (typeof value === "string") return value;
  if (Array.isArray(value)) return value.map(extractBlockText).join("\n");
  return extractBlockText(value);
}

export function extractBlockText(value: unknown): string {
  if (typeof value === "string") return value;
  if (Array.isArray(value)) return value.map(extractBlockText).join("\n");
  const record = asRecord(value);
  const nestedContent = record.content;
  const rawOutput = record.rawOutput;
  const nestedText =
    nestedContent && typeof nestedContent === "object"
      ? extractBlockText(nestedContent)
      : "";
  const rawOutputText =
    rawOutput === undefined || rawOutput === null
      ? ""
      : typeof rawOutput === "string"
        ? rawOutput
        : JSON.stringify(rawOutput, null, 2);
  const directText = asString(record.text) ?? asString(record.content);
  return directText || nestedText || rawOutputText || "";
}

/**
 * Build markdown checklist text for a `plan` session update.
 *
 * The standard ACP shape (`@agentclientprotocol/codex-acp`) sends
 * `entries[]` — `{ status, content, priority }` — with no top-level
 * `content` field. Older/non-standard adapters instead send
 * `content: { type: "text", text }` directly on the update. `entries`
 * (even empty) is treated as authoritative when present; `content` is
 * only consulted when `entries` is absent, and the raw update is
 * stringified only when neither yields usable text.
 */
export function extractPlanText(update: Record<string, unknown>): string {
  if (Array.isArray(update.entries)) {
    return update.entries
      .map((entry) => formatPlanEntry(asRecord(entry)))
      .filter(Boolean)
      .join("\n");
  }
  const contentText = extractContentText(update.content);
  return contentText || JSON.stringify(update, null, 2);
}

function formatPlanEntry(entry: Record<string, unknown>): string {
  const content = asString(entry.content);
  if (!content) return "";
  const checkbox = entry.status === "completed" ? "[x]" : "[ ]";
  const suffix = entry.status === "in_progress" ? " (in progress)" : "";
  return `- ${checkbox} ${content}${suffix}`;
}

export function extractToolArgs(
  update: Record<string, unknown>,
): Record<string, unknown> {
  const candidates = [
    update.args,
    update.arguments,
    update.input,
    update.rawInput,
  ];
  for (const candidate of candidates) {
    if (
      candidate &&
      typeof candidate === "object" &&
      !Array.isArray(candidate)
    ) {
      return candidate as Record<string, unknown>;
    }
  }
  return {};
}

export function extractToolIdentity(update: Record<string, unknown>): {
  title: string;
  toolName: string;
  buzzToolName: string | null;
} {
  const candidates = collectToolNameCandidates(update);
  const knownName = candidates
    .map((candidate) => findBuzzToolName(candidate, true))
    .find((candidate): candidate is string => Boolean(candidate));
  const firstSpecific = candidates.find(
    (candidate) => !isGenericToolTitle(candidate),
  );
  const title =
    asString(update.title) ?? knownName ?? firstSpecific ?? "Tool call";
  return {
    title,
    toolName: knownName ?? normalizeToolName(firstSpecific ?? title),
    buzzToolName: knownName ?? null,
  };
}

function collectToolNameCandidates(update: Record<string, unknown>): string[] {
  const args = extractToolArgs(update);
  const tool = asRecord(update.tool);
  const input = asRecord(update.input);
  const rawInput = asRecord(update.rawInput);
  const candidates = [
    update.toolName,
    update.tool_name,
    update.name,
    update.title,
    update.kind,
    tool.name,
    tool.toolName,
    args.toolName,
    args.tool_name,
    args.name,
    args.method,
    input.toolName,
    input.tool_name,
    input.name,
    rawInput.toolName,
    rawInput.tool_name,
    rawInput.name,
  ];

  return candidates.flatMap((candidate) => {
    const value = asString(candidate);
    return value ? [value] : [];
  });
}

export function extractToolResult(update: Record<string, unknown>): string {
  const contentText = extractContentText(update.content);
  if (contentText) return contentText;
  return extractBlockText(update.rawOutput);
}

export function extractTriggeringEventIds(payload: unknown): string[] {
  const record = asRecord(payload);
  return Array.isArray(record.triggeringEventIds)
    ? record.triggeringEventIds.filter(
        (id): id is string => typeof id === "string",
      )
    : [];
}

export function describeTurnStarted(payload: unknown): string {
  const ids = extractTriggeringEventIds(payload);
  return ids.length > 0
    ? `Triggered by ${ids.length === 1 ? "1 event" : `${ids.length} events`}.`
    : "";
}

export function describeSessionResolved(payload: unknown): string {
  const record = asRecord(payload);
  const isNewSession = record.isNewSession === true;
  return isNewSession ? "New session created." : "";
}

export function describeRawEvent(event: ObserverEvent): string {
  const payload = asRecord(event.payload);
  const method = asString(payload.method);
  if (method === "session/update") {
    const update = asRecord(asRecord(payload.params).update);
    return asString(update.sessionUpdate) ?? method;
  }
  return method ?? event.kind;
}

/* ------------------------------------------------------------------
 * Payload formatters, moved from `agentSessionTranscript.ts`.
 *
 * That file sits well over the repo's 1000-line budget, so the ratchet
 * blocks any growth in it. These are pure functions over a JSON-RPC
 * payload with no module state, which makes them the cleanest thing to
 * relocate -- and this module already owns the same kind of helper.
 * ------------------------------------------------------------------ */

export function maybeNostrEventId(id: string | null | undefined) {
  return id && /^[0-9a-fA-F]{64}$/.test(id) ? id : null;
}

export function stringifyPayload(value: unknown) {
  try {
    return JSON.stringify(value, null, 2) ?? String(value);
  } catch {
    return String(value);
  }
}

export function describePermissionRequest(payload: Record<string, unknown>) {
  const params = asRecord(payload.params);
  const title =
    asString(params.title) ??
    asString(params.message) ??
    asString(params.reason) ??
    "Permission requested";
  const toolCallId =
    asString(params.toolCallId) ?? asString(params.tool_call_id);
  const options = Array.isArray(params.options)
    ? params.options
        .map((option) => {
          const record = asRecord(option);
          return (
            asString(record.name) ??
            asString(record.kind) ??
            asString(record.optionId)
          );
        })
        .filter((option): option is string => Boolean(option))
    : [];
  const detail: string[] = [];
  if (title !== "Permission requested") detail.push(title);
  if (toolCallId) detail.push(`Tool call: ${toolCallId}`);
  if (options.length > 0) detail.push(`Options: ${options.join(", ")}`);

  // Build optionId → kind map for outcome labeling on the response.
  const optionNames = new Map<string, string>();
  if (Array.isArray(params.options)) {
    for (const option of params.options) {
      const record = asRecord(option);
      const optionId = asString(record.optionId);
      const kind = asString(record.kind);
      if (optionId && kind) {
        optionNames.set(optionId, kind);
      }
    }
  }

  return {
    title,
    text: detail.join("\n"),
    optionNames,
    descriptor: {
      renderClass: "permission" as const,
      label: "Permission requested",
      preview: title,
      action: { verb: "Requested", object: title },
      tone: "admin" as const,
      operation: "session/request_permission",
      object: title,
      source: "acp" as const,
      groupKey: "permission:request",
    },
  };
}

/**
 * Format a human-readable outcome label from a permission response.
 * kind values from ACP: allow_once, allow_always, reject_once, reject_always.
 * "reject_*" kinds are denials; anything else that is selected is an approval.
 */
export function describePermissionOutcome(
  outcome: string,
  optionId: string | null,
  optionNames: Map<string, string>,
): string {
  if (outcome === "cancelled") {
    return "Cancelled";
  }
  if (outcome === "selected" && optionId) {
    const kind = optionNames.get(optionId) ?? optionId;
    const isDenial = kind.startsWith("reject");
    const verb = isDenial ? "Denied" : "Approved";
    return `${verb} (${kind})`;
  }
  return outcome;
}

/**
 * Stable map key for a JSON-RPC id, which may be a string or a finite number
 * per the spec. Using JSON.stringify avoids collisions between the number 1 and
 * the string "1". Returns null for null, undefined, or non-id values (objects,
 * booleans) so callers can gate on presence without a separate type check.
 */
export function jsonRpcId(value: unknown): string | null {
  if (typeof value === "string") return JSON.stringify(value);
  if (typeof value === "number" && Number.isFinite(value))
    return JSON.stringify(value);
  return null;
}

export function describeFreeformStatus(payload: Record<string, unknown>) {
  const statusType = asString(payload.type) ?? asString(payload.status);
  const title =
    asString(payload.title) ?? (statusType ? titleCase(statusType) : null);
  const text = asString(payload.text) ?? asString(payload.message);
  if (!title || !text) return null;
  return { statusType: statusType ?? title.toLowerCase(), title, text };
}

export function rawPayloadTitle(payload: unknown) {
  const record = asRecord(payload);
  return asString(record.method) ?? asString(record.type) ?? "raw_json_rpc";
}
