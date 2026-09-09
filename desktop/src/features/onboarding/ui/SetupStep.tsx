import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, ChevronRight, ExternalLink } from "lucide-react";

import {
  useAcpAuthMethodsQuery,
  useAcpRuntimesQueryForced,
  useConnectAcpRuntimeMutation,
  useInstallAcpRuntimeMutation,
} from "@/features/agents/hooks";
import { useInstallOutputLine } from "@/features/agents/lib/useInstallOutputLine";
import { describeResolvedCommand } from "@/features/agents/ui/agentUi";
import type { AcpAuthMethod, AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { getInstallErrorMessage } from "@/shared/lib/installError";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { Spinner } from "@/shared/ui/spinner";
import { ConnectionMethodSection } from "./ConnectionMethodSection";
import {
  getReadyOnboardingRuntimes,
  getVisibleOnboardingRuntimes,
  runtimeIsReadyForOnboarding,
} from "./onboardingRuntimeSelection";
import {
  getRuntimesForConnectionMethod,
  type HarnessConnectionMethod,
} from "./harnessConnectionOptions";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { useOnboardingCardLayout } from "./OnboardingCard";
import { RuntimeErrorTooltip } from "./RuntimeErrorTooltip";
import { OnboardingFooter } from "./OnboardingFooter";
import { getRuntimeDisplayLabel, RuntimeIcon } from "./RuntimeIcon";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import type { SetupStepActions, SetupStepState } from "./types";

type SetupStepProps = {
  actions: SetupStepActions;
  direction: OnboardingTransitionDirection;
  initialMethod?: HarnessConnectionMethod | null;
  onInitialListBack?: () => void;
  onBackActionChange?: (backAction: () => void) => void;
  onMethodChange?: (method: HarnessConnectionMethod | null) => void;
  onReadyRuntimeIdsChange: (runtimeIds: readonly string[]) => void;
};

type SetupStepContentProps = SetupStepProps & {
  onRefresh: () => void;
  state: SetupStepState;
};

type InstallResultState = {
  error: string | null;
  success: boolean;
};

type InstallResultsState = Record<string, InstallResultState>;

const SUBSCRIPTION_NAMES: Record<string, string> = {
  amp: "Amp subscription",
  claude: "Claude subscription",
  codex: "ChatGPT subscription",
  cursor: "Cursor subscription",
  devin: "Devin account",
};

function useSetupStepState(): {
  onRefresh: () => void;
  state: SetupStepState;
} {
  const runtimesQuery = useAcpRuntimesQueryForced();
  const items = runtimesQuery.data ?? [];
  const isChecking = runtimesQuery.isFetching;
  const errorMessage =
    runtimesQuery.error instanceof Error ? runtimesQuery.error.message : null;

  return {
    onRefresh: () => void runtimesQuery.forceRefresh(),
    state: {
      runtimeProviders: {
        errorMessage,
        isChecking,
        items,
      },
    },
  };
}

function RuntimeReadinessIndicator({
  runtime,
  ready,
}: {
  runtime: AcpRuntimeCatalogEntry;
  ready: boolean;
}) {
  // Checkmark temporarily hidden; flip to true to restore it.
  const showReadinessCheckmark = false;
  if (!ready || !showReadinessCheckmark) return null;

  return (
    <span
      aria-hidden="true"
      className="pointer-events-none absolute right-8 top-8 flex h-8 w-8 items-center justify-center rounded-full border border-[var(--buzz-welcome-chartreuse)] bg-[var(--buzz-welcome-chartreuse)]"
      data-testid={`onboarding-runtime-check-${runtime.id}`}
    >
      <Check
        className="h-4 w-4 text-foreground"
        data-testid={`onboarding-runtime-checkmark-${runtime.id}`}
        strokeWidth={3}
      />
    </span>
  );
}

function RuntimeStatus({
  installError,
  isInstalling,
  onInstall,
  prominent = false,
  runtime,
}: {
  installError: string | null;
  isInstalling: boolean;
  onInstall: () => void;
  prominent?: boolean;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const shouldSignIn =
    runtime.availability === "available" &&
    runtime.authStatus.status === "logged_out";
  const methodsQuery = useAcpAuthMethodsQuery(runtime.id, {
    enabled: shouldSignIn && prominent,
  });
  const connectMutation = useConnectAcpRuntimeMutation();
  // Child rows share the surface owner's forced query state + refresh callback
  // (`useSetupStepState` owns the single force-on-mount). Each row must not
  // mount its own force effect, or onboarding entry re-runs discovery once per
  // row instead of once for the surface.
  const runtimesQuery = useAcpRuntimesQueryForced({ forceOnMount: false });
  const [isWaitingForSignIn, setIsWaitingForSignIn] = React.useState(false);
  const [didSignInCheckTimeOut, setDidSignInCheckTimeOut] =
    React.useState(false);
  const isReady = runtimeIsReadyForOnboarding(runtime);

  React.useEffect(() => {
    if (!isWaitingForSignIn || !isReady) return;
    setIsWaitingForSignIn(false);
    setDidSignInCheckTimeOut(false);
  }, [isReady, isWaitingForSignIn]);

  React.useEffect(() => {
    if (!isWaitingForSignIn) return;

    const interval = window.setInterval(() => {
      void runtimesQuery.forceRefresh();
    }, 2_000);
    const timeout = window.setTimeout(() => {
      setIsWaitingForSignIn(false);
      setDidSignInCheckTimeOut(true);
    }, 120_000);

    return () => {
      window.clearInterval(interval);
      window.clearTimeout(timeout);
    };
  }, [isWaitingForSignIn, runtimesQuery.forceRefresh]);
  const authMethods = getOnboardingAuthMethods(
    runtime,
    methodsQuery.data?.methods ?? [],
  );
  const authMethod = authMethods[0] ?? null;

  if (shouldSignIn) {
    if (!prominent) {
      return (
        <span
          className="inline-flex h-5 cursor-default items-center rounded-md bg-[#EBEFEF] px-2.5 text-xs font-medium text-foreground/70"
          data-testid={`onboarding-runtime-sign-in-required-${runtime.id}`}
        >
          Sign in required
        </span>
      );
    }

    return (
      <div className="flex flex-col items-center gap-1.5">
        <Button
          aria-label={`Sign in to ${runtime.label}`}
          className="h-7 shrink-0 rounded-full bg-foreground px-3 text-xs font-normal text-background shadow-none hover:bg-foreground/85 hover:text-background"
          data-testid={`onboarding-runtime-instructions-${runtime.id}`}
          onClick={() => {
            if (didSignInCheckTimeOut) {
              setDidSignInCheckTimeOut(false);
              setIsWaitingForSignIn(true);
              void runtimesQuery.forceRefresh();
              return;
            }
            if (!authMethod) {
              void methodsQuery.refetch();
              return;
            }
            connectMutation.mutate(
              {
                methodId: authMethod.id,
                runtimeId: runtime.id,
              },
              {
                onSuccess: () => setIsWaitingForSignIn(true),
              },
            );
          }}
          type="button"
          variant="ghost"
        >
          {isWaitingForSignIn
            ? "Checking…"
            : didSignInCheckTimeOut
              ? "Check again"
              : "Sign in"}
        </Button>
        {methodsQuery.error instanceof Error ? (
          <RuntimeErrorTooltip
            className="absolute inset-x-3 bottom-2 truncate text-xs leading-4 text-destructive"
            detail="Couldn’t load sign-in options."
            label="Sign-in unavailable"
          />
        ) : null}
        {connectMutation.error instanceof Error ? (
          <RuntimeErrorTooltip
            className="absolute inset-x-3 bottom-2 truncate text-xs leading-4 text-destructive"
            detail="Couldn’t start sign-in. Try again."
            label="Sign-in failed"
          />
        ) : null}
      </div>
    );
  }

  if (isInstalling) {
    return (
      <div
        aria-label={`Installing ${runtime.label}`}
        className="flex h-5 items-center gap-2 rounded-md bg-white/60 px-2.5 text-xs font-medium text-foreground/70"
        role="status"
      >
        <Spinner className="h-3 w-3 border-2 text-foreground" />
        Installing
      </div>
    );
  }

  if (runtimeIsReadyForOnboarding(runtime)) {
    // Installed harnesses are already grouped above the "Not installed"
    // section, so a second Ready label only repeats the list structure.
    if (runtimesQuery.isError) {
      return (
        <Button
          aria-label={`Check ${runtime.label} again`}
          className="buzz-onboarding-runtime-setup h-5 rounded-md bg-[var(--buzz-welcome-chartreuse)]/30 px-2.5 text-xs font-medium text-foreground hover:bg-[var(--buzz-welcome-chartreuse)]/40"
          data-testid={`onboarding-runtime-recheck-${runtime.id}`}
          onClick={() => void runtimesQuery.forceRefresh()}
          type="button"
          variant="ghost"
        >
          Check again
        </Button>
      );
    }
    return null;
  }

  if (
    runtime.availability === "available" &&
    runtime.authStatus.status === "unknown"
  ) {
    return (
      <Button
        aria-label={`Check ${runtime.label} again`}
        className="buzz-onboarding-runtime-setup h-5 rounded-md bg-[var(--buzz-welcome-chartreuse)]/30 px-2.5 text-xs font-medium text-foreground hover:bg-[var(--buzz-welcome-chartreuse)]/40"
        disabled={runtimesQuery.isFetching}
        onClick={() => void runtimesQuery.forceRefresh()}
        type="button"
        variant="ghost"
      >
        {runtimesQuery.isFetching ? "Checking…" : "Check again"}
      </Button>
    );
  }

  const installLabel = installError ? "Retry install" : "Install";
  if (runtime.canAutoInstall) {
    return (
      <Button
        aria-label={`${installError ? "Retry installing" : "Install"} ${runtime.label}`}
        className="buzz-onboarding-runtime-setup h-5 rounded-md bg-[var(--buzz-welcome-chartreuse)]/30 px-2.5 text-xs font-medium text-foreground hover:bg-[var(--buzz-welcome-chartreuse)]/40"
        data-testid={`onboarding-runtime-install-${runtime.id}`}
        onClick={onInstall}
        type="button"
        variant="ghost"
      >
        {installLabel}
      </Button>
    );
  }

  return (
    <Button
      aria-label={`View ${runtime.label} install instructions`}
      className="buzz-onboarding-runtime-setup h-5 rounded-md bg-[var(--buzz-welcome-chartreuse)]/30 px-2.5 text-xs font-medium text-foreground hover:bg-[var(--buzz-welcome-chartreuse)]/40"
      data-testid={`onboarding-runtime-instructions-${runtime.id}`}
      onClick={() => void openUrl(runtime.installInstructionsUrl)}
      type="button"
      variant="ghost"
    >
      Install
    </Button>
  );
}

function runtimeDetailText(runtime: AcpRuntimeCatalogEntry): string {
  if (
    runtime.availability === "available" &&
    runtime.command &&
    runtime.binaryPath
  ) {
    const description = describeResolvedCommand(
      runtime.command,
      runtime.binaryPath,
    );
    return description.charAt(0).toUpperCase() + description.slice(1);
  }
  if (runtime.availability === "adapter_missing") {
    return "CLI detected; ACP adapter missing.";
  }
  if (runtime.availability === "adapter_outdated") {
    return "ACP adapter detected but outdated — reinstall required.";
  }
  if (
    runtime.availability === "cli_missing" ||
    runtime.availability === "not_installed"
  ) {
    return "CLI not detected.";
  }
  return "";
}

function isSupportedOnboardingAuthMethod(
  runtime: AcpRuntimeCatalogEntry,
  method: AcpAuthMethod,
) {
  if (runtime.id !== "codex") return true;
  return !/api[-_ ]?key/i.test(`${method.id} ${method.name}`);
}

function isPreferredClaudeAuthMethod(method: AcpAuthMethod) {
  const haystack = [
    method.id,
    method.name,
    method.description ?? "",
    method.command.join(" "),
    method.args.join(" "),
  ]
    .join(" ")
    .toLowerCase();
  return (
    haystack.includes("claudeai") ||
    haystack.includes("claude ai") ||
    haystack.includes("claude.ai") ||
    haystack.includes("subscription")
  );
}

function getOnboardingAuthMethods(
  runtime: AcpRuntimeCatalogEntry,
  methods: AcpAuthMethod[],
) {
  const supported = methods.filter((method) =>
    isSupportedOnboardingAuthMethod(runtime, method),
  );

  if (runtime.id === "claude") {
    const preferred =
      supported.find(isPreferredClaudeAuthMethod) ?? supported[0];
    return preferred ? [preferred] : [];
  }

  if (runtime.id === "codex") {
    return supported.slice(0, 1);
  }

  return supported;
}

function RuntimeAuthError({ runtime }: { runtime: AcpRuntimeCatalogEntry }) {
  if (runtime.authStatus.status === "config_invalid") {
    return (
      <RuntimeErrorTooltip
        className="absolute inset-x-3 bottom-2 truncate text-xs leading-4 text-destructive"
        detail="Check this runtime’s configuration and try again."
        label="Configuration invalid"
      />
    );
  }
  if (
    runtime.availability === "available" &&
    runtime.authStatus.status === "unknown"
  ) {
    return (
      <RuntimeErrorTooltip
        className="absolute inset-x-3 bottom-2 truncate text-xs leading-4 text-destructive"
        detail="Couldn’t verify authentication."
        label="Status unavailable"
      />
    );
  }
  return null;
}

function RuntimeCard({
  detailCopy,
  installResults,
  isRecommended = false,
  onOpenDetails,
  onInstallResultsChange,
  runtime,
}: {
  detailCopy?: { description: string; title: string };
  installResults: InstallResultsState;
  isRecommended?: boolean;
  onOpenDetails?: () => void;
  onInstallResultsChange: React.Dispatch<
    React.SetStateAction<InstallResultsState>
  >;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const cardLayout = useOnboardingCardLayout();
  // Each card owns its own mutation instance so concurrent installs on
  // different cards each track their own isPending state and callbacks
  // independently (react-query v5 per-mutate callbacks only fire for the
  // latest mutate() call on a shared instance, silently dropping earlier ones).
  const installMutation = useInstallAcpRuntimeMutation();
  const installError = installResults[runtime.id]?.error ?? null;
  const isInstalling = installMutation.isPending;
  const installOutputLine = useInstallOutputLine(runtime.id, isInstalling);
  const isAvailable = runtime.availability === "available";
  const isReady = runtimeIsReadyForOnboarding(runtime);
  const runtimeIdentity = (
    <>
      <span
        className="flex size-10 shrink-0 items-center justify-start"
        data-testid={`onboarding-runtime-icon-${runtime.id}`}
      >
        <RuntimeIcon className="size-9" runtime={runtime} />
      </span>
      <span className="min-w-0 flex-1">
        <span
          className="block truncate font-medium"
          data-testid={`onboarding-runtime-title-${runtime.id}`}
        >
          {detailCopy?.title ?? getRuntimeDisplayLabel(runtime)}
        </span>
        {detailCopy ? (
          <span className="mt-0.5 block text-xs leading-5 text-foreground/70">
            {detailCopy.description}
          </span>
        ) : null}
      </span>
    </>
  );

  function handleInstall() {
    onInstallResultsChange((current) => ({
      ...current,
      [runtime.id]: { error: null, success: false },
    }));

    installMutation.mutate(runtime.id, {
      onSuccess: (result) => {
        onInstallResultsChange((current) => ({
          ...current,
          [runtime.id]: result.success
            ? { error: null, success: true }
            : {
                error: getInstallErrorMessage(result),
                success: false,
              },
        }));
      },
      onError: (error) => {
        onInstallResultsChange((current) => ({
          ...current,
          [runtime.id]: {
            error: error instanceof Error ? error.message : "Install failed.",
            success: false,
          },
        }));
      },
    });
  }

  if (cardLayout) {
    return (
      <div
        className={cn(
          "group relative flex min-h-14 w-full items-center rounded-xl px-2 py-2 text-left text-sm text-foreground",
          detailCopy && "bg-[#e2e2e2]/30 px-4 py-4",
          onOpenDetails &&
            "transition-colors duration-150 ease-out hover:bg-foreground/[0.04] motion-reduce:transition-none",
          installError && "ring-1 ring-destructive/40",
        )}
        data-ready={isReady ? "true" : "false"}
        data-testid={`onboarding-runtime-${runtime.id}`}
      >
        {onOpenDetails ? (
          <button
            aria-label={`Open ${getRuntimeDisplayLabel(runtime)} setup`}
            className="absolute inset-0 rounded-xl focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-foreground/20"
            data-testid={`onboarding-runtime-details-${runtime.id}`}
            onClick={onOpenDetails}
            type="button"
          >
            <span className="sr-only">
              Open {getRuntimeDisplayLabel(runtime)} setup
            </span>
          </button>
        ) : null}
        <div className="pointer-events-none relative z-10 flex min-w-0 w-full items-center gap-3">
          {runtimeIdentity}
          <span className="flex shrink-0 items-center gap-2">
            {isRecommended ? (
              <span
                className="inline-flex h-5 shrink-0 items-center rounded-md bg-muted px-2 text-xs font-medium text-muted-foreground transition-colors duration-150 ease-out group-hover:bg-background group-hover:text-foreground motion-reduce:transition-none"
                data-testid={`onboarding-runtime-recommended-${runtime.id}`}
              >
                Recommended
              </span>
            ) : null}
            {isAvailable && !isReady ? (
              <span
                className={cn(
                  "flex min-w-20 shrink-0 justify-end",
                  runtime.authStatus.status === "logged_out" && !detailCopy
                    ? "pointer-events-none"
                    : "pointer-events-auto",
                )}
              >
                <RuntimeStatus
                  installError={installError}
                  isInstalling={isInstalling}
                  onInstall={handleInstall}
                  prominent={Boolean(detailCopy)}
                  runtime={runtime}
                />
              </span>
            ) : null}
          </span>
          {onOpenDetails ? (
            <span className="flex size-5 shrink-0 items-center justify-end">
              <ChevronRight
                aria-hidden
                className="size-4 text-muted-foreground transition-colors duration-150 ease-out group-hover:text-foreground motion-reduce:transition-none"
                data-testid={`onboarding-runtime-chevron-${runtime.id}`}
              />
            </span>
          ) : null}
        </div>
        {installError ? (
          <RuntimeErrorTooltip
            className="absolute bottom-0 left-16 right-28 z-20 truncate text-xs leading-4 text-destructive"
            detail={installError}
            label="Installation failed"
            showIcon
            testId={`onboarding-runtime-error-${runtime.id}`}
          />
        ) : (
          <RuntimeAuthError runtime={runtime} />
        )}
      </div>
    );
  }

  return (
    <Card
      className={cn(
        "group w-full select-none items-center px-3 py-1.5 text-center",
        cardLayout ? "h-40 max-w-none" : "h-[224px] max-w-[288px]",
        installError && "ring-1 ring-destructive/40",
        isReady && "brightness-[0.98]",
      )}
      data-ready={isReady ? "true" : "false"}
      data-testid={`onboarding-runtime-${runtime.id}`}
      variant="textured"
    >
      <RuntimeReadinessIndicator ready={isReady} runtime={runtime} />

      <div className="flex min-w-0 flex-col items-center gap-2.5">
        <div className="flex min-w-0 items-center justify-center gap-3">
          <RuntimeIcon className="h-7 w-7" runtime={runtime} />
          <h2 className="truncate text-sm font-normal leading-5 text-foreground">
            {getRuntimeDisplayLabel(runtime)}
          </h2>
        </div>
        <RuntimeStatus
          installError={installError}
          isInstalling={isInstalling}
          onInstall={handleInstall}
          runtime={runtime}
        />
        {isInstalling && installOutputLine ? (
          // Takes the detail text's slot rather than adding a row: the card is
          // fixed-height, and during an install the live line is the more
          // useful of the two.
          <p
            aria-live="polite"
            className="max-w-[13rem] truncate font-mono text-2xs leading-4 text-muted-foreground"
            data-testid={`onboarding-runtime-install-output-${runtime.id}`}
          >
            {installOutputLine}
          </p>
        ) : !isAvailable && runtimeDetailText(runtime) ? (
          <p
            aria-hidden={installError ? "true" : undefined}
            className={cn(
              "max-w-[13rem] text-2xs leading-4 text-muted-foreground",
              installError && "invisible",
            )}
          >
            {runtimeDetailText(runtime)}
          </p>
        ) : null}
      </div>
      {installError ? (
        <RuntimeErrorTooltip
          className="absolute inset-x-3 bottom-2 flex min-w-0 items-center justify-center gap-1.5 overflow-hidden whitespace-nowrap text-xs leading-4 text-destructive"
          detail={installError}
          label="Installation failed"
          showIcon
          testId={`onboarding-runtime-error-${runtime.id}`}
        />
      ) : (
        <RuntimeAuthError runtime={runtime} />
      )}
    </Card>
  );
}

function RuntimeProvidersLoadingState() {
  return (
    <div
      aria-live="polite"
      className="flex w-full items-center justify-start gap-2 py-6 text-sm text-muted-foreground"
      data-testid="onboarding-runtime-loading"
      role="status"
    >
      <Spinner aria-hidden className="h-4 w-4 border-2" />
      <span>Loading providers…</span>
    </div>
  );
}

function RuntimeProvidersSection({
  installResults,
  method,
  onInstallResultsChange,
  onOpenDetails,
  runtimeProviders,
}: {
  installResults: InstallResultsState;
  method: HarnessConnectionMethod;
  onInstallResultsChange: React.Dispatch<
    React.SetStateAction<InstallResultsState>
  >;
  onOpenDetails: (runtimeId: string) => void;
  runtimeProviders: SetupStepState["runtimeProviders"];
}) {
  const cardLayout = useOnboardingCardLayout();
  const { errorMessage, isChecking, items } = runtimeProviders;
  const scrollRef = React.useRef<HTMLDivElement | null>(null);
  const [canScrollUp, setCanScrollUp] = React.useState(false);
  const [canScrollDown, setCanScrollDown] = React.useState(false);
  const orderedItems = React.useMemo(() => {
    const visible = getRuntimesForConnectionMethod(
      getVisibleOnboardingRuntimes(items),
      method,
    );
    if (method !== "api") return visible;

    const group = (runtime: AcpRuntimeCatalogEntry) => {
      if (runtime.id === "buzz-agent") return 0;
      if (runtime.id === "goose") return 1;
      return runtime.availability === "available" ? 2 : 3;
    };
    return [...visible].sort((left, right) => group(left) - group(right));
  }, [items, method]);
  const updateScrollEdges = React.useCallback(() => {
    const element = scrollRef.current;
    if (!element) return;
    setCanScrollUp(element.scrollTop > 1);
    setCanScrollDown(
      element.scrollTop + element.clientHeight < element.scrollHeight - 1,
    );
  }, []);

  React.useEffect(() => {
    updateScrollEdges();
    const element = scrollRef.current;
    if (!element) return;
    const observer = new ResizeObserver(updateScrollEdges);
    observer.observe(element);
    return () => observer.disconnect();
  }, [updateScrollEdges]);

  return (
    <section
      className={cn(
        "flex min-h-full w-full flex-col",
        cardLayout ? "items-stretch" : "items-center",
      )}
    >
      <div
        className={cn(
          "w-full",
          cardLayout ? "text-left" : "max-w-[820px] text-center",
        )}
      >
        <h1 className="text-title font-normal text-foreground">
          {method === "subscription"
            ? "Continue with an AI subscription"
            : "Choose a harness"}
        </h1>
        <p
          className={cn(
            "max-w-[760px] leading-6 text-foreground/90",
            cardLayout ? "mt-2 text-base" : "mx-auto mt-3 text-sm",
          )}
        >
          {method === "subscription"
            ? "Subscriptions connect through a compatible harness, like Claude Code or Codex. Choose yours to sign in."
            : "Choose how your agents will connect to AI providers. You can change this at any time."}
        </p>
      </div>

      <div
        className={cn(
          "flex min-h-0 w-full flex-1 flex-col items-center",
          cardLayout ? "gap-3 pt-6" : "gap-8 py-10",
        )}
      >
        {orderedItems.length > 0 ? (
          <div
            className={cn(
              "relative min-h-0 min-w-0",
              cardLayout
                ? "-mx-2 w-[calc(100%+1rem)] flex-1"
                : "w-full max-w-[1200px]",
            )}
          >
            {cardLayout ? (
              <>
                <div
                  aria-hidden="true"
                  className={cn(
                    "pointer-events-none absolute inset-x-0 top-0 z-10 h-4 transition-opacity duration-150 motion-reduce:transition-none",
                    canScrollUp ? "opacity-100" : "opacity-0",
                  )}
                  style={{
                    background:
                      "linear-gradient(to bottom, white, rgb(255 255 255 / 0))",
                  }}
                />
                <div
                  aria-hidden="true"
                  className={cn(
                    "pointer-events-none absolute inset-x-0 bottom-0 z-10 h-4 transition-opacity duration-150 motion-reduce:transition-none",
                    canScrollDown ? "opacity-100" : "opacity-0",
                  )}
                  style={{
                    background:
                      "linear-gradient(to top, white, rgb(255 255 255 / 0))",
                  }}
                />
              </>
            ) : null}
            <div
              className={cn(
                "min-h-0 min-w-0",
                cardLayout
                  ? "h-full space-y-1 overflow-y-auto overscroll-contain pr-1"
                  : "grid w-full grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-4",
              )}
              data-testid="onboarding-harness-list"
              onScroll={cardLayout ? updateScrollEdges : undefined}
              ref={cardLayout ? scrollRef : undefined}
            >
              {orderedItems.map((runtime, index) => {
                const previousRuntime = orderedItems[index - 1];
                const startsNotInstalledSection =
                  runtime.availability !== "available" &&
                  (previousRuntime === undefined ||
                    previousRuntime.availability === "available");

                return (
                  <React.Fragment key={runtime.id}>
                    {startsNotInstalledSection ? (
                      <p className="px-2 pb-1 pt-4 text-xs font-medium text-muted-foreground">
                        Not installed
                      </p>
                    ) : null}
                    <RuntimeCard
                      installResults={installResults}
                      isRecommended={
                        method === "api" && runtime.id === "buzz-agent"
                      }
                      onInstallResultsChange={onInstallResultsChange}
                      onOpenDetails={() => onOpenDetails(runtime.id)}
                      runtime={runtime}
                    />
                  </React.Fragment>
                );
              })}
            </div>
          </div>
        ) : isChecking ? (
          <RuntimeProvidersLoadingState />
        ) : errorMessage ? null : (
          <p
            className="max-w-[560px] rounded-2xl bg-white/70 px-6 py-6 text-sm text-muted-foreground"
            data-testid="onboarding-acp-empty"
          >
            No supported harnesses are available for this connection method.
          </p>
        )}

        {errorMessage ? (
          <p
            className="max-w-[560px] rounded-2xl bg-destructive/10 px-6 py-3 text-sm text-destructive"
            data-testid="onboarding-setup-error"
          >
            {errorMessage}
          </p>
        ) : null}
      </div>
    </section>
  );
}

function RuntimeSetupGuide({
  installResults,
  method,
  onInstallResultsChange,
  onRefresh,
  runtime,
}: {
  installResults: InstallResultsState;
  method: HarnessConnectionMethod;
  onInstallResultsChange: React.Dispatch<
    React.SetStateAction<InstallResultsState>
  >;
  onRefresh: () => void;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const label = getRuntimeDisplayLabel(runtime);
  const available = runtime.availability === "available";
  const subscriptionDetail =
    method === "subscription"
      ? {
          description: `Buzz will open a sign-in window for ${label}.`,
          title: SUBSCRIPTION_NAMES[runtime.id] ?? label,
        }
      : undefined;

  if (!available) {
    return (
      <section
        className="flex min-h-0 w-full flex-1 flex-col"
        data-testid="onboarding-harness-setup-guide"
      >
        <h1 className="text-title font-normal text-foreground">
          Set up {label}
        </h1>
        <p className="mt-2 w-full text-base leading-6 text-foreground/80">
          Follow the setup guide to install {label}. When you’re done, come back
          and check again.
        </p>

        <div className="mt-8 flex min-h-0 flex-1 flex-col">
          <div
            className="flex items-center gap-3 rounded-xl bg-[#e2e2e2]/30 px-4 py-4"
            data-testid="onboarding-harness-setup-guide-card"
          >
            <RuntimeIcon className="size-9 shrink-0" runtime={runtime} />
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium text-foreground">{label}</p>
              <p className="mt-0.5 text-xs leading-5 text-foreground/70">
                {label} is not detected on this computer.
              </p>
            </div>
            <Button
              className="ml-auto h-7 shrink-0 rounded-full bg-foreground px-3 text-xs text-background shadow-none hover:bg-foreground/85"
              data-testid="onboarding-harness-open-setup-guide"
              onClick={() => void openUrl(runtime.installInstructionsUrl)}
              size="xs"
              type="button"
            >
              Open guide
              <ExternalLink aria-hidden />
            </Button>
          </div>
        </div>

        <OnboardingFooter>
          <Button
            className={ONBOARDING_PRIMARY_CTA_CLASS}
            data-testid="onboarding-harness-check-again"
            onClick={onRefresh}
            type="button"
          >
            Check again
          </Button>
        </OnboardingFooter>
      </section>
    );
  }

  return (
    <section
      className="flex min-h-0 w-full flex-1 flex-col"
      data-testid="onboarding-harness-setup-guide"
    >
      <div className="shrink-0">
        <h1 className="text-title font-normal text-foreground">
          Connect {label}
        </h1>
        <p className="mt-2 w-full text-base leading-6 text-foreground/80">
          Sign in to connect {label}. You can change this anytime.
        </p>
      </div>

      <div className="mt-8 flex min-h-0 flex-1 flex-col">
        <div
          className={cn(
            !subscriptionDetail && "rounded-xl bg-[#e2e2e2]/30 px-2 py-2",
          )}
        >
          <RuntimeCard
            detailCopy={subscriptionDetail}
            installResults={installResults}
            onInstallResultsChange={onInstallResultsChange}
            runtime={runtime}
          />
        </div>
      </div>

      <OnboardingFooter>
        <Button
          className={ONBOARDING_PRIMARY_CTA_CLASS}
          data-testid="onboarding-harness-check-again"
          onClick={onRefresh}
          type="button"
        >
          Check again
        </Button>
      </OnboardingFooter>
    </section>
  );
}

function SetupStepContent({
  actions,
  direction,
  initialMethod = null,
  onInitialListBack,
  onBackActionChange,
  onMethodChange,
  onRefresh,
  onReadyRuntimeIdsChange,
  state,
}: SetupStepContentProps) {
  const cardLayout = useOnboardingCardLayout();
  const { runtimeProviders } = state;
  const [stage, setStage] = React.useState<"method" | "list" | "detail">(
    initialMethod ? "list" : "method",
  );
  const [method, setMethod] = React.useState<HarnessConnectionMethod | null>(
    initialMethod,
  );
  const [selectedRuntimeId, setSelectedRuntimeId] = React.useState<
    string | null
  >(null);
  const [localDirection, setLocalDirection] =
    React.useState<OnboardingTransitionDirection>(direction);
  const [installResults, setInstallResults] =
    React.useState<InstallResultsState>({});
  const readyRuntimeIds = React.useMemo(
    () =>
      getReadyOnboardingRuntimes(runtimeProviders.items).map(
        (runtime) => runtime.id,
      ),
    [runtimeProviders.items],
  );
  const readyRuntimeIdsKey = readyRuntimeIds.join("\0");
  // The key prevents catalog object refreshes from creating an effect loop
  // when the detected ready IDs have not changed.
  // biome-ignore lint/correctness/useExhaustiveDependencies: keyed by ID content
  React.useEffect(() => {
    if (runtimeProviders.isChecking && runtimeProviders.items.length === 0) {
      return;
    }
    onReadyRuntimeIdsChange(readyRuntimeIds);
  }, [
    onReadyRuntimeIdsChange,
    readyRuntimeIdsKey,
    runtimeProviders.isChecking,
    runtimeProviders.items.length,
  ]);

  const selectedRuntime = runtimeProviders.items.find(
    (runtime) => runtime.id === selectedRuntimeId,
  );
  const selectedRuntimeIsReady = selectedRuntime
    ? runtimeIsReadyForOnboarding(selectedRuntime)
    : false;
  const actionsRef = React.useRef(actions);
  actionsRef.current = actions;
  const navigateBack = React.useCallback(() => {
    setLocalDirection("backward");
    if (stage === "detail") {
      setStage("list");
      setSelectedRuntimeId(null);
      return;
    }
    if (stage === "list") {
      if (onInitialListBack) {
        onInitialListBack();
        return;
      }
      setStage("method");
      setMethod(null);
      onMethodChange?.(null);
      return;
    }
    actionsRef.current.back();
  }, [onInitialListBack, onMethodChange, stage]);

  React.useEffect(() => {
    onBackActionChange?.(navigateBack);
  }, [navigateBack, onBackActionChange]);

  React.useLayoutEffect(() => {
    if (stage !== "detail" || !selectedRuntime || !selectedRuntimeIsReady) {
      return;
    }
    setLocalDirection("forward");
    actionsRef.current.next([selectedRuntime.id]);
  }, [selectedRuntime, selectedRuntimeIsReady, stage]);

  function chooseMethod(nextMethod: HarnessConnectionMethod) {
    setMethod(nextMethod);
    onMethodChange?.(nextMethod);
    setLocalDirection("forward");

    if (nextMethod === "api") {
      const buzzRuntime = runtimeProviders.items.find(
        (runtime) => runtime.id === "buzz-agent",
      );
      if (buzzRuntime) {
        if (runtimeIsReadyForOnboarding(buzzRuntime)) {
          actions.next([buzzRuntime.id], "method");
          return;
        }
        setSelectedRuntimeId(buzzRuntime.id);
        setStage("detail");
        return;
      }
    }

    setStage("list");
  }

  function openRuntime(runtimeId: string) {
    setLocalDirection("forward");
    const runtime = runtimeProviders.items.find(
      (item) => item.id === runtimeId,
    );
    if (runtime && runtimeIsReadyForOnboarding(runtime)) {
      actions.next([runtime.id]);
      return;
    }
    setSelectedRuntimeId(runtimeId);
    setStage("detail");
  }

  const transitionKey =
    stage === "method"
      ? "setup-method"
      : stage === "list"
        ? `setup-list-${method ?? "none"}`
        : `setup-detail-${selectedRuntimeId ?? "none"}`;
  return (
    <OnboardingSlideTransition
      className={cn(
        "flex min-h-full w-full flex-col",
        cardLayout ? "items-stretch" : "items-center",
      )}
      data-testid="onboarding-page-2"
      direction={localDirection}
      transitionKey={transitionKey}
    >
      {stage === "method" ? (
        <>
          <ConnectionMethodSection onSelect={chooseMethod} />
          <OnboardingFooter>
            <Button
              className="h-9 whitespace-nowrap rounded-full px-6 text-sm hover:bg-foreground/10"
              data-testid="onboarding-setup-skip"
              onClick={() => actions.next([])}
              type="button"
              variant="ghost"
            >
              Set up later
            </Button>
          </OnboardingFooter>
        </>
      ) : stage === "list" && method ? (
        <>
          <RuntimeProvidersSection
            installResults={installResults}
            method={method}
            onInstallResultsChange={setInstallResults}
            onOpenDetails={openRuntime}
            runtimeProviders={runtimeProviders}
          />
          <OnboardingFooter>
            <Button
              className="h-9 whitespace-nowrap rounded-full px-6 text-sm hover:bg-foreground/10"
              data-testid="onboarding-setup-skip"
              onClick={() => actions.next([])}
              type="button"
              variant="ghost"
            >
              Set up later
            </Button>
          </OnboardingFooter>
        </>
      ) : selectedRuntime && !selectedRuntimeIsReady ? (
        <RuntimeSetupGuide
          installResults={installResults}
          method={method ?? "api"}
          onInstallResultsChange={setInstallResults}
          onRefresh={onRefresh}
          runtime={selectedRuntime}
        />
      ) : null}
    </OnboardingSlideTransition>
  );
}

export function SetupStep({
  actions,
  direction,
  initialMethod,
  onInitialListBack,
  onBackActionChange,
  onMethodChange,
  onReadyRuntimeIdsChange,
}: SetupStepProps) {
  const { onRefresh, state } = useSetupStepState();
  return (
    <SetupStepContent
      actions={actions}
      direction={direction}
      initialMethod={initialMethod}
      onInitialListBack={onInitialListBack}
      onBackActionChange={onBackActionChange}
      onMethodChange={onMethodChange}
      onRefresh={onRefresh}
      onReadyRuntimeIdsChange={onReadyRuntimeIdsChange}
      state={state}
    />
  );
}
