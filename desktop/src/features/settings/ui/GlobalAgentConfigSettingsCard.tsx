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

import {
  getGlobalAgentConfig,
  setGlobalAgentConfig,
} from "@/shared/api/tauriGlobalAgentConfig";
import type { GlobalAgentConfig } from "@/shared/api/types";
import { useAcpRuntimesQuery } from "@/features/agents/hooks";
import { EnvVarsEditor } from "@/features/agents/ui/EnvVarsEditor";
import {
  AUTO_PROVIDER_DROPDOWN_VALUE,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  getPersonaProviderOptions,
} from "@/features/agents/ui/personaDialogPickers";
import { AgentModelField } from "@/features/agents/ui/personaProviderModelFields";
import { usePersonaModelDiscovery } from "@/features/agents/ui/usePersonaModelDiscovery";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { SettingsSectionHeader } from "./SettingsSectionHeader";
import { SettingsOptionGroup } from "./SettingsOptionGroup";

const EMPTY_CONFIG: GlobalAgentConfig = {
  env_vars: {},
  provider: null,
  model: null,
};

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

  // Load on mount.
  React.useEffect(() => {
    let cancelled = false;
    getGlobalAgentConfig()
      .then((loaded) => {
        if (!cancelled) {
          setConfig(loaded);
          setIsLoading(false);
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
  }, []);

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
      if (savedTimerRef.current) clearTimeout(savedTimerRef.current);
      savedTimerRef.current = setTimeout(() => setSaveState("idle"), 2500);
    } catch (err) {
      setSaveState("error");
      setSaveError(typeof err === "string" ? err : "Failed to save.");
    }
  }

  const providerOptions = getPersonaProviderOptions(providerValue, "goose");
  const providerSelectValue = isCustomProvider
    ? CUSTOM_PROVIDER_DROPDOWN_VALUE
    : providerValue || AUTO_PROVIDER_DROPDOWN_VALUE;

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
                  {opt.label}
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

          {/* Env vars */}
          <div className="p-3">
            <EnvVarsEditor
              value={config.env_vars}
              onChange={handleEnvVarsChange}
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
              Saved
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
