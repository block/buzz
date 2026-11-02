import * as React from "react";

import { FREE_LIMIT_COPY } from "@/features/agents/lib/friendlyAgentLastError";

import { FirstRunButton, FirstRunError } from "./firstRunControls";
import { FirstRunHead, ScoutHeadMark } from "./FirstRunFrame";
import {
  AlertIcon,
  CheckIcon,
  ClockIcon,
  ShieldCheckIcon,
  WaitingCircleIcon,
  WorkingIcon,
} from "./firstRunIcons";
import type {
  OnboardingSceneData,
  PresentationProps,
} from "./OnboardingSceneTypes";

const SLOW_REPLY_MS = 20_000;
export const SLOW_REPLY_COPY =
  "Still waiting. When the AI is busy, this can take a few minutes. You can keep waiting or cancel.";

/** "this Mac" on a Mac, "this computer" anywhere else. */
export function thisComputer(): string {
  const platform =
    typeof navigator === "undefined" ? "" : (navigator.platform ?? "");
  return /mac/i.test(platform) ? "this Mac" : "this computer";
}

/** Reassures the owner when a first hello is slow: busy free models can take minutes. */
function SlowReplyHint() {
  const [slow, setSlow] = React.useState(false);
  React.useEffect(() => {
    const timer = window.setTimeout(() => setSlow(true), SLOW_REPLY_MS);
    return () => window.clearTimeout(timer);
  }, []);
  return (
    <p
      aria-live="polite"
      className="c3fr-note"
      data-testid="onboarding-slow-reply"
      role="status"
    >
      {slow ? SLOW_REPLY_COPY : null}
    </p>
  );
}

type StepState = "done" | "working" | "next";

function StepIcon({ state }: { state: StepState }) {
  if (state === "done") return <CheckIcon size={17} strokeWidth={3} />;
  if (state === "working")
    return <WorkingIcon className="c3fr-live" size={18} strokeWidth={2.6} />;
  return <WaitingCircleIcon size={17} strokeWidth={2.2} />;
}

const STEP_WORDS: Record<StepState, string> = {
  done: "Done",
  working: "Working",
  next: "Next",
};

/** A first hello: three steps, driven by the real connection phases. */
export function TestingScene({
  data,
  headingRef,
  onCancelTest,
  onNavigate,
}: Pick<
  PresentationProps,
  "data" | "headingRef" | "onCancelTest" | "onNavigate"
>) {
  const phase = data.visualOnly ? "starting" : data.connectionPhase;
  const saving = phase === "saving";
  const waiting = phase === "waiting" || saving;
  const app = data.harnessLabel || "Your AI";
  const steps: Array<{
    title: string;
    detail: string;
    state: StepState;
  }> = [
    {
      title: "Connection ready",
      detail: `${app} on ${thisComputer()}`,
      state: "done",
    },
    {
      title: "Starting your agent",
      detail: data.shellTools ? "Setting up shell tools" : "Scout is waking up",
      state: waiting ? "done" : "working",
    },
    {
      title: saving ? "Reply received" : "Waiting for a reply",
      detail: "Scout will say hello right here",
      state: saving ? "done" : waiting ? "working" : "next",
    },
  ];
  return (
    <div
      className="c3fr-form"
      data-testid="onboarding-testing"
      style={{ "--c3fr-gap": 26 } as React.CSSProperties}
    >
      <FirstRunHead
        headingRef={headingRef}
        lede="Checking that Scout can think and reply before you go in."
        title="A first hello."
      />
      <ol aria-label="Connection test" className="c3fr-steps">
        {steps.map((step) => (
          <li
            aria-current={step.state === "working" ? "step" : undefined}
            data-state={step.state}
            key={step.title}
          >
            <span aria-hidden="true" className="c3fr-step-icon">
              <StepIcon state={step.state} />
            </span>
            <span className="c3fr-step-text">
              <span className="c3fr-step-title">{step.title}</span>
              <span className="c3fr-step-detail">{step.detail}</span>
            </span>
            <span className="c3fr-step-word">{STEP_WORDS[step.state]}</span>
          </li>
        ))}
      </ol>
      <div className="c3fr-actions" data-stack="true">
        <div className="c3fr-banner" role="status">
          <span aria-hidden="true" className="c3fr-banner-dot c3fr-live" />
          {data.shellTools
            ? data.shellTools
            : saving
              ? "Saving your connection…"
              : "Waiting for Scout’s reply…"}
        </div>
        <FirstRunButton
          disabled={saving}
          onClick={onCancelTest ?? (() => onNavigate?.("connect"))}
        >
          Cancel test
        </FirstRunButton>
      </div>
      {waiting && !saving && !data.visualOnly ? <SlowReplyHint /> : null}
    </div>
  );
}

/** Scout's first reply, the proof the connection works. */
export function ConnectedScene({
  data,
  headingRef,
  onNavigate,
}: Pick<PresentationProps, "data" | "headingRef" | "onNavigate">) {
  const reply =
    data.connectionReply ??
    data.firstReply ??
    (data.visualOnly
      ? `Hi ${data.name.trim().split(" ")[0] || "there"}! I’m here and ready when you are.`
      : "");
  const where = data.connectionLabel
    ? data.connectionLabel
    : `${data.harnessLabel ?? "Your AI"} · ${data.effectiveModel ?? thisComputer()}`;
  return (
    <div
      className="c3fr-form"
      style={{ "--c3fr-gap": 24 } as React.CSSProperties}
    >
      <FirstRunHead
        headingRef={headingRef}
        lede="Scout replied, so your AI connection works. You can change it any time in Settings › Agents."
        title="That’s a good start."
      />
      <article className="c3fr-card c3fr-reply">
        <div className="c3fr-reply-head">
          <span aria-hidden="true" className="c3fr-reply-avatar">
            <ScoutHeadMark />
          </span>
          <div className="c3fr-reply-body">
            <div className="c3fr-reply-who">
              <strong>Scout</strong>
              <span>Chief of Staff</span>
              <span className="c3fr-pill" data-tone="ok">
                <CheckIcon size={12} strokeWidth={3.2} />
                Replied
              </span>
            </div>
            <p className="c3fr-reply-text">{reply}</p>
          </div>
        </div>
        <div className="c3fr-reply-meta">
          <span className="c3fr-verified">
            <ShieldCheckIcon size={16} strokeWidth={2.4} />
            Connection verified
          </span>
          <span>{where}</span>
        </div>
      </article>
      <div className="c3fr-actions" data-stack="true">
        <FirstRunButton
          kind="primary"
          onClick={() => onNavigate?.("workspace")}
        >
          Open my Colony
        </FirstRunButton>
        <FirstRunButton onClick={() => onNavigate?.("connect")}>
          Change connection
        </FirstRunButton>
      </div>
    </div>
  );
}

/** Scout did not reply: the connection is not saved, and there are two ways on. */
export function ConnectionErrorScene({
  data,
  headingRef,
  onNavigate,
}: Pick<PresentationProps, "data" | "headingRef" | "onNavigate">) {
  return (
    <div
      className="c3fr-form"
      style={{ "--c3fr-gap": 24 } as React.CSSProperties}
    >
      <section
        aria-label="No reply"
        className="c3fr-callout"
        data-testid="connection-error"
      >
        <span aria-hidden="true" className="c3fr-callout-icon">
          <ClockIcon size={17} strokeWidth={2.4} />
        </span>
        <div className="c3fr-callout-body">
          <div className="c3fr-callout-text">
            <h2 className="c3fr-callout-title" ref={headingRef} tabIndex={-1}>
              No reply just yet.
            </h2>
            <p>
              Your account and business are saved. Try again, or pick another
              way for Scout to run.
            </p>
          </div>
          {data.error ? <FirstRunError>{data.error}</FirstRunError> : null}
          <div className="c3fr-callout-actions">
            <FirstRunButton
              onClick={() => onNavigate?.("testing")}
              size="small"
            >
              Try again
            </FirstRunButton>
            <button
              className="c3fr-link-action alternate-connection"
              onClick={() => onNavigate?.("connect")}
              type="button"
            >
              {data.error === FREE_LIMIT_COPY
                ? "Connect your own AI"
                : "Choose another connection"}
            </button>
          </div>
        </div>
      </section>
    </div>
  );
}

type ReadyStep = { title: string; detail?: string; state: StepState };

/** How far through "Getting your Colony ready" the real stages say we are. */
function readySteps(
  entry: NonNullable<PresentationProps["entry"]>["stage"] | undefined,
  business: string,
): { steps: ReadyStep[]; percent: number } {
  const entering = entry === "entering";
  const channels = entry === "profile" || entry === "finalizing";
  return {
    percent: entering ? 92 : channels ? 66 : 33,
    steps: [
      {
        title: `${business || "Your business"} is set up`,
        state: "done",
      },
      {
        title: "Opening your channels",
        detail: "#Welcome, #general and #welcome-everyone",
        state: entering ? "done" : "working",
      },
      {
        title: "Scout joins as Chief of Staff",
        state: entering ? "working" : "next",
      },
    ],
  };
}

/** "Getting your Colony ready", or the card for when setup stops. */
export function ReadyScene({
  data,
  entry,
  headingRef,
  onNavigate,
}: Pick<PresentationProps, "data" | "entry" | "headingRef" | "onNavigate">) {
  const stopped = Boolean(data.error);
  const { steps, percent } = readySteps(
    data.visualOnly ? "profile" : entry?.stage,
    data.business,
  );
  if (stopped) {
    return (
      <div
        className="c3fr-form"
        style={{ "--c3fr-gap": 24 } as React.CSSProperties}
      >
        <section
          aria-label="Setup stopped"
          className="c3fr-callout"
          data-testid="community-entry-error"
          data-tone="bad"
        >
          <span aria-hidden="true" className="c3fr-callout-icon">
            <AlertIcon size={17} strokeWidth={2.4} />
          </span>
          <div className="c3fr-callout-body">
            <div className="c3fr-callout-text">
              <h2 className="c3fr-callout-title" ref={headingRef} tabIndex={-1}>
                Your Colony isn’t ready yet.
              </h2>
              <p>
                Something interrupted the setup. Your account and business are
                saved.
              </p>
            </div>
            <FirstRunError>{data.error}</FirstRunError>
            <div className="c3fr-callout-actions">
              <FirstRunButton
                kind="primary"
                onClick={() => onNavigate?.("community-entry")}
                size="small"
              >
                Try again
              </FirstRunButton>
              <FirstRunButton
                onClick={() => onNavigate?.("workspace")}
                size="small"
              >
                {data.entryCanOpen === false
                  ? "Change community"
                  : "Open my Colony for now"}
              </FirstRunButton>
            </div>
          </div>
        </section>
      </div>
    );
  }
  return (
    <div
      className="c3fr-form"
      style={{ "--c3fr-gap": 24 } as React.CSSProperties}
    >
      <FirstRunHead
        headingRef={headingRef}
        lede="We’re preparing your Welcome channel and Scout."
        title="Getting your Colony ready."
      />
      <div className="c3fr-card c3fr-ready-card" style={{ gap: 18 }}>
        <div
          aria-label={`Setting up ${data.business || "your Colony"}`}
          aria-valuemax={100}
          aria-valuemin={0}
          aria-valuenow={percent}
          className="c3fr-bar"
          role="progressbar"
        >
          <div style={{ width: `${percent}%` }} />
        </div>
        <ol aria-label="Setup steps" className="c3fr-ready-steps" role="status">
          {steps.map((step) => (
            <li
              aria-current={step.state === "working" ? "step" : undefined}
              data-state={step.state}
              key={step.title}
            >
              <span aria-hidden="true" className="c3fr-ready-icon">
                <StepIcon state={step.state} />
              </span>
              <span className="c3fr-ready-text">
                <span className="c3fr-ready-title">{step.title}</span>
                {step.detail ? (
                  <span className="c3fr-step-detail">{step.detail}</span>
                ) : null}
              </span>
            </li>
          ))}
        </ol>
      </div>
      <FirstRunButton
        disabled={entry?.openRequested}
        kind="primary"
        onClick={entry?.onOpen ?? (() => onNavigate?.("workspace"))}
      >
        {entry?.openRequested ? "Opening…" : "Open my Colony"}
      </FirstRunButton>
    </div>
  );
}

export type { OnboardingSceneData };
