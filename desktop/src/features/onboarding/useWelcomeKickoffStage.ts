import * as React from "react";

import { isWelcomeChannel } from "@/features/onboarding/welcome";
import type { Channel } from "@/shared/api/types";

/**
 * Stage lifecycle for the Welcome kickoff loading animation.
 *
 * - `hidden`: not shown yet (not Welcome, or the timeline hasn't settled)
 * - `active`: characters on stage — the team is genuinely being set up
 * - `timed-out`: nothing arrived within the window; leave quietly (see below)
 * - `exiting`: a message landed — play the exit animation
 * - `done`: finished for this channel; terminal, never replays
 *
 * `hidden` and `done` are deliberately separate. `hidden` means "not yet",
 * `done` means "already happened" — collapsing them would let the resolver
 * re-enter `active` off a still-empty timeline the moment the stage left,
 * looping the characters forever.
 *
 * `timed-out` is a real, user-visible resolution, not a bookkeeping flag: the
 * characters leave and the banner stops claiming setup is in progress, so a
 * failed kickoff degrades to an ordinary empty channel the user can type in.
 * Explaining *why* it failed is follow-up work — see
 * docs/welcome-kickoff-silent-failures.md.
 */
export type WelcomeKickoffStagePhase =
  | "hidden"
  | "active"
  | "timed-out"
  | "exiting"
  | "done";

/**
 * How long the stage waits for the first agent message before settling into
 * the quiet timed-out state. Generous because the teammate presence wait
 * alone can take up to 60s (see welcomeKickoff.ts TEAMMATE_READY_WAIT_MS).
 */
export const WELCOME_KICKOFF_STAGE_TIMEOUT_MS = 90_000;

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
 * and `done` is terminal, so a stage that already left never replays.
 */
export function resolveWelcomeKickoffStagePhase(
  current: WelcomeKickoffStagePhase,
  input: WelcomeKickoffStageInput,
): WelcomeKickoffStagePhase {
  // Checked before `isWelcome` so the terminal state can never be laundered
  // back into `hidden` (and from there into a replay) by a channel that
  // momentarily reads as non-Welcome. Real channel changes reset the hook.
  if (current === "done") return "done";
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
 * Whether the banner copy may claim the team is still being set up. True only
 * while that is actually happening — a timed-out stage has given up, so it
 * must stop promising a team is coming.
 */
export function isWelcomeKickoffSettingUp(phase: WelcomeKickoffStagePhase) {
  return phase === "active";
}

/**
 * Whether the stage should play its exit animation. Both resolutions leave:
 * `exiting` because a message landed, `timed-out` because none ever will.
 */
export function isWelcomeKickoffStageExiting(phase: WelcomeKickoffStagePhase) {
  return phase === "exiting" || phase === "timed-out";
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
    setPhase("done");
  }, []);

  return { phase, handleExitComplete };
}
