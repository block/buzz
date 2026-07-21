import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Check } from "lucide-react";

import {
  useAcpAuthMethodsQuery,
  useAcpRuntimesQuery,
  useConnectAcpRuntimeMutation,
  useInstallAcpRuntimeMutation,
} from "@/features/agents/hooks";
import { describeResolvedCommand } from "@/features/agents/ui/agentUi";
import type { AcpAuthMethod, AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { getInstallErrorMessage } from "@/shared/lib/installError";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { FlappingBee } from "@/shared/ui/buzz-logo/FlappingBee";
import { Spinner } from "@/shared/ui/spinner";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import {
  getReadyOnboardingRuntimes,
  getVisibleOnboardingRuntimes,
  runtimeIsReadyForOnboarding,
} from "./onboardingRuntimeSelection";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
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
  onReadyRuntimeIdsChange: (runtimeIds: readonly string[]) => void;
};

type SetupStepContentProps = SetupStepProps & {
  state: SetupStepState;
};

type InstallResultState = {
  error: string | null;
  success: boolean;
};

type InstallResultsState = Record<string, InstallResultState>;

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
  const runtimesQuery = useAcpRuntimesQuery();
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
      void runtimesQuery.refetch();
    }, 2_000);
    const timeout = window.setTimeout(() => {
      setIsWaitingForSignIn(false);
      setDidSignInCheckTimeOut(true);
    }, 120_000);

    return () => {
      window.clearInterval(interval);
      window.clearTimeout(timeout);
    };
  }, [isWaitingForSignIn, runtimesQuery.refetch]);
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
        <Button
          aria-label={`Sign in to ${runtime.label}`}
          className="buzz-onboarding-runtime-setup h-5 rounded-full bg-[var(--buzz-welcome-chartreuse)]/30 px-2.5 font-mono !text-badge font-normal uppercase text-foreground hover:bg-[var(--buzz-welcome-chartreuse)]/40"
          data-testid={`onboarding-runtime-instructions-${runtime.id}`}
          onClick={() => {
            if (didSignInCheckTimeOut) {
              setDidSignInCheckTimeOut(false);
              setIsWaitingForSignIn(true);
              void runtimesQuery.refetch();
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
            ? "CHECKING…"
            : didSignInCheckTimeOut
              ? "CHECK AGAIN"
              : "SIGN IN"}
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
        className="flex h-5 items-center gap-2 rounded-full bg-white/60 px-2.5 font-mono text-badge font-normal uppercase text-foreground"
        role="status"
      >
        <Spinner className="h-3 w-3 border-2 text-foreground" />
        INSTALLING
      </div>
    );
  }

  if (runtimeIsReadyForOnboarding(runtime)) {
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <span
            className="inline-flex h-5 cursor-default items-center rounded-full bg-[#EBEFEF] px-2.5 font-mono text-badge font-normal uppercase text-foreground"
            data-testid={`onboarding-runtime-ready-${runtime.id}`}
          >
            READY
          </span>
        </TooltipTrigger>
        <TooltipContent
          className="max-w-80 bg-black text-left text-xs text-white shadow-sm"
          side="top"
        >
          <RuntimeDetails runtime={runtime} />
        </TooltipContent>
      </Tooltip>
    );
  }

  if (
    runtime.availability === "available" &&
    runtime.authStatus.status === "unknown"
  ) {
    return (
      <Button
        aria-label={`Check ${runtime.label} again`}
        className="buzz-onboarding-runtime-setup h-5 rounded-full bg-[var(--buzz-welcome-chartreuse)]/30 px-2.5 font-mono !text-badge font-normal uppercase text-foreground hover:bg-[var(--buzz-welcome-chartreuse)]/40"
        disabled={runtimesQuery.isFetching}
        onClick={() => void runtimesQuery.refetch()}
        type="button"
        variant="ghost"
      >
        {runtimesQuery.isFetching ? "CHECKING…" : "CHECK AGAIN"}
      </Button>
    );
  }

  const installLabel = installError ? "RETRY INSTALL" : "INSTALL";
  if (runtime.canAutoInstall) {
    return (
      <Button
        aria-label={`${installError ? "Retry installing" : "Install"} ${runtime.label}`}
        className="buzz-onboarding-runtime-setup h-5 rounded-full bg-[var(--buzz-welcome-chartreuse)]/30 px-2.5 font-mono !text-badge font-normal uppercase text-foreground hover:bg-[var(--buzz-welcome-chartreuse)]/40"
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
      className="buzz-onboarding-runtime-setup h-5 rounded-full bg-[var(--buzz-welcome-chartreuse)]/30 px-2.5 font-mono !text-badge font-normal uppercase text-foreground hover:bg-[var(--buzz-welcome-chartreuse)]/40"
      data-testid={`onboarding-runtime-instructions-${runtime.id}`}
      onClick={() => void openUrl(runtime.installInstructionsUrl)}
      type="button"
      variant="ghost"
    >
      INSTALL
    </Button>
  );
}

function RuntimeDetails({ runtime }: { runtime: AcpRuntimeCatalogEntry }) {
  if (
    runtime.availability === "available" &&
    runtime.command &&
    runtime.binaryPath
  ) {
    const description = describeResolvedCommand(
      runtime.command,
      runtime.binaryPath,
    );
    return (
      <>
        <p className="text-xs leading-4 text-white">
          {description.charAt(0).toUpperCase() + description.slice(1)}
        </p>
        {runtime.defaultArgs.length > 0 ? (
          <p className="mt-1 text-xs leading-4 text-white">
            Args:{" "}
            <code className="font-mono">{runtime.defaultArgs.join(", ")}</code>
          </p>
        ) : null}
      </>
    );
  }

  if (runtime.availability === "adapter_missing") {
    return (
      <>
        <p className="text-xs leading-4 text-white">
          CLI detected; ACP adapter missing.
        </p>
        <p className="mt-1 text-xs leading-4 text-white">
          {runtime.installHint}
        </p>
      </>
    );
  }

  if (runtime.availability === "adapter_outdated") {
    return (
      <>
        <p className="text-xs leading-4 text-white">
          ACP adapter detected but outdated — reinstall required.
        </p>
        <p className="mt-1 text-xs leading-4 text-white">
          This updates the machine-global{" "}
          <code className="rounded bg-white/10 px-0.5 font-mono text-xs text-white">
            codex-acp
          </code>{" "}
          adapter. Older Buzz releases using the legacy adapter contract may
          lose community access until{" "}
          <code className="rounded bg-white/10 px-0.5 font-mono text-xs text-white">
            @zed-industries/codex-acp@0.16.0
          </code>{" "}
          is restored.
        </p>
        <p className="mt-1 text-xs leading-4 text-white">
          {runtime.installHint}
        </p>
      </>
    );
  }

  if (runtime.availability === "cli_missing") {
    return (
      <>
        <p className="text-xs leading-4 text-white">
          ACP adapter detected; CLI missing.
        </p>
        <p className="mt-1 text-xs leading-4 text-white">
          {runtime.installHint}
        </p>
      </>
    );
  }

  return (
    <>
      <p className="text-xs leading-4 text-white">Not installed yet.</p>
      <p className="mt-1 text-xs leading-4 text-white">{runtime.installHint}</p>
    </>
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
  if (runtime.availability === "cli_missing") {
    return "ACP adapter detected; CLI missing.";
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
  const isAvailable = runtime.availability === "available";
  const isReady = runtimeIsReadyForOnboarding(runtime);

  return (
    <Card
      className={cn(
        "group h-[224px] w-full max-w-[288px] select-none items-center px-3 py-1.5 text-center",
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
          onInstall={onInstall}
          runtime={runtime}
        />
        {!isAvailable && runtimeDetailText(runtime) ? (
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
          detail="Installation couldn’t be completed. Try again."
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
  onInstallResultsChange,
  runtimeProviders,
}: {
  installResults: InstallResultsState;
  onInstallResultsChange: React.Dispatch<
    React.SetStateAction<InstallResultsState>
  >;
  runtimeProviders: SetupStepState["runtimeProviders"];
}) {
  const { errorMessage, isChecking, items } = runtimeProviders;
  const orderedItems = getVisibleOnboardingRuntimes(items);
  const installMutation = useInstallAcpRuntimeMutation();

  function handleInstall(runtimeId: string) {
    onInstallResultsChange((current) => ({
      ...current,
      [runtimeId]: { error: null, success: false },
    }));

    installMutation.mutate(runtimeId, {
      onSuccess: (result) => {
        onInstallResultsChange((current) => ({
          ...current,
          [runtimeId]: result.success
            ? { error: null, success: true }
            : { error: getInstallErrorMessage(result.steps), success: false },
        }));
      },
      onError: (error) => {
        onInstallResultsChange((current) => ({
          ...current,
          [runtimeId]: {
            error: error instanceof Error ? error.message : "Install failed.",
            success: false,
          },
        }));
      },
    });
  }

  return (
    <section className="flex min-h-full w-full flex-col items-center">
      <div className="w-full max-w-[820px] text-center">
        <h1 className="text-title font-normal text-foreground">
          Set up your agent harnesses
        </h1>
        <p className="mx-auto mt-3 max-w-[760px] text-sm leading-6 text-foreground/90">
          Buzz detected the harnesses available on this machine. Install or sign
          in to at least one to continue.
        </p>
      </div>

      <div className="flex w-full flex-1 flex-col items-center justify-center gap-8 py-10">
        {orderedItems.length > 0 ? (
          <div className="grid min-w-0 w-full max-w-[592px] grid-cols-1 gap-4 md:grid-cols-2">
            {orderedItems.map((runtime) => (
              <RuntimeCard
                installError={installResults[runtime.id]?.error ?? null}
                isInstalling={
                  installMutation.isPending &&
                  installMutation.variables === runtime.id
                }
                key={runtime.id}
                onInstall={() => handleInstall(runtime.id)}
                runtime={runtime}
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
            No supported agent harnesses were detected yet. Install Claude Code
            or Codex, then check again.
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
  onReadyRuntimeIdsChange,
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
        onInstallResultsChange={setInstallResults}
        runtimeProviders={runtimeProviders}
      />

      <OnboardingFooter>
        <Button
          className={`${ONBOARDING_PRIMARY_CTA_CLASS} text-sm`}
          data-testid="onboarding-setup-next"
          disabled={readyRuntimeIds.length === 0}
          onClick={() => actions.next(readyRuntimeIds)}
          type="button"
        >
          Next
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
