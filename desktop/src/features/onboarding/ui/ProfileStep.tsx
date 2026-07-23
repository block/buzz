import * as React from "react";
import { toast } from "sonner";

import { SidebarRelayConnectionCompactCard } from "@/features/sidebar/ui/SidebarRelayConnectionCard";
import { useRelayConnection } from "@/shared/api/useRelayConnection";
import { useReconnectRelay } from "@/shared/api/useReconnectRelay";
import { cn } from "@/shared/lib/cn";
import { isRelayUnreachableError } from "@/shared/lib/relayError";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";
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
  usesExistingIdentity?: boolean;
};

const ONBOARDING_CONNECTIVITY_SUCCESS_AUTO_DISMISS_MS = 2_500;

function OnboardingRelayConnectionErrorCard({
  isSaving,
  message,
}: {
  isSaving: boolean;
  message: string;
}) {
  const {
    isPending: isReconnectPending,
    isWaitingOnReconnectHook,
    reconnect,
  } = useReconnectRelay();
  // Track whether a reconnect attempt was ever initiated from this card so we
  // don't call markSuccess() on a "connected" state that pre-dates any click.
  const hadActiveReconnectRef = React.useRef(false);
  const relayConnectionState = useRelayConnection();
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

  // Observe real relay connection state. When this card initiated a reconnect
  // attempt (phase-3 path: reconnect() returns false) and the relay later
  // becomes connected, mark success. Guard: only fire when we actually
  // started a reconnect so a pre-existing connected state doesn't trigger it.
  React.useEffect(() => {
    if (relayConnectionState === "connected" && hadActiveReconnectRef.current) {
      hadActiveReconnectRef.current = false;
      markSuccess();
    }
  }, [relayConnectionState, markSuccess]);

  const runConnectivityAction = React.useCallback(
    (runAction: () => Promise<boolean | undefined>) => {
      if (reconnectActionPendingRef.current) {
        return;
      }

      hadActiveReconnectRef.current = true;
      reconnectActionPendingRef.current = true;
      setIsReconnectActionPending(true);
      setHasSuccess(false);
      void Promise.resolve()
        .then(runAction)
        .then((didReconnect) => {
          if (didReconnect !== false) {
            // Synchronous success (phase 1) — clear the ref and mark success
            // immediately. The connection-state effect won't fire because the
            // ref was just cleared.
            hadActiveReconnectRef.current = false;
            markSuccess();
          }
          // didReconnect === false means phase 3 is active; the
          // connection-state effect will call markSuccess() when the relay
          // becomes connected.
        })
        .catch((error) => {
          hadActiveReconnectRef.current = false;
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
        isWaitingOnReconnectHook={isWaitingOnReconnectHook}
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
  usesExistingIdentity = false,
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
      <div className="w-full max-w-2xl">
        <h1 className="text-title font-normal text-foreground">
          What should we call you?
        </h1>
        <p className="mt-5 text-sm leading-6 text-muted-foreground">
          Pick the name people and agents will see in Buzz. You can change it
          anytime.
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
                Enter your name
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

      <OnboardingFooter>
        <Button
          className={ONBOARDING_PRIMARY_CTA_CLASS}
          data-testid="onboarding-next"
          disabled={!canSubmit}
          onClick={submit}
          type="button"
        >
          {isSaving ? (
            <Spinner aria-label="Saving profile" className="h-4 w-4 border-2" />
          ) : usesExistingIdentity ? (
            "Continue"
          ) : (
            "Create an identity key"
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

        {!usesExistingIdentity ? (
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
        ) : null}

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
      </OnboardingFooter>
    </OnboardingSlideTransition>
  );
}
