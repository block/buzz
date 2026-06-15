import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";
import {
  type OnboardingTransitionDirection,
  type OnboardingTransitionEffect,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import type { ProfileStepActions, ProfileStepState } from "./types";

type ProfileStepProps = {
  actions: ProfileStepActions;
  direction: OnboardingTransitionDirection;
  transitionEffect?: OnboardingTransitionEffect;
  state: ProfileStepState;
};

function ErrorBanner({ message }: { message: string | null }) {
  if (!message) {
    return null;
  }

  return (
    <p className="mt-4 rounded-md border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
      {message}
    </p>
  );
}

export function ProfileStep({
  actions,
  direction,
  transitionEffect = "line-slide",
  state,
}: ProfileStepProps) {
  const {
    advanceWithoutSaving,
    back,
    importExistingKey,
    skipForNow,
    submit,
    updateDisplayName,
  } = actions;
  const { isSaving, name, saveRecovery } = state;
  const displayNameDraft = name.draftValue;
  const hasDisplayNameDraft = displayNameDraft.length > 0;
  const canSubmit = displayNameDraft.trim().length > 0 && !isSaving;
  const inputRef = React.useRef<HTMLInputElement | null>(null);

  React.useLayoutEffect(() => {
    inputRef.current?.focus();
  }, []);

  return (
    <OnboardingSlideTransition
      className="flex w-full flex-col items-center text-center"
      data-testid="onboarding-page-1"
      direction={direction}
      effect={transitionEffect}
      transitionKey={`profile-${direction}`}
    >
      <div className="w-full max-w-[500px]">
        <h1 className="text-3xl font-semibold text-foreground">
          First, let's start with your name
        </h1>
        <p className="mt-3 text-sm leading-6 text-muted-foreground">
          Enter a nickname or whatever you want people to call you.
        </p>
      </div>

      <label
        className="mt-12 flex w-full cursor-text flex-col items-center"
        htmlFor="onboarding-display-name"
      >
        <span className="sr-only">Name</span>
        <div className="relative h-20 w-full max-w-[576px]">
          {!hasDisplayNameDraft ? (
            <div
              aria-hidden="true"
              className="pointer-events-none absolute inset-0 flex select-none items-center justify-center"
            >
              <span className="relative inline-flex select-none items-center gap-0 text-4xl font-semibold text-muted-foreground/35 sm:text-5xl">
                <span
                  aria-hidden="true"
                  className="buzz-onboarding-name-placeholder-caret h-[0.9em] w-0.5 rounded-full bg-primary"
                />
                Name
              </span>
            </div>
          ) : null}
          <input
            aria-label="Name"
            autoCapitalize="none"
            autoComplete="off"
            autoCorrect="off"
            className={cn(
              "h-full w-full border-0 bg-transparent px-0 py-0 text-center text-4xl font-semibold text-foreground shadow-none outline-none caret-foreground disabled:cursor-not-allowed disabled:opacity-50 sm:text-5xl",
              !hasDisplayNameDraft && "text-transparent caret-transparent",
            )}
            data-testid="onboarding-display-name"
            disabled={isSaving}
            id="onboarding-display-name"
            onChange={(event) => updateDisplayName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && canSubmit) {
                event.preventDefault();
                submit();
              }
            }}
            ref={inputRef}
            spellCheck={false}
            value={displayNameDraft}
          />
        </div>
      </label>

      {saveRecovery.errorMessage ? (
        <ErrorBanner message={saveRecovery.errorMessage} />
      ) : null}

      <div className="mt-12 flex w-full max-w-[500px] flex-col gap-3">
        <Button
          className="h-10 w-full"
          data-testid="onboarding-next"
          disabled={!canSubmit}
          onClick={submit}
          type="button"
        >
          {isSaving ? (
            <Spinner aria-label="Saving profile" className="h-4 w-4 border-2" />
          ) : (
            "Next"
          )}
        </Button>

        {back ? (
          <Button
            className="h-10 w-full text-muted-foreground hover:text-accent-foreground"
            data-testid="onboarding-back"
            disabled={isSaving}
            onClick={back}
            type="button"
            variant="ghost"
          >
            Back
          </Button>
        ) : null}

        <Button
          className="text-muted-foreground hover:text-accent-foreground"
          data-testid="onboarding-import-key"
          disabled={isSaving}
          onClick={importExistingKey}
          type="button"
          variant="ghost"
        >
          I already have a key
        </Button>

        <div className="flex min-h-8 items-center gap-2">
          <div className="flex-1" />
          {saveRecovery.canSkipForNow ? (
            <Button
              className="text-muted-foreground hover:text-accent-foreground"
              data-testid="onboarding-skip"
              onClick={skipForNow}
              type="button"
              variant="ghost"
            >
              Skip for now
            </Button>
          ) : null}
          {saveRecovery.canAdvanceWithoutSaving ? (
            <Button
              className="text-muted-foreground hover:text-accent-foreground"
              data-testid="onboarding-next-without-saving"
              onClick={advanceWithoutSaving}
              type="button"
              variant="ghost"
            >
              Continue without saving
            </Button>
          ) : null}
          <div className="flex-1" />
        </div>
      </div>
    </OnboardingSlideTransition>
  );
}
