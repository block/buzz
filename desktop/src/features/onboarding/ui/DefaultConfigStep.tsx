import * as React from "react";

import { useAcpRuntimesQuery } from "@/features/agents/hooks";
import {
  GlobalAgentConfigFields,
  EMPTY_GLOBAL_CONFIG,
} from "@/features/agents/ui/GlobalAgentConfigFields";
import { BUZZ_AGENT_THINKING_EFFORT } from "@/features/agents/ui/buzzAgentConfig";
import { runtimeSupportsLlmProviderSelection } from "@/features/agents/ui/personaDialogPickers";
import { AgentDropdownSelect } from "@/features/agents/ui/personaProviderModelFields";
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
import type { DefaultConfigStepActions } from "./types";

type DefaultConfigStepProps = {
  actions: DefaultConfigStepActions;
  direction: OnboardingTransitionDirection;
  selectedRuntimeIds: readonly string[];
};

const RUNTIME_ORDER = ["claude", "codex", "goose", "buzz-agent"];

function formatHarnessLabel(runtime: AcpRuntimeCatalogEntry | undefined) {
  if (!runtime) return "Select a harness";
  return runtime.id === "buzz-agent" ? "Buzz" : runtime.label;
}

function sortSelectedRuntimes(
  runtimes: readonly AcpRuntimeCatalogEntry[],
  selectedRuntimeIds: readonly string[],
) {
  const selectedRuntimeIdSet = new Set(selectedRuntimeIds);
  return runtimes
    .filter((runtime) => selectedRuntimeIdSet.has(runtime.id))
    .sort((left, right) => {
      const leftIndex = RUNTIME_ORDER.indexOf(left.id);
      const rightIndex = RUNTIME_ORDER.indexOf(right.id);
      return (
        (leftIndex === -1 ? RUNTIME_ORDER.length : leftIndex) -
        (rightIndex === -1 ? RUNTIME_ORDER.length : rightIndex)
      );
    });
}

function AgentDefaultsSection({
  selectedRuntimeIds,
}: {
  selectedRuntimeIds: readonly string[];
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
    cancel: () => void;
  } | null>(null);

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

  const selectedRuntimes = React.useMemo(
    () => sortSelectedRuntimes(runtimesQuery.data ?? [], selectedRuntimeIds),
    [runtimesQuery.data, selectedRuntimeIds],
  );
  const selectedRuntime = React.useMemo(() => {
    const preferredRuntime = selectedRuntimes.find(
      (runtime) => runtime.id === config.preferred_runtime,
    );
    return preferredRuntime ?? selectedRuntimes[0];
  }, [config.preferred_runtime, selectedRuntimes]);
  const selectedRuntimeId =
    selectedRuntime?.id ?? config.preferred_runtime ?? "";
  const selectedRuntimeSupportsModelProvider =
    runtimeSupportsLlmProviderSelection(selectedRuntimeId);
  const harnessOptions = React.useMemo(
    () =>
      selectedRuntimes.map((runtime) => ({
        label: formatHarnessLabel(runtime),
        value: runtime.id,
      })),
    [selectedRuntimes],
  );

  function handleHarnessChange(runtimeId: string) {
    const nextEnvVars = { ...config.env_vars };
    delete nextEnvVars[BUZZ_AGENT_THINKING_EFFORT];
    const nextProvider =
      runtimeSupportsLlmProviderSelection(runtimeId) &&
      config.provider !== "relay-mesh"
        ? config.provider
        : null;
    const next = {
      ...config,
      env_vars: nextEnvVars,
      model: null,
      preferred_runtime: runtimeId || null,
      provider: nextProvider,
    };
    setIsCustomModelEditing(false);
    setIsCustomProvider(false);
    setConfig(next);
    coalescerRef.current?.enqueue(next);
  }

  return (
    <section className="w-full space-y-4 text-left text-sm">
      {isLoading ? (
        <div className="flex items-center justify-center gap-2 py-4 text-sm text-muted-foreground">
          <Spinner className="h-4 w-4 border-2" />
          Loading…
        </div>
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

          <GlobalAgentConfigFields
            bakedEnv={bakedEnv}
            selectedRuntime={selectedRuntime}
            config={config}
            isCustomModelEditing={isCustomModelEditing}
            isCustomProvider={isCustomProvider}
            autoSelectModelOnProviderChange
            disableModelSelectDuringDiscovery={false}
            effortPlaceholderLabel="Select effort level"
            keepSelectedModelValueLabel
            modelPlaceholderLabel="Select a model"
            onConfigChange={(next) => {
              // Always apply optimistically so the UI never reverts mid-save,
              // then enqueue the persist — the coalescer serialises multiple
              // rapid edits into a single trailing request.
              setConfig(next);
              coalescerRef.current?.enqueue(next);
            }}
            onCustomModelEditingChange={setIsCustomModelEditing}
            onIsCustomProviderChange={setIsCustomProvider}
            preserveCredentialEnvVarsOnProviderChange
            effortLabel="Effort"
            placeholderClassName="text-foreground/70"
            providerLabel="Provider"
            requireProviderForModelAndEffort
            selectClassName="h-12 rounded-2xl border-foreground/15 bg-white px-4 py-2 text-sm shadow-none hover:bg-white/95"
            showAdvancedFields={false}
            showCustomModelOption={false}
            showCustomProviderOption={false}
            showDescriptions={false}
            showProviderField={selectedRuntimeSupportsModelProvider}
            showRequiredIndicators={false}
            showProviderPlaceholderOption={false}
            showUnavailableEffortOptions={false}
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
  selectedRuntimeIds,
}: DefaultConfigStepProps) {
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
          <AgentDefaultsSection selectedRuntimeIds={selectedRuntimeIds} />
        </div>
      </div>

      <OnboardingFooter>
        <Button
          className={`${ONBOARDING_PRIMARY_CTA_CLASS} text-sm`}
          data-testid="onboarding-finish"
          onClick={actions.complete}
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
