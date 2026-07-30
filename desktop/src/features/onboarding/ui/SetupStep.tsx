import * as React from "react";
import { useReducedMotion } from "motion/react";

import { useAcpRuntimesQuery } from "@/features/agents/hooks";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { FlappingBee } from "@/shared/ui/buzz-logo/FlappingBee";
import { HarnessChoiceCard } from "./HarnessChoiceCard";
import { getVisibleOnboardingRuntimes } from "./onboardingRuntimeSelection";
import { OnboardingFooter } from "./OnboardingFooter";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import type { SetupStepActions, SetupStepState } from "./types";

/** How long unselected cards fade before the page advances (ms). */
const CHOICE_FADE_MS = 200;

type SetupStepProps = {
  actions: SetupStepActions;
  direction: OnboardingTransitionDirection;
  selectedRuntimeId: string | null;
};

type SetupStepContentProps = SetupStepProps & {
  state: SetupStepState;
};

function useSetupStepState(): SetupStepState {
  const runtimesQuery = useAcpRuntimesQuery();
  const items = runtimesQuery.data ?? [];
  const isChecking = runtimesQuery.isLoading;
  const errorMessage =
    runtimesQuery.error instanceof Error ? runtimesQuery.error.message : null;

  return {
    runtimeProviders: {
      errorMessage,
      isChecking,
      items,
    },
  };
}

function RuntimeChoiceCard({
  dimmed,
  onChoose,
  runtime,
  selected,
}: {
  dimmed: boolean;
  onChoose: () => void;
  runtime: AcpRuntimeCatalogEntry;
  selected: boolean;
}) {
  return (
    <div
      className={cn(
        "relative w-full max-w-[288px] transition-opacity duration-200",
        dimmed && "pointer-events-none opacity-0",
      )}
    >
      <HarnessChoiceCard
        aria-pressed={selected}
        className="cursor-pointer transition duration-150 hover:-translate-y-0.5 hover:brightness-[0.98] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-foreground/35"
        data-selected={selected ? "true" : "false"}
        data-testid={`onboarding-runtime-${runtime.id}`}
        onClick={onChoose}
        onKeyDown={(event) => {
          if (event.key !== "Enter" && event.key !== " ") return;
          event.preventDefault();
          onChoose();
        }}
        role="button"
        runtime={runtime}
        tabIndex={0}
      />
    </div>
  );
}

function RuntimeProvidersLoadingState() {
  return (
    <div
      aria-live="polite"
      className="flex min-h-[260px] w-full items-center justify-center"
      data-testid="onboarding-runtime-loading"
      role="status"
    >
      <div className="flex flex-col items-center text-foreground opacity-35">
        <FlappingBee className="h-auto w-16" />
        <p className="mt-5 text-2xl font-normal leading-8">
          Finding your providers...
        </p>
      </div>
    </div>
  );
}

function RuntimeProvidersSection({
  onChooseRuntime,
  pendingRuntimeId,
  runtimeProviders,
  selectedRuntimeId,
}: {
  onChooseRuntime: (runtimeId: string) => void;
  pendingRuntimeId: string | null;
  runtimeProviders: SetupStepState["runtimeProviders"];
  selectedRuntimeId: string | null;
}) {
  const { errorMessage, isChecking, items } = runtimeProviders;
  const orderedItems = getVisibleOnboardingRuntimes(items);
  const highlightedRuntimeId = pendingRuntimeId ?? selectedRuntimeId;

  return (
    <section className="flex min-h-full w-full flex-col items-center">
      <div className="w-full max-w-[820px] text-center">
        <h1 className="text-title font-normal text-foreground">
          Choose your default harness
        </h1>
        <p className="mx-auto mt-3 max-w-[640px] text-sm leading-6 text-foreground/90">
          Pick the harness Buzz should use by default. You can always connect
          and add more harnesses later in Settings.
        </p>
      </div>

      <div className="flex w-full flex-1 flex-col items-center justify-center gap-8 py-6 [@media(max-height:560px)]:py-2">
        {orderedItems.length > 0 ? (
          // 4-across from md so the grid fits the 800×500 minimum window
          // without scrolling under the docked footer (cards shrink below
          // their 288px max); single column only on very narrow layouts.
          <div className="grid min-w-0 w-full max-w-[1200px] grid-cols-1 justify-items-center gap-4 md:grid-cols-4">
            {orderedItems.map((runtime) => (
              <RuntimeChoiceCard
                dimmed={pendingRuntimeId !== null}
                key={runtime.id}
                onChoose={() => onChooseRuntime(runtime.id)}
                runtime={runtime}
                selected={runtime.id === highlightedRuntimeId}
              />
            ))}
          </div>
        ) : isChecking ? (
          <RuntimeProvidersLoadingState />
        ) : errorMessage ? null : (
          <p
            className="max-w-[560px] rounded-2xl bg-white/70 px-6 py-6 text-sm text-muted-foreground"
            data-testid="onboarding-acp-empty"
          >
            No supported agent harnesses were detected yet. You can finish
            onboarding now and connect a harness later in Settings.
          </p>
        )}

        {errorMessage ? (
          <p className="max-w-[560px] rounded-2xl bg-destructive/10 px-6 py-3 text-sm text-destructive">
            {errorMessage}
          </p>
        ) : null}
      </div>
    </section>
  );
}

function SetupStepContent({
  actions,
  direction,
  selectedRuntimeId,
  state,
}: SetupStepContentProps) {
  const shouldReduceMotion = useReducedMotion() ?? false;
  const [pendingRuntimeId, setPendingRuntimeId] = React.useState<string | null>(
    null,
  );

  // Card click: fade the unselected cards out, then advance into the setup
  // page's vanilla fade.
  const chooseRuntime = React.useCallback(
    (runtimeId: string) => {
      if (pendingRuntimeId) return;
      if (shouldReduceMotion) {
        actions.next(runtimeId);
        return;
      }
      setPendingRuntimeId(runtimeId);
    },
    [actions, pendingRuntimeId, shouldReduceMotion],
  );

  React.useEffect(() => {
    if (!pendingRuntimeId) return;
    const timeout = window.setTimeout(
      () => actions.next(pendingRuntimeId),
      CHOICE_FADE_MS,
    );
    return () => window.clearTimeout(timeout);
  }, [actions, pendingRuntimeId]);

  return (
    <OnboardingSlideTransition
      className="flex min-h-full w-full flex-col items-center"
      data-testid="onboarding-page-2"
      direction={direction}
      effect="fade"
      transitionKey={`setup-${direction}`}
    >
      <RuntimeProvidersSection
        onChooseRuntime={chooseRuntime}
        pendingRuntimeId={pendingRuntimeId}
        runtimeProviders={state.runtimeProviders}
        selectedRuntimeId={selectedRuntimeId}
      />

      <OnboardingFooter>
        <Button
          className="h-9 rounded-full bg-foreground/10 px-6 text-sm hover:bg-foreground/15"
          data-testid="onboarding-setup-skip"
          onClick={actions.skip}
          type="button"
          variant="ghost"
        >
          Skip for now
        </Button>

        <Button
          className="h-9 rounded-full bg-foreground/10 px-6 text-sm hover:bg-foreground/15"
          data-testid="onboarding-back"
          onClick={actions.back}
          type="button"
          variant="ghost"
        >
          Back
        </Button>

        <p className="text-xs text-foreground/50">
          More harnesses (Cursor, Grok, Amp&hellip;){" "}
          {actions.navigateToAgentSettings ? (
            <button
              className="text-foreground/70 underline underline-offset-2 hover:text-foreground"
              data-testid="onboarding-setup-more-harnesses"
              onClick={actions.navigateToAgentSettings}
              type="button"
            >
              Settings → Agents
            </button>
          ) : (
            <span className="text-foreground/70">Settings → Agents</span>
          )}{" "}
          after setup.
        </p>
      </OnboardingFooter>
    </OnboardingSlideTransition>
  );
}

export function SetupStep({
  actions,
  direction,
  selectedRuntimeId,
}: SetupStepProps) {
  const state = useSetupStepState();

  return (
    <SetupStepContent
      actions={actions}
      direction={direction}
      selectedRuntimeId={selectedRuntimeId}
      state={state}
    />
  );
}
