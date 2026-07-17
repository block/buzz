import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  AlertTriangle,
  Check,
  ExternalLink,
  Info,
  Plus,
  TerminalSquare,
} from "lucide-react";

import {
  useAcpRuntimesQuery,
  useInstallAcpRuntimeMutation,
  useGitBashPrerequisiteQuery,
} from "@/features/agents/hooks";
import { describeResolvedCommand } from "@/features/agents/ui/agentUi";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { getInstallErrorMessage } from "@/shared/lib/installError";
import { cn } from "@/shared/lib/cn";
import { useTheme } from "@/shared/theme/ThemeProvider";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Spinner } from "@/shared/ui/spinner";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import type { SetupStepActions, SetupStepState } from "./types";

type SetupStepProps = {
  actions: SetupStepActions;
  direction: OnboardingTransitionDirection;
};

type SetupStepContentProps = {
  actions: SetupStepActions;
  direction: OnboardingTransitionDirection;
  state: SetupStepState;
};

type InstallResultState = {
  error: string | null;
  success: boolean;
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

function RuntimeIcon({ runtime }: { runtime: AcpRuntimeCatalogEntry }) {
  const [imageFailed, setImageFailed] = React.useState(false);
  const { isDark } = useTheme();
  const shouldForceForegroundColor = runtime.id === "goose";

  if (runtime.avatarUrl && !imageFailed) {
    return (
      <img
        alt=""
        className={cn(
          "h-12 w-12 rounded-md object-contain",
          shouldForceForegroundColor &&
            (isDark ? "brightness-0 invert" : "brightness-0"),
        )}
        onError={() => setImageFailed(true)}
        src={runtime.avatarUrl}
      />
    );
  }

  return (
    <TerminalSquare
      className="h-12 w-12 text-muted-foreground"
      strokeWidth={1.25}
    />
  );
}

function RuntimeStatus({
  installError,
  installSuccess,
  isInstalling,
  onInstall,
  runtime,
}: {
  installError: string | null;
  installSuccess: boolean;
  isInstalling: boolean;
  onInstall: () => void;
  runtime: AcpRuntimeCatalogEntry;
}) {
  if (isInstalling) {
    return (
      <div
        aria-label={`Installing ${runtime.label}`}
        className="flex h-8 w-8 items-center justify-center"
        role="status"
      >
        <Spinner className="h-4 w-4 border-2 text-foreground" />
      </div>
    );
  }

  if (installError) {
    return (
      <div className="flex h-8 w-8 items-center justify-center">
        <AlertTriangle className="h-4 w-4 text-destructive" />
      </div>
    );
  }

  if (runtime.availability === "available" || installSuccess) {
    return (
      <div
        aria-label={`${runtime.label} available`}
        className="flex h-6 w-6 items-center justify-center rounded-full bg-primary shadow-sm"
        role="img"
      >
        <Check
          className="h-3.5 w-3.5 text-primary-foreground"
          strokeWidth={3}
        />
      </div>
    );
  }

  if (runtime.canAutoInstall) {
    return (
      <Button
        aria-label={`Install ${runtime.label}`}
        className="h-8 w-8 text-muted-foreground hover:text-foreground"
        data-testid={`onboarding-runtime-install-${runtime.id}`}
        onClick={onInstall}
        size="icon"
        type="button"
        variant="ghost"
      >
        <Plus className="h-4 w-4" />
      </Button>
    );
  }

  return (
    <Button
      aria-label={`View ${runtime.label} setup instructions`}
      className="h-8 w-8 text-muted-foreground hover:text-foreground"
      data-testid={`onboarding-runtime-instructions-${runtime.id}`}
      onClick={() => void openUrl(runtime.installInstructionsUrl)}
      size="icon"
      type="button"
      variant="ghost"
    >
      <ExternalLink className="h-4 w-4" />
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
        <p className="text-sm leading-5 text-muted-foreground">
          {description.charAt(0).toUpperCase() + description.slice(1)}
        </p>
        {runtime.defaultArgs.length > 0 ? (
          <p className="mt-1 text-xs text-muted-foreground/80">
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
        <p className="text-sm leading-5 text-muted-foreground">
          CLI detected; ACP adapter missing.
        </p>
        <p className="mt-1 text-xs leading-5 text-muted-foreground/80">
          {runtime.installHint}
        </p>
      </>
    );
  }

  if (runtime.availability === "adapter_outdated") {
    return (
      <>
        <p className="text-sm leading-5 text-muted-foreground">
          ACP adapter detected but outdated — reinstall required.
        </p>
        <p className="mt-1 text-xs leading-5 text-muted-foreground/80">
          This updates the machine-global{" "}
          <code className="rounded bg-muted px-0.5 text-2xs">codex-acp</code>{" "}
          adapter. Older Buzz releases using the legacy adapter contract may
          lose community access until{" "}
          <code className="rounded bg-muted px-0.5 text-2xs">
            @zed-industries/codex-acp@0.16.0
          </code>{" "}
          is restored.
        </p>
        <p className="mt-1 text-xs leading-5 text-muted-foreground/80">
          {runtime.installHint}
        </p>
      </>
    );
  }

  if (runtime.availability === "cli_missing") {
    return (
      <>
        <p className="text-sm leading-5 text-muted-foreground">
          ACP adapter detected; CLI missing.
        </p>
        <p className="mt-1 text-xs leading-5 text-muted-foreground/80">
          {runtime.installHint}
        </p>
      </>
    );
  }

  return (
    <>
      <p className="text-sm leading-5 text-muted-foreground">
        Not installed yet.
      </p>
      <p className="mt-1 text-xs leading-5 text-muted-foreground/80">
        {runtime.installHint}
      </p>
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
  return "Not installed yet.";
}

function RuntimeCard({
  installError,
  installSuccess,
  isInstalling,
  onInstall,
  runtime,
}: {
  installError: string | null;
  installSuccess: boolean;
  isInstalling: boolean;
  onInstall: () => void;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const isAvailable = runtime.availability === "available" || installSuccess;

  return (
    <div
      className={cn(
        "relative flex min-h-40 w-40 flex-col items-center justify-center gap-3 rounded-2xl bg-white/85 p-4 text-center",
        isAvailable
          ? "shadow-[0_0_55px_25px_rgba(255,255,255,0.85)]"
          : "shadow-[0_0_45px_18px_rgba(255,255,255,0.55)] opacity-90",
        installError && "ring-1 ring-destructive/40",
      )}
      data-testid={`onboarding-runtime-${runtime.id}`}
    >
      <div className="absolute right-2 top-2">
        <RuntimeStatus
          installError={installError}
          installSuccess={installSuccess}
          isInstalling={isInstalling}
          onInstall={onInstall}
          runtime={runtime}
        />
      </div>

      <div className="absolute left-2 top-2">
        <Popover>
          <PopoverTrigger asChild>
            <Button
              aria-label={`${runtime.label} details`}
              className="h-6 w-6 text-muted-foreground/70 hover:text-foreground"
              data-testid={`onboarding-runtime-details-${runtime.id}`}
              size="icon"
              type="button"
              variant="ghost"
            >
              <Info className="h-3.5 w-3.5" />
            </Button>
          </PopoverTrigger>
          <PopoverContent align="start" className="w-80 text-left">
            <RuntimeDetails runtime={runtime} />
          </PopoverContent>
        </Popover>
      </div>

      <RuntimeIcon runtime={runtime} />

      <div className="min-w-0">
        <h2 className="text-sm font-medium leading-5 text-foreground">
          {runtime.label}
        </h2>
        {!isAvailable && !installError ? (
          <p className="mt-1 text-2xs leading-4 text-muted-foreground">
            {runtimeDetailText(runtime)}
          </p>
        ) : null}
        {installError ? (
          <p className="mt-1 text-2xs leading-4 text-destructive">
            {installError}
          </p>
        ) : null}
        {installSuccess && runtime.availability !== "available" ? (
          <p className="mt-1 text-2xs leading-4 text-primary">Installed</p>
        ) : null}
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
        "mx-auto w-full max-w-[560px] rounded-2xl bg-white/85 p-3 text-left sm:p-4",
        prerequisite.available
          ? "shadow-[0_0_45px_18px_rgba(255,255,255,0.7)]"
          : "ring-1 ring-amber-500/40 shadow-[0_0_45px_18px_rgba(255,255,255,0.55)]",
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

function RuntimeProvidersSection({
  runtimeProviders,
}: {
  runtimeProviders: SetupStepState["runtimeProviders"];
}) {
  const { errorMessage, isChecking, items } = runtimeProviders;
  const installMutation = useInstallAcpRuntimeMutation();
  const [installResults, setInstallResults] = React.useState<
    Record<string, InstallResultState>
  >({});

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
    <section className="flex w-full flex-col items-center gap-8">
      <div className="w-full max-w-[520px] text-center">
        <h1 className="text-title font-normal text-foreground">
          Use the models that fit the task
        </h1>
        <p className="mt-3 text-sm leading-6 text-foreground/80">
          These are the local agent harnesses Buzz detected. You choose a
          harness when creating each agent.
        </p>
      </div>

      <GitBashPrerequisiteCard />

      {items.length > 0 ? (
        <div className="flex flex-wrap items-stretch justify-center gap-4">
          {items.map((runtime) => (
            <RuntimeCard
              installError={installResults[runtime.id]?.error ?? null}
              installSuccess={installResults[runtime.id]?.success ?? false}
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
        <div className="rounded-2xl bg-white/70 px-6 py-6 text-sm text-muted-foreground">
          Looking for compatible runtimes...
        </div>
      ) : errorMessage ? null : (
        <p
          className="max-w-[560px] rounded-2xl bg-white/70 px-6 py-6 text-sm text-muted-foreground"
          data-testid="onboarding-acp-empty"
        >
          No compatible ACP runtimes detected yet. You can finish setup now and
          come back later in Settings &gt; Doctor.
        </p>
      )}

      {errorMessage ? (
        <p className="max-w-[560px] rounded-2xl bg-destructive/10 px-6 py-3 text-sm text-destructive">
          {errorMessage}
        </p>
      ) : null}
    </section>
  );
}

function SetupStepContent({
  actions,
  direction,
  state,
}: SetupStepContentProps) {
  const { runtimeProviders } = state;

  return (
    <OnboardingSlideTransition
      // pb clears the always-docked footer: the provider list can overflow on
      // short windows, so reserve room to scroll clear of the fixed CTA group.
      className="flex w-full flex-col items-center pb-20"
      data-testid="onboarding-page-2"
      direction={direction}
      transitionKey={`setup-${direction}`}
    >
      <RuntimeProvidersSection runtimeProviders={runtimeProviders} />

      <OnboardingFooter>
        <Button
          className={ONBOARDING_PRIMARY_CTA_CLASS}
          data-testid="onboarding-setup-next"
          onClick={actions.next}
          type="button"
        >
          Next
        </Button>

        <Button
          className="h-9 rounded-full bg-foreground/10 px-6 hover:bg-foreground/15"
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

export function SetupStep({ actions, direction }: SetupStepProps) {
  const state = useSetupStepState();

  return (
    <SetupStepContent actions={actions} direction={direction} state={state} />
  );
}
