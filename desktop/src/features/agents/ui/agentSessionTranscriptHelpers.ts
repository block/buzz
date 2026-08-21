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
  const semanticTurn = parseSemanticTurn(text);
  if (semanticTurn) return semanticTurn;

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
 * Read the deterministic Buzz-owned turn envelope without treating it as
 * strict XML. Natural-language bodies are escaped by the harness, so the
 * semantic closing tags are unambiguous while preserving a lightweight wire
 * format for every ACP adapter.
 */
function parseSemanticTurn(text: string): {
  sections: PromptSection[];
  userText: string;
  userTitle: string;
  userPubkey: string | null;
  userEventId: string | null;
} | null {
  const turnStart = text.indexOf("<buzz-turn");
  if (turnStart === -1) return null;

  const prefix = text.slice(0, turnStart).trim();
  const turn = text.slice(turnStart);
  const sections = prefix ? parseSystemPromptSections(prefix) : [];

  const ambient = turn.match(
    /<ambient-context\b[^>]*>([\s\S]*?)<\/ambient-context>/,
  );
  if (ambient?.[1]?.trim()) {
    sections.push({ title: "Ambient context", body: ambient[1].trim() });
  }

  const legacyInstruction = turn.match(
    /<current-instruction\b([^>]*)>([\s\S]*?)<\/current-instruction>/,
  );
  // Current events follow ambient context; older envelopes may still wrap
  // them in <current-instruction>.
  const ambientEnd =
    ambient?.index === undefined ? 0 : ambient.index + ambient[0].length;
  const eventSource = legacyInstruction?.[2] ?? turn.slice(ambientEnd);
  const eventMatches = Array.from(
    eventSource.matchAll(/<event\b([^>]*)>([\s\S]*?)<\/event>/g),
  );
  if (!eventMatches.length) {
    return {
      sections,
      userText: "",
      userTitle: "Buzz event",
      userPubkey: null,
      userEventId: null,
    };
  }

  sections.push({
    title: "Current instruction",
    body: eventMatches
      .map((match) => match[0])
      .join("\n")
      .trim(),
  });
  const delivery = turn.match(/<delivery\b[^>]*\/>/);
  if (delivery) sections.push({ title: "Delivery", body: delivery[0] });

  const instructionAttributes = legacyInstruction
    ? parseSemanticAttributes(legacyInstruction[1])
    : {};
  const userText = eventMatches
    .map((match) => {
      const taggedContent = match[2].match(
        /<content>([\s\S]*?)<\/content>/,
      )?.[1];
      const content = taggedContent ?? match[2].replace(/^\s*Content:\s?/, "");
      return decodeSemanticText(content).trim();
    })
    .filter(Boolean)
    .join("\n\n");
  const lastEventAttributes = eventMatches.length
    ? parseSemanticAttributes(eventMatches[eventMatches.length - 1][1])
    : {};
  const promptTag = lastEventAttributes.type ?? lastEventAttributes.prompt_tag;
  const actor = instructionAttributes.actor ?? lastEventAttributes.actor;
  const eventId =
    instructionAttributes.event_id ?? lastEventAttributes.event_id;

  return {
    sections,
    userText,
    userTitle: promptTag ? titleCase(promptTag) : "Buzz event",
    userPubkey: actor?.toLowerCase() ?? null,
    userEventId: eventId?.toLowerCase() ?? null,
  };
}

function parseSemanticAttributes(value: string): Record<string, string> {
  const attributes: Record<string, string> = {};
  for (const match of value.matchAll(/([a-zA-Z_][\w-]*)="([^"]*)"/g)) {
    attributes[match[1]] = decodeSemanticText(match[2]);
  }
  return attributes;
}

function decodeSemanticText(value: string): string {
  return value
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&quot;", '"')
    .replaceAll("&apos;", "'")
    .replaceAll("&amp;", "&");
}

/**
 * Split paired semantic standing-context tags into transcript sections.
 * Bracket-header parsing below remains for stored observer history.
 */
export function parseSystemPromptSections(
  systemPrompt: string,
): PromptSection[] {
  const semantic = parseSemanticStandingSections(systemPrompt);
  if (semantic) return semantic;

  // Legacy bracket-framed prompts.
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

function parseSemanticStandingSections(
  systemPrompt: string,
): PromptSection[] | null {
  const titles: Record<string, string> = {
    workspace: "Workspace",
    base: "Base",
    system: "System",
    "team-instructions": "Team Instructions",
    "core-memory": "Core Memory",
    "huddle-instructions": "Huddle Instructions",
    "channel-canvas": "Channel Canvas",
  };
  const tags = Object.keys(titles).join("|");
  const matches = Array.from(
    systemPrompt.matchAll(new RegExp(`<(${tags})>([\\s\\S]*?)<\\/\\1>`, "g")),
  );
  if (!matches.length) return null;
  return matches.map((match) => ({
    title: titles[match[1]],
    body: decodeSemanticText(match[2]).trim(),
  }));
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
