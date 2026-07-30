import React from "react";

import { getSpawners, useSpawners } from "./spawnerPreference";

/**
 * Where an agent runs — deliberately separate from *which runtime* it runs.
 *
 * The two are orthogonal. A runtime (goose, Claude Code, codex, buzz-agent) says
 * which binary drives the agent; a location says whose machine that binary runs
 * on. Folding "server" into the runtime list would conflate them, and would make
 * "Claude Code" silently mean "Claude Code, locally".
 *
 * `{ kind: "local" }` is the historical behaviour: the desktop spawns the
 * harness itself, and the agent dies with the app. `{ kind: "spawner" }` means a
 * `buzz-spawner` runs it, and it keeps working when the app is closed.
 *
 * The *runtime* of a server agent is not represented here on purpose: the host
 * decides what executes there, and the spawner merely advertises it for display
 * (see `SpawnerAnnouncement.runtime`).
 */
export type AgentLocation =
  | { kind: "local" }
  | { kind: "spawner"; spawnerPubkey: string };

/** The location used for new agents when nothing more specific applies. */
const STORAGE_KEY = "buzz:default-agent-location";

export const LOCAL: AgentLocation = { kind: "local" };

const listeners = new Set<() => void>();

let defaultLocation: AgentLocation = readStored();

function isPubkeyHex(value: string): boolean {
  return value.length === 64 && /^[0-9a-f]+$/i.test(value);
}

function readStored(): AgentLocation {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return LOCAL;
    // Stored as the spawner pubkey, or "local". A bare pubkey keeps the format
    // trivially inspectable in devtools.
    if (raw === "local") return LOCAL;
    if (isPubkeyHex(raw)) {
      return { kind: "spawner", spawnerPubkey: raw.toLowerCase() };
    }
    return LOCAL;
  } catch {
    return LOCAL;
  }
}

function notify(): void {
  for (const listener of listeners) listener();
}

/**
 * Set the default location for new agents.
 *
 * Returns false for a malformed spawner pubkey so the caller can surface a
 * validation message instead of silently falling back to local.
 */
export function setDefaultAgentLocation(location: AgentLocation): boolean {
  if (location.kind === "spawner" && !isPubkeyHex(location.spawnerPubkey)) {
    return false;
  }
  defaultLocation =
    location.kind === "local"
      ? LOCAL
      : {
          kind: "spawner",
          spawnerPubkey: location.spawnerPubkey.toLowerCase(),
        };
  try {
    window.localStorage.setItem(
      STORAGE_KEY,
      defaultLocation.kind === "local"
        ? "local"
        : defaultLocation.spawnerPubkey,
    );
  } catch {
    // Keep the in-memory value; the choice still applies this session.
  }
  notify();
  return true;
}

/**
 * The default location, falling back to local when its spawner is gone.
 *
 * A device can be disconnected from the spawner that was the default. Returning
 * a location that points at a spawner this device no longer manages would make
 * every new agent fail to deploy, so the fallback is explicit rather than
 * stored — reconnecting restores the preference.
 */
export function resolveDefaultAgentLocation(
  connectedSpawners: readonly string[] = getSpawners(),
): AgentLocation {
  if (
    defaultLocation.kind === "spawner" &&
    !connectedSpawners.includes(defaultLocation.spawnerPubkey)
  ) {
    return LOCAL;
  }
  return defaultLocation;
}

/** The raw stored default, ignoring whether its spawner is still connected. */
export function getStoredDefaultAgentLocation(): AgentLocation {
  return defaultLocation;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): AgentLocation {
  return defaultLocation;
}

function getServerSnapshot(): AgentLocation {
  return LOCAL;
}

/**
 * Reactive default location, already resolved against connected spawners.
 *
 * This is what a create-agent flow should read: personas — including the
 * built-in Fizz/Honey/Bumble — inherit it rather than each carrying their own
 * copy, so changing the default moves every agent that has not been given an
 * explicit location.
 */
export function useDefaultAgentLocation(): AgentLocation {
  const stored = React.useSyncExternalStore(
    subscribe,
    getSnapshot,
    getServerSnapshot,
  );
  const spawners = useSpawners();
  if (stored.kind === "spawner" && !spawners.includes(stored.spawnerPubkey)) {
    return LOCAL;
  }
  return stored;
}

/** Whether two locations refer to the same place. */
export function sameLocation(a: AgentLocation, b: AgentLocation): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "spawner" && b.kind === "spawner") {
    return a.spawnerPubkey === b.spawnerPubkey;
  }
  return true;
}
