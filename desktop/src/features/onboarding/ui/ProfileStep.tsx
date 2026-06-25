import * as React from "react";
import { toast } from "sonner";

import { SidebarRelayConnectionCompactCard } from "@/features/sidebar/ui/SidebarRelayConnectionCard";
import { useReconnectRelay } from "@/shared/api/useReconnectRelay";
import { cn } from "@/shared/lib/cn";
import { isRelayUnreachableError } from "@/shared/lib/relayError";
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

const ONBOARDING_CONNECTIVITY_SUCCESS_AUTO_DISMISS_MS = 2_500;

function OnboardingRelayConnectionErrorCard({
  isSaving,
  message,
}: {
  isSaving: boolean;
  message: string;
}) {
  const { isPending: isReconnectPending, reconnect } = useReconnectRelay();
  const [dismissedErrorMessage, setDismissedErrorMessage] = React.useState<
    string | null
  >(null);
  const [isReconnectActionPending, setIsReconnectActionPending] =
    React.useState(false);
  const [hasSuccess, setHasSuccess] = React.useState(false);
  const reconnectActionPendingRef = React.useRef(false);
  const successTimeoutRef = React.useRef<number | null>(null);
  const wasSavingRef = React.useRef(isSaving);
  const isActionPending = isReconnectActionPending || isReconnectPending;

  React.useEffect(() => {
    return () => {
      if (successTimeoutRef.current !== null) {
        window.clearTimeout(successTimeoutRef.current);
      }
    };
  }, []);

  React.useEffect(() => {
    if (isSaving && !wasSavingRef.current) {
      if (successTimeoutRef.current !== null) {
        window.clearTimeout(successTimeoutRef.current);
        successTimeoutRef.current = null;
      }
      setDismissedErrorMessage(null);
      setHasSuccess(false);
    }
    wasSavingRef.current = isSaving;
  }, [isSaving]);

  const markSuccess = React.useCallback(() => {
    setHasSuccess(true);
    if (successTimeoutRef.current !== null) {
      window.clearTimeout(successTimeoutRef.current);
    }
    successTimeoutRef.current = window.setTimeout(() => {
      successTimeoutRef.current = null;
      setDismissedErrorMessage(message);
    }, ONBOARDING_CONNECTIVITY_SUCCESS_AUTO_DISMISS_MS);
  }, [message]);

  const runConnectivityAction = React.useCallback(
    (runAction: () => Promise<boolean | undefined>) => {
      if (reconnectActionPendingRef.current) {
        return;
      }

      reconnectActionPendingRef.current = true;
      setIsReconnectActionPending(true);
      setHasSuccess(false);
      void Promise.resolve()
        .then(runAction)
        .then((didReconnect) => {
          if (didReconnect !== false) {
            markSuccess();
          }
        })
        .catch((error) => {
          const detail = error instanceof Error ? error.message : String(error);
          toast.error(`Could not reconnect to the relay. ${detail}`);
        })
        .finally(() => {
          reconnectActionPendingRef.current = false;
          setIsReconnectActionPending(false);
        });
    },
    [markSuccess],
  );

  const handleReconnectRelay = React.useCallback(() => {
    runConnectivityAction(reconnect);
  }, [reconnect, runConnectivityAction]);

  if (dismissedErrorMessage === message) {
    return null;
  }

  return (
    <div className="fixed bottom-4 left-4 z-50 w-[calc(100vw-2rem)] text-left sm:bottom-6 sm:left-6 sm:w-[22rem]">
      <SidebarRelayConnectionCompactCard
        actionTestId="onboarding-reconnect-relay"
        isActionDisabled={isActionPending}
        isConnected={hasSuccess}
        isReconnectPending={isActionPending}
        onDismiss={() => setDismissedErrorMessage(message)}
        onReconnect={handleReconnectRelay}
        surface="secondary"
        testId="onboarding-relay-reconnect-card"
      />
    </div>
  );
}

function ErrorBanner({
  isSaving,
  message,
}: {
  isSaving: boolean;
  message: string | null;
}) {
  if (!message) {
    return null;
  }

  if (isRelayUnreachableError(message)) {
    return (
      <OnboardingRelayConnectionErrorCard
        isSaving={isSaving}
        key={message}
        message={message}
      />
    );
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
        <ErrorBanner isSaving={isSaving} message={saveRecovery.errorMessage} />
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
