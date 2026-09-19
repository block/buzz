// Ported/adapted from Berd's agent-activity-panel (branch
// zmarley/agent-activity-panel). Data hook: derives activity events + turns
// from an agent's observer frames and compiles the render scene, with a
// fingerprint cache so unchanged frame windows never recompile (the lazy
// -compile lesson from Berd's review).

import * as React from "react";

import type { ObserverEvent } from "../ui/agentSessionTypes";
import type { ActivityAgent, ActivityEvent } from "./activityModel";
import { type ActivityScene, compileScene } from "./activityScene";
import {
  type ActivityTurn,
  deriveActivityTurns,
  eventsForTurn,
} from "./activityTurns";
import { hashActivityEvents } from "./activityUtils";
import { deriveActivityFromObserverFrames } from "./deriveActivity";

/** "session" for the whole stream, or a turnId to scope to one turn. */
export type ActivityScope = "session" | `turn:${string}`;

export interface ActivitySceneState {
  scene: ActivityScene | null;
  turns: ActivityTurn[];
  events: ActivityEvent[];
  agents: ActivityAgent[];
}

export interface UseActivitySceneOptions {
  agentId: string;
  agentName?: string;
  agentIndex?: number;
  roots?: readonly string[];
  scope?: ActivityScope;
}

export function turnScope(turnId: string): ActivityScope {
  return `turn:${turnId}`;
}

function scopeTurnId(scope: ActivityScope): string | null {
  return scope.startsWith("turn:") ? scope.slice("turn:".length) : null;
}

export function useActivityScene(
  frames: readonly ObserverEvent[],
  options: UseActivitySceneOptions,
): ActivitySceneState {
  const { agentId, agentName, agentIndex, roots, scope = "session" } = options;

  const derived = React.useMemo(
    () =>
      deriveActivityFromObserverFrames(frames, {
        agentId,
        ...(agentName !== undefined ? { agentName } : {}),
        ...(agentIndex !== undefined ? { agentIndex } : {}),
        ...(roots !== undefined ? { roots } : {}),
      }),
    [frames, agentId, agentName, agentIndex, roots],
  );

  const turns = React.useMemo(() => deriveActivityTurns(frames), [frames]);

  const scopedEvents = React.useMemo(() => {
    const turnId = scopeTurnId(scope);
    return turnId ? eventsForTurn(derived.events, turnId) : derived.events;
  }, [derived.events, scope]);

  // Fingerprint cache: recompiling the scene on every observer append is the
  // hot path during live turns. The hash covers kind/path/intent (late-
  // patched narration invalidates), and length + last timestamp cover pure
  // appends.
  const cacheRef = React.useRef<{ key: string; scene: ActivityScene } | null>(
    null,
  );

  const scene = React.useMemo(() => {
    if (scopedEvents.length === 0) return null;
    const lastT = scopedEvents[scopedEvents.length - 1]?.t ?? 0;
    const key = `${scope}:${scopedEvents.length}:${lastT}:${hashActivityEvents(
      scopedEvents,
    )}`;
    if (cacheRef.current?.key === key) return cacheRef.current.scene;
    const compiled = compileScene(scopedEvents, derived.agents);
    cacheRef.current = { key, scene: compiled };
    return compiled;
  }, [scopedEvents, derived.agents, scope]);

  return { scene, turns, events: scopedEvents, agents: derived.agents };
}
