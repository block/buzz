import { randomUUID } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const DATA_FILE = process.env.TRIAGE_DATA_FILE
  ? pathToFileURL(process.env.TRIAGE_DATA_FILE)
  : new URL("./data.json", import.meta.url);

const EMPTY = { fibres: {}, ingested: {}, feedback: {} };
const INGESTED_CAP = 8_000;

function load() {
  try {
    const parsed = JSON.parse(readFileSync(DATA_FILE, "utf8"));
    return {
      ...EMPTY,
      ...parsed,
      fibres: parsed.fibres ?? {},
      ingested: parsed.ingested ?? {},
      feedback: parsed.feedback ?? {},
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

export function listFibres(pubkey) {
  return bucket("fibres", pubkey);
}

export function listOpenFibres(pubkey) {
  return sortOpen(listFibres(pubkey).filter((fibre) => fibre.status === "open"));
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

export function listFeedback(pubkey) {
  return bucket("feedback", pubkey);
}

export function fibresPayload(pubkey) {
  const open = listOpenFibres(pubkey);
  return {
    fibres: open,
    openCount: open.length,
    clearedCount: clearedCount(pubkey),
  };
}
