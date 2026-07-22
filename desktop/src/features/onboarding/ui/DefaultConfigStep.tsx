import * as React from "react";

import { useAcpRuntimesQuery } from "@/features/agents/hooks";
import {
  AgentConfigFields,
  EMPTY_GLOBAL_CONFIG,
} from "@/features/agents/ui/AgentConfigFields";
import { resetConfigForHarnessChange } from "@/features/agents/ui/agentConfigOptions";
import { AgentDropdownSelect } from "@/features/agents/ui/agentConfigControls";
import {
  CUSTOM_RUNTIME_ID,
  CUSTOM_RUNTIME_LABEL,
  applyCustomHarnessPreference,
  buildCustomAcpRuntime,
  customHarnessCatalogStub,
  formatAgentArgsInput,
  isCustomRuntimeId,
  type ByoHarnessDraft,
} from "@/features/agents/lib/customHarness";
import { CustomHarnessFields } from "@/features/agents/ui/CustomHarnessFields";
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
  byoDraft?: ByoHarnessDraft | null;
  direction: OnboardingTransitionDirection;
  readyRuntimeIds: readonly string[];
};

function formatHarnessLabel(runtime: AcpRuntimeCatalogEntry | undefined) {
  if (!runtime) return "Select a harness";
  if (isCustomRuntimeId(runtime.id)) return CUSTOM_RUNTIME_LABEL;
  return runtime.id === "buzz-agent" ? "Buzz" : runtime.label;
}

function AgentDefaultsSection({
  byoDraft,
  onPersistenceStateChange,
  readyRuntimeIds,
}: {
  byoDraft?: ByoHarnessDraft | null;
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
  const seededByoRef = React.useRef(false);

  React.useEffect(() => {
    let unmounted = false;

    const coalescer = createSaveCoalescer<GlobalAgentConfig>(
      async (next) => (await setGlobalAgentConfig(next)).config,
      (saving) => {
        if (!unmounted) setIsSaving(saving);
      },
      (saved) => {
        if (!unmounted) setConfig(saved);
      },
    );
    coalescerRef.current = coalescer;

    async function loadDefaults() {
      const [configResult, bakedEnvResult] = await Promise.allSettled([
        getGlobalAgentConfig(),
        getBakedBuildEnv(),
      ]);

      if (unmounted) return;

      let next =
        configResult.status === "fulfilled"
          ? configResult.value
          : EMPTY_GLOBAL_CONFIG;
      if (byoDraft?.command.trim() && !seededByoRef.current) {
        next = applyCustomHarnessPreference(next, byoDraft);
        seededByoRef.current = true;
        coalescer.enqueue(next);
      }
      setConfig(next);
      if (bakedEnvResult.status === "fulfilled") {
        setBakedEnv(bakedEnvResult.value);
      }
      setIsLoading(false);
    }

    void loadDefaults();

    return () => {
      unmounted = true;
      coalescer.cancel();
    };
  }, [byoDraft]);

  const customReady = readyRuntimeIds.includes(CUSTOM_RUNTIME_ID);
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
  const readyCatalogRuntimes = React.useMemo(
    () =>
      getVisibleOnboardingRuntimes(runtimesQuery.data ?? []).filter((runtime) =>
        readyRuntimeIdSet.has(runtime.id),
      ),
    [readyRuntimeIdSet, runtimesQuery.data],
  );
  const customRuntime = React.useMemo(
    () =>
      customReady || isCustomRuntimeId(config.preferred_runtime)
        ? buildCustomAcpRuntime(
            config.preferred_agent_command ?? "",
            config.preferred_agent_args ?? [],
          )
        : null,
    [
      config.preferred_agent_args,
      config.preferred_agent_command,
      config.preferred_runtime,
      customReady,
    ],
  );
  const readyRuntimes = React.useMemo(() => {
    const items: AcpRuntimeCatalogEntry[] = [...readyCatalogRuntimes];
    if (customRuntime) {
      items.push(customRuntime);
    } else if (customReady || isCustomRuntimeId(config.preferred_runtime)) {
      items.push(customHarnessCatalogStub());
    }
    return items;
  }, [config.preferred_runtime, customReady, customRuntime, readyCatalogRuntimes]);
  const selectedRuntime = React.useMemo(
    () =>
      readyRuntimes.find((runtime) => runtime.id === config.preferred_runtime),
    [config.preferred_runtime, readyRuntimes],
  );
  const selectedRuntimeId = selectedRuntime?.id ?? "";
  const configSurfaceLoading = isLoading || runtimesQuery.isLoading;
  const isCustomSelected = isCustomRuntimeId(selectedRuntimeId);

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

  const handleCustomCommandChange = React.useCallback(
    (command: string) => {
      const next = applyCustomHarnessPreference(config, {
        command,
        args: formatAgentArgsInput(config.preferred_agent_args),
      });
      setConfig(next);
      coalescerRef.current?.enqueue(next);
    },
    [config],
  );

  const handleCustomArgsChange = React.useCallback(
    (value: string) => {
      const next = applyCustomHarnessPreference(config, {
        command: config.preferred_agent_command ?? "",
        args: value,
      });
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
  const customCommandReady =
    !isCustomSelected ||
    (config.preferred_agent_command ?? "").trim().length > 0;
  React.useEffect(() => {
    onPersistenceStateChange({
      canComplete:
        selectedRuntimeId.length > 0 && customCommandReady && !isSaving,
      flush: flushPersistence,
    });
  }, [
    customCommandReady,
    flushPersistence,
    isSaving,
    onPersistenceStateChange,
    selectedRuntimeId,
  ]);

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

          {isCustomSelected ? (
            <CustomHarnessFields
              args={formatAgentArgsInput(config.preferred_agent_args)}
              argsId="global-agent-custom-args"
              argsTestId="global-agent-custom-args"
              command={config.preferred_agent_command ?? ""}
              commandId="global-agent-custom-command"
              commandTestId="global-agent-custom-command"
              onArgsChange={handleCustomArgsChange}
              onCommandChange={handleCustomCommandChange}
              size="onboarding"
            />
          ) : (
            <AgentConfigFields
              bakedEnv={bakedEnv}
              selectedRuntime={selectedRuntime}
              config={config}
              isCustomModelEditing={isCustomModelEditing}
              isCustomProvider={isCustomProvider}
              onConfigChange={(next) => {
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
          )}
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
  byoDraft = null,
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
  const customOnly =
    readyRuntimeIds.length === 1 &&
    isCustomRuntimeId(readyRuntimeIds[0] ?? "");

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
          {customOnly
            ? "Confirm your custom harness"
            : "Configure your default model settings"}
        </h1>
        <p className="mx-auto mt-3 max-w-[440px] text-sm leading-5 text-foreground/80">
          {customOnly
            ? "Buzz will spawn this ACP command for new agents. Make sure the binary speaks ACP over stdio and is on your PATH."
            : "This will be set as your default model configuration across Buzz. You can always change this in your Settings or give specific agents a different configuration."}
        </p>
      </div>

      <div className="flex w-full flex-1 items-center justify-center py-10">
        <div className="w-full max-w-[328px]">
          <AgentDefaultsSection
            byoDraft={byoDraft}
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
