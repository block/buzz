import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { AlertTriangle, Check, ExternalLink } from "lucide-react";

import {
  useAcpAuthMethodsQuery,
  useAcpRuntimesQuery,
  useConnectAcpRuntimeMutation,
  useInstallAcpRuntimeMutation,
  useGitBashPrerequisiteQuery,
} from "@/features/agents/hooks";
import { describeResolvedCommand } from "@/features/agents/ui/agentUi";
import type { AcpAuthMethod, AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { getInstallErrorMessage } from "@/shared/lib/installError";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { FlappingBee } from "@/shared/ui/buzz-logo/FlappingBee";
import { Spinner } from "@/shared/ui/spinner";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import {
  runtimeCanAdvanceOnboarding,
  runtimeCanBeSelected,
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
  isSelectionSaving: boolean;
  onSelectedRuntimeIdsChange: (runtimeIds: readonly string[]) => void;
  selectionError: string | null;
  selectedRuntimeIds: readonly string[];
};

type SetupStepContentProps = {
  actions: SetupStepActions;
  direction: OnboardingTransitionDirection;
  isSelectionSaving: boolean;
  onSelectedRuntimeIdsChange: (runtimeIds: readonly string[]) => void;
  selectionError: string | null;
  selectedRuntimeIds: readonly string[];
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

function RuntimeSelectionIndicator({
  runtime,
  selected,
}: {
  runtime: AcpRuntimeCatalogEntry;
  selected: boolean;
}) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "pointer-events-none absolute right-8 top-8 flex h-8 w-8 scale-90 items-center justify-center rounded-full border border-[var(--buzz-welcome-chartreuse)] bg-white/75 opacity-0 transition-[background-color,opacity,transform] duration-200 ease-out group-hover:scale-100 group-hover:opacity-100 group-focus-visible:scale-100 group-focus-visible:opacity-100",
        selected &&
          "scale-100 bg-[var(--buzz-welcome-chartreuse)] opacity-100 group-hover:opacity-100",
      )}
      data-testid={`onboarding-runtime-check-${runtime.id}`}
    >
      <Check
        className={cn(
          "h-4 w-4 text-foreground transition-[opacity,transform] duration-150 ease-out",
          selected ? "scale-100 opacity-100" : "scale-50 opacity-0",
        )}
        data-testid={`onboarding-runtime-checkmark-${runtime.id}`}
        strokeWidth={3}
      />
    </span>
  );
}

function runtimeIsInstalled(runtime: AcpRuntimeCatalogEntry) {
  return runtimeCanBeSelected(runtime) && runtimeCanAdvanceOnboarding(runtime);
}

function useSetupFlashState(setupFlashToken: number) {
  const [isFlashing, setIsFlashing] = React.useState(false);

  React.useEffect(() => {
    if (setupFlashToken === 0) return;

    setIsFlashing(false);
    const frame = window.requestAnimationFrame(() => {
      setIsFlashing(true);
    });
    const timeout = window.setTimeout(() => {
      setIsFlashing(false);
    }, 650);

    return () => {
      window.cancelAnimationFrame(frame);
      window.clearTimeout(timeout);
    };
  }, [setupFlashToken]);

  return isFlashing;
}

function RuntimeStatus({
  installError,
  installSuccess,
  isInstalling,
  onInstall,
  onSelect,
  runtime,
  setupFlashToken,
}: {
  installError: string | null;
  installSuccess: boolean;
  isInstalling: boolean;
  onInstall: () => void;
  onSelect: () => void;
  runtime: AcpRuntimeCatalogEntry;
  setupFlashToken: number;
}) {
  const isSetupFlashing = useSetupFlashState(setupFlashToken);
  const methodsQuery = useAcpAuthMethodsQuery(runtime.id, {
    enabled:
      runtime.availability === "available" &&
      runtime.authStatus.status === "logged_out",
  });
  const connectMutation = useConnectAcpRuntimeMutation();
  const runtimesQuery = useAcpRuntimesQuery();
  const authMethods = getOnboardingAuthMethods(
    runtime,
    methodsQuery.data?.methods ?? [],
  );
  const authMethod = authMethods[0] ?? null;
  const shouldRunAuthSetup =
    runtime.availability === "available" &&
    runtime.authStatus.status === "logged_out";

  if (shouldRunAuthSetup) {
    return (
      <div className="flex flex-col items-center gap-1.5">
        <Button
          aria-label={`Set up ${runtime.label}`}
          className="buzz-onboarding-runtime-setup h-5 rounded-full bg-[var(--buzz-welcome-chartreuse)]/30 px-2.5 font-mono !text-badge font-normal uppercase text-foreground hover:bg-[var(--buzz-welcome-chartreuse)]/40"
          data-setup-flash={isSetupFlashing ? "true" : undefined}
          data-testid={`onboarding-runtime-instructions-${runtime.id}`}
          onClick={(event) => {
            event.stopPropagation();
            onSelect();
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
                onSuccess: () => {
                  if (runtime.id === "claude" || runtime.id === "codex") {
                    onSelect();
                  }
                },
              },
            );
          }}
          type="button"
          variant="ghost"
        >
          SET UP
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

  if (
    runtime.availability === "available" &&
    runtime.authStatus.status === "unknown"
  ) {
    return (
      <Button
        aria-label={`Check ${runtime.label} again`}
        className="buzz-onboarding-runtime-setup h-5 rounded-full bg-[var(--buzz-welcome-chartreuse)]/30 px-2.5 font-mono !text-badge font-normal uppercase text-foreground hover:bg-[var(--buzz-welcome-chartreuse)]/40"
        disabled={runtimesQuery.isFetching}
        onClick={(event) => {
          event.stopPropagation();
          void runtimesQuery.refetch();
        }}
        type="button"
        variant="ghost"
      >
        {runtimesQuery.isFetching ? "CHECKING…" : "CHECK AGAIN"}
      </Button>
    );
  }

  if (installError && runtime.canAutoInstall) {
    return (
      <Button
        aria-label={`Retry ${runtime.label} setup`}
        className="buzz-onboarding-runtime-setup h-5 rounded-full bg-[var(--buzz-welcome-chartreuse)]/30 px-2.5 font-mono !text-badge font-normal uppercase text-foreground hover:bg-[var(--buzz-welcome-chartreuse)]/40"
        data-setup-flash={isSetupFlashing ? "true" : undefined}
        data-testid={`onboarding-runtime-install-${runtime.id}`}
        onClick={(event) => {
          event.stopPropagation();
          onSelect();
          onInstall();
        }}
        type="button"
        variant="ghost"
      >
        SET UP
      </Button>
    );
  }

  if (runtimeIsInstalled(runtime) || installSuccess) {
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <span
            aria-label={`${runtime.label} installed`}
            className="inline-flex h-5 cursor-default items-center rounded-full bg-[#EBEFEF] px-2.5 font-mono text-badge font-normal uppercase text-foreground"
            data-testid={`onboarding-runtime-installed-${runtime.id}`}
            role="img"
          >
            INSTALLED
          </span>
        </TooltipTrigger>
        <TooltipContent
          className="max-w-80 bg-black text-left text-xs text-white shadow-sm"
          side="top"
        >
          {runtimeIsInstalled(runtime) ? (
            <RuntimeDetails runtime={runtime} />
          ) : (
            <p className="text-xs leading-4 text-white">Setup completed.</p>
          )}
        </TooltipContent>
      </Tooltip>
    );
  }

  if (runtime.canAutoInstall) {
    return (
      <Button
        aria-label={`Install ${runtime.label}`}
        className="buzz-onboarding-runtime-setup h-5 rounded-full bg-[var(--buzz-welcome-chartreuse)]/30 px-2.5 font-mono !text-badge font-normal uppercase text-foreground hover:bg-[var(--buzz-welcome-chartreuse)]/40"
        data-setup-flash={isSetupFlashing ? "true" : undefined}
        data-testid={`onboarding-runtime-install-${runtime.id}`}
        onClick={(event) => {
          event.stopPropagation();
          onSelect();
          onInstall();
        }}
        type="button"
        variant="ghost"
      >
        SET UP
      </Button>
    );
  }

  return (
    <Button
      aria-label={`View ${runtime.label} setup instructions`}
      className="buzz-onboarding-runtime-setup h-5 rounded-full bg-[var(--buzz-welcome-chartreuse)]/30 px-2.5 font-mono !text-badge font-normal uppercase text-foreground hover:bg-[var(--buzz-welcome-chartreuse)]/40"
      data-setup-flash={isSetupFlashing ? "true" : undefined}
      data-testid={`onboarding-runtime-instructions-${runtime.id}`}
      onClick={(event) => {
        event.stopPropagation();
        onSelect();
        void openUrl(runtime.installInstructionsUrl);
      }}
      type="button"
      variant="ghost"
    >
      SET UP
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
  installSuccess,
  isInstalling,
  onInstall,
  onSelect,
  onToggle,
  runtime,
  selected,
  setupFlashToken,
}: {
  installError: string | null;
  installSuccess: boolean;
  isInstalling: boolean;
  onInstall: () => void;
  onSelect: () => void;
  onToggle: () => void;
  runtime: AcpRuntimeCatalogEntry;
  selected: boolean;
  setupFlashToken: number;
}) {
  const isAvailable = runtime.availability === "available" || installSuccess;
  const canSelect = runtimeCanBeSelected(runtime);

  return (
    <Card
      aria-checked={selected}
      aria-disabled={!canSelect}
      className={cn(
        "group h-[224px] w-full max-w-[288px] select-none items-center px-3 py-1.5 text-center outline-none transition-[filter] duration-150 ease-out hover:brightness-[0.98] active:brightness-[0.96] focus-visible:ring-2 focus-visible:ring-foreground/40",
        installError && "ring-1 ring-destructive/40",
        selected && "brightness-[0.98] hover:brightness-[0.98]",
        canSelect ? "cursor-pointer" : "cursor-default",
      )}
      data-testid={`onboarding-runtime-${runtime.id}`}
      onClick={canSelect ? onToggle : undefined}
      onKeyDown={(event) => {
        if (event.target !== event.currentTarget) return;
        if (canSelect && (event.key === "Enter" || event.key === " ")) {
          event.preventDefault();
          onToggle();
        }
      }}
      role="checkbox"
      tabIndex={canSelect ? 0 : -1}
      variant="textured"
    >
      <RuntimeSelectionIndicator runtime={runtime} selected={selected} />

      <div className="flex min-w-0 flex-col items-center gap-2.5">
        <div className="flex min-w-0 items-center justify-center gap-3">
          <RuntimeIcon className="h-7 w-7" runtime={runtime} />
          <h2 className="truncate text-sm font-normal leading-5 text-foreground">
            {getRuntimeDisplayLabel(runtime)}
          </h2>
        </div>
        <RuntimeStatus
          installError={installError}
          installSuccess={installSuccess}
          isInstalling={isInstalling}
          onInstall={onInstall}
          onSelect={onSelect}
          runtime={runtime}
          setupFlashToken={setupFlashToken}
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
          detail="Setup couldn’t be completed. Try again."
          label="Setup failed"
          showIcon
          testId={`onboarding-runtime-error-${runtime.id}`}
        />
      ) : (
        <RuntimeAuthError runtime={runtime} />
      )}
    </Card>
  );
}

function GitBashPrerequisiteCard() {
  const query = useGitBashPrerequisiteQuery();
  const prerequisite = query.data;
  if (!prerequisite) return null;

  return (
    <div
      className={cn(
        "mx-auto w-full max-w-[560px] rounded-2xl bg-white/75 p-3 text-left sm:p-4",
        !prerequisite.available && "ring-1 ring-amber-500/40",
      )}
      data-testid="onboarding-git-bash"
    >
      <div className="flex items-center gap-2">
        {prerequisite.available ? (
          <Check className="h-4 w-4 text-primary" />
        ) : (
          <AlertTriangle className="h-4 w-4 text-warning" />
        )}
        <h2 className="text-base font-medium">Git Bash</h2>
        {prerequisite.available ? (
          <Badge
            className="border border-primary/20 bg-primary/10 text-primary"
            variant="outline"
          >
            Installed
          </Badge>
        ) : null}
      </div>
      {prerequisite.available ? (
        <p className="mt-2 break-all font-mono text-xs text-muted-foreground">
          {prerequisite.path}
        </p>
      ) : (
        <>
          <p className="mt-2 text-sm text-muted-foreground">
            Required for buzz-agent shell tools on Windows.
          </p>
          <p className="mt-1 text-xs text-muted-foreground/80">
            {prerequisite.installHint}
          </p>
          <Button
            className="mt-3"
            onClick={() => void openUrl(prerequisite.installInstructionsUrl)}
            size="sm"
            type="button"
            variant="outline"
          >
            <ExternalLink className="h-4 w-4" /> Install Git for Windows
          </Button>
        </>
      )}
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
      <div className="flex flex-col items-center text-foreground/35">
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
  onSelectedRuntimeIdsChange,
  runtimeProviders,
  setupFlashToken,
  setupRequiredRuntimeIds,
  selectedRuntimeIds,
}: {
  installResults: InstallResultsState;
  onInstallResultsChange: React.Dispatch<
    React.SetStateAction<InstallResultsState>
  >;
  onSelectedRuntimeIdsChange: (runtimeIds: readonly string[]) => void;
  runtimeProviders: SetupStepState["runtimeProviders"];
  setupFlashToken: number;
  setupRequiredRuntimeIds: readonly string[];
  selectedRuntimeIds: readonly string[];
}) {
  const { errorMessage, isChecking, items } = runtimeProviders;
  const runtimeOrder = ["claude", "codex", "goose", "buzz-agent"];
  const orderedItems = [...items].sort((left, right) => {
    const leftIndex = runtimeOrder.indexOf(left.id);
    const rightIndex = runtimeOrder.indexOf(right.id);
    return (
      (leftIndex === -1 ? runtimeOrder.length : leftIndex) -
      (rightIndex === -1 ? runtimeOrder.length : rightIndex)
    );
  });
  const installMutation = useInstallAcpRuntimeMutation();
  const selectedRuntimeIdSet = React.useMemo(
    () => new Set(selectedRuntimeIds),
    [selectedRuntimeIds],
  );
  const setupRequiredRuntimeIdSet = React.useMemo(
    () => new Set(setupRequiredRuntimeIds),
    [setupRequiredRuntimeIds],
  );

  function handleRuntimeToggle(runtimeId: string) {
    if (selectedRuntimeIdSet.has(runtimeId)) {
      onSelectedRuntimeIdsChange(
        selectedRuntimeIds.filter((selectedId) => selectedId !== runtimeId),
      );
      return;
    }
    onSelectedRuntimeIdsChange([...selectedRuntimeIds, runtimeId]);
  }

  function handleRuntimeSelect(runtimeId: string) {
    if (selectedRuntimeIdSet.has(runtimeId)) return;
    onSelectedRuntimeIdsChange([...selectedRuntimeIds, runtimeId]);
  }

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
          Use the models that fit the task
        </h1>
        <p className="mx-auto mt-3 max-w-[760px] text-sm leading-6 text-foreground/90">
          <span>
            Connect your model providers here. Each agent can use the one that’s
            best for their work.
          </span>
          <span className="mt-1 block">
            Choose at least one to start using Buzz.
          </span>
        </p>
      </div>

      <div className="flex w-full flex-1 flex-col items-center justify-center gap-8 py-10">
        <GitBashPrerequisiteCard />

        {items.length > 0 ? (
          <fieldset className="grid min-w-0 w-full max-w-[592px] grid-cols-1 gap-4 border-0 p-0 md:grid-cols-2">
            <legend className="sr-only">Agent harnesses</legend>
            {orderedItems.map((runtime) => (
              <RuntimeCard
                installError={installResults[runtime.id]?.error ?? null}
                installSuccess={installResults[runtime.id]?.success ?? false}
                isInstalling={
                  installMutation.isPending &&
                  installMutation.variables === runtime.id
                }
                key={runtime.id}
                onInstall={() => handleInstall(runtime.id)}
                onSelect={() => handleRuntimeSelect(runtime.id)}
                onToggle={() => handleRuntimeToggle(runtime.id)}
                runtime={runtime}
                selected={selectedRuntimeIdSet.has(runtime.id)}
                setupFlashToken={
                  setupRequiredRuntimeIdSet.has(runtime.id)
                    ? setupFlashToken
                    : 0
                }
              />
            ))}
          </fieldset>
        ) : isChecking ? (
          <RuntimeProvidersLoadingState />
        ) : errorMessage ? null : (
          <p
            className="max-w-[560px] rounded-2xl bg-white/70 px-6 py-6 text-sm text-muted-foreground"
            data-testid="onboarding-acp-empty"
          >
            No compatible agent runtimes detected yet. You can finish setup now
            and come back later in Settings &gt; Agents.
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
  isSelectionSaving,
  onSelectedRuntimeIdsChange,
  selectionError,
  selectedRuntimeIds,
  state,
}: SetupStepContentProps) {
  const { runtimeProviders } = state;
  const [installResults, setInstallResults] =
    React.useState<InstallResultsState>({});
  const [setupFlashToken, setSetupFlashToken] = React.useState(0);
  const [setupRequiredHintKey, setSetupRequiredHintKey] = React.useState<
    string | null
  >(null);
  const runtimeById = React.useMemo(
    () =>
      new Map(runtimeProviders.items.map((runtime) => [runtime.id, runtime])),
    [runtimeProviders.items],
  );
  const setupRequiredRuntimeIds = React.useMemo(
    () =>
      selectedRuntimeIds.filter((runtimeId) => {
        const runtime = runtimeById.get(runtimeId);
        if (!runtime) return false;
        return !runtimeCanAdvanceOnboarding(runtime);
      }),
    [runtimeById, selectedRuntimeIds],
  );
  const hasSetupRequiredSelection = setupRequiredRuntimeIds.length > 0;
  const setupRequiredRuntimeIdsKey = setupRequiredRuntimeIds.join("\0");
  const showSetupRequiredHint =
    hasSetupRequiredSelection &&
    setupRequiredHintKey === setupRequiredRuntimeIdsKey;

  React.useEffect(() => {
    if (
      setupRequiredHintKey !== null &&
      setupRequiredHintKey !== setupRequiredRuntimeIdsKey
    ) {
      setSetupRequiredHintKey(null);
    }
  }, [setupRequiredHintKey, setupRequiredRuntimeIdsKey]);

  function handleNext() {
    if (selectedRuntimeIds.length === 0 || isSelectionSaving) return;
    if (hasSetupRequiredSelection) {
      setSetupRequiredHintKey(setupRequiredRuntimeIdsKey);
      setSetupFlashToken((current) => current + 1);
      return;
    }
    actions.next();
  }

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
        onSelectedRuntimeIdsChange={onSelectedRuntimeIdsChange}
        runtimeProviders={runtimeProviders}
        setupFlashToken={setupFlashToken}
        setupRequiredRuntimeIds={setupRequiredRuntimeIds}
        selectedRuntimeIds={selectedRuntimeIds}
      />

      <OnboardingFooter>
        {selectionError ? (
          <p
            className="max-w-sm text-center text-xs text-destructive"
            role="alert"
          >
            {selectionError}
          </p>
        ) : null}
        {hasSetupRequiredSelection && showSetupRequiredHint ? (
          <p
            className="-mb-1 text-center text-xs leading-4 text-foreground/60"
            data-testid="onboarding-setup-next-hint"
          >
            Please finish set up
          </p>
        ) : null}
        <Button
          className={cn(
            ONBOARDING_PRIMARY_CTA_CLASS,
            "text-sm",
            hasSetupRequiredSelection &&
              "cursor-default opacity-50 hover:opacity-50",
          )}
          data-soft-disabled={hasSetupRequiredSelection ? "true" : undefined}
          data-testid="onboarding-setup-next"
          disabled={selectedRuntimeIds.length === 0 || isSelectionSaving}
          onClick={handleNext}
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
  isSelectionSaving,
  onSelectedRuntimeIdsChange,
  selectionError,
  selectedRuntimeIds,
}: SetupStepProps) {
  const state = useSetupStepState();

  return (
    <SetupStepContent
      actions={actions}
      direction={direction}
      isSelectionSaving={isSelectionSaving}
      onSelectedRuntimeIdsChange={onSelectedRuntimeIdsChange}
      selectionError={selectionError}
      selectedRuntimeIds={selectedRuntimeIds}
      state={state}
    />
  );
}
