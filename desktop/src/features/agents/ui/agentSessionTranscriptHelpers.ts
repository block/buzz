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
 *   `[Base]\n{base}\n\n[System]\n{persona}\n\n[Agent Memory — core]\n{core}\n\n[Channel Canvas]\n{canvas}`
 * with any section omitted when absent. For team-pack agents the persona body
 * already contains the pack-level instructions appended by `compose_prompt()`
 * in `buzz-persona/src/resolve.rs`:
 *   `{persona_body}\n\n---\n# Team Instructions\n{pack_instructions}`
 * Extraction runs in reverse producer order so that each `lastIndexOf` search
 * operates on the full input and each extraction boundary is unambiguous.
 *
 * Four extraction passes before Base/System parsing:
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
 * 3. **Base/System**: remainder after canvas and core extraction.
 *    Split on the first `\n[System]\n` boundary; no embedded `[...]` line
 *    inside a body can start a new section.
 *
 * 4. **Team Instructions**: if the `System` body contains the exact canonical
 *    delimiter `\n\n---\n# Team Instructions\n` (produced by `compose_prompt()`),
 *    the body is split at the **last** occurrence of that boundary (same
 *    last-occurrence guard as canvas and core). The text before becomes the
 *    `System` body; the text after becomes a `Team Instructions` section
 *    inserted immediately after `System`. Non-canonical lookalikes (bare `---`
 *    without the heading, a `# Team Instructions` on a different line, or only
 *    a single preceding newline) are kept literal inside `System`.
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

  // ── 3. Parse Base/System from the remaining prefix ────────────────────────
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

  // ── 4. Append core and canvas sections in producer order ──────────────────
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
