import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  AlertTriangle,
  Check,
  ExternalLink,
  TerminalSquare,
} from "lucide-react";

import {
  useAcpAuthMethodsQuery,
  useAcpRuntimesQuery,
  useConnectAcpRuntimeMutation,
  useInstallAcpRuntimeMutation,
  useGitBashPrerequisiteQuery,
} from "@/features/agents/hooks";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { getInstallErrorMessage } from "@/shared/lib/installError";
import { cn } from "@/shared/lib/cn";
import { useTheme } from "@/shared/theme/ThemeProvider";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { BuzzMark } from "@/shared/ui/buzz-logo/BuzzMark";
import { FlappingBee } from "@/shared/ui/buzz-logo/FlappingBee";
import { Spinner } from "@/shared/ui/spinner";
import { runtimeCanBeSelected } from "./onboardingRuntimeSelection";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";
import chatgptLogoUrl from "../assets/harness-logos/chatgpt.png";
import claudeLogoUrl from "../assets/harness-logos/claude.png";
import geminiLogoUrl from "../assets/harness-logos/gemini.png";
import gooseLogoUrl from "../assets/harness-logos/goose.png";
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

const RUNTIME_LOGOS: Record<string, string> = {
  chatgpt: chatgptLogoUrl,
  claude: claudeLogoUrl,
  "claude-code": claudeLogoUrl,
  codex: chatgptLogoUrl,
  gemini: geminiLogoUrl,
  goose: gooseLogoUrl,
  openai: chatgptLogoUrl,
};

function isBuzzRuntime(runtime: AcpRuntimeCatalogEntry): boolean {
  const runtimeId = runtime.id.trim().toLowerCase();
  const runtimeLabel = runtime.label.trim().toLowerCase();
  return runtimeId === "buzz-agent" || runtimeLabel === "buzz";
}

function getRuntimeLogoUrl(runtime: AcpRuntimeCatalogEntry): string | null {
  const runtimeId = runtime.id.trim().toLowerCase();
  const runtimeLabel = runtime.label.trim().toLowerCase();
  return (
    RUNTIME_LOGOS[runtimeId] ??
    (runtimeLabel.includes("claude")
      ? claudeLogoUrl
      : runtimeLabel.includes("goose")
        ? gooseLogoUrl
        : runtimeLabel.includes("gemini")
          ? geminiLogoUrl
          : runtimeLabel.includes("codex") || runtimeLabel.includes("chatgpt")
            ? chatgptLogoUrl
            : null)
  );
}

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

function RuntimeIcon({
  className = "h-8 w-8",
  runtime,
}: {
  className?: string;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const [imageFailed, setImageFailed] = React.useState(false);
  const { isDark } = useTheme();
  const runtimeLogoUrl = getRuntimeLogoUrl(runtime);
  const imageUrl = runtimeLogoUrl ?? runtime.avatarUrl;
  const shouldForceForegroundColor = !runtimeLogoUrl && runtime.id === "goose";

  if (isBuzzRuntime(runtime)) {
    return <BuzzMark className="h-8 w-12 text-foreground" />;
  }

  if (imageUrl && !imageFailed) {
    return (
      <img
        alt=""
        className={cn(
          "rounded-md object-contain",
          className,
          shouldForceForegroundColor &&
            (isDark ? "brightness-0 invert" : "brightness-0"),
        )}
        onError={() => setImageFailed(true)}
        src={imageUrl}
      />
    );
  }

  return (
    <TerminalSquare
      className={cn(className, "text-foreground")}
      strokeWidth={1.25}
    />
  );
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
        "buzz-onboarding-runtime-check pointer-events-none absolute right-3 top-3 flex h-8 w-8 scale-90 items-center justify-center rounded-full border border-[var(--buzz-welcome-chartreuse)] bg-white/75 opacity-0 transition-[background-color,opacity,transform] duration-200 ease-out group-hover:scale-100 group-hover:opacity-100 group-focus-visible:scale-100 group-focus-visible:opacity-100",
        selected &&
          "scale-100 bg-[var(--buzz-welcome-chartreuse)] opacity-100 group-hover:opacity-100",
      )}
      data-testid={`onboarding-runtime-check-${runtime.id}`}
    >
      <Check
        className={cn(
          "buzz-onboarding-runtime-checkmark h-4 w-4 text-foreground transition-[opacity,transform] duration-150 ease-out",
          selected ? "scale-100 opacity-100" : "scale-50 opacity-0",
        )}
        data-testid={`onboarding-runtime-checkmark-${runtime.id}`}
        strokeWidth={3}
      />
    </span>
  );
}

function runtimeIsReadyForOnboarding(
  runtime: AcpRuntimeCatalogEntry,
  installSuccess: boolean,
) {
  if (installSuccess) return true;
  if (runtime.availability !== "available") return false;
  return (
    runtime.authStatus.status === "logged_in" ||
    runtime.authStatus.status === "not_applicable"
  );
}

function runtimeNeedsInstallSetup(
  runtime: AcpRuntimeCatalogEntry,
  installSuccess: boolean,
) {
  return runtime.availability !== "available" && !installSuccess;
}

function RuntimeSetupPill({
  installError,
  installSuccess,
  isInstalling,
  runtime,
}: {
  installError: string | null;
  installSuccess: boolean;
  isInstalling: boolean;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const isReady = runtimeIsReadyForOnboarding(runtime, installSuccess);

  if (isReady) {
    return (
      <span
        className="buzz-onboarding-runtime-pill inline-flex h-5 items-center rounded-full bg-[var(--buzz-welcome-chartreuse)]/30 px-2.5 font-mono font-normal text-foreground"
        data-testid={`onboarding-runtime-installed-${runtime.id}`}
      >
        INSTALLED
      </span>
    );
  }

  if (isInstalling) {
    return (
      <div
        aria-label={`Installing ${runtime.label}`}
        className="buzz-onboarding-runtime-pill flex h-5 items-center gap-2 rounded-full bg-white/60 px-2.5 font-mono font-normal text-foreground"
        role="status"
      >
        <Spinner className="h-3 w-3 border-2 text-foreground" />
        INSTALLING
      </div>
    );
  }

  return (
    <span
      className={cn(
        "buzz-onboarding-runtime-pill inline-flex h-5 items-center rounded-full bg-[var(--buzz-welcome-chartreuse)]/30 px-2.5 font-mono font-normal text-foreground",
        installError && "bg-destructive/10 text-destructive",
      )}
      data-runtime-setup
      data-testid={
        runtime.canAutoInstall
          ? `onboarding-runtime-install-${runtime.id}`
          : `onboarding-runtime-instructions-${runtime.id}`
      }
    >
      {installError ? "RETRY" : "SET UP"}
    </span>
  );
}

function isSupportedOnboardingAuthMethod(
  runtime: AcpRuntimeCatalogEntry,
  method: { id: string; name: string },
) {
  if (runtime.id !== "codex") return true;
  return !/api[-_ ]?key/i.test(`${method.id} ${method.name}`);
}

function onboardingAuthMethodLabel(
  runtime: AcpRuntimeCatalogEntry,
  method: { name: string },
) {
  if (runtime.id === "codex") return "Log in";
  return method.name || "Sign in";
}

function RuntimeAuthActions({
  onAuthenticated,
  runtime,
}: {
  onAuthenticated: () => void;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const runtimesQuery = useAcpRuntimesQuery();
  const methodsQuery = useAcpAuthMethodsQuery(runtime.id, {
    enabled:
      runtime.availability === "available" &&
      runtime.authStatus.status === "logged_out",
  });
  const connectMutation = useConnectAcpRuntimeMutation();

  if (runtime.authStatus.status === "config_invalid") {
    return (
      <p className="mt-1 max-w-[15rem] text-center text-2xs leading-4 text-destructive">
        {runtime.authStatus.diagnostic}
      </p>
    );
  }

  if (runtime.authStatus.status === "unknown") {
    return (
      <div className="mt-1 flex flex-col items-center gap-1.5">
        <span className="text-2xs text-muted-foreground">
          Couldn’t verify authentication.
        </span>
        <Button
          className="h-6 rounded-full px-2 text-2xs"
          disabled={runtimesQuery.isFetching}
          onClick={(event) => {
            event.stopPropagation();
            void runtimesQuery.refetch();
          }}
          type="button"
          variant="ghost"
        >
          {runtimesQuery.isFetching ? "Checking…" : "Check again"}
        </Button>
      </div>
    );
  }

  if (runtime.authStatus.status !== "logged_out") return null;

  const methods = (methodsQuery.data?.methods ?? []).filter((method) =>
    isSupportedOnboardingAuthMethod(runtime, method),
  );

  return (
    <div className="mt-1 flex flex-col items-center gap-1.5">
      {methodsQuery.isLoading ? (
        <span className="text-2xs text-muted-foreground">Loading sign-in…</span>
      ) : methods.length > 0 ? (
        methods.map((method) => (
          <Button
            className="h-6 rounded-full px-2 text-2xs"
            disabled={connectMutation.isPending}
            key={method.id}
            onClick={(event) => {
              event.stopPropagation();
              connectMutation.mutate(
                {
                  methodId: method.id,
                  runtimeId: runtime.id,
                },
                {
                  onSuccess: () => {
                    if (runtime.id === "claude" || runtime.id === "codex") {
                      onAuthenticated();
                    }
                  },
                },
              );
            }}
            type="button"
            variant="outline"
          >
            {connectMutation.isPending
              ? "Opening…"
              : onboardingAuthMethodLabel(runtime, method)}
          </Button>
        ))
      ) : (
        <span className="text-2xs text-muted-foreground">
          {methodsQuery.error instanceof Error
            ? "Couldn’t load sign-in options."
            : runtime.loginHint || "Sign in from the CLI."}
        </span>
      )}
      {connectMutation.error instanceof Error ? (
        <span className="text-2xs text-destructive">
          {connectMutation.error.message}
        </span>
      ) : null}
      <Button
        className="h-6 rounded-full px-2 text-2xs"
        disabled={runtimesQuery.isFetching}
        onClick={(event) => {
          event.stopPropagation();
          void Promise.all([runtimesQuery.refetch(), methodsQuery.refetch()]);
        }}
        type="button"
        variant="ghost"
      >
        {runtimesQuery.isFetching ? "Checking…" : "Check again"}
      </Button>
    </div>
  );
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
}: {
  installError: string | null;
  installSuccess: boolean;
  isInstalling: boolean;
  onInstall: () => void;
  onSelect: () => void;
  onToggle: () => void;
  runtime: AcpRuntimeCatalogEntry;
  selected: boolean;
}) {
  const needsInstallSetup = runtimeNeedsInstallSetup(runtime, installSuccess);
  const isReady = runtimeIsReadyForOnboarding(runtime, installSuccess);
  const canSelect = runtimeCanBeSelected(runtime);
  const canSetup = needsInstallSetup && !isInstalling;
  const showStatusPill =
    selected ||
    !isReady ||
    installSuccess ||
    Boolean(installError) ||
    isInstalling;

  function handleSetup() {
    if (runtime.canAutoInstall) {
      onInstall();
      return;
    }
    void openUrl(runtime.installInstructionsUrl);
  }

  return (
    // biome-ignore lint/a11y/useSemanticElements: Cannot use <input> because this card contains nested setup and auth buttons.
    <div
      aria-checked={selected}
      aria-disabled={!canSelect}
      className={cn(
        "group relative flex aspect-[288/132] min-h-[96px] w-full select-none flex-col items-center justify-center rounded-2xl border-0 bg-white/75 px-3 py-1.5 text-center outline-none transition-colors duration-150 ease-out hover:bg-white/80 active:bg-white/90 focus-visible:ring-2 focus-visible:ring-foreground/40",
        selected && "bg-white/90 hover:bg-white/90",
        installError && "ring-1 ring-destructive/40",
        canSelect ? "cursor-pointer" : "cursor-default",
        canSetup && "cursor-pointer",
      )}
      data-testid={`onboarding-runtime-${runtime.id}`}
      onClick={(event) => {
        const target =
          event.target instanceof HTMLElement ? event.target : null;
        if (canSetup && target?.closest("[data-runtime-setup]")) {
          handleSetup();
          return;
        }
        if (canSelect) onToggle();
      }}
      onKeyDown={(event) => {
        if (event.repeat || (event.key !== "Enter" && event.key !== " ")) {
          return;
        }
        if (canSetup) {
          event.preventDefault();
          handleSetup();
          return;
        }
        if (canSelect) {
          event.preventDefault();
          onToggle();
        }
      }}
      role="checkbox"
      tabIndex={canSelect || canSetup ? 0 : -1}
    >
      <RuntimeSelectionIndicator runtime={runtime} selected={selected} />

      <div className="flex flex-col items-center gap-2">
        <div className="flex items-center justify-center gap-3">
          <RuntimeIcon className="h-8 w-8" runtime={runtime} />
          {!isBuzzRuntime(runtime) ? (
            <h2 className="text-sm font-normal leading-5 text-foreground">
              {runtime.label}
            </h2>
          ) : null}
        </div>
        {showStatusPill ? (
          <RuntimeSetupPill
            installError={installError}
            installSuccess={installSuccess}
            isInstalling={isInstalling}
            runtime={runtime}
          />
        ) : null}
        {selected && installError ? (
          <p className="max-w-[22rem] text-2xs leading-4 text-destructive">
            {installError}
          </p>
        ) : null}
        <RuntimeAuthActions onAuthenticated={onSelect} runtime={runtime} />
      </div>
    </div>
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
  onSelectedRuntimeIdsChange,
  runtimeProviders,
  selectedRuntimeIds,
}: {
  onSelectedRuntimeIdsChange: (runtimeIds: readonly string[]) => void;
  runtimeProviders: SetupStepState["runtimeProviders"];
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
  const [installResults, setInstallResults] = React.useState<
    Record<string, InstallResultState>
  >({});
  const selectedRuntimeIdSet = React.useMemo(
    () => new Set(selectedRuntimeIds),
    [selectedRuntimeIds],
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
    setInstallResults((current) => ({
      ...current,
      [runtimeId]: { error: null, success: false },
    }));

    installMutation.mutate(runtimeId, {
      onSuccess: (result) => {
        setInstallResults((current) => ({
          ...current,
          [runtimeId]: result.success
            ? { error: null, success: true }
            : { error: getInstallErrorMessage(result.steps), success: false },
        }));
      },
      onError: (error) => {
        setInstallResults((current) => ({
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
      <div className="w-full max-w-[620px] text-center">
        <h1 className="text-title font-normal text-foreground">
          Use the models that fit the task
        </h1>
        <p className="mx-auto mt-3 max-w-[520px] text-sm leading-6 text-foreground/90">
          Connect different model providers so each agent can use the right
          model for the work.
        </p>
        <p className="mt-4 text-sm leading-5 text-foreground/90">
          Choose at least one to start using Buzz.
        </p>
      </div>

      <div className="flex w-full flex-1 flex-col items-center justify-center gap-8 py-10">
        <GitBashPrerequisiteCard />

        {items.length > 0 ? (
          <fieldset className="grid min-w-0 w-full max-w-[640px] grid-cols-1 gap-4 border-0 p-0 md:grid-cols-2">
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
            No compatible ACP runtimes detected yet. You can finish setup now
            and come back later in Settings &gt; Doctor.
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

  return (
    <OnboardingSlideTransition
      className="flex min-h-full w-full flex-col items-center"
      data-testid="onboarding-page-2"
      direction={direction}
      transitionKey={`setup-${direction}`}
    >
      <RuntimeProvidersSection
        onSelectedRuntimeIdsChange={onSelectedRuntimeIdsChange}
        runtimeProviders={runtimeProviders}
        selectedRuntimeIds={selectedRuntimeIds}
      />

      <OnboardingFooter>
        {selectionError ? (
          <p className="max-w-[24rem] text-center text-sm text-destructive">
            {selectionError}
          </p>
        ) : null}

        <Button
          className={`${ONBOARDING_PRIMARY_CTA_CLASS} text-sm`}
          data-testid="onboarding-setup-next"
          disabled={selectedRuntimeIds.length === 0 || isSelectionSaving}
          onClick={actions.next}
          type="button"
        >
          {isSelectionSaving ? "Saving…" : "Next"}
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
