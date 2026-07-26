import { validateEvent, type Event, type Filter } from "nostr-tools";

const HEX_32_BYTES = /^[0-9a-f]{64}$/;
const HEX_64_BYTES = /^[0-9a-f]{128}$/;
const FILTER_FIELDS = new Set([
  "ids",
  "authors",
  "kinds",
  "since",
  "until",
  "limit",
]);

export class ProtocolInputError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ProtocolInputError";
  }
}

export function eventFromUnknown(value: unknown): Event {
  if (!isRecord(value) || !validateEvent(value)) {
    throw new ProtocolInputError("invalid event envelope");
  }
  const record = value as Record<string, unknown>;
  if (
    typeof record.id !== "string" ||
    !HEX_32_BYTES.test(record.id) ||
    typeof record.pubkey !== "string" ||
    !HEX_32_BYTES.test(record.pubkey) ||
    typeof record.sig !== "string" ||
    !HEX_64_BYTES.test(record.sig)
  ) {
    throw new ProtocolInputError(
      "invalid event identity or signature encoding",
    );
  }
  if (
    !Number.isSafeInteger(record.kind) ||
    (record.kind as number) < 0 ||
    (record.kind as number) > 65_535
  ) {
    throw new ProtocolInputError(
      "event kind must be an unsigned 16-bit integer",
    );
  }
  return value as Event;
}

export function filtersFromUnknown(value: unknown): Filter[] {
  if (!Array.isArray(value)) {
    throw new ProtocolInputError("filters must be a JSON array");
  }
  return value.map((candidate) => filterFromUnknown(candidate));
}

function filterFromUnknown(value: unknown): Filter {
  if (!isRecord(value)) {
    throw new ProtocolInputError("each filter must be a JSON object");
  }
  if ("search" in value) {
    throw new ProtocolInputError("NIP-50 search is unsupported");
  }

  for (const [field, fieldValue] of Object.entries(value)) {
    if (field.startsWith("#")) {
      requireStringArray(field, fieldValue);
      continue;
    }
    if (!FILTER_FIELDS.has(field)) {
      throw new ProtocolInputError(`unsupported filter field: ${field}`);
    }
    if (field === "ids" || field === "authors") {
      requireStringArray(field, fieldValue);
    } else if (field === "kinds") {
      requireIntegerArray(field, fieldValue);
    } else if (
      !Number.isSafeInteger(fieldValue) ||
      (field === "limit" && (fieldValue as number) < 0)
    ) {
      throw new ProtocolInputError(`${field} must be an integer`);
    }
  }
  return value as Filter;
}

function requireStringArray(field: string, value: unknown): void {
  if (
    !Array.isArray(value) ||
    !value.every((item) => typeof item === "string")
  ) {
    throw new ProtocolInputError(`${field} must be an array of strings`);
  }
}

function requireIntegerArray(field: string, value: unknown): void {
  if (
    !Array.isArray(value) ||
    !value.every((item) => Number.isSafeInteger(item))
  ) {
    throw new ProtocolInputError(`${field} must be an array of integers`);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
