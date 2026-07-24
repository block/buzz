import { createModelRoute, parseAdviserContribution } from "./contracts";
import type {
  AdviserContribution,
  EgressDecision,
  ModelRoute,
} from "./contracts";

/** The closed adviser identity set established by the Phase 1 Command Console. */
export const APPROVED_ADVISERS = Object.freeze([
  "Chief of Staff",
  "Operations",
  "Navigation",
  "Daily Routine",
  "Reporting",
  "Plans",
] as const);

export type AdviserIdentity = (typeof APPROVED_ADVISERS)[number];

const MAX_NATIVE_MESSAGE_BYTES = 256 * 1024;
const MAX_VALUE_STRING_LENGTH = 16_384;
const MAX_ARRAY_ITEMS = 64;
const MAX_VALUE_DEPTH = 16;
const MAX_VALUE_NODES = 4_096;
const DANGEROUS_KEYS = new Set(["__proto__", "constructor", "prototype"]);
const LOOPBACK_ENDPOINT = /^http:\/\/(?:127\.0\.0\.1|\[::1\]):([1-9]\d{0,4})$/;
const UTF8_ENCODER = new TextEncoder();

type JsonBudgetFrame = {
  readonly depth: number;
  readonly value: unknown;
};

function isApprovedAdviser(value: unknown): value is AdviserIdentity {
  return (
    typeof value === "string" &&
    (APPROVED_ADVISERS as readonly string[]).includes(value)
  );
}

function hasControlCharacters(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code <= 31 || code === 127) return true;
  }
  return false;
}

function isBoundedSafeJson(root: unknown): boolean {
  const stack: JsonBudgetFrame[] = [{ depth: 0, value: root }];
  let nodes = 0;
  while (stack.length > 0) {
    const frame = stack.pop();
    if (!frame || frame.depth > MAX_VALUE_DEPTH) return false;
    nodes += 1;
    if (nodes > MAX_VALUE_NODES) return false;

    if (typeof frame.value === "string") {
      if (
        frame.value.length > MAX_VALUE_STRING_LENGTH ||
        hasControlCharacters(frame.value)
      ) {
        return false;
      }
      continue;
    }
    if (
      frame.value === null ||
      typeof frame.value === "boolean" ||
      (typeof frame.value === "number" && Number.isFinite(frame.value))
    ) {
      continue;
    }
    if (Array.isArray(frame.value)) {
      if (frame.value.length > MAX_ARRAY_ITEMS) return false;
      for (const value of frame.value) {
        stack.push({ depth: frame.depth + 1, value });
      }
      continue;
    }
    if (typeof frame.value !== "object") return false;
    const record = frame.value as Record<string, unknown>;
    const keys = Object.keys(record);
    if (keys.length > MAX_ARRAY_ITEMS) return false;
    for (const key of keys) {
      if (
        DANGEROUS_KEYS.has(key) ||
        key.length > MAX_VALUE_STRING_LENGTH ||
        hasControlCharacters(key)
      ) {
        return false;
      }
      stack.push({ depth: frame.depth + 1, value: record[key] });
    }
  }
  return true;
}

/**
 * Parses one native terminal message into an exact adviser contribution.
 *
 * Findings are supported by the contribution-level evidence array defined in
 * the Phase 1 contract. This boundary does not infer per-finding citations or
 * consolidate dissent and limitations.
 */
export function parseNativeAdviserContribution(
  terminalMessages: readonly unknown[],
  expectedAdviser: AdviserIdentity,
): AdviserContribution | null {
  if (
    !isApprovedAdviser(expectedAdviser) ||
    terminalMessages.length !== 1 ||
    typeof terminalMessages[0] !== "string" ||
    terminalMessages[0].length > MAX_NATIVE_MESSAGE_BYTES ||
    UTF8_ENCODER.encode(terminalMessages[0]).byteLength >
      MAX_NATIVE_MESSAGE_BYTES
  ) {
    return null;
  }

  let candidate: unknown;
  try {
    candidate = JSON.parse(terminalMessages[0]);
  } catch {
    return null;
  }
  if (!isBoundedSafeJson(candidate)) return null;

  const contribution = parseAdviserContribution(candidate);
  if (
    contribution === null ||
    contribution.classification !== "OFFICIAL" ||
    contribution.adviser !== expectedAdviser ||
    !isApprovedAdviser(contribution.adviser) ||
    (contribution.findings.length > 0 && contribution.evidence.length === 0) ||
    contribution.evidence.some(
      (reference) => reference.classification !== "OFFICIAL",
    ) ||
    contribution.proposedActions.some(
      (action) =>
        action.classification !== "OFFICIAL" ||
        action.approvalState !== "pending",
    )
  ) {
    return null;
  }
  return contribution;
}

export type LmStudioNativeModelRouteInput = {
  readonly endpoint: string;
  readonly model: string;
  readonly permittedTools: readonly string[];
  readonly rustEgressDecision: EgressDecision;
};

function isLiteralLoopbackEndpoint(endpoint: string): boolean {
  const match = LOOPBACK_ENDPOINT.exec(endpoint);
  if (!match) return false;
  const port = Number(match[1]);
  return Number.isSafeInteger(port) && port >= 1 && port <= 65_535;
}

function isBoundedUniqueTextList(values: readonly string[]): boolean {
  if (values.length > MAX_ARRAY_ITEMS) return false;
  const unique = new Set<string>();
  for (const value of values) {
    if (
      value.trim().length === 0 ||
      value.length > MAX_VALUE_STRING_LENGTH ||
      hasControlCharacters(value) ||
      unique.has(value)
    ) {
      return false;
    }
    unique.add(value);
  }
  return true;
}

/**
 * Builds the display projection of a Rust-authorised native LM Studio route.
 *
 * Rust remains the enforcement authority. This TypeScript value is auditable
 * display data and cannot authorise network egress.
 */
export function buildLmStudioNativeModelRoute(
  input: LmStudioNativeModelRouteInput,
): ModelRoute {
  if (
    !isLiteralLoopbackEndpoint(input.endpoint) ||
    input.model.trim().length === 0 ||
    input.model.length > MAX_VALUE_STRING_LENGTH ||
    hasControlCharacters(input.model) ||
    !isBoundedUniqueTextList(input.permittedTools) ||
    input.rustEgressDecision.rationale.trim().length === 0 ||
    input.rustEgressDecision.rationale.length > MAX_VALUE_STRING_LENGTH ||
    hasControlCharacters(input.rustEgressDecision.rationale)
  ) {
    throw new TypeError("Invalid LM Studio native model route.");
  }
  return createModelRoute({
    classification: "OFFICIAL",
    selectedEndpoint: input.endpoint,
    selectedProvider: "lmstudio-native",
    selectedModel: input.model,
    permittedTools: input.permittedTools,
    fallbackChain: [],
    egressDecision: {
      allowed: input.rustEgressDecision.allowed,
      rationale: `Rust enforcement authority: ${input.rustEgressDecision.rationale}`,
    },
  });
}
