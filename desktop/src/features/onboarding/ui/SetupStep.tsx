import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  AlertTriangle,
  Check,
  ExternalLink,
  Plus,
  RefreshCw,
  TerminalSquare,
} from "lucide-react";

import {
  useAcpRuntimesQuery,
  useInstallAcpRuntimeMutation,
  useGitBashPrerequisiteQuery,
} from "@/features/agents/hooks";
import { describeResolvedCommand } from "@/features/agents/ui/agentUi";
import {
  GlobalAgentConfigFields,
  EMPTY_GLOBAL_CONFIG,
} from "@/features/agents/ui/GlobalAgentConfigFields";
import { createSaveCoalescer } from "./saveCoalescer";
import { getBakedBuildEnv, type BakedEnvEntry } from "@/shared/api/tauri";
import {
  getGlobalAgentConfig,
  setGlobalAgentConfig,
} from "@/shared/api/tauriGlobalAgentConfig";
import type {
  AcpRuntimeCatalogEntry,
  GlobalAgentConfig,
} from "@/shared/api/types";
import { getInstallErrorMessage } from "@/shared/lib/installError";
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
import { resolveAgentReadiness } from "./agentReadiness";

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

function AgentDefaultsSection() {
  const runtimesQuery = useAcpRuntimesQuery();
  const [config, setConfig] =
    React.useState<GlobalAgentConfig>(EMPTY_GLOBAL_CONFIG);
  const [isLoading, setIsLoading] = React.useState(true);
  const [isCustomProvider, setIsCustomProvider] = React.useState(false);
  const [isCustomModelEditing, setIsCustomModelEditing] = React.useState(false);
  const [bakedEnv, setBakedEnv] = React.useState<BakedEnvEntry[]>([]);
  const coalescerRef = React.useRef<{
    enqueue: (value: GlobalAgentConfig) => void;
    cancel: () => void;
  } | null>(null);

  React.useEffect(() => {
    let unmounted = false;

    getGlobalAgentConfig()
      .then((loaded) => {
        if (!unmounted) {
          setConfig(loaded);
          setIsLoading(false);
        }
      })
      .catch(() => {
        if (!unmounted) setIsLoading(false);
      });
    getBakedBuildEnv()
      .then((env) => {
        if (!unmounted) setBakedEnv(env);
      })
      .catch(() => undefined);

    // The coalescer serializes autosaves and drains any edit that arrived
    // while a previous save was in flight. Cancel on unmount so a slow
    // in-flight request never calls setState on an unmounted component.
    const coalescer = createSaveCoalescer<GlobalAgentConfig>(
      // set_global_agent_config returns a save result (config + restart
      // counts); the coalescer round-trips the persisted config only.
      async (next) => (await setGlobalAgentConfig(next)).config,
      () => undefined, // saving state not surfaced in this autosave UX
      (saved) => {
        if (!unmounted) setConfig(saved);
      },
    );
    coalescerRef.current = coalescer;

    return () => {
      unmounted = true;
      coalescer.cancel();
    };
  }, []);

  const buzzAgentRuntime = React.useMemo(
    () => (runtimesQuery.data ?? []).find((r) => r.id === "buzz-agent"),
    [runtimesQuery.data],
  );

  const readiness = resolveAgentReadiness(runtimesQuery.data ?? [], config);

  return (
    <section className="space-y-4">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div className="space-y-1">
          <h2 className="text-xl font-semibold tracking-tight text-foreground">
            Agent defaults
          </h2>
          <p className="max-w-2xl text-sm leading-6 text-muted-foreground">
            Configure the LLM provider and credentials that buzz-agent uses, or
            connect a CLI harness like Claude or Goose above.
          </p>
        </div>

        <div className="flex shrink-0 items-center gap-2">
          {readiness.ready ? (
            <Badge
              className="border border-primary/20 bg-primary/10 text-primary"
              data-testid="agent-readiness-badge"
              variant="outline"
            >
              {readiness.reason === "cli"
                ? `${readiness.runtimeLabel} ready`
                : "buzz-agent configured"}
            </Badge>
          ) : (
            <Badge
              className="border border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-400"
              data-testid="agent-readiness-badge"
              variant="outline"
            >
              Not configured
            </Badge>
          )}
          <Button
            className="h-7 gap-1.5 text-xs text-muted-foreground hover:text-foreground"
            data-testid="agent-readiness-recheck"
            disabled={runtimesQuery.isFetching}
            onClick={() => void runtimesQuery.refetch()}
            size="sm"
            type="button"
            variant="ghost"
          >
            {runtimesQuery.isFetching ? (
              <Spinner className="h-3 w-3 border-[1.5px]" />
            ) : (
              <RefreshCw className="h-3 w-3" />
            )}
            Re-check
          </Button>
        </div>
      </div>

      {isLoading ? (
        <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
          <Spinner className="h-4 w-4 border-2" />
          Loading…
        </div>
      ) : (
        <GlobalAgentConfigFields
          bakedEnv={bakedEnv}
          buzzAgentRuntime={buzzAgentRuntime}
          config={config}
          isCustomModelEditing={isCustomModelEditing}
          isCustomProvider={isCustomProvider}
          onConfigChange={(next) => {
            // Always apply optimistically so the UI never reverts mid-save,
            // then enqueue the persist — the coalescer serialises multiple
            // rapid edits into a single trailing request.
            setConfig(next);
            coalescerRef.current?.enqueue(next);
          }}
          onCustomModelEditingChange={setIsCustomModelEditing}
          onIsCustomProviderChange={setIsCustomProvider}
        />
      )}

      {!readiness.ready ? (
        <p className="text-sm text-muted-foreground">
          You can finish now and configure agents later in Settings.
        </p>
      ) : null}
    </section>
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
          <p className="mt-3 whitespace-pre-line rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-xs leading-5 text-destructive">
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

function GitBashPrerequisiteCard() {
  const query = useGitBashPrerequisiteQuery();
  const prerequisite = query.data;
  if (!prerequisite) return null;

  return (
    <div
      className={cn(
        "rounded-lg border p-3 text-left sm:p-4",
        prerequisite.available
          ? "border-primary/25 bg-primary/[0.055]"
          : "border-amber-500/30 bg-amber-500/5",
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

      <GitBashPrerequisiteCard />

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

      <AgentDefaultsSection />

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
