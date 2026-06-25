import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  AlertTriangle,
  Check,
  ExternalLink,
  Plus,
  TerminalSquare,
} from "lucide-react";

import {
  useAcpRuntimesQuery,
  useInstallAcpRuntimeMutation,
} from "@/features/agents/hooks";
import { describeResolvedCommand } from "@/features/agents/ui/agentUi";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { useTheme } from "@/shared/theme/ThemeProvider";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";
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
      <div className="flex h-9 w-9 items-center justify-center rounded-md border border-border/45 bg-background/80">
        <img
          alt=""
          className={cn(
            "h-7 w-7 rounded-sm object-contain",
            shouldForceForegroundColor &&
              (isDark ? "brightness-0 invert" : "brightness-0"),
          )}
          onError={() => setImageFailed(true)}
          src={runtime.avatarUrl}
        />
      </div>
    );
  }

  return (
    <div className="flex h-9 w-9 items-center justify-center rounded-md border border-border/45 bg-background/80 text-muted-foreground">
      <TerminalSquare className="h-4 w-4" />
    </div>
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
        className="flex h-8 shrink-0 items-center justify-center"
        role="status"
      >
        <Spinner className="h-4 w-4 border-2 text-foreground" />
      </div>
    );
  }

  if (installError) {
    return (
      <div className="flex h-8 shrink-0 items-center justify-center">
        <AlertTriangle className="h-4 w-4 text-destructive" />
      </div>
    );
  }

  if (runtime.availability === "available" || installSuccess) {
    return (
      <div className="flex h-8 shrink-0 items-center justify-center">
        <Check className="h-4 w-4 text-primary" />
      </div>
    );
  }

  if (runtime.canAutoInstall) {
    return (
      <Button
        aria-label={`Install ${runtime.label}`}
        className="h-8 w-8 shrink-0 text-muted-foreground hover:text-foreground"
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
      className="h-8 w-8 shrink-0 text-muted-foreground hover:text-foreground"
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
    return (
      <>
        <p className="mt-2 text-sm leading-5 text-muted-foreground">
          {describeResolvedCommand(runtime.command, runtime.binaryPath)}
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
        <p className="mt-2 text-sm leading-5 text-muted-foreground">
          CLI detected; ACP adapter missing.
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
        <p className="mt-2 text-sm leading-5 text-muted-foreground">
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
      <p className="mt-2 text-sm leading-5 text-muted-foreground">
        Not installed yet.
      </p>
      <p className="mt-1 text-xs leading-5 text-muted-foreground/80">
        {runtime.installHint}
      </p>
    </>
  );
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
        "grid min-h-28 grid-cols-[auto_1fr_auto] items-start gap-3 rounded-lg border bg-background p-3 text-left transition-colors sm:p-4",
        isAvailable
          ? "border-primary/25 bg-primary/[0.055] shadow-[0_12px_30px_hsl(var(--primary)/0.08)] dark:bg-primary/[0.08]"
          : installError
            ? "border-destructive/45 bg-destructive/5 shadow-xs"
            : "border-2 border-dashed border-muted-foreground/35 bg-muted/20 shadow-none",
      )}
      data-testid={`onboarding-runtime-${runtime.id}`}
    >
      <RuntimeIcon runtime={runtime} />

      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2">
          <h2 className="text-base font-medium leading-6 text-foreground">
            {runtime.label}
          </h2>
          {isAvailable ? (
            <Badge
              className="border border-primary/20 bg-primary/10 text-primary"
              variant="outline"
            >
              Installed
            </Badge>
          ) : null}
        </div>

        <RuntimeDetails runtime={runtime} />

        {installError ? (
          <p className="mt-3 rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-xs leading-5 text-destructive">
            {installError}
          </p>
        ) : null}

        {installSuccess && runtime.availability !== "available" ? (
          <p className="mt-3 rounded-md border border-primary/25 bg-primary/10 px-3 py-2 text-xs leading-5 text-primary">
            Installed successfully. You can finish onboarding now.
          </p>
        ) : null}
      </div>

      <RuntimeStatus
        installError={installError}
        installSuccess={installSuccess}
        isInstalling={isInstalling}
        onInstall={onInstall}
        runtime={runtime}
      />
    </div>
  );
}

function getInstallErrorMessage(result: {
  steps: { stderr: string; stdout: string; step: string }[];
}) {
  const lastStep = result.steps[result.steps.length - 1];
  if (!lastStep) {
    return "Install failed with no output.";
  }

  return `Step "${lastStep.step}" failed: ${
    lastStep.stderr || lastStep.stdout || "unknown error"
  }`;
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
            : { error: getInstallErrorMessage(result), success: false },
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
    <section className="space-y-4">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="space-y-1">
          <h1 className="text-3xl font-semibold tracking-tight text-foreground">
            Agent harnesses
          </h1>
          <p className="max-w-2xl text-sm leading-6 text-muted-foreground">
            Buzz can launch local ACP-compatible agent harnesses. Install or
            verify the runtimes this desktop app can see.
          </p>
        </div>
      </div>

      {items.length > 0 ? (
        <div className="grid gap-3 lg:grid-cols-2">
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
        <div className="rounded-lg border border-border/70 bg-background px-4 py-6 text-sm text-muted-foreground">
          Looking for compatible runtimes...
        </div>
      ) : errorMessage ? null : (
        <p
          className="rounded-lg border border-border/70 bg-background px-4 py-6 text-sm text-muted-foreground"
          data-testid="onboarding-acp-empty"
        >
          No compatible ACP runtimes detected yet. You can finish setup now and
          come back later in Settings &gt; Doctor.
        </p>
      )}

      {errorMessage ? (
        <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
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
      className="space-y-7 text-left"
      data-testid="onboarding-page-2"
      direction={direction}
      transitionKey={`setup-${direction}`}
    >
      <RuntimeProvidersSection runtimeProviders={runtimeProviders} />

      <div className="mx-auto flex w-full max-w-md flex-col gap-3">
        <Button
          className="h-10 w-full"
          data-testid="onboarding-finish"
          onClick={actions.complete}
          type="button"
        >
          Finish
        </Button>

        <Button
          className="h-10 w-full text-muted-foreground hover:text-accent-foreground"
          data-testid="onboarding-back"
          onClick={actions.back}
          type="button"
          variant="ghost"
        >
          Back
        </Button>
      </div>
    </OnboardingSlideTransition>
  );
}

export function SetupStep({ actions, direction }: SetupStepProps) {
  const state = useSetupStepState();

  return (
    <SetupStepContent actions={actions} direction={direction} state={state} />
  );
}
