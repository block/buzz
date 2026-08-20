/**
 * Parses the `buzz-workstream-card` sentinel that a channel canvas may embed
 * to describe the workstream running in that channel.
 *
 * Wire format (authored by hand or by an orchestrating agent):
 *
 * ```
 * ```buzz-workstream-card
 * {"version":1,"synopsis":"…","orchestrator":{"pubkey":"…","name":"…"},"assignees":[{"pubkey":"…","name":"…"}]}
 * ```
 * ```
 *
 * Only one block per canvas is supported. A missing block, malformed JSON,
 * an unrecognized version, or missing/invalid required fields are all
 * card-local parse failures — the caller degrades just that card, it never
 * throws.
 */

const FENCE_OPEN = "```buzz-workstream-card";
const FENCE_CLOSE = "```";

export type WorkstreamIdentity = {
  pubkey: string;
  name: string;
};

export type WorkstreamCardV1 = {
  version: 1;
  synopsis: string;
  orchestrator: WorkstreamIdentity;
  assignees: WorkstreamIdentity[];
  pullRequests: unknown[];
  waitingOn: unknown[];
};

export type WorkstreamCardParseFailureReason =
  | "not-found"
  | "invalid-json"
  | "duplicate-block"
  | "unknown-version"
  | "invalid-fields";

export type WorkstreamCardParseResult =
  | { ok: true; card: WorkstreamCardV1 }
  | { ok: false; reason: WorkstreamCardParseFailureReason };

function findFencedBlocks(content: string): string[] {
  const lines = content.split(/\r?\n/);
  const blocks: string[] = [];

  for (let index = 0; index < lines.length; index += 1) {
    if (lines[index].trimEnd() !== FENCE_OPEN) continue;

    const body: string[] = [];
    let closeIndex = index + 1;
    while (
      closeIndex < lines.length &&
      lines[closeIndex].trim() !== FENCE_CLOSE
    ) {
      body.push(lines[closeIndex]);
      closeIndex += 1;
    }
    if (closeIndex === lines.length) break;

    blocks.push(body.join("\n").trim());
    index = closeIndex;
  }

  return blocks;
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.trim() !== "";
}

function isOneLineString(value: unknown): value is string {
  return isNonEmptyString(value) && !/[\r\n]/.test(value);
}

function isWorkstreamIdentity(value: unknown): value is WorkstreamIdentity {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }

  const raw = value as Record<string, unknown>;
  return isNonEmptyString(raw.pubkey) && isNonEmptyString(raw.name);
}

function isWorkstreamIdentityArray(
  value: unknown,
): value is WorkstreamIdentity[] {
  return Array.isArray(value) && value.every(isWorkstreamIdentity);
}

/**
 * Parse the single `buzz-workstream-card` block out of a channel canvas.
 * Never throws — every failure mode maps to a `WorkstreamCardParseFailureReason`.
 */
export function parseWorkstreamCard(
  content: string | null | undefined,
): WorkstreamCardParseResult {
  if (!content) {
    return { ok: false, reason: "not-found" };
  }

  const blocks = findFencedBlocks(content);
  if (blocks.length === 0) {
    return { ok: false, reason: "not-found" };
  }
  if (blocks.length > 1) {
    return { ok: false, reason: "duplicate-block" };
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(blocks[0]);
  } catch {
    return { ok: false, reason: "invalid-json" };
  }

  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    return { ok: false, reason: "invalid-fields" };
  }

  const raw = parsed as Record<string, unknown>;

  if (raw.version !== 1) {
    return { ok: false, reason: "unknown-version" };
  }

  const synopsis = raw.synopsis;
  if (!isOneLineString(synopsis)) {
    return { ok: false, reason: "invalid-fields" };
  }

  const orchestrator = raw.orchestrator;
  if (!isWorkstreamIdentity(orchestrator)) {
    return { ok: false, reason: "invalid-fields" };
  }

  const assignees = raw.assignees === undefined ? [] : raw.assignees;
  if (!isWorkstreamIdentityArray(assignees)) {
    return { ok: false, reason: "invalid-fields" };
  }

  const pullRequests = raw.pullRequests === undefined ? [] : raw.pullRequests;
  if (!Array.isArray(pullRequests)) {
    return { ok: false, reason: "invalid-fields" };
  }

  const waitingOn = raw.waitingOn === undefined ? [] : raw.waitingOn;
  if (!Array.isArray(waitingOn)) {
    return { ok: false, reason: "invalid-fields" };
  }

  return {
    ok: true,
    card: {
      version: 1,
      synopsis,
      orchestrator,
      assignees,
      pullRequests,
      waitingOn,
    },
  };
}
