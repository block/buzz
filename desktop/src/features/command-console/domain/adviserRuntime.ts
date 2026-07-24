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
const LOOPBACK_ENDPOINT =
  /^http:\/\/(?:127\.0\.0\.1|\[::1\]):([1-9]\d{0,4})(\/)?$/;
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

class BoundedJsonMemberScanner {
  readonly #source: string;
  #index = 0;
  #nodes = 0;

  constructor(source: string) {
    this.#source = source;
  }

  scan(): boolean {
    if (!this.#parseValue(0)) return false;
    this.#skipWhitespace();
    return this.#index === this.#source.length;
  }

  #parseValue(depth: number): boolean {
    if (depth > MAX_VALUE_DEPTH || this.#nodes >= MAX_VALUE_NODES) return false;
    this.#nodes += 1;
    this.#skipWhitespace();
    const token = this.#source[this.#index];
    if (token === "{") return this.#parseObject(depth);
    if (token === "[") return this.#parseArray(depth);
    if (token === '"') return this.#parseString() !== null;
    return this.#parseScalar();
  }

  #parseObject(depth: number): boolean {
    this.#index += 1;
    this.#skipWhitespace();
    if (this.#source[this.#index] === "}") {
      this.#index += 1;
      return true;
    }

    const keys = new Set<string>();
    while (keys.size < MAX_ARRAY_ITEMS) {
      const key = this.#parseString();
      if (key === null || keys.has(key)) return false;
      keys.add(key);
      this.#skipWhitespace();
      if (this.#source[this.#index] !== ":") return false;
      this.#index += 1;
      if (!this.#parseValue(depth + 1)) return false;
      this.#skipWhitespace();
      const separator = this.#source[this.#index];
      this.#index += 1;
      if (separator === "}") return true;
      if (separator !== ",") return false;
      this.#skipWhitespace();
    }
    return false;
  }

  #parseArray(depth: number): boolean {
    this.#index += 1;
    this.#skipWhitespace();
    if (this.#source[this.#index] === "]") {
      this.#index += 1;
      return true;
    }

    let itemCount = 0;
    while (itemCount < MAX_ARRAY_ITEMS) {
      if (!this.#parseValue(depth + 1)) return false;
      itemCount += 1;
      this.#skipWhitespace();
      const separator = this.#source[this.#index];
      this.#index += 1;
      if (separator === "]") return true;
      if (separator !== ",") return false;
      this.#skipWhitespace();
    }
    return false;
  }

  #parseString(): string | null {
    if (this.#source[this.#index] !== '"') return null;
    const start = this.#index;
    this.#index += 1;
    while (this.#index < this.#source.length) {
      const token = this.#source[this.#index];
      if (token === "\\") {
        this.#index += 2;
        continue;
      }
      this.#index += 1;
      if (token !== '"') continue;
      try {
        const parsed = JSON.parse(this.#source.slice(start, this.#index));
        return typeof parsed === "string" ? parsed : null;
      } catch {
        return null;
      }
    }
    return null;
  }

  #parseScalar(): boolean {
    const start = this.#index;
    while (this.#index < this.#source.length) {
      const token = this.#source[this.#index];
      if (
        token === "," ||
        token === "]" ||
        token === "}" ||
        this.#isWhitespace(token)
      ) {
        break;
      }
      this.#index += 1;
    }
    if (start === this.#index) return false;
    try {
      const parsed = JSON.parse(this.#source.slice(start, this.#index));
      return (
        parsed === null ||
        typeof parsed === "boolean" ||
        (typeof parsed === "number" && Number.isFinite(parsed))
      );
    } catch {
      return false;
    }
  }

  #skipWhitespace(): void {
    while (this.#isWhitespace(this.#source[this.#index])) {
      this.#index += 1;
    }
  }

  #isWhitespace(token: string | undefined): boolean {
    return token === " " || token === "\n" || token === "\r" || token === "\t";
  }
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
    if (!new BoundedJsonMemberScanner(terminalMessages[0]).scan()) return null;
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

function canonicalLiteralLoopbackEndpoint(endpoint: string): string | null {
  const match = LOOPBACK_ENDPOINT.exec(endpoint);
  if (!match) return null;
  const port = Number(match[1]);
  if (!Number.isSafeInteger(port) || port < 1 || port > 65_535) return null;
  return match[2] === "/" ? endpoint.slice(0, -1) : endpoint;
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
  const selectedEndpoint = canonicalLiteralLoopbackEndpoint(input.endpoint);
  if (
    selectedEndpoint === null ||
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
    selectedEndpoint,
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
