import * as React from "react";

import { useAcpRuntimesQuery } from "@/features/agents/hooks";
import {
  AgentConfigFields,
  EMPTY_GLOBAL_CONFIG,
} from "@/features/agents/ui/AgentConfigFields";
import { resetConfigForHarnessChange } from "@/features/agents/ui/agentConfigOptions";
import { AgentDropdownSelect } from "@/features/agents/ui/agentConfigControls";
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
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import {
  getReadyOnboardingRuntimes,
  getVisibleOnboardingRuntimes,
} from "./onboardingRuntimeSelection";
import type { DefaultConfigStepActions } from "./types";

type DefaultConfigStepProps = {
  actions: DefaultConfigStepActions;
  direction: OnboardingTransitionDirection;
  readyRuntimeIds: readonly string[];
};

function formatHarnessLabel(runtime: AcpRuntimeCatalogEntry | undefined) {
  if (!runtime) return "Select a harness";
  return runtime.id === "buzz-agent" ? "Buzz" : runtime.label;
}

function AgentDefaultsSection({
  onPersistenceStateChange,
  readyRuntimeIds,
}: {
  onPersistenceStateChange: (state: {
    canComplete: boolean;
    flush: () => Promise<void>;
  }) => void;
  readyRuntimeIds: readonly string[];
}) {
  const runtimesQuery = useAcpRuntimesQuery();
  const [config, setConfig] =
    React.useState<GlobalAgentConfig>(EMPTY_GLOBAL_CONFIG);
  const [isLoading, setIsLoading] = React.useState(true);
  const [isCustomProvider, setIsCustomProvider] = React.useState(false);
  const [isCustomModelEditing, setIsCustomModelEditing] = React.useState(false);
  const [bakedEnv, setBakedEnv] = React.useState<BakedEnvEntry[]>([]);
  const coalescerRef = React.useRef<{
    enqueue: (value: GlobalAgentConfig) => void;
    flush: () => Promise<void>;
    cancel: () => void;
  } | null>(null);
  const [isSaving, setIsSaving] = React.useState(false);

  React.useEffect(() => {
    let unmounted = false;

    async function loadDefaults() {
      const [configResult, bakedEnvResult] = await Promise.allSettled([
        getGlobalAgentConfig(),
        getBakedBuildEnv(),
      ]);

      if (unmounted) return;

      if (configResult.status === "fulfilled") {
        setConfig(configResult.value);
      }
      if (bakedEnvResult.status === "fulfilled") {
        setBakedEnv(bakedEnvResult.value);
      }
      setIsLoading(false);
    }

    void loadDefaults();

    // The coalescer serializes autosaves and drains any edit that arrived
    // while a previous save was in flight. Cancel on unmount so a slow
    // in-flight request never calls setState on an unmounted component.
    const coalescer = createSaveCoalescer<GlobalAgentConfig>(
      // set_global_agent_config returns a save result (config + restart
      // counts); the coalescer round-trips the persisted config only.
      async (next) => (await setGlobalAgentConfig(next)).config,
      (saving) => {
        if (!unmounted) setIsSaving(saving);
      },
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

  const effectiveReadyRuntimeIds = React.useMemo(
    () =>
      readyRuntimeIds.length > 0
        ? readyRuntimeIds
        : getReadyOnboardingRuntimes(runtimesQuery.data ?? []).map(
            (runtime) => runtime.id,
          ),
    [readyRuntimeIds, runtimesQuery.data],
  );
  const readyRuntimeIdSet = React.useMemo(
    () => new Set(effectiveReadyRuntimeIds),
    [effectiveReadyRuntimeIds],
  );
  // Setup already confirmed readiness. Re-filter only for onboarding
  // visibility here; a transient auth recheck must not invalidate that handoff.
  const readyRuntimes = React.useMemo(
    () =>
      getVisibleOnboardingRuntimes(runtimesQuery.data ?? []).filter((runtime) =>
        readyRuntimeIdSet.has(runtime.id),
      ),
    [readyRuntimeIdSet, runtimesQuery.data],
  );
  const selectedRuntime = React.useMemo(
    () =>
      readyRuntimes.find((runtime) => runtime.id === config.preferred_runtime),
    [config.preferred_runtime, readyRuntimes],
  );
  const selectedRuntimeId = selectedRuntime?.id ?? "";
  const configSurfaceLoading = isLoading || runtimesQuery.isLoading;

  const configSurfaceError =
    runtimesQuery.isError ||
    (!configSurfaceLoading &&
      effectiveReadyRuntimeIds.length > 0 &&
      readyRuntimes.length === 0);
  const harnessOptions = React.useMemo(
    () =>
      readyRuntimes.map((runtime) => ({
        label: formatHarnessLabel(runtime),
        value: runtime.id,
      })),
    [readyRuntimes],
  );

  const handleHarnessChange = React.useCallback(
    (runtimeId: string) => {
      const next = resetConfigForHarnessChange(config, runtimeId);
      setIsCustomModelEditing(false);
      setIsCustomProvider(false);
      setConfig(next);
      coalescerRef.current?.enqueue(next);
    },
    [config],
  );

  React.useEffect(() => {
    if (configSurfaceLoading || selectedRuntimeId) return;
    if (readyRuntimes.length !== 1) return;
    handleHarnessChange(readyRuntimes[0].id);
  }, [
    configSurfaceLoading,
    handleHarnessChange,
    readyRuntimes,
    selectedRuntimeId,
  ]);

  const flushPersistence = React.useCallback(
    () => coalescerRef.current?.flush() ?? Promise.resolve(),
    [],
  );
  React.useEffect(() => {
    onPersistenceStateChange({
      canComplete: selectedRuntimeId.length > 0 && !isSaving,
      flush: flushPersistence,
    });
  }, [flushPersistence, isSaving, onPersistenceStateChange, selectedRuntimeId]);

  return (
    <section className="w-full space-y-4 text-left text-sm">
      {configSurfaceLoading ? (
        <div className="flex items-center justify-center gap-2 py-4 text-sm text-muted-foreground">
          <Spinner className="h-4 w-4 border-2" />
          Loading…
        </div>
      ) : configSurfaceError ? (
        <p className="py-4 text-center text-sm text-destructive">
          Couldn't load harness settings. Go back and try again.
        </p>
      ) : (
        <div className="space-y-7">
          <div className="space-y-4">
            <label
              className="pl-3 text-sm font-medium"
              htmlFor="global-agent-default-harness"
            >
              Default harness
            </label>
            <AgentDropdownSelect
              className="h-12 rounded-2xl border-foreground/15 bg-white px-4 py-2 text-sm shadow-none hover:bg-white/95"
              id="global-agent-default-harness"
              onValueChange={handleHarnessChange}
              options={harnessOptions}
              placeholder="Select a harness"
              placeholderClassName="text-foreground/70"
              testId="global-agent-default-harness"
              value={selectedRuntimeId}
            />
          </div>

          <AgentConfigFields
            bakedEnv={bakedEnv}
            selectedRuntime={selectedRuntime}
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
            placeholderClassName="text-foreground/70"
            selectClassName="h-12 rounded-2xl border-foreground/15 bg-white px-4 py-2 text-sm shadow-none hover:bg-white/95"
            disclosure="onboarding-essential"
            unstyled
            useCustomSelect
          />
        </div>
      )}
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
  readyRuntimeIds,
}: DefaultConfigStepProps) {
  const [persistenceState, setPersistenceState] = React.useState<{
    canComplete: boolean;
    flush: () => Promise<void>;
  }>({ canComplete: false, flush: () => Promise.resolve() });
  const [completionError, setCompletionError] = React.useState<string | null>(
    null,
  );
  const [isCompleting, setIsCompleting] = React.useState(false);

  const handleComplete = React.useCallback(async () => {
    setIsCompleting(true);
    setCompletionError(null);
    try {
      await persistenceState.flush();
      actions.complete();
    } catch {
      setCompletionError("Couldn't save your default harness. Try again.");
      setIsCompleting(false);
    }
  }, [actions, persistenceState]);

  return (
    <OnboardingSlideTransition
      className="flex min-h-full w-full flex-col items-center"
      data-testid="onboarding-page-config"
      direction={direction}
      transitionKey={`default-config-${direction}`}
    >
      <div className="w-full max-w-[500px] text-center">
        <h1 className="text-title font-normal text-foreground">
          Configure your default model settings
        </h1>
        <p className="mx-auto mt-3 max-w-[440px] text-sm leading-5 text-foreground/80">
          This will be set as your default model configuration across Buzz. You
          can always change this in your Settings or give specific agents a
          different configuration.
        </p>
      </div>

      <div className="flex w-full flex-1 items-center justify-center py-10">
        <div className="w-full max-w-[328px]">
          <AgentDefaultsSection
            onPersistenceStateChange={setPersistenceState}
            readyRuntimeIds={readyRuntimeIds}
          />
          {completionError ? (
            <p
              className="mt-3 text-center text-xs text-destructive"
              role="alert"
            >
              {completionError}
            </p>
          ) : null}
        </div>
      </div>

      <OnboardingFooter>
        <Button
          className={`${ONBOARDING_PRIMARY_CTA_CLASS} text-sm`}
          data-testid="onboarding-finish"
          disabled={!persistenceState.canComplete || isCompleting}
          onClick={() => void handleComplete()}
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
