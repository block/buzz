import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_BATTLE_RHYTHM_EVENT,
  KIND_BATTLE_RHYTHM_REVISION,
  KIND_BATTLE_RHYTHM_SOURCE,
} from "@/shared/constants/kinds";
import {
  parseBattleRhythmEvent,
  parseBattleRhythmRevision,
  parseBattleRhythmRevisionChunk,
  parseBattleRhythmSource,
  type BattleRhythmEvent,
  type BattleRhythmRevision,
  type BattleRhythmRevisionChunk,
  type BattleRhythmSource,
} from "./contracts";

export const MAX_REVISION_CHUNK_BYTES = 240 * 1024;
const encoder = new TextEncoder();
const nowAfter = (prior?: number) =>
  Math.max(Math.floor(Date.now() / 1000), (prior ?? 0) + 1);
async function sha256(value: string): Promise<string> {
  const bytes = await crypto.subtle.digest("SHA-256", encoder.encode(value));
  return Array.from(new Uint8Array(bytes), (b) =>
    b.toString(16).padStart(2, "0"),
  ).join("");
}
const content = (value: unknown) => JSON.stringify(value);
let eventSigner = signRelayEvent;
/** Test seam for the native signer; production always uses the Tauri signer. */
export function setBattleRhythmEventSignerForTests(
  signer: typeof signRelayEvent | undefined,
): void {
  eventSigner = signer ?? signRelayEvent;
}
export async function buildSourceEvent(
  sourceInput: BattleRhythmSource,
  priorCreatedAt?: number,
): Promise<RelayEvent> {
  const source = parseBattleRhythmSource(sourceInput);
  return eventSigner({
    kind: KIND_BATTLE_RHYTHM_SOURCE,
    content: content(source),
    createdAt: nowAfter(priorCreatedAt),
    tags: [
      ["d", source.id],
      ["source", source.id],
      ["revision", source.revisionId],
      ["start", source.coverageStart],
      ["end", source.coverageEnd],
    ],
  });
}
export async function buildCalendarEvent(
  eventInput: BattleRhythmEvent,
  priorCreatedAt?: number,
): Promise<RelayEvent> {
  const event = parseBattleRhythmEvent(eventInput);
  const tags = [
    ["d", event.id],
    ["start", event.start],
    ["end", event.end],
  ];
  if (event.ownership.kind === "source")
    tags.push(
      ["source", event.ownership.sourceId],
      ["revision", event.ownership.revisionId],
    );
  return eventSigner({
    kind: KIND_BATTLE_RHYTHM_EVENT,
    content: content(event),
    createdAt: nowAfter(priorCreatedAt),
    tags,
  });
}
function baseChunk(
  revision: BattleRhythmRevision,
  manifestHash: string,
  changes: BattleRhythmRevision["changes"],
): Omit<BattleRhythmRevisionChunk, "chunkIndex" | "chunkCount"> {
  return {
    schemaVersion: 1,
    revisionId: revision.id,
    sourceId: revision.sourceId,
    manifestHash,
    changes,
  };
}
export async function buildRevisionEvents(
  revisionInput: BattleRhythmRevision,
): Promise<readonly RelayEvent[]> {
  const revision = parseBattleRhythmRevision(revisionInput);
  const manifestHash = await sha256(content(revision));
  const groups: BattleRhythmRevision["changes"][] = [];
  let group: BattleRhythmRevision["changes"] = [];
  for (const item of revision.changes) {
    const candidate = [...group, item];
    const provisional = {
      ...baseChunk(revision, manifestHash, candidate),
      chunkIndex: groups.length,
      chunkCount: 1,
    };
    if (
      encoder.encode(content(provisional)).byteLength > MAX_REVISION_CHUNK_BYTES
    ) {
      if (group.length === 0)
        throw new Error("Battle Rhythm revision change exceeds 240 KiB");
      groups.push(group);
      group = [item];
    } else group = candidate;
  }
  if (group.length || groups.length === 0) groups.push(group);
  const chunks = groups.map((changes, chunkIndex) =>
    parseBattleRhythmRevisionChunk({
      ...baseChunk(revision, manifestHash, changes),
      chunkIndex,
      chunkCount: groups.length,
    }),
  );
  if (
    chunks.some(
      (chunk) =>
        encoder.encode(content(chunk)).byteLength > MAX_REVISION_CHUNK_BYTES,
    )
  )
    throw new Error("Battle Rhythm revision chunks exceed 240 KiB");
  return Promise.all(
    chunks.map((chunk) =>
      eventSigner({
        kind: KIND_BATTLE_RHYTHM_REVISION,
        content: content(chunk),
        tags: [
          ["revision", chunk.revisionId],
          ["source", chunk.sourceId],
          ["chunk", String(chunk.chunkIndex), String(chunk.chunkCount)],
          ["hash", chunk.manifestHash],
        ],
      }),
    ),
  );
}
function tag(event: RelayEvent, name: string): string | undefined {
  return event.tags.find((item) => item[0] === name)?.[1];
}
export function parseRelayCalendarEvent(
  event: RelayEvent,
): BattleRhythmEvent | null {
  if (
    event.kind !== KIND_BATTLE_RHYTHM_EVENT ||
    !tag(event, "d") ||
    !tag(event, "start") ||
    !tag(event, "end")
  )
    return null;
  try {
    const parsed = parseBattleRhythmEvent(JSON.parse(event.content));
    return parsed.id === tag(event, "d") &&
      parsed.start === tag(event, "start") &&
      parsed.end === tag(event, "end")
      ? parsed
      : null;
  } catch {
    return null;
  }
}
export function parseRelaySourceEvent(
  event: RelayEvent,
): BattleRhythmSource | null {
  if (
    event.kind !== KIND_BATTLE_RHYTHM_SOURCE ||
    !tag(event, "d") ||
    !tag(event, "revision")
  )
    return null;
  try {
    const parsed = parseBattleRhythmSource(JSON.parse(event.content));
    return parsed.id === tag(event, "d") &&
      parsed.id === tag(event, "source") &&
      parsed.revisionId === tag(event, "revision")
      ? parsed
      : null;
  } catch {
    return null;
  }
}
export function parseRelayRevisionChunk(
  event: RelayEvent,
): BattleRhythmRevisionChunk | null {
  if (event.kind !== KIND_BATTLE_RHYTHM_REVISION) return null;
  try {
    const parsed = parseBattleRhythmRevisionChunk(JSON.parse(event.content));
    const chunk = event.tags.find((item) => item[0] === "chunk");
    return tag(event, "revision") === parsed.revisionId &&
      tag(event, "source") === parsed.sourceId &&
      tag(event, "hash") === parsed.manifestHash &&
      chunk?.[1] === String(parsed.chunkIndex) &&
      chunk?.[2] === String(parsed.chunkCount)
      ? parsed
      : null;
  } catch {
    return null;
  }
}
