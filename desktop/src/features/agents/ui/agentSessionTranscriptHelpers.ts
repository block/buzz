import type { ObserverEvent, PromptSection } from "./agentSessionTypes";
import {
  findBuzzToolName,
  isGenericToolTitle,
  normalizeToolName,
} from "./agentSessionToolCatalog";
import { asRecord, asString, titleCase } from "./agentSessionUtils";

export function extractPromptBlocks(
  payload: Record<string, unknown>,
): string[] {
  const params = asRecord(payload.params);
  const prompt = params.prompt;
  if (!Array.isArray(prompt)) return [];
  return prompt.map(extractBlockText).filter(Boolean);
}

export function extractPromptText(payload: Record<string, unknown>): string {
  return extractPromptBlocks(payload).join("\n");
}

const SEMANTIC_PROMPT_SECTION_START =
  /^\s*<(?:workspace|base|system|team-instructions|core-memory|huddle-instructions|channel-canvas|context|thread-context|conversation-context|buzz-event|buzz-events|what-you-were-working-on|new-message-arrived-while-you-were-working|previous-request-interrupted-before-completion|new-request-supersedes-previous)(?:\s[^>]*)?>/;

/**
 * Parse ACP prompt blocks without losing the connector-facing slash-command
 * boundary. The harness emits that command as block zero and semantic prompt
 * sections in subsequent blocks; arbitrary leading text remains on the normal
 * parsing path.
 */
export function parsePromptBlocks(
  blocks: readonly string[],
): ReturnType<typeof parsePromptText> {
  const [firstBlock, ...remainingBlocks] = blocks;
  const hasSlashCommandPreamble =
    /^\/[A-Za-z0-9]/.test(firstBlock?.trimStart() ?? "") &&
    remainingBlocks.length > 0 &&
    SEMANTIC_PROMPT_SECTION_START.test(remainingBlocks[0]);

  if (!hasSlashCommandPreamble) {
    return parsePromptText(blocks.join("\n"));
  }

  const parsed = parsePromptText(remainingBlocks.join("\n"));
  return {
    ...parsed,
    sections: [{ title: "Prompt", body: firstBlock }, ...parsed.sections],
  };
}

export function parsePromptText(text: string): {
  sections: PromptSection[];
  userText: string;
  userTitle: string;
  userPubkey: string | null;
  userEventId: string | null;
} {
  const semanticPrefix = splitSemanticStandingPrefix(text);
  const semanticTurn = splitSemanticTurnSections(semanticPrefix.remainder);
  const sections = [
    ...semanticPrefix.sections,
    ...semanticTurn.sections,
    ...parsePromptSections(semanticTurn.remainder),
  ].filter((s) => s.body.trim().length > 0);
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
 * Split `session/new`'s paired standing-context tags into transcript sections.
 * The bracket parser is retained below for observer history captured before
 * the framing experiment.
 *
 * Archived harness versions composed the value in order:
 *   `[Base]\n{base}\n\n[Agent Instructions]\n{persona}\n\n[Team Instructions]\n{team}\n\n[Agent Memory — core]\n{core}\n\n[Channel Canvas]\n{canvas}`
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
 *    position: after Agent Instructions, before Core Memory.
 *
 * 4. **Base/Agent Instructions**: remainder after the three top-level section
 *    extractions. Split on the first `\n[Agent Instructions]\n` boundary.
 *    Archived frames using the former `[System]` header remain supported and
 *    retain their historical observer label.
 *
 * 5. **Legacy Team Instructions** (backward compat): if the agent-instructions body
 *    contains the exact canonical delimiter `\n\n---\n# Team Instructions\n`
 *    (produced by the now-removed `compose_prompt()` in buzz-persona), the body
 *    is split at the **last** occurrence of that boundary. The text before
 *    becomes the agent-instructions body; the text after becomes a `Team Instructions`
 *    section inserted immediately after it. Non-canonical lookalikes
 *    (bare `---` without the heading, a `# Team Instructions` on a different
 *    line, or only a single preceding newline) are kept literal inside `System`.
 */
export function parseSystemPromptSections(
  systemPrompt: string,
): PromptSection[] {
  const semantic = parseSemanticStandingSections(systemPrompt);
  if (semantic) return semantic;

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
  // after [Agent Instructions] and before core/canvas. Same two cases as canvas/core:
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

  // ── 4. Parse Base/Workspace/Agent Instructions from the remaining prefix ─
  // The canonical team-instructions delimiter produced by compose_prompt() in
  // buzz-persona/src/resolve.rs:
  //   format!("{persona_prompt}\n\n---\n# Team Instructions\n{instructions}")
  const TEAM_DELIMITER = "\n\n---\n# Team Instructions\n";

  // splitInstructionsBody: split a raw agent-instructions body string at the last occurrence
  // of the canonical team delimiter, returning { instructionsBody, teamBody | null }.
  // Using lastIndexOf mirrors the canvas/core last-occurrence guard: a persona
  // author can embed an exact delimiter-like passage inside the persona body;
  // only the final occurrence is the producer boundary appended by compose_prompt().
  function splitInstructionsBody(raw: string): {
    instructionsBody: string;
    teamBody: string | null;
  } {
    const at = raw.lastIndexOf(TEAM_DELIMITER);
    if (at === -1) return { instructionsBody: raw.trim(), teamBody: null };
    return {
      instructionsBody: raw.slice(0, at).trim(),
      teamBody: raw.slice(at + TEAM_DELIMITER.length).trim() || null,
    };
  }

  const instructionFrames = [
    { header: "[Agent Instructions]", title: "Agent Instructions" },
    { header: "[System]", title: "System" },
  ] as const;

  function appendBaseAndWorkspace(raw: string): void {
    const BASE_HEADER = "[Base]";
    const WORKSPACE_HEADER = "[Workspace]";
    const workspaceMarker = `\n\n${WORKSPACE_HEADER}\n`;
    const baseMarker = `\n\n${BASE_HEADER}\n`;

    // Current framing keeps the static base first, followed by the dynamic cwd.
    if (raw.startsWith(`${BASE_HEADER}\n`)) {
      const workspaceAt = raw.lastIndexOf(workspaceMarker);
      if (workspaceAt !== -1) {
        const baseBody = raw
          .slice(`${BASE_HEADER}\n`.length, workspaceAt)
          .trim();
        const workspaceBody = raw
          .slice(workspaceAt + workspaceMarker.length)
          .trim();
        if (baseBody) sections.push({ title: "Base", body: baseBody });
        if (workspaceBody)
          sections.push({ title: "Workspace", body: workspaceBody });
        return;
      }
    }

    // Preserve readable transcripts for sessions captured with the former
    // Workspace-before-Base framing.
    if (raw.startsWith(`${WORKSPACE_HEADER}\n`)) {
      const baseAt = raw.lastIndexOf(baseMarker);
      if (baseAt !== -1) {
        const workspaceBody = raw
          .slice(`${WORKSPACE_HEADER}\n`.length, baseAt)
          .trim();
        const baseBody = raw.slice(baseAt + baseMarker.length).trim();
        if (workspaceBody)
          sections.push({ title: "Workspace", body: workspaceBody });
        if (baseBody) sections.push({ title: "Base", body: baseBody });
        return;
      }
    }

    const baseBody = raw.replace(/^\[Base]\n/, "").trim();
    if (baseBody) sections.push({ title: "Base", body: baseBody });
  }

  const baseAndInstructions = remainder;
  if (baseAndInstructions) {
    const leadingFrame = instructionFrames.find(({ header }) =>
      baseAndInstructions.startsWith(`${header}\n`),
    );
    if (leadingFrame) {
      const raw = baseAndInstructions.slice(`${leadingFrame.header}\n`.length);
      const { instructionsBody, teamBody } = splitInstructionsBody(raw);
      if (instructionsBody)
        sections.push({ title: leadingFrame.title, body: instructionsBody });
      if (teamBody)
        sections.push({ title: "Team Instructions", body: teamBody });
    } else {
      const boundary = instructionFrames
        .map((frame) => ({
          ...frame,
          marker: `\n${frame.header}\n`,
          at: baseAndInstructions.indexOf(`\n${frame.header}\n`),
        }))
        .filter(({ at }) => at !== -1)
        .sort((a, b) => a.at - b.at)[0];
      const head = boundary
        ? baseAndInstructions.slice(0, boundary.at)
        : baseAndInstructions;
      appendBaseAndWorkspace(head);

      if (boundary) {
        const raw = baseAndInstructions.slice(
          boundary.at + boundary.marker.length,
        );
        const { instructionsBody, teamBody } = splitInstructionsBody(raw);
        if (instructionsBody)
          sections.push({ title: boundary.title, body: instructionsBody });
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

/**
 * Split current paired-tag standing context while retaining the bracket parser
 * below for observer history captured before the framing experiment.
 */
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
  // Archived bracket-framed personas may contain literal balanced tag examples.
  // Only classify a capture as semantic when its framing starts at the input boundary.
  if (!new RegExp(`^\\s*<(${tags})>`).test(systemPrompt)) return null;

  const parsed = splitSemanticStandingPrefix(systemPrompt);
  // Current producers emit only paired sections separated by whitespace. Any
  // other text makes the boundary ambiguous, so show the complete capture.
  if (parsed.sections.length > 0 && parsed.remainder.trim().length === 0) {
    return parsed.sections;
  }

  return [{ title: "Prompt", body: systemPrompt }];
}

function splitSemanticStandingPrefix(text: string): {
  sections: PromptSection[];
  remainder: string;
} {
  const sections: PromptSection[] = [];
  let remainder = text;
  const tags = [
    "workspace",
    "base",
    "system",
    "team-instructions",
    "core-memory",
    "huddle-instructions",
    "channel-canvas",
  ].join("|");
  const titles: Record<string, string> = {
    workspace: "Workspace",
    base: "Base",
    system: "System",
    "team-instructions": "Team Instructions",
    "core-memory": "Core Memory",
    "huddle-instructions": "Huddle Instructions",
    "channel-canvas": "Channel Canvas",
  };
  if (hasAmbiguousSemanticBoundary(text, Object.keys(titles))) {
    return { sections, remainder: text };
  }
  const leadingSection = new RegExp(`^\\s*<(${tags})>([\\s\\S]*?)<\\/\\1>\\s*`);

  for (;;) {
    const match = remainder.match(leadingSection);
    if (!match) break;
    sections.push({
      title: titles[match[1]],
      body: stripSemanticBoundaryNewlines(match[2]),
    });
    remainder = remainder.slice(match[0].length);
  }
  return { sections, remainder };
}

function hasAmbiguousSemanticBoundary(value: string, tags: string[]): boolean {
  return tags.some((tag) => {
    const openingCount = Array.from(
      value.matchAll(new RegExp(`<${tag}(?:\\s[^>]*)?>`, "g")),
    ).length;
    const closingCount = value.split(`</${tag}>`).length - 1;
    return openingCount !== closingCount || openingCount > 1;
  });
}

function splitSemanticTurnSections(text: string): {
  sections: PromptSection[];
  remainder: string;
} {
  const sections: PromptSection[] = [];
  let remainder = text;
  const tags = [
    "context",
    "thread-context",
    "conversation-context",
    "buzz-event",
    "buzz-events",
    "what-you-were-working-on",
    "new-message-arrived-while-you-were-working",
    "previous-request-interrupted-before-completion",
    "new-request-supersedes-previous",
  ];
  if (hasAmbiguousSemanticBoundary(text, tags)) {
    return { sections, remainder: text };
  }
  const leadingSection = new RegExp(
    `^\\s*<(${tags.join("|")})([^>]*)>([\\s\\S]*?)<\\/\\1>\\s*`,
  );

  for (;;) {
    const match = remainder.match(leadingSection);
    if (!match) break;
    sections.push({
      title: semanticTurnTitle(match[1], parseSemanticAttributes(match[2])),
      body: stripSemanticBoundaryNewlines(match[3]),
    });
    remainder = remainder.slice(match[0].length);
  }
  return { sections, remainder };
}

function parseSemanticAttributes(raw: string): Record<string, string> {
  return Object.fromEntries(
    Array.from(raw.matchAll(/([a-z-]+)="([^"]*)"/g), ([, name, value]) => [
      name,
      decodeSemanticAttribute(value),
    ]),
  );
}

function decodeSemanticAttribute(value: string): string {
  return value
    .replaceAll("&quot;", '"')
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&amp;", "&");
}

function semanticTurnTitle(
  tag: string,
  attributes: Record<string, string>,
): string {
  switch (tag) {
    case "context":
      return "Context";
    case "thread-context":
    case "conversation-context": {
      const label =
        tag === "thread-context" ? "Thread Context" : "Conversation Context";
      const truncated = attributes.truncated === "true" ? ", truncated" : "";
      return `${label} (${attributes.included} of ${attributes.total} messages${truncated})`;
    }
    case "buzz-event":
      return attributes.type ? `Buzz event: ${attributes.type}` : "Buzz event";
    case "buzz-events":
      return `Buzz events — ${attributes.count} events`;
    case "what-you-were-working-on":
      return "What you were working on";
    case "new-message-arrived-while-you-were-working":
      return attributes.count
        ? `New messages — arrived while you were working — ${attributes.count} events`
        : "New message — arrived while you were working";
    case "previous-request-interrupted-before-completion":
      return "Previous request — interrupted before completion";
    case "new-request-supersedes-previous":
      return attributes.count
        ? `New request — supersedes previous — ${attributes.count} events`
        : "New request — supersedes previous";
    default:
      return tag;
  }
}

function stripSemanticBoundaryNewlines(value: string): string {
  const withoutOpeningNewline = value.startsWith("\n") ? value.slice(1) : value;
  return withoutOpeningNewline.endsWith("\n")
    ? withoutOpeningNewline.slice(0, -1)
    : withoutOpeningNewline;
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
