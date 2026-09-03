import type { DirectAgentCreationResult } from "./directAgentCreationResult";

const STORAGE_KEY = "buzz.agents.directCreationJournal";
const MAX_ENTRIES = 100;

type ProcessingEntry = {
  ownerPubkey: string;
  requestId: string;
  status: "processing";
  displayName: string;
  recordedAt: number;
};
type JournalEntry =
  | (DirectAgentCreationResult & {
      ownerPubkey: string;
      recordedAt: number;
    })
  | ProcessingEntry;

function readJournal(): JournalEntry[] {
  try {
    const parsed: unknown = JSON.parse(
      globalThis.localStorage?.getItem(STORAGE_KEY) ?? "[]",
    );
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (entry): entry is JournalEntry =>
        typeof entry === "object" &&
        entry !== null &&
        typeof (entry as JournalEntry).ownerPubkey === "string" &&
        typeof (entry as JournalEntry).requestId === "string" &&
        typeof (entry as JournalEntry).recordedAt === "number",
    );
  } catch {
    return [];
  }
}

export function getDirectAgentCreationResult(
  ownerPubkey: string,
  requestId: string,
): DirectAgentCreationResult | null {
  const entry = readJournal().find(
    (candidate) =>
      candidate.ownerPubkey === ownerPubkey &&
      candidate.requestId === requestId,
  );
  if (!entry) return null;
  if (entry.status !== "processing") {
    const {
      ownerPubkey: _ownerPubkey,
      recordedAt: _recordedAt,
      ...result
    } = entry;
    return result;
  }
  return {
    requestId: entry.requestId,
    status: "failed",
    displayName: entry.displayName,
    message:
      "A previous attempt did not record a terminal result. Inspect the agent roster before retrying with a new request ID.",
  };
}

export function beginDirectAgentCreation(
  ownerPubkey: string,
  requestId: string,
  displayName: string,
): void {
  writeEntry({
    ownerPubkey,
    requestId,
    status: "processing",
    displayName,
    recordedAt: Date.now(),
  });
}

export function recordDirectAgentCreationResult(
  ownerPubkey: string,
  result: DirectAgentCreationResult,
): void {
  writeEntry({ ownerPubkey, ...result, recordedAt: Date.now() });
}

function writeEntry(entry: JournalEntry): void {
  const next = [
    entry,
    ...readJournal().filter(
      (candidate) =>
        candidate.ownerPubkey !== entry.ownerPubkey ||
        candidate.requestId !== entry.requestId,
    ),
  ].slice(0, MAX_ENTRIES);
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    throw new Error(
      "Could not persist the direct-create result before acknowledgement.",
    );
  }
}
