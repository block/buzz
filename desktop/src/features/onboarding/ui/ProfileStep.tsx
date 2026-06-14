import * as React from "react";
import { toast } from "sonner";

import {
  SidebarBlockAccessRefreshCompactCard,
  SidebarBlockVpnOffCompactCard,
  SidebarRelayConnectionCompactCard,
} from "@/features/sidebar/ui/SidebarRelayConnectionCard";
import { connectWarpVpn, refreshWarpAccess } from "@/shared/api/warp";
import { useReconnectRelay } from "@/shared/api/useReconnectRelay";
import { cn } from "@/shared/lib/cn";
import {
  isRelayUnreachableError,
  relayErrorDetail,
} from "@/shared/lib/relayError";
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
  relayUrl?: string | null;
  transitionEffect?: OnboardingTransitionEffect;
  state: ProfileStepState;
};

type OnboardingConnectivityAction =
  | "connect-vpn"
  | "reconnect-relay"
  | "refresh-access";
type OnboardingRelayCardVariant =
  | "connect-vpn"
  | "reconnect-relay"
  | "refresh-access";

const ONBOARDING_CONNECTIVITY_SUCCESS_AUTO_DISMISS_MS = 2_500;

function isBlockRelayUrl(relayUrl: string | null | undefined) {
  if (!relayUrl) {
    return false;
  }

  try {
    const url = new URL(
      relayUrl.replace("ws://", "http://").replace("wss://", "https://"),
    );
    const host = url.hostname.toLowerCase();
    return (
      host === "block.xyz" ||
      host.endsWith(".block.xyz") ||
      host === "sqprod.co" ||
      host.endsWith(".sqprod.co") ||
      host === "squareup.com" ||
      host.endsWith(".squareup.com")
    );
  } catch {
    return false;
  }
}

function shouldRefreshVpnAccess(errorMessage: string) {
  const detail = relayErrorDetail(errorMessage).toLowerCase();
  return (
    detail.includes("cloudflare") ||
    detail.includes("access") ||
    detail.includes("sign-in") ||
    detail.includes("re-authenticate") ||
    detail.includes("reauth") ||
    detail.includes("proxy")
  );
}

function resolveOnboardingRelayCardVariant(
  errorMessage: string,
  relayUrl: string | null | undefined,
): OnboardingRelayCardVariant {
  if (shouldRefreshVpnAccess(errorMessage)) {
    return "refresh-access";
  }

  if (isBlockRelayUrl(relayUrl)) {
    return "connect-vpn";
  }

  return "reconnect-relay";
}

function OnboardingRelayConnectionErrorCard({
  message,
  relayUrl,
}: {
  message: string;
  relayUrl?: string | null;
}) {
  const { isPending: isReconnectPending, reconnect } = useReconnectRelay();
  const [dismissedErrorMessage, setDismissedErrorMessage] = React.useState<
    string | null
  >(null);
  const [connectivityAction, setConnectivityAction] =
    React.useState<OnboardingConnectivityAction | null>(null);
  const [successAction, setSuccessAction] =
    React.useState<OnboardingConnectivityAction | null>(null);
  const connectivityActionRef =
    React.useRef<OnboardingConnectivityAction | null>(null);
  const successTimeoutRef = React.useRef<number | null>(null);
  const cardVariant = resolveOnboardingRelayCardVariant(message, relayUrl);
  const isActionPending = connectivityAction !== null || isReconnectPending;

  React.useEffect(() => {
    return () => {
      if (successTimeoutRef.current !== null) {
        window.clearTimeout(successTimeoutRef.current);
      }
    };
  }, []);

  const markSuccess = React.useCallback(
    (action: OnboardingConnectivityAction) => {
      setSuccessAction(action);
      if (successTimeoutRef.current !== null) {
        window.clearTimeout(successTimeoutRef.current);
      }
      successTimeoutRef.current = window.setTimeout(() => {
        successTimeoutRef.current = null;
        setDismissedErrorMessage(message);
      }, ONBOARDING_CONNECTIVITY_SUCCESS_AUTO_DISMISS_MS);
    },
    [message],
  );

  const runConnectivityAction = React.useCallback(
    (
      action: OnboardingConnectivityAction,
      runAction: () => Promise<boolean | undefined>,
    ) => {
      if (connectivityActionRef.current !== null) {
        return;
      }

      connectivityActionRef.current = action;
      setConnectivityAction(action);
      setSuccessAction(null);
      void Promise.resolve()
        .then(runAction)
        .then((didReconnect) => {
          if (didReconnect !== false) {
            markSuccess(action);
          }
        })
        .catch((error) => {
          const detail = error instanceof Error ? error.message : String(error);
          const label =
            action === "refresh-access"
              ? "Could not refresh VPN access."
              : action === "connect-vpn"
                ? "Could not turn on VPN."
                : "Could not reconnect to the relay.";
          toast.error(`${label} ${detail}`);
        })
        .finally(() => {
          connectivityActionRef.current = null;
          setConnectivityAction(null);
        });
    },
    [markSuccess],
  );

  const handleConnectWarpVpn = React.useCallback(() => {
    runConnectivityAction("connect-vpn", async () => {
      await connectWarpVpn();
      return reconnect();
    });
  }, [reconnect, runConnectivityAction]);

  const handleReconnectRelay = React.useCallback(() => {
    runConnectivityAction("reconnect-relay", reconnect);
  }, [reconnect, runConnectivityAction]);

  const handleRefreshWarpAccess = React.useCallback(() => {
    runConnectivityAction("refresh-access", async () => {
      await refreshWarpAccess();
      return reconnect();
    });
  }, [reconnect, runConnectivityAction]);

  if (dismissedErrorMessage === message) {
    return null;
  }

  return (
    <div className="fixed bottom-4 left-4 z-50 w-[calc(100vw-2rem)] text-left sm:bottom-6 sm:left-6 sm:w-[22rem]">
      {cardVariant === "refresh-access" ? (
        <SidebarBlockAccessRefreshCompactCard
          actionTestId="onboarding-refresh-vpn-access"
          isActionDisabled={isActionPending}
          isActionPending={connectivityAction === "refresh-access"}
          isActionSuccess={successAction === "refresh-access"}
          onAction={handleRefreshWarpAccess}
          onDismiss={() => setDismissedErrorMessage(message)}
          surface="secondary"
          testId="onboarding-vpn-access-refresh-card"
        />
      ) : cardVariant === "connect-vpn" ? (
        <SidebarBlockVpnOffCompactCard
          actionTestId="onboarding-connect-vpn"
          isActionDisabled={isActionPending}
          isActionPending={connectivityAction === "connect-vpn"}
          isActionSuccess={successAction === "connect-vpn"}
          onAction={handleConnectWarpVpn}
          onDismiss={() => setDismissedErrorMessage(message)}
          surface="secondary"
          testId="onboarding-vpn-off-card"
        />
      ) : (
        <SidebarRelayConnectionCompactCard
          actionTestId="onboarding-reconnect-relay"
          isActionDisabled={isActionPending}
          isConnected={successAction === "reconnect-relay"}
          isReconnectPending={
            connectivityAction === "reconnect-relay" || isReconnectPending
          }
          onDismiss={() => setDismissedErrorMessage(message)}
          onReconnect={handleReconnectRelay}
          surface="secondary"
          testId="onboarding-relay-reconnect-card"
        />
      )}
    </div>
  );
}

function ErrorBanner({
  message,
  relayUrl,
}: {
  message: string | null;
  relayUrl?: string | null;
}) {
  if (!message) {
    return null;
  }

  if (isRelayUnreachableError(message)) {
    return (
      <OnboardingRelayConnectionErrorCard
        key={message}
        message={message}
        relayUrl={relayUrl}
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
  relayUrl,
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
        <ErrorBanner message={saveRecovery.errorMessage} relayUrl={relayUrl} />
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
            <Spinner aria-label="Saving profile" className="h-4 w-4" />
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
