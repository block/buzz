// Multi-agent activity merge: derives per-agent activity events from each
// agent's observer frames, disambiguates path namespaces across differing
// workspace roots, and compiles a single combined scene. This is the
// concatenation layer the single-agent panel was designed for ("callers
// compose multi-agent by concatenating results").

import * as React from "react";

import {
  getAgentObserverSnapshot,
  getArchivedChannelEvents,
  getObservedAgentPubkeys,
  subscribeAgentObserverStore,
} from "../observerRelayStore";
import {
  mergeObserverEventWindows,
  scopeByChannel,
} from "../ui/agentSessionPanelLayout";
import type { ObserverEvent } from "../ui/agentSessionTypes";
import type { ActivityAgent, ActivityEvent } from "./activityModel";
import { type ActivityScene, compileScene } from "./activityScene";
import { filterFramesByTurnIds } from "./activityThreads";
import { basename } from "./activityUtils";
import {
  deriveActivityFromObserverFrames,
  inferActivityRoots,
} from "./deriveActivity";

/**
 * Per-agent allowlist of turn ids (thread scope). Agents absent from the
 * map contribute no frames; a null filter means "no scoping" (all turns).
 */
export type TurnFilter = ReadonlyMap<string, ReadonlySet<string>> | null;

export interface MultiAgentActivityInput {
  pubkey: string;
  name?: string;
}

export interface MultiAgentActivityState {
  scene: ActivityScene | null;
  events: ActivityEvent[];
  agents: ActivityAgent[];
  /** Pubkeys that contributed at least one activity event. */
  activeAgentIds: string[];
}

interface PerAgentDerivation {
  agentId: string;
  events: ActivityEvent[];
  agents: ActivityAgent[];
  roots: readonly string[];
  frameCount: number;
  lastSeq: number;
}

function deriveForAgent(
  agent: MultiAgentActivityInput,
  index: number,
  frames: readonly ObserverEvent[],
): PerAgentDerivation {
  const roots = inferActivityRoots(frames);
  const derived = deriveActivityFromObserverFrames(frames, {
    agentId: agent.pubkey,
    ...(agent.name !== undefined ? { agentName: agent.name } : {}),
    agentIndex: index,
    roots,
  });
  return {
    agentId: agent.pubkey,
    events: derived.events,
    agents: derived.agents,
    roots,
    frameCount: frames.length,
    lastSeq: frames.at(-1)?.seq ?? -1,
  };
}

/**
 * Prefix each agent's paths with its workspace-root basename when agents are
 * working out of DIFFERENT roots, so `src/main.rs` in two repos lands on two
 * distinct rows (`buzz/src/main.rs` vs `berd/src/main.rs`). Agents sharing a
 * root (or with no inferred root) keep unprefixed paths and merge naturally.
 */
export function disambiguatePathNamespaces(
  derivations: readonly PerAgentDerivation[],
): ActivityEvent[][] {
  const rootKeys = new Set(
    derivations.map((derivation) => derivation.roots[0] ?? ""),
  );
  const distinctRoots = [...rootKeys].filter((root) => root.length > 0);
  const needsPrefix = distinctRoots.length > 1;

  return derivations.map((derivation) => {
    const root = derivation.roots[0];
    if (!needsPrefix || !root) return derivation.events;
    const prefix = basename(root);
    if (!prefix) return derivation.events;
    return derivation.events.map((event) =>
      event.path ? { ...event, path: `${prefix}/${event.path}` } : event,
    );
  });
}

function framesForAgent(
  pubkey: string,
  channelId: string | null,
): ObserverEvent[] {
  const live = getAgentObserverSnapshot(pubkey).events;
  const scopedLive = channelId ? scopeByChannel(live, channelId) : live;
  if (!channelId) return scopedLive;
  const archived = getArchivedChannelEvents(pubkey, channelId);
  return mergeObserverEventWindows(scopedLive, archived);
}

/**
 * Combined activity scene across a set of agents, optionally scoped to one
 * channel (live + archived windows merged per agent) and to a per-agent set
 * of turn ids (thread scope). Per-agent derivations are memo-cached on
 * (frameCount, lastSeq, filter identity) so live appends to one agent do
 * not re-derive the others; the scene compiles once over the merged stream.
 */
export function useMultiAgentActivityScene(
  agents: readonly MultiAgentActivityInput[],
  channelId: string | null,
  turnFilter: TurnFilter = null,
): MultiAgentActivityState {
  // Reactive tick: any store change re-runs the snapshot reads below.
  const storeTick = React.useSyncExternalStore(
    subscribeAgentObserverStore,
    getObservedAgentPubkeys,
  );

  const derivationCacheRef = React.useRef(
    new Map<string, PerAgentDerivation>(),
  );

  return React.useMemo(() => {
    // storeTick is the reactive dependency that re-runs this memo.
    void storeTick;

    const cache = derivationCacheRef.current;
    const derivations: PerAgentDerivation[] = [];
    const liveIds = new Set<string>();

    agents.forEach((agent, index) => {
      const unfiltered = framesForAgent(agent.pubkey, channelId);
      let frames = unfiltered;
      let filterKey = "all";
      if (turnFilter) {
        const allowedTurnIds = turnFilter.get(agent.pubkey);
        if (!allowedTurnIds || allowedTurnIds.size === 0) return;
        frames = filterFramesByTurnIds(unfiltered, allowedTurnIds);
        filterKey = [...allowedTurnIds].sort().join(",");
      }
      if (frames.length === 0) return;
      liveIds.add(agent.pubkey);

      const cacheKey = `${agent.pubkey}:${channelId ?? ""}:${filterKey}`;
      const cached = cache.get(cacheKey);
      const lastSeq = frames.at(-1)?.seq ?? -1;
      if (
        cached &&
        cached.frameCount === frames.length &&
        cached.lastSeq === lastSeq
      ) {
        derivations.push(cached);
        return;
      }
      const derivation = deriveForAgent(agent, index, frames);
      cache.set(cacheKey, derivation);
      derivations.push(derivation);
    });

    // Drop cache entries for agents no longer in the input set. Entries for
    // other thread scopes of a live agent stay (bounded by scopes visited;
    // freed on unmount or when the agent leaves).
    for (const key of [...cache.keys()]) {
      const pubkey = key.slice(0, key.indexOf(":"));
      if (!liveIds.has(pubkey)) cache.delete(key);
    }

    const eventStreams = disambiguatePathNamespaces(derivations);
    const events = eventStreams
      .flat()
      .sort((a, b) => a.t - b.t || (a.sourceSeq ?? 0) - (b.sourceSeq ?? 0));
    const sceneAgents = derivations.flatMap((derivation) => derivation.agents);
    const activeAgentIds = derivations
      .filter((derivation) => derivation.events.length > 0)
      .map((derivation) => derivation.agentId);

    const scene = events.length > 0 ? compileScene(events, sceneAgents) : null;

    return { scene, events, agents: sceneAgents, activeAgentIds };
  }, [agents, channelId, storeTick, turnFilter]);
}

/**
 * Per-agent merged frame windows (live + archive) for a channel, rebuilt on
 * store ticks. Reference-stable input for useThreadScope: agents with no
 * frames in this channel are omitted.
 */
export function useFramesByAgent(
  agents: readonly MultiAgentActivityInput[],
  channelId: string | null,
): ReadonlyMap<string, readonly ObserverEvent[]> {
  const storeTick = React.useSyncExternalStore(
    subscribeAgentObserverStore,
    getObservedAgentPubkeys,
  );

  return React.useMemo(() => {
    void storeTick;
    const result = new Map<string, readonly ObserverEvent[]>();
    for (const agent of agents) {
      const frames = framesForAgent(agent.pubkey, channelId);
      if (frames.length > 0) result.set(agent.pubkey, frames);
    }
    return result;
  }, [agents, channelId, storeTick]);
}
