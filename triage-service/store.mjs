import { randomUUID } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const DATA_FILE = new URL("./data.json", import.meta.url);

const EMPTY = { suggestions: {}, todos: {}, feedback: {} };

function load() {
  try {
    return { ...EMPTY, ...JSON.parse(readFileSync(DATA_FILE, "utf8")) };
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

export function putSuggestions(pubkey, suggestions) {
  state.suggestions[pubkey] = suggestions;
  persist();
  return suggestions;
}

export function getSuggestions(pubkey) {
  return state.suggestions[pubkey] ?? [];
}

/**
 * Applies a user decision to the stored verdict so it survives a client
 * reload. Without this the client would have to shadow decisions in local
 * state, which is lost the moment the view unmounts.
 */
export function patchSuggestion(pubkey, eventId, patch) {
  const suggestion = (state.suggestions[pubkey] ?? []).find(
    (candidate) => candidate.eventId === eventId,
  );
  if (!suggestion) return null;
  Object.assign(suggestion, patch);
  persist();
  return suggestion;
}

export function listTodos(pubkey) {
  return bucket("todos", pubkey);
}

export function createTodo(pubkey, input) {
  // Adopting the same message twice is a repeated intent, not a second task.
  const existing = bucket("todos", pubkey).find(
    (candidate) =>
      candidate.eventId === input.eventId && candidate.status === "open",
  );
  if (existing) return existing;

  const todo = {
    id: randomUUID(),
    pubkey,
    eventId: input.eventId,
    channelId: input.channelId ?? null,
    channelName: input.channelName ?? null,
    threadRootId: input.threadRootId ?? null,
    authorLabel: input.authorLabel ?? null,
    preview: input.preview ?? "",
    reason: input.reason ?? "",
    status: "open",
    createdAt: Math.floor(Date.now() / 1000),
  };
  bucket("todos", pubkey).unshift(todo);
  persist();
  return todo;
}

export function updateTodo(pubkey, id, status) {
  const todo = bucket("todos", pubkey).find((candidate) => candidate.id === id);
  if (!todo) return null;
  todo.status = status;
  todo.resolvedAt = Math.floor(Date.now() / 1000);
  persist();
  return todo;
}

export function recordFeedback(pubkey, entry) {
  const row = {
    ...entry,
    id: randomUUID(),
    createdAt: Math.floor(Date.now() / 1000),
  };
  bucket("feedback", pubkey).unshift(row);
  // Keep the learning window bounded so prompts and heuristics stay small.
  state.feedback[pubkey] = bucket("feedback", pubkey).slice(0, 200);
  persist();
  return row;
}

export function listFeedback(pubkey) {
  return bucket("feedback", pubkey);
}
