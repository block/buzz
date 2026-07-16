import * as React from "react";
import { RefreshCw } from "lucide-react";

import { useAcpRuntimesQuery } from "@/features/agents/hooks";
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
import type { GlobalAgentConfig } from "@/shared/api/types";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import type { DefaultConfigStepActions } from "./types";
import { resolveAgentReadiness } from "./agentReadiness";

type DefaultConfigStepProps = {
  actions: DefaultConfigStepActions;
  direction: OnboardingTransitionDirection;
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
    <section className="w-full space-y-4 text-left">
      <div className="flex items-center justify-center gap-2">
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

      {isLoading ? (
        <div className="flex items-center justify-center gap-2 py-4 text-sm text-muted-foreground">
          <Spinner className="h-4 w-4 border-2" />
          Loading…
        </div>
      ) : (
        <div className="rounded-2xl bg-white/85 p-2 shadow-[0_0_55px_25px_rgba(255,255,255,0.6)]">
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
        </div>
      )}

      {!readiness.ready ? (
        <p className="text-center text-sm text-muted-foreground">
          You can finish now and configure agents later in Settings.
        </p>
      ) : null}
    </section>
  );
}

/**
 * Machine onboarding page 4 — default model configuration. Presents the
 * global agent defaults (provider, model, effort, env vars) centered under
 * the mock's "Configure your default model settings" heading.
 */
export function DefaultConfigStep({
  actions,
  direction,
}: DefaultConfigStepProps) {
  return (
    <OnboardingSlideTransition
      className="flex w-full flex-col items-center"
      data-testid="onboarding-page-config"
      direction={direction}
      transitionKey={`default-config-${direction}`}
    >
      <div className="w-full max-w-[500px] text-center">
        <h1 className="text-3xl font-semibold tracking-tight text-foreground">
          Configure your default model settings
        </h1>
        <p className="mt-3 text-sm leading-6 text-foreground/80">
          This will be set as your default model configuration across Buzz. You
          can always change this in your Settings.
        </p>
      </div>

      <div className="mt-8 w-full max-w-[560px]">
        <AgentDefaultsSection />
      </div>

      <div className="mt-10 flex flex-col items-center gap-3">
        <Button
          className="h-10 rounded-full px-8"
          data-testid="onboarding-finish"
          onClick={actions.complete}
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
      </div>
    </OnboardingSlideTransition>
  );
}
