/**
 * Settings card for global agent configuration defaults.
 *
 * Lets the user set env vars, provider, and model that apply to ALL local
 * agents as the lowest-precedence user layer. Per-agent and persona configs
 * always win on collision.
 *
 * Precedence: baked floor < GLOBAL (this card) < persona < per-agent.
 */
import { AlertCircle, Check, Loader } from "lucide-react";
import * as React from "react";

import { useQueryClient } from "@tanstack/react-query";

import {
  getGlobalAgentConfig,
  setGlobalAgentConfig,
} from "@/shared/api/tauriGlobalAgentConfig";
import type { GlobalAgentConfig } from "@/shared/api/types";
import { getBakedBuildEnv, type BakedEnvEntry } from "@/shared/api/tauri";
import { globalAgentConfigQueryKey } from "@/features/agents/useGlobalAgentConfig";
import { useAcpRuntimesQuery } from "@/features/agents/hooks";
import { EnvVarsEditor } from "@/features/agents/ui/EnvVarsEditor";
import type { InheritedEnvRow } from "@/features/agents/ui/EnvVarsEditor";
import { getBakedProviderInheritLabel } from "@/features/agents/ui/bakedEnvHelpers";
import {
  AUTO_PROVIDER_DROPDOWN_VALUE,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  getPersonaProviderOptions,
} from "@/features/agents/ui/personaDialogPickers";
import { AgentModelField } from "@/features/agents/ui/personaProviderModelFields";
import { usePersonaModelDiscovery } from "@/features/agents/ui/usePersonaModelDiscovery";
import {
  BUZZ_AGENT_THINKING_EFFORT,
  getProviderEffortConfig,
} from "@/features/agents/ui/buzzAgentConfig";
import {
  EffortSelectField,
  useEffortAutoClear,
} from "@/features/agents/ui/buzzAgentModelTuningFields";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { SettingsSectionHeader } from "./SettingsSectionHeader";
import { SettingsOptionGroup } from "./SettingsOptionGroup";

const EMPTY_CONFIG: GlobalAgentConfig = {
  env_vars: {},
  provider: null,
  model: null,
};

/**
 * Baked env keys that map to structured fields (provider/model/effort dropdowns).
 * These are routed to their own UI controls and must NOT appear as generic
 * inherited rows in the env editor.
 */
const BAKED_STRUCTURED_KEYS = new Set([
  "BUZZ_AGENT_PROVIDER",
  "BUZZ_AGENT_MODEL",
  BUZZ_AGENT_THINKING_EFFORT,
]);

type SaveState = "idle" | "saving" | "saved" | "error";

export function GlobalAgentConfigSettingsCard() {
  const [config, setConfig] = React.useState<GlobalAgentConfig>(EMPTY_CONFIG);
  const [dirty, setDirty] = React.useState(false);
  const [saveState, setSaveState] = React.useState<SaveState>("idle");
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [isLoading, setIsLoading] = React.useState(true);
  const [loadError, setLoadError] = React.useState(false);
  const [isCustomProvider, setIsCustomProvider] = React.useState(false);
  const [isCustomModelEditing, setIsCustomModelEditing] = React.useState(false);
  const savedTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const queryClient = useQueryClient();
  const [bakedEnv, setBakedEnv] = React.useState<BakedEnvEntry[]>([]);

  // Load on mount — seed the shared TanStack Query cache so any dialog that
  // opens after this point reads the populated value on first render (no async
  // race). The query is also backed by its own queryFn for first-consumer
  // scenarios, but this eager seed eliminates the "settings card loaded, user
  // opens Create Agent before the lazy query fires" window.
  React.useEffect(() => {
    let cancelled = false;
    getGlobalAgentConfig()
      .then((loaded) => {
        if (!cancelled) {
          setConfig(loaded);
          setIsLoading(false);
          queryClient.setQueryData(globalAgentConfigQueryKey, loaded);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setIsLoading(false);
          setLoadError(true);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [queryClient]);

  // Load baked build env once on mount. OSS builds return [] — the section
  // stays hidden. Failures are silently swallowed (non-critical display data).
  React.useEffect(() => {
    getBakedBuildEnv()
      .then(setBakedEnv)
      .catch(() => {
        // non-critical — leave bakedEnv empty
      });
  }, []);

  // Derive structured-field values and generic env rows from bakedEnv.
  // Structured keys (BUZZ_AGENT_PROVIDER / BUZZ_AGENT_MODEL / BUZZ_AGENT_THINKING_EFFORT)
  // route to their respective dropdown controls and are excluded from the
  // generic inherited-rows list passed to EnvVarsEditor.
  const bakedProvider = React.useMemo(
    () => bakedEnv.find((e) => e.key === "BUZZ_AGENT_PROVIDER")?.value ?? null,
    [bakedEnv],
  );
  const bakedModel = React.useMemo(
    () => bakedEnv.find((e) => e.key === "BUZZ_AGENT_MODEL")?.value ?? null,
    [bakedEnv],
  );
  const bakedEffort = React.useMemo(
    () =>
      bakedEnv.find((e) => e.key === BUZZ_AGENT_THINKING_EFFORT)?.value ?? null,
    [bakedEnv],
  );
  const bakedGenericRows = React.useMemo<readonly InheritedEnvRow[]>(
    () => bakedEnv.filter((e) => !BAKED_STRUCTURED_KEYS.has(e.key)),
    [bakedEnv],
  );

  // Resolve the buzz-agent runtime catalog entry for model discovery.
  // The card is always visible (open=true), so the query is always enabled.
  const runtimesQuery = useAcpRuntimesQuery();
  const buzzAgentRuntime = React.useMemo(
    () => (runtimesQuery.data ?? []).find((r) => r.id === "buzz-agent"),
    [runtimesQuery.data],
  );

  // Provider value used for discovery — empty string when custom provider text
  // field is being edited (discovery can't run against a partial/uncommitted value).
  const providerValue = config.provider ?? "";
  const providerForDiscovery = isCustomProvider ? "" : providerValue;

  const {
    discoveredModelOptions,
    modelDiscoveryLoading,
    modelDiscoveryStatus,
  } = usePersonaModelDiscovery({
    envVars: config.env_vars,
    isCustomProviderEditing: isCustomProvider,
    modelFieldVisible: true,
    open: true,
    provider: providerForDiscovery,
    selectedRuntime: buzzAgentRuntime,
  });

  // Auto-clear BUZZ_AGENT_THINKING_EFFORT when provider/model change makes the
  // current value invalid. Prevents stale invalid values from being saved.
  const currentEffortForAutoClear =
    config.env_vars[BUZZ_AGENT_THINKING_EFFORT] ?? "";
  const { validValues: effortValidForAutoClear } = getProviderEffortConfig(
    config.provider ?? "",
    config.model ?? "",
  );
  useEffortAutoClear({
    currentEffort: currentEffortForAutoClear,
    effortValid: effortValidForAutoClear,
    onClear: () => {
      setConfig((prev) => {
        const nextEnvVars = { ...prev.env_vars };
        delete nextEnvVars[BUZZ_AGENT_THINKING_EFFORT];
        return { ...prev, env_vars: nextEnvVars };
      });
      setDirty(true);
    },
  });

  function handleEnvVarsChange(next: Record<string, string>) {
    setConfig((prev) => ({ ...prev, env_vars: next }));
    setDirty(true);
    setSaveState("idle");
    setSaveError(null);
  }

  function handleProviderChange(value: string) {
    if (value === CUSTOM_PROVIDER_DROPDOWN_VALUE) {
      setIsCustomProvider(true);
      return;
    }
    if (value === AUTO_PROVIDER_DROPDOWN_VALUE || value === "") {
      setIsCustomProvider(false);
      setConfig((prev) => ({ ...prev, provider: null }));
    } else {
      setIsCustomProvider(false);
      setConfig((prev) => ({ ...prev, provider: value }));
    }
    setDirty(true);
    setSaveState("idle");
    setSaveError(null);
  }

  function handleCustomProviderInput(value: string) {
    setConfig((prev) => ({ ...prev, provider: value || null }));
    setDirty(true);
    setSaveState("idle");
    setSaveError(null);
  }

  function handleModelChange(value: string) {
    setConfig((prev) => ({ ...prev, model: value || null }));
    setDirty(true);
    setSaveState("idle");
    setSaveError(null);
  }

  async function handleSave() {
    setSaveState("saving");
    setSaveError(null);
    try {
      const saved = await setGlobalAgentConfig(config);
      setConfig(saved);
      setDirty(false);
      setSaveState("saved");
      // Seed the shared TanStack Query cache with the canonical saved value so
      // all open dialogs (and any that open afterward) see the new config
      // synchronously — no second IPC round-trip needed.
      queryClient.setQueryData(globalAgentConfigQueryKey, saved);
      if (savedTimerRef.current) clearTimeout(savedTimerRef.current);
      savedTimerRef.current = setTimeout(() => setSaveState("idle"), 2500);
    } catch (err) {
      setSaveState("error");
      setSaveError(typeof err === "string" ? err : "Failed to save.");
    }
  }

  const providerOptions = getPersonaProviderOptions(
    providerValue,
    "buzz-agent",
  );
  const providerSelectValue = isCustomProvider
    ? CUSTOM_PROVIDER_DROPDOWN_VALUE
    : providerValue || AUTO_PROVIDER_DROPDOWN_VALUE;

  // When a baked provider is present and no explicit global provider is set,
  // relabel the zero-value option to surface the inherited-from-build value.
  // When an explicit provider IS set, the zero-value option is still shown in
  // the list but never selected — its label doesn't matter.
  const providerZeroLabel = React.useMemo(() => {
    if (!bakedProvider) return null;
    return getBakedProviderInheritLabel(bakedProvider, providerOptions);
  }, [bakedProvider, providerOptions]);

  return (
    <section className="min-w-0" data-testid="settings-global-agent-config">
      <SettingsSectionHeader
        title="Agent defaults"
        description="Global configuration inherited by all local agents. Per-agent and persona settings always take priority."
      />

      {isLoading ? (
        <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
          <Loader className="size-4 animate-spin" />
          Loading…
        </div>
      ) : loadError ? (
        <div className="flex items-center gap-2 py-4 text-sm text-destructive">
          <AlertCircle className="size-4" />
          Failed to load agent defaults. Restart the app to try again.
        </div>
      ) : (
        <SettingsOptionGroup>
          {/* Provider field */}
          <div className="space-y-1.5 p-3">
            <label
              className="text-sm font-medium"
              htmlFor="global-agent-provider"
            >
              Default LLM provider
            </label>
            <select
              className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
              id="global-agent-provider"
              onChange={(e) => handleProviderChange(e.target.value)}
              value={providerSelectValue}
            >
              {providerOptions.map((opt) => (
                <option
                  key={opt.id}
                  value={opt.id || AUTO_PROVIDER_DROPDOWN_VALUE}
                >
                  {opt.id === ""
                    ? (providerZeroLabel ?? opt.label)
                    : opt.label}
                </option>
              ))}
              <option value={CUSTOM_PROVIDER_DROPDOWN_VALUE}>
                Custom provider…
              </option>
            </select>
            {isCustomProvider ? (
              <Input
                aria-label="Custom global provider ID"
                autoCorrect="off"
                onChange={(e) => handleCustomProviderInput(e.target.value)}
                placeholder="Custom provider ID"
                value={providerValue}
              />
            ) : null}
            <p className="text-xs text-muted-foreground">
              Applies to all agents that don't have a per-agent provider set.
            </p>
          </div>

          {/* Model field */}
          <div className="space-y-1.5 p-3">
            <AgentModelField
              disabled={false}
              discoveredModelOptions={discoveredModelOptions}
              globalModel={bakedModel ?? undefined}
              id="global-agent-model"
              isCustomModelEditing={isCustomModelEditing}
              isRequired={false}
              model={config.model ?? ""}
              modelDiscoveryLoading={modelDiscoveryLoading}
              modelDiscoveryStatus={modelDiscoveryStatus}
              onIsCustomModelEditingChange={setIsCustomModelEditing}
              onModelChange={(value) => handleModelChange(value)}
            />
            <p className="text-xs text-muted-foreground">
              Applies to all agents that don't have a per-agent model set.
            </p>
          </div>

          {/* Thinking / Effort — tier-1 dropdown, single editable surface for BUZZ_AGENT_THINKING_EFFORT */}
          <div className="p-3">
            {(() => {
              const { validValues: effortValid, defaultValue: effortDefault } =
                getProviderEffortConfig(
                  config.provider ?? "",
                  config.model ?? "",
                );
              const currentEffort =
                config.env_vars[BUZZ_AGENT_THINKING_EFFORT] ?? "";
              return (
                <>
                  <EffortSelectField
                    currentEffort={currentEffort}
                    effortDefault={effortDefault}
                    effortValid={effortValid}
                    htmlFor="global-agent-thinking-effort"
                    inheritedEffort={bakedEffort ?? undefined}
                    label="Default thinking / effort"
                    onChange={(value) => {
                      setConfig((prev) => {
                        const nextEnvVars = { ...prev.env_vars };
                        if (value === "") {
                          delete nextEnvVars[BUZZ_AGENT_THINKING_EFFORT];
                        } else {
                          nextEnvVars[BUZZ_AGENT_THINKING_EFFORT] = value;
                        }
                        return { ...prev, env_vars: nextEnvVars };
                      });
                      setDirty(true);
                      setSaveState("idle");
                      setSaveError(null);
                    }}
                    testId="global-agent-thinking-effort-select"
                  />
                  <p className="text-xs text-muted-foreground mt-1.5">
                    Default thinking/reasoning effort applied to all agents.
                    Per-agent settings override this.
                  </p>
                </>
              );
            })()}
          </div>

          {/* Env vars */}
          <div className="p-3">
            <EnvVarsEditor
              value={Object.fromEntries(
                Object.entries(config.env_vars).filter(
                  ([k]) => k !== BUZZ_AGENT_THINKING_EFFORT,
                ),
              )}
              onChange={(next) => {
                // Merge with the thinking-effort value managed by the tier-1
                // dropdown above, preserving it across raw env-var edits.
                const effort = config.env_vars[BUZZ_AGENT_THINKING_EFFORT];
                const merged =
                  effort !== undefined
                    ? { ...next, [BUZZ_AGENT_THINKING_EFFORT]: effort }
                    : next;
                handleEnvVarsChange(merged);
              }}
              inheritedRows={bakedGenericRows}
              inheritedRowsLabel="build"
              label="Global environment variables"
              helperText="Injected into all agents as the lowest-priority layer. Per-agent values override these."
            />
          </div>
        </SettingsOptionGroup>
      )}

      {/* Save bar */}
      {!isLoading && !loadError && (
        <div className="mt-4 flex items-center gap-3">
          <Button
            disabled={!dirty || saveState === "saving"}
            onClick={() => void handleSave()}
            size="sm"
          >
            {saveState === "saving" ? (
              <Loader className="mr-1.5 size-3.5 animate-spin" />
            ) : null}
            Save defaults
          </Button>
          {saveState === "saved" && (
            <span className="flex items-center gap-1 text-sm text-green-600 dark:text-green-400">
              <Check className="size-3.5" />
              Saved. Running agents keep their current settings until restarted.
            </span>
          )}
          {saveState === "error" && saveError && (
            <span className="flex items-center gap-1 text-sm text-destructive">
              <AlertCircle className="size-3.5" />
              {saveError}
            </span>
          )}
        </div>
      )}
    </section>
  );
}
