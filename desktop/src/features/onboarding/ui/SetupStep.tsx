import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Info } from "lucide-react";

import {
  useAcpAuthMethodsQuery,
  useAcpRuntimesQueryForced,
  useConnectAcpRuntimeMutation,
  useInstallAcpRuntimeMutation,
} from "@/features/agents/hooks";
import { useInstallOutputLine } from "@/features/agents/lib/useInstallOutputLine";
import type { AcpAuthMethod, AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { getInstallErrorMessage } from "@/shared/lib/installError";
import { Button } from "@/shared/ui/button";
import { FlappingBee } from "@/shared/ui/buzz-logo/FlappingBee";
import {
  getReadyOnboardingRuntimes,
  getVisibleOnboardingRuntimes,
  runtimeIsReadyForOnboarding,
} from "./onboardingRuntimeSelection";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { RuntimeErrorTooltip } from "./RuntimeErrorTooltip";
import { OnboardingFooter } from "./OnboardingFooter";
import {
  SetupRuntimeAction,
  SetupRuntimeCardPresentation,
  SetupRuntimeCheckingStatus,
  SetupRuntimeInstallingStatus,
  SetupRuntimeReadyStatus,
} from "./SetupRuntimeCard";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import type { SetupStepActions, SetupStepState } from "./types";

type SetupStepProps = {
  actions: SetupStepActions;
  direction: OnboardingTransitionDirection;
  onReadyRuntimeIdsChange: (runtimeIds: readonly string[]) => void;
};

type SetupStepContentProps = SetupStepProps & {
  renderRuntimeCard?: (runtime: AcpRuntimeCatalogEntry) => React.ReactNode;
  state: SetupStepState;
};

type InstallResultState = {
  error: string | null;
  success: boolean;
};

type InstallResultsState = Record<string, InstallResultState>;

function useSetupStepState(): SetupStepState {
  const runtimesQuery = useAcpRuntimesQueryForced();
  const items = runtimesQuery.data ?? [];
  const isChecking = runtimesQuery.isFetching;
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

function RuntimeStatus({
  installError,
  isInstalling,
  onInstall,
  runtime,
}: {
  installError: string | null;
  isInstalling: boolean;
  onInstall: () => void;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const methodsQuery = useAcpAuthMethodsQuery(runtime.id, {
    enabled:
      runtime.availability === "available" &&
      runtime.authStatus.status === "logged_out",
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
  const shouldSignIn =
    runtime.availability === "available" &&
    runtime.authStatus.status === "logged_out";

  if (shouldSignIn) {
    return (
      <div className="flex flex-col items-center gap-1.5">
        <SetupRuntimeAction
          ariaLabel={`Sign in to ${runtime.label}`}
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
          testId={`onboarding-runtime-instructions-${runtime.id}`}
        >
          {isWaitingForSignIn
            ? "CHECKING…"
            : didSignInCheckTimeOut
              ? "CHECK AGAIN"
              : "SIGN IN"}
        </SetupRuntimeAction>
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
    return <SetupRuntimeInstallingStatus runtime={runtime} />;
  }

  if (runtimeIsReadyForOnboarding(runtime)) {
    // Cached readiness must not read as freshly confirmed while a warm forced
    // probe is revalidating (or has rejected) over it. `runtimesQuery` shares
    // the surface owner's forced-query state, so its fetching/error flags track
    // the in-flight recheck. Pending → a visible CHECKING… state; a warm
    // rejection → a recheck affordance (never an unqualified READY). On success
    // both clear and READY returns. Next stays gated by isChecking/errorMessage
    // in SetupStepContent, so this only governs the per-card claim.
    if (runtimesQuery.isFetching) {
      return <SetupRuntimeCheckingStatus runtime={runtime} />;
    }
    if (runtimesQuery.isError) {
      return (
        <SetupRuntimeAction
          ariaLabel={`Check ${runtime.label} again`}
          onClick={() => void runtimesQuery.forceRefresh()}
          testId={`onboarding-runtime-recheck-${runtime.id}`}
        >
          CHECK AGAIN
        </SetupRuntimeAction>
      );
    }
    return <SetupRuntimeReadyStatus runtime={runtime} />;
  }

  if (
    runtime.availability === "available" &&
    runtime.authStatus.status === "unknown"
  ) {
    return (
      <SetupRuntimeAction
        ariaLabel={`Check ${runtime.label} again`}
        disabled={runtimesQuery.isFetching}
        onClick={() => void runtimesQuery.forceRefresh()}
      >
        {runtimesQuery.isFetching ? "CHECKING…" : "CHECK AGAIN"}
      </SetupRuntimeAction>
    );
  }

  const installLabel = installError ? "RETRY INSTALL" : "INSTALL";
  if (runtime.canAutoInstall) {
    return (
      <SetupRuntimeAction
        ariaLabel={`${installError ? "Retry installing" : "Install"} ${runtime.label}`}
        onClick={onInstall}
        testId={`onboarding-runtime-install-${runtime.id}`}
      >
        {installLabel}
      </SetupRuntimeAction>
    );
  }

  return (
    <SetupRuntimeAction
      ariaLabel={`View ${runtime.label} install instructions`}
      onClick={() => void openUrl(runtime.installInstructionsUrl)}
      testId={`onboarding-runtime-instructions-${runtime.id}`}
    >
      INSTALL
    </SetupRuntimeAction>
  );
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
  installResults,
  onInstallResultsChange,
  runtime,
}: {
  installResults: InstallResultsState;
  onInstallResultsChange: React.Dispatch<
    React.SetStateAction<InstallResultsState>
  >;
  runtime: AcpRuntimeCatalogEntry;
}) {
  // Each card owns its own mutation instance so concurrent installs on
  // different cards each track their own isPending state and callbacks
  // independently (react-query v5 per-mutate callbacks only fire for the
  // latest mutate() call on a shared instance, silently dropping earlier ones).
  const installMutation = useInstallAcpRuntimeMutation();
  const installError = installResults[runtime.id]?.error ?? null;
  const isInstalling = installMutation.isPending;
  const installOutputLine = useInstallOutputLine(runtime.id, isInstalling);

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

  return (
    <SetupRuntimeCardPresentation
      footer={
        installError ? (
          <RuntimeErrorTooltip
            className="absolute inset-x-3 bottom-2 flex min-w-0 items-center justify-center gap-1.5 overflow-hidden whitespace-nowrap text-xs leading-4 text-destructive"
            detail={installError}
            label="Installation failed"
            showIcon
            testId={`onboarding-runtime-error-${runtime.id}`}
          />
        ) : (
          <RuntimeAuthError runtime={runtime} />
        )
      }
      installError={installError}
      installOutputLine={installOutputLine}
      isInstalling={isInstalling}
      runtime={runtime}
      status={
        <RuntimeStatus
          installError={installError}
          isInstalling={isInstalling}
          onInstall={handleInstall}
          runtime={runtime}
        />
      }
    />
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
  installResults,
  navigateToAgentSettings,
  onInstallResultsChange,
  renderRuntimeCard,
  runtimeProviders,
}: {
  installResults: InstallResultsState;
  navigateToAgentSettings?: () => void;
  onInstallResultsChange: React.Dispatch<
    React.SetStateAction<InstallResultsState>
  >;
  renderRuntimeCard?: (runtime: AcpRuntimeCatalogEntry) => React.ReactNode;
  runtimeProviders: SetupStepState["runtimeProviders"];
}) {
  const { errorMessage, isChecking, items } = runtimeProviders;
  const orderedItems = getVisibleOnboardingRuntimes(items);

  return (
    <section className="flex min-h-full w-full flex-col items-center">
      <div className="w-full max-w-[820px] text-center">
        <h1 className="text-title font-normal text-foreground">
          Set up your agent harnesses
        </h1>
        <p className="mx-auto mt-3 max-w-[760px] text-sm leading-6 text-foreground/90">
          Buzz checks for command-line harnesses on this machine. Install the
          CLI or sign in to at least one to continue.
        </p>
      </div>

      <div className="flex w-full flex-1 flex-col items-center justify-center gap-8 py-10">
        {orderedItems.length > 0 ? (
          <div className="grid min-w-0 w-full max-w-[1200px] grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-4">
            {orderedItems.map((runtime) =>
              renderRuntimeCard ? (
                <React.Fragment key={runtime.id}>
                  {renderRuntimeCard(runtime)}
                </React.Fragment>
              ) : (
                <RuntimeCard
                  installResults={installResults}
                  key={runtime.id}
                  onInstallResultsChange={onInstallResultsChange}
                  runtime={runtime}
                />
              ),
            )}
          </div>
        ) : isChecking ? (
          <RuntimeProvidersLoadingState />
        ) : errorMessage ? null : (
          <p
            className="max-w-[560px] rounded-2xl bg-white/70 px-6 py-6 text-sm text-muted-foreground"
            data-testid="onboarding-acp-empty"
          >
            No supported command-line harnesses were detected yet. Install a
            supported CLI, then check again.
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

        <p className="mx-auto flex max-w-[440px] items-start justify-center gap-1.5 text-center text-xs leading-5 text-[var(--buzz-onboarding-backup-ink)]">
          <Info aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>
            More harnesses (Cursor, Grok, Amp&hellip;){" "}
            {navigateToAgentSettings ? (
              <button
                className="underline underline-offset-2 hover:text-foreground"
                data-testid="onboarding-setup-more-harnesses"
                onClick={navigateToAgentSettings}
                type="button"
              >
                Settings → Agents
              </button>
            ) : (
              <span>Settings → Agents</span>
            )}{" "}
            after setup.
          </span>
        </p>
      </div>
    </section>
  );
}

export function SetupStepContent({
  actions,
  direction,
  onReadyRuntimeIdsChange,
  renderRuntimeCard,
  state,
}: SetupStepContentProps) {
  const { runtimeProviders } = state;
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
    onReadyRuntimeIdsChange(readyRuntimeIds);
  }, [onReadyRuntimeIdsChange, readyRuntimeIdsKey]);

  return (
    <OnboardingSlideTransition
      className="flex min-h-full w-full flex-col items-center"
      data-testid="onboarding-page-2"
      direction={direction}
      transitionKey={`setup-${direction}`}
    >
      <RuntimeProvidersSection
        installResults={installResults}
        navigateToAgentSettings={actions.navigateToAgentSettings}
        onInstallResultsChange={setInstallResults}
        renderRuntimeCard={renderRuntimeCard}
        runtimeProviders={runtimeProviders}
      />

      <OnboardingFooter>
        <Button
          className={`${ONBOARDING_PRIMARY_CTA_CLASS} text-sm`}
          data-testid="onboarding-setup-next"
          disabled={
            readyRuntimeIds.length === 0 ||
            runtimeProviders.isChecking ||
            !!runtimeProviders.errorMessage
          }
          onClick={() => actions.next(readyRuntimeIds)}
          type="button"
        >
          Next
        </Button>
        <Button
          className="h-9 whitespace-nowrap rounded-full px-6 text-sm hover:bg-foreground/10"
          data-testid="onboarding-setup-skip"
          onClick={() => actions.next([])}
          type="button"
          variant="ghost"
        >
          Skip for now
        </Button>
      </OnboardingFooter>
    </OnboardingSlideTransition>
  );
}

export function SetupStep({
  actions,
  direction,
  onReadyRuntimeIdsChange,
}: SetupStepProps) {
  const state = useSetupStepState();
  return (
    <SetupStepContent
      actions={actions}
      direction={direction}
      onReadyRuntimeIdsChange={onReadyRuntimeIdsChange}
      state={state}
    />
  );
}
