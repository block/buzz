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
 * `Core Memory` sub-sections deterministically.
 *
 * The harness frames the value as:
 *   `[Base]\n{base}\n\n[System]\n{persona}\n\n[Agent Memory — core]\n{core}`
 * with any section omitted when absent. Two core extraction cases:
 *
 * - **Start of string** (`[Agent Memory — core]\n…`): core-only input with no
 *   Base/System prefix.
 * - **Appended frame** (`\n\n[Agent Memory — core]\n…`): the blank-line separator
 *   that `with_core()` always emits before appending the core. Using `LAST`
 *   occurrence ensures an earlier mention of the header inside a persona body
 *   (with only a single preceding newline) stays literal.
 *
 * The remaining prefix after core extraction is split on the FIRST
 * `\n[System]\n` boundary into Base/System. Unlike the generic
 * `parsePromptSections`, no embedded `[...]` line inside a body can start a new
 * section — so a persona containing a bracketed line, or a mid-string-elided
 * header on an oversize prompt, can never drop a label or inflate the count.
 */
export function parseSystemPromptSections(
  systemPrompt: string,
): PromptSection[] {
  const sections: PromptSection[] = [];

  // ── 1. Extract the [Agent Memory — core] block ───────────────────────────
  // Two producer shapes:
  //   • Core-only: systemPrompt starts with the header (no Base/System).
  //   • Appended: `with_core()` emits "\n\n[Agent Memory — core]\n{core}".
  //     Using the LAST occurrence of the double-newline-prefixed boundary
  //     means a bare "[Agent Memory — core]" line inside a persona (which
  //     can only be preceded by a single newline in the section body) is
  //     never confused with the real appended frame.
  const CORE_HEADER = "[Agent Memory — core]";
  const CORE_MARKER_INLINE = `\n\n${CORE_HEADER}\n`; // blank-line-prefixed appended boundary
  let coreBody: string | null = null;
  let baseAndSystem = systemPrompt;

  if (systemPrompt.startsWith(`${CORE_HEADER}\n`)) {
    // Core-only input (no Base/System).
    coreBody = systemPrompt.slice(`${CORE_HEADER}\n`.length).trim();
    baseAndSystem = "";
  } else {
    const lastAt = systemPrompt.lastIndexOf(CORE_MARKER_INLINE);
    if (lastAt !== -1) {
      coreBody = systemPrompt.slice(lastAt + CORE_MARKER_INLINE.length).trim();
      baseAndSystem = systemPrompt.slice(0, lastAt);
    }
  }

  // ── 2. Parse Base/System from the remaining prefix ────────────────────────
  if (baseAndSystem) {
    // Persona-only frame: no [Base], starts directly with [System].
    if (baseAndSystem.startsWith("[System]\n")) {
      const body = baseAndSystem.slice("[System]\n".length).trim();
      if (body) sections.push({ title: "System", body });
    } else {
      // Base (up to the first [System] boundary) or base-only.
      const marker = "\n[System]\n";
      const at = baseAndSystem.indexOf(marker);
      const head = at === -1 ? baseAndSystem : baseAndSystem.slice(0, at);
      const baseBody = head.replace(/^\[Base]\n/, "").trim();
      if (baseBody) sections.push({ title: "Base", body: baseBody });

      if (at !== -1) {
        const systemBody = baseAndSystem.slice(at + marker.length).trim();
        if (systemBody) sections.push({ title: "System", body: systemBody });
      }
    }
  }

  // ── 3. Append core section last ───────────────────────────────────────────
  // Map the wire header to a human-readable display title.
  if (coreBody) sections.push({ title: "Core Memory", body: coreBody });

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
