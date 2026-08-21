import { randomUUID } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

import { FIBRE_LANES, deriveLane } from "./apply.mjs";

const DATA_FILE = process.env.TRIAGE_DATA_FILE
  ? pathToFileURL(process.env.TRIAGE_DATA_FILE)
  : new URL("./data.json", import.meta.url);

const EMPTY = {
  fibres: {},
  ingested: {},
  feedback: {},
  transcripts: {},
  ignored: {},
};
const INGESTED_CAP = 8_000;

/** Audit trail depth for messages triage decided not to surface. */
const IGNORED_CAP = 500;

/** Per-channel transcript depth. Deep enough for thread history, shallow
 * enough that `data.json` stays a readable PoC artifact. */
const TRANSCRIPT_CAP = 300;

/** Transcript bodies only ever feed a prompt, so long posts are truncated. */
const TRANSCRIPT_CONTENT_CHARS = 500;

function load() {
  try {
    const parsed = JSON.parse(readFileSync(DATA_FILE, "utf8"));
    return {
      ...EMPTY,
      ...parsed,
      fibres: parsed.fibres ?? {},
      ingested: parsed.ingested ?? {},
      feedback: parsed.feedback ?? {},
      transcripts: parsed.transcripts ?? {},
      ignored: parsed.ignored ?? {},
    };
  } catch {
    return structuredClone(EMPTY);
  }
}

let state = load();

function persist() {
  writeFileSync(DATA_FILE, `${JSON.stringify(state, null, 2)}\n`);
}

function bucket(collection, pubkey) {
  state[collection][pubkey] ??= [];
  return state[collection][pubkey];
}

function sortOpen(fibres) {
  return [...fibres].sort((a, b) => {
    if (b.score !== a.score) return b.score - a.score;
    return (b.updatedAt ?? 0) - (a.updatedAt ?? 0);
  });
}

function sortDone(fibres) {
  return [...fibres].sort(
    (a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0),
  );
}

export function listFibres(pubkey) {
  return bucket("fibres", pubkey);
}

export function listOpenFibres(pubkey) {
  return sortOpen(listFibres(pubkey).filter((fibre) => fibre.status === "open"));
}

export function listDoneFibres(pubkey) {
  return sortDone(listFibres(pubkey).filter((fibre) => fibre.status === "done"));
}

export function clearedCount(pubkey) {
  return listFibres(pubkey).filter((fibre) => fibre.status !== "open").length;
}

export function putFibres(pubkey, fibres) {
  state.fibres[pubkey] = fibres;
  persist();
  return fibres;
}

export function getFibre(pubkey, id) {
  return listFibres(pubkey).find((fibre) => fibre.id === id) ?? null;
}

export function patchFibre(pubkey, id, patch) {
  const fibre = getFibre(pubkey, id);
  if (!fibre) return null;
  Object.assign(fibre, patch, {
    updatedAt: Math.floor(Date.now() / 1000),
  });
  persist();
  return fibre;
}

export function restoreFibres(pubkey) {
  const now = Math.floor(Date.now() / 1000);
  for (const fibre of listFibres(pubkey)) {
    if (fibre.status === "open") continue;
    fibre.status = "open";
    fibre.updatedAt = now;
  }
  persist();
  return listOpenFibres(pubkey);
}

export function ingestedIds(pubkey) {
  return new Set(bucket("ingested", pubkey));
}

export function markIngested(pubkey, eventIds) {
  const existing = bucket("ingested", pubkey);
  const seen = new Set(existing);
  for (const eventId of eventIds) {
    if (!eventId || seen.has(eventId)) continue;
    existing.push(eventId);
    seen.add(eventId);
  }
  state.ingested[pubkey] = existing.slice(-INGESTED_CAP);
  persist();
}

function transcriptRecord(message) {
  return {
    eventId: message.eventId,
    threadRootId: message.threadRootId ?? null,
    authorPubkey: message.authorPubkey ?? null,
    authorLabel: message.authorLabel ?? null,
    content: (message.content ?? "").slice(0, TRANSCRIPT_CONTENT_CHARS),
    createdAt: message.createdAt ?? 0,
    isMention: Boolean(message.isMention),
    isSelf: Boolean(message.isSelf),
  };
}

// Ties on createdAt are broken by eventId so the ordering is stable across
// runs — `channelWindow` slices by position, which needs a total order.
function compareTranscript(left, right) {
  if (left.createdAt !== right.createdAt) {
    return left.createdAt - right.createdAt;
  }
  return left.eventId < right.eventId ? -1 : left.eventId > right.eventId ? 1 : 0;
}

/**
 * Records every message the service sees, including ones classification
 * ignores and ones the viewer wrote. Context and signals are derived from
 * this, so gaps here become blind spots in triage.
 */
export function recordTranscript(pubkey, messages) {
  state.transcripts[pubkey] ??= {};
  const byChannel = state.transcripts[pubkey];
  const touched = new Set();

  for (const message of messages ?? []) {
    const channelId = message?.channelId;
    if (!channelId || !message.eventId) continue;
    byChannel[channelId] ??= [];
    const rows = byChannel[channelId];
    if (rows.some((row) => row.eventId === message.eventId)) continue;
    rows.push(transcriptRecord(message));
    touched.add(channelId);
  }

  for (const channelId of touched) {
    byChannel[channelId] = byChannel[channelId]
      .sort(compareTranscript)
      .slice(-TRANSCRIPT_CAP);
  }

  if (touched.size > 0) persist();
}

export function channelTranscript(pubkey, channelId) {
  return state.transcripts[pubkey]?.[channelId] ?? [];
}

/** The `limit` messages immediately preceding `eventId` in its channel. */
export function channelWindow(pubkey, channelId, eventId, limit) {
  const rows = channelTranscript(pubkey, channelId);
  const index = rows.findIndex((row) => row.eventId === eventId);
  const end = index === -1 ? rows.length : index;
  return rows.slice(Math.max(0, end - limit), end);
}

/** Every message in a thread, root included, oldest first. */
export function threadMessages(pubkey, channelId, threadRootId) {
  if (!threadRootId) return [];
  return channelTranscript(pubkey, channelId).filter(
    (row) => row.threadRootId === threadRootId || row.eventId === threadRootId,
  );
}

export function recordFeedback(pubkey, entry) {
  const row = {
    ...entry,
    id: randomUUID(),
    createdAt: Math.floor(Date.now() / 1000),
  };
  bucket("feedback", pubkey).unshift(row);
  state.feedback[pubkey] = bucket("feedback", pubkey).slice(0, 200);
  persist();
  return row;
}

export function patchFeedback(pubkey, id, patch) {
  const row = bucket("feedback", pubkey).find((entry) => entry.id === id);
  if (!row) return null;
  Object.assign(row, patch);
  persist();
  return row;
}

export function listFeedback(pubkey) {
  return bucket("feedback", pubkey);
}

/**
 * Audit trail for messages triage decided not to surface. Nothing reads it in
 * the product — it exists so a missed true positive can be found after the
 * fact instead of disappearing.
 */
export function recordIgnored(pubkey, entries) {
  if (!entries || entries.length === 0) return;
  const existing = bucket("ignored", pubkey);
  state.ignored[pubkey] = [...entries, ...existing].slice(0, IGNORED_CAP);
  persist();
}

export function listIgnored(pubkey) {
  return bucket("ignored", pubkey);
}

// Rows written before lanes existed still need one to be routable.
function withLane(fibre) {
  const engagement = fibre.engagement ?? 0;
  return {
    ...fibre,
    engagement,
    lane: fibre.lane ?? deriveLane(fibre.score, engagement),
  };
}

function countByLane(fibres) {
  const counts = Object.fromEntries(FIBRE_LANES.map((lane) => [lane, 0]));
  for (const fibre of fibres) counts[fibre.lane] += 1;
  return counts;
}

export function fibresPayload(pubkey) {
  const open = listOpenFibres(pubkey).map(withLane);
  const done = listDoneFibres(pubkey).map(withLane);
  return {
    fibres: open,
    done,
    openCount: open.length,
    doneCount: done.length,
    clearedCount: clearedCount(pubkey),
    laneCounts: countByLane(open),
  };
}

export function resetStore() {
  state = structuredClone(EMPTY);
  persist();
}
