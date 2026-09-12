import type { ControlResultFrame } from "@/shared/api/types";
import type { ObserverEvent } from "../ui/agentSessionTypes";
import { compareObserverEvents } from "../observerEventOrdering";

type NativeOption = { value: string; label: string };
export type ConversationEffort = {
  channelId: string;
  label: string | null;
  sessionId: string;
  sessionToken: string;
  timestamp: string;
  value: string;
  options: NativeOption[];
};

function object(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function values(input: unknown): NativeOption[] {
  if (!Array.isArray(input)) return [];
  return input.flatMap((item) => {
    const option = object(item);
    if (!option) return [];
    if (typeof option.value === "string") {
      return [
        {
          value: option.value,
          label: typeof option.name === "string" ? option.name : option.value,
        },
      ];
    }
    return values(option.options);
  });
}

/** Keep small native snapshots independent of the bounded activity transcript.
 * A new session for the same conversation supersedes the previous target. */
export function retainSessionConfigs(
  current: readonly ObserverEvent[],
  arrivals: readonly ObserverEvent[],
): readonly ObserverEvent[] {
  const captures = arrivals.filter(
    (event) =>
      event.kind === "session_config_captured" &&
      event.channelId &&
      event.sessionId,
  );
  if (captures.length === 0) return current;
  const incoming = new Set(captures);
  const latest = new Map<string, ObserverEvent>();
  let changed = false;
  for (const event of [...current, ...captures]) {
    if (
      !event.channelId ||
      !event.sessionId ||
      event.kind !== "session_config_captured"
    )
      continue;
    const config = object(event.payload);
    const key = JSON.stringify([
      event.channelId,
      config?.conversationId ?? event.sessionId,
    ]);
    const previous = latest.get(key);
    if (previous && compareObserverEvents(event, previous) <= 0) continue;
    latest.set(key, {
      ...event,
      payload: {
        conversationId: config?.conversationId,
        effortSessionToken: config?.effortSessionToken,
        conversationLabel: config?.conversationLabel,
        liveEffortSwitching: config?.liveEffortSwitching,
        configOptions: Array.isArray(config?.configOptions)
          ? config.configOptions.filter(
              (option) => object(option)?.category === "thought_level",
            )
          : [],
      },
    });
    if (incoming.has(event)) changed = true;
  }
  return changed
    ? [...latest.values()].sort(compareObserverEvents).slice(-128)
    : current;
}

/** Read only exact-session native snapshots; a later empty snapshot removes support. */
export function conversationEfforts(
  events: readonly ObserverEvent[],
  channelId: string | null,
): ConversationEffort[] {
  return retainSessionConfigs([], events)
    .flatMap((event) => {
      if (channelId && event.channelId !== channelId) return [];
      const config = object(event.payload);
      if (
        config?.liveEffortSwitching !== true ||
        typeof config.effortSessionToken !== "string" ||
        !config.effortSessionToken
      )
        return [];
      if (!Array.isArray(config?.configOptions)) return [];
      const option = config.configOptions
        .map(object)
        .find((o) => o?.category === "thought_level" && o.type === "select");
      if (!option || typeof option.currentValue !== "string") return [];
      const options = values(option.options);
      if (options.length === 0) return [];
      return [
        {
          channelId: event.channelId as string,
          label:
            typeof config.conversationLabel === "string"
              ? config.conversationLabel
              : null,
          sessionId: event.sessionId as string,
          sessionToken: config.effortSessionToken,
          timestamp: event.timestamp,
          value: option.currentValue,
          options,
        },
      ];
    })
    .sort((a, b) => b.timestamp.localeCompare(a.timestamp));
}

export type EffortRequest = {
  requestId: string;
  sessionId: string;
  sessionToken: string;
  channelId: string;
  effort: string;
};

export function matchingEffortStatus(
  frame: ControlResultFrame,
  request: EffortRequest,
): string | null {
  return frame.type === "switch_effort" &&
    frame.requestId === request.requestId &&
    frame.sessionId === request.sessionId &&
    frame.sessionToken === request.sessionToken &&
    frame.channelId === request.channelId &&
    frame.effort === request.effort
    ? frame.status
    : null;
}
