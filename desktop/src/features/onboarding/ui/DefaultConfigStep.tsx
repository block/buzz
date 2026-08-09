import * as React from "react";

import {
  useAcpRuntimesQuery,
  useRuntimeFileConfigQuery,
} from "@/features/agents/hooks";
import {
  AgentConfigFields,
  EMPTY_GLOBAL_CONFIG,
} from "@/features/agents/ui/AgentConfigFields";
import { resetConfigForHarnessChange } from "@/features/agents/ui/agentConfigOptions";
import { AgentDropdownSelect } from "@/features/agents/ui/agentConfigControls";
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
import type { DefaultConfigDraft, DefaultConfigStepActions } from "./types";

type DefaultConfigStepProps = {
  actions: DefaultConfigStepActions;
  direction: OnboardingTransitionDirection;
  draft: DefaultConfigDraft | null;
  readyRuntimeIds: readonly string[];
};

function formatHarnessLabel(runtime: AcpRuntimeCatalogEntry | undefined) {
  if (!runtime) return "Select a harness";
  return runtime.id === "buzz-agent" ? "Buzz" : runtime.label;
}

function AgentDefaultsSection({
  draft,
  isPending,
  onDraftChange,
  onPersistenceStateChange,
  readyRuntimeIds,
}: {
  draft: DefaultConfigDraft | null;
  isPending: boolean;
  onDraftChange: (draft: DefaultConfigDraft) => void;
  onPersistenceStateChange: (state: {
    canComplete: boolean;
    commit: () => Promise<void>;
  }) => void;
  readyRuntimeIds: readonly string[];
}) {
  const runtimesQuery = useAcpRuntimesQuery();
  const initialDraftRef = React.useRef(draft);
  const [config, setConfig] = React.useState<GlobalAgentConfig>(
    initialDraftRef.current?.config ?? EMPTY_GLOBAL_CONFIG,
  );
  const [isLoading, setIsLoading] = React.useState(
    initialDraftRef.current === null,
  );
  const [isCustomProvider, setIsCustomProvider] = React.useState(
    initialDraftRef.current?.isCustomProvider ?? false,
  );
  const [isCustomModelEditing, setIsCustomModelEditing] = React.useState(
    initialDraftRef.current?.isCustomModelEditing ?? false,
  );
  const [bakedEnv, setBakedEnv] = React.useState<BakedEnvEntry[]>([]);
  const configRef = React.useRef<GlobalAgentConfig>(
    initialDraftRef.current?.config ?? EMPTY_GLOBAL_CONFIG,
  );
  const isDirtyRef = React.useRef(initialDraftRef.current?.isDirty ?? false);
  const [configIsValid, setConfigIsValid] = React.useState(false);

  React.useEffect(() => {
    let unmounted = false;

    async function loadDefaults() {
      const [configResult, bakedEnvResult] = await Promise.allSettled([
        getGlobalAgentConfig(),
        getBakedBuildEnv(),
      ]);

      if (unmounted) return;

      if (
        initialDraftRef.current === null &&
        configResult.status === "fulfilled"
      ) {
        configRef.current = configResult.value;
        setConfig(configResult.value);
      }
      if (bakedEnvResult.status === "fulfilled") {
        setBakedEnv(bakedEnvResult.value);
      }
      setIsLoading(false);
    }

    void loadDefaults();

    return () => {
      unmounted = true;
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
  const { data: runtimeFileConfig } =
    useRuntimeFileConfigQuery(selectedRuntimeId);
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

  const updateDraft = React.useCallback(
    (next: GlobalAgentConfig, overrides: Partial<DefaultConfigDraft> = {}) => {
      isDirtyRef.current = overrides.isDirty ?? true;
      configRef.current = next;
      setConfig(next);
      onDraftChange({
        config: next,
        isCustomModelEditing,
        isCustomProvider,
        isDirty: isDirtyRef.current,
        ...overrides,
      });
    },
    [isCustomModelEditing, isCustomProvider, onDraftChange],
  );

  const handleHarnessChange = React.useCallback(
    (runtimeId: string) => {
      const next = resetConfigForHarnessChange(config, runtimeId);
      setIsCustomModelEditing(false);
      setIsCustomProvider(false);
      updateDraft(next, {
        isCustomModelEditing: false,
        isCustomProvider: false,
      });
    },
    [config, updateDraft],
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

  const commitPersistence = React.useCallback(async () => {
    if (!isDirtyRef.current) return;
    const saved = await setGlobalAgentConfig(configRef.current);
    isDirtyRef.current = false;
    configRef.current = saved.config;
    setConfig(saved.config);
  }, []);
  React.useEffect(() => {
    onPersistenceStateChange({
      // configIsValid comes from AgentConfigFields' onValidityChange and
      // covers model + provider credentials — a harness selection alone is
      // not a working default (e.g. buzz-agent with no provider configured).
      canComplete: selectedRuntimeId.length > 0 && configIsValid,
      commit: commitPersistence,
    });
  }, [
    commitPersistence,
    configIsValid,
    onPersistenceStateChange,
    selectedRuntimeId,
  ]);

  return (
    <fieldset
      aria-busy={isPending}
      className="w-full space-y-4 text-left text-sm disabled:pointer-events-none disabled:opacity-70"
      disabled={isPending}
    >
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
            onConfigChange={updateDraft}
            onCustomModelEditingChange={(next) => {
              setIsCustomModelEditing(next);
              onDraftChange({
                config: configRef.current,
                isCustomModelEditing: next,
                isCustomProvider,
                isDirty: isDirtyRef.current,
              });
            }}
            onIsCustomProviderChange={(next) => {
              setIsCustomProvider(next);
              onDraftChange({
                config: configRef.current,
                isCustomModelEditing,
                isCustomProvider: next,
                isDirty: isDirtyRef.current,
              });
            }}
            onValidityChange={setConfigIsValid}
            placeholderClassName="text-foreground/70"
            runtimeFileConfig={runtimeFileConfig}
            selectClassName="h-12 rounded-2xl border-foreground/15 bg-white px-4 py-2 text-sm shadow-none hover:bg-white/95"
            disclosure="onboarding-essential"
            unstyled
            useCustomSelect
          />
        </div>
      )}
    </fieldset>
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
  draft,
  readyRuntimeIds,
}: DefaultConfigStepProps) {
  const [persistenceState, setPersistenceState] = React.useState<{
    canComplete: boolean;
    commit: () => Promise<void>;
  }>({ canComplete: false, commit: () => Promise.resolve() });
  const [isSaving, setIsSaving] = React.useState(false);
  const [saveError, setSaveError] = React.useState<string | null>(null);

  const handleComplete = React.useCallback(async () => {
    if (isSaving) return;
    setIsSaving(true);
    setSaveError(null);
    try {
      await persistenceState.commit();
      actions.discardDraft();
      actions.complete();
    } catch (cause) {
      setSaveError(
        cause instanceof Error
          ? cause.message
          : "Couldn’t save model settings.",
      );
    } finally {
      setIsSaving(false);
    }
  }, [actions, isSaving, persistenceState]);

  const handleSkip = React.useCallback(() => {
    actions.discardDraft();
    actions.complete();
  }, [actions]);

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
            draft={draft}
            isPending={isSaving}
            onDraftChange={actions.updateDraft}
            onPersistenceStateChange={setPersistenceState}
            readyRuntimeIds={readyRuntimeIds}
          />
        </div>
      </div>

      <OnboardingFooter>
        {/* Keep Next centered while the optional action sits beside it. */}
        <div className="relative flex items-center justify-center">
          <Button
            className={`${ONBOARDING_PRIMARY_CTA_CLASS} text-sm`}
            data-testid="onboarding-finish"
            disabled={!persistenceState.canComplete || isSaving}
            onClick={() => void handleComplete()}
            type="button"
          >
            {isSaving ? "Saving…" : "Next"}
          </Button>
          <Button
            className="absolute left-full ml-3 h-9 animate-in whitespace-nowrap rounded-full px-6 text-sm fade-in fill-mode-backwards [animation-delay:1000ms] animation-duration-[500ms] hover:bg-foreground/10 motion-reduce:animate-none"
            data-testid="onboarding-config-skip"
            disabled={isSaving}
            onClick={handleSkip}
            type="button"
            variant="ghost"
          >
            Skip for now
          </Button>
        </div>

        <Button
          className="h-9 rounded-full bg-foreground/10 px-6 text-sm hover:bg-foreground/15"
          data-testid="onboarding-back"
          disabled={isSaving}
          onClick={actions.back}
          type="button"
          variant="ghost"
        >
          Back
        </Button>

        {saveError ? (
          <p
            className="max-w-[440px] text-center text-xs text-destructive"
            data-testid="onboarding-config-save-error"
            role="alert"
          >
            Couldn’t save model settings. {saveError} Try again.
          </p>
        ) : null}

        <p className="text-xs text-foreground/50">
          Configure default models in{" "}
          <span className="text-foreground/70">Settings → Agents</span> after
          setup.
        </p>
      </OnboardingFooter>
    </OnboardingSlideTransition>
  );
}
