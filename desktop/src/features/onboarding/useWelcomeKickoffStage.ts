import * as React from "react";

import { isWelcomeChannel } from "@/features/onboarding/welcome";
import type { Channel } from "@/shared/api/types";

/**
 * Stage lifecycle for the Welcome kickoff loading animation.
 *
 * - `hidden`: not shown (not Welcome, timeline not settled, or already done)
 * - `active`: characters on stage — the team is being set up
 * - `timed-out`: no agent message arrived within the window; quiet fallback
 * - `exiting`: a message landed — play the exit animation, then hide
 */
export type WelcomeKickoffStagePhase =
  | "hidden"
  | "active"
  | "timed-out"
  | "exiting";

/**
 * How long the stage waits for the first agent message before settling into
 * the quiet timed-out state. Generous because the teammate presence wait
 * alone can take up to 60s (see welcomeKickoff.ts TEAMMATE_READY_WAIT_MS).
 */
export const WELCOME_KICKOFF_STAGE_TIMEOUT_MS = 90_000;

/**
 * Dev-only preview switch: forces the stage to `active` on the Welcome
 * channel regardless of messages, so choreography can be iterated on a
 * running dev app (HMR) without re-running fresh onboarding. Stripped from
 * production builds via the `import.meta.env.DEV` guard below.
 *
 * TODO(morganm): flip back to false before merging.
 */
const DEV_FORCE_STAGE = false;

export type WelcomeKickoffStageInput = {
  /** The active channel is the private Welcome channel. */
  isWelcome: boolean;
  /** The timeline query has settled — an empty list means truly empty. */
  timelineSettled: boolean;
  /** Any message exists in the channel (agent or user authored). */
  hasMessages: boolean;
  /** The timeout window elapsed while the stage was active. */
  timedOut: boolean;
};

/**
 * Pure phase transition — one rule dismisses the stage for every resolution
 * (happy-path opener, provider fallback, setup nudge, or a user message):
 * the first message in the channel moves the stage to `exiting`.
 *
 * The stage only ever *enters* from `hidden` on a confirmed-empty timeline,
 * so revisits after the kickoff completed never replay it.
 */
export function resolveWelcomeKickoffStagePhase(
  current: WelcomeKickoffStagePhase,
  input: WelcomeKickoffStageInput,
): WelcomeKickoffStagePhase {
  if (!input.isWelcome) return "hidden";
  if (current === "hidden") {
    return input.timelineSettled && !input.hasMessages ? "active" : "hidden";
  }
  if (current === "exiting") return "exiting";
  if (input.hasMessages) return "exiting";
  if (input.timedOut && current === "active") return "timed-out";
  return current;
}

/**
 * Drives the Welcome kickoff stage from local state only — no network
 * round-trips. The stage appears the instant the user lands on a confirmed
 * empty Welcome channel and dismisses when the first message arrives.
 *
 * `hasTimelineMessages` must reflect *visible timeline rows* (the formatted
 * message list), not raw channel events. A fresh Welcome channel already
 * carries non-message events (canvas seed, membership records) that render
 * nothing — gating on raw events keeps the stage hidden forever.
 */
export function useWelcomeKickoffStage(
  activeChannel: Channel | null,
  hasTimelineMessages: boolean,
  timelineLoading: boolean,
) {
  const channelId = activeChannel?.id ?? null;
  const isWelcome = isWelcomeChannel(activeChannel);
  const forceStage = import.meta.env.DEV && DEV_FORCE_STAGE;
  const [phase, setPhase] = React.useState<WelcomeKickoffStagePhase>("hidden");
  const [timedOut, setTimedOut] = React.useState(false);

  // biome-ignore lint/correctness/useExhaustiveDependencies: reset stage state exactly when the active channel changes.
  React.useEffect(() => {
    setPhase("hidden");
    setTimedOut(false);
  }, [channelId]);

  React.useEffect(() => {
    setPhase((current) =>
      resolveWelcomeKickoffStagePhase(current, {
        isWelcome,
        timelineSettled: !timelineLoading,
        hasMessages: hasTimelineMessages,
        timedOut,
      }),
    );
  }, [hasTimelineMessages, isWelcome, timedOut, timelineLoading]);

  React.useEffect(() => {
    if (phase !== "active") return;
    const timer = globalThis.setTimeout(
      () => setTimedOut(true),
      WELCOME_KICKOFF_STAGE_TIMEOUT_MS,
    );
    return () => globalThis.clearTimeout(timer);
  }, [phase]);

  const handleExitComplete = React.useCallback(() => {
    setPhase("hidden");
  }, []);

  if (forceStage && isWelcome) {
    return { phase: "active" as const, handleExitComplete };
  }
  return { phase, handleExitComplete };
}
