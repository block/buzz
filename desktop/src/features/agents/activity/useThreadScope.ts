// React wiring for thread-scoped activity: derives each agent's turns from
// its observer frames, resolves turn → thread root through a module-level
// event cache, and exposes ordered thread groups for the chip row.
//
// The event cache is module-level (not per-mount) because thread roots are
// immutable once fetched — a channel revisit or a second surface mounting the
// map should never refetch the same roots. Successful lookups and definitive
// not-found errors are cached; transient failures evict their entry so a later
// mount can retry.

import * as React from "react";

import { getEventById } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { isDefinitiveEventNotFound } from "@/shared/lib/eventLookupError";
import type { ObserverEvent } from "../ui/agentSessionTypes";
import {
  buildThreadGroups,
  resolveTurnThread,
  type ThreadGroups,
  type TurnThreadResolution,
} from "./activityThreads";
import { type ActivityTurn, deriveActivityTurns } from "./activityTurns";

export type EventByIdFetcher = (eventId: string) => Promise<RelayEvent>;

export function createThreadEventCache(fetchEventById: EventByIdFetcher): {
  fetchEventCached: (eventId: string) => Promise<RelayEvent | null>;
  clear: () => void;
} {
  const cache = new Map<string, Promise<RelayEvent | null>>();
  return {
    fetchEventCached(eventId) {
      let promise = cache.get(eventId);
      if (!promise) {
        promise = fetchEventById(eventId).catch((error) => {
          if (isDefinitiveEventNotFound(error)) {
            return null;
          }
          cache.delete(eventId);
          throw error;
        });
        cache.set(eventId, promise);
      }
      return promise;
    },
    clear() {
      cache.clear();
    },
  };
}

const moduleEventCache = createThreadEventCache(getEventById);

function fetchEventCached(eventId: string): Promise<RelayEvent | null> {
  return moduleEventCache.fetchEventCached(eventId);
}

export async function resolveTurnThreadsRetriable(
  turns: readonly ActivityTurn[],
  fetchEvent: (eventId: string) => Promise<RelayEvent | null>,
): Promise<Map<string, TurnThreadResolution>> {
  const entries = await Promise.all(
    turns.map(async (turn) => {
      try {
        return [turn.id, await resolveTurnThread(turn, fetchEvent)] as const;
      } catch {
        return null;
      }
    }),
  );
  return new Map(entries.filter((entry) => entry !== null));
}

/** Test-only: reset the module cache between specs. */
export function _testResetThreadEventCache() {
  moduleEventCache.clear();
}

export interface ThreadScopeState {
  groups: ThreadGroups;
  /**
   * True while any derived turn lacks a resolution — including the first
   * render after mount, BEFORE the resolution effect has fired. Derived
   * (not effect-set state) so consumers can trust it in their own
   * first-commit effects: a panel mounted pinned to a thread must not
   * mistake "not resolved yet" for "thread has no activity".
   */
  resolving: boolean;
}

const EMPTY_GROUPS: ThreadGroups = { threads: [], unthreaded: null };

/**
 * Thread groups across a set of agents' frame windows.
 *
 * `framesByAgent` must be reference-stable per recompute (the caller builds
 * it inside its own memo). Resolution results accumulate in React state; a
 * turn appears in a group only after its thread root resolves, so a group
 * chip can lag a live turn by one fetch round-trip — acceptable, because the
 * default view (latest thread) already contains the new turn once resolved.
 */
export function useThreadScope(
  framesByAgent: ReadonlyMap<string, readonly ObserverEvent[]>,
): ThreadScopeState {
  const [resolutions, setResolutions] = React.useState<
    ReadonlyMap<string, TurnThreadResolution>
  >(new Map());

  const turnsByAgent = React.useMemo(() => {
    const result = new Map<string, readonly ActivityTurn[]>();
    for (const [agentId, frames] of framesByAgent) {
      const turns = deriveActivityTurns(frames);
      if (turns.length > 0) result.set(agentId, turns);
    }
    return result;
  }, [framesByAgent]);

  // Derived, so it is truthful from the very first render (see interface).
  const resolving = React.useMemo(() => {
    for (const turns of turnsByAgent.values()) {
      for (const turn of turns) {
        if (!resolutions.has(turn.id)) return true;
      }
    }
    return false;
  }, [turnsByAgent, resolutions]);

  // Resolve any turns not yet in the resolutions map. Generation counter
  // discards stale async completions after the input set changes.
  const generationRef = React.useRef(0);
  React.useEffect(() => {
    const unresolved: ActivityTurn[] = [];
    for (const turns of turnsByAgent.values()) {
      for (const turn of turns) {
        if (!resolutions.has(turn.id)) unresolved.push(turn);
      }
    }
    if (unresolved.length === 0) return;

    const generation = ++generationRef.current;
    let cancelled = false;

    void resolveTurnThreadsRetriable(unresolved, fetchEventCached).then(
      (resolved) => {
        if (cancelled || generation !== generationRef.current) return;
        if (resolved.size === 0) return;
        setResolutions((current) => {
          const next = new Map(current);
          for (const [turnId, resolution] of resolved) {
            next.set(turnId, resolution);
          }
          return next;
        });
      },
    );

    return () => {
      cancelled = true;
    };
  }, [turnsByAgent, resolutions]);

  const groups = React.useMemo(() => {
    if (turnsByAgent.size === 0) return EMPTY_GROUPS;
    return buildThreadGroups(turnsByAgent, resolutions);
  }, [turnsByAgent, resolutions]);

  return { groups, resolving };
}
