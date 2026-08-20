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
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import * as React from "react";

import { useQueryClient } from "@tanstack/react-query";

import {
  getGlobalAgentConfig,
  setGlobalAgentConfig,
} from "@/shared/api/tauriGlobalAgentConfig";
import type { GlobalAgentConfig } from "@/shared/api/types";
import { getBakedBuildEnv, type BakedEnvEntry } from "@/shared/api/tauri";
import { globalAgentConfigQueryKey } from "@/features/agents/useGlobalAgentConfig";
import {
  useAcpRuntimesQuery,
  useRuntimeFileConfigQuery,
} from "@/features/agents/hooks";
import {
  formatRuntimeOptionLabel,
  getDefaultPersonaRuntime,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  resetConfigForHarnessChange,
  sortPersonaRuntimes,
} from "@/features/agents/ui/agentConfigOptions";
import { AgentDropdownSelect } from "@/features/agents/ui/agentConfigControls";
import {
  AgentConfigFields,
  EMPTY_GLOBAL_CONFIG,
} from "@/features/agents/ui/AgentConfigFields";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

type SaveState = "idle" | "saving" | "saved" | "error";

const PROGRESSIVE_FIELDS_TRANSITION = {
  duration: 0.22,
  ease: [0.23, 1, 0.32, 1],
} as const;

const PERSONA_SELECT_TRIGGER_CLASS = cn(
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  "h-11 px-3 py-2 leading-6 hover:bg-muted/40 focus:bg-muted/40 [&>svg]:text-muted-foreground/60",
);

export type GlobalAgentConfigSaveResult = Awaited<
  ReturnType<typeof setGlobalAgentConfig>
>;

type AgentDefaultsEditorProps = {
  layout?: "flat" | "grouped";
  onDirtyChange?: (dirty: boolean) => void;
  onSaveSuccess?: (result: GlobalAgentConfigSaveResult) => void;
  onSavingChange?: (saving: boolean) => void;
  secondaryAction?: React.ReactNode;
};

export function AgentDefaultsEditor({
  layout = "grouped",
  onDirtyChange,
  onSaveSuccess,
  onSavingChange,
  secondaryAction,
}: AgentDefaultsEditorProps) {
  const flatLayout = layout === "flat";
  const shouldReduceMotion = useReducedMotion();
  const [config, setConfig] =
    React.useState<GlobalAgentConfig>(EMPTY_GLOBAL_CONFIG);
  const configRef = React.useRef(config);
  const [dirty, setDirty] = React.useState(false);
  const [saveState, setSaveState] = React.useState<SaveState>("idle");
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [restartedCount, setRestartedCount] = React.useState(0);
  const [failedRestartCount, setFailedRestartCount] = React.useState(0);
  const [isLoading, setIsLoading] = React.useState(true);
  const [loadError, setLoadError] = React.useState(false);
  const [configIsValid, setConfigIsValid] = React.useState(true);
  const [isCustomProvider, setIsCustomProvider] = React.useState(false);
  const [isCustomModelEditing, setIsCustomModelEditing] = React.useState(false);
  const savedTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const queryClient = useQueryClient();
  const [bakedEnv, setBakedEnv] = React.useState<BakedEnvEntry[]>([]);

  React.useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);

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
          configRef.current = loaded;
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

  const runtimesQuery = useAcpRuntimesQuery();
  const sortedRuntimes = React.useMemo(
    () => sortPersonaRuntimes(runtimesQuery.data ?? []),
    [runtimesQuery.data],
  );
  // An unset preferred runtime uses the same Buzz Agent-first fallback as
  // deployment. The rendered draft below carries that fallback forward so the
  // next user edit persists the visible harness instead of saving null.
  const selectedRuntime = React.useMemo(() => {
    const configuredRuntime = sortedRuntimes.find(
      (runtime) => runtime.id === config.preferred_runtime,
    );
    return (
      configuredRuntime ??
      getDefaultPersonaRuntime(sortedRuntimes) ??
      sortedRuntimes[0]
    );
  }, [config.preferred_runtime, sortedRuntimes]);
  const renderedConfig = React.useMemo(
    () =>
      config.preferred_runtime || !selectedRuntime
        ? config
        : { ...config, preferred_runtime: selectedRuntime.id },
    [config, selectedRuntime],
  );
  const { data: runtimeFileConfig } = useRuntimeFileConfigQuery(
    selectedRuntime?.id ?? "",
  );
  const harnessOptions = React.useMemo(
    () =>
      sortedRuntimes.map((runtime) => ({
        label: formatRuntimeOptionLabel(runtime),
        value: runtime.id,
      })),
    [sortedRuntimes],
  );
  const configSurfaceLoading = isLoading || runtimesQuery.isLoading;
  const configSurfaceError =
    loadError ||
    runtimesQuery.isError ||
    (!configSurfaceLoading && sortedRuntimes.length === 0);

  function handleConfigChange(next: GlobalAgentConfig) {
    configRef.current = next;
    setConfig(next);
    setDirty(true);
    setSaveState("idle");
    setSaveError(null);
  }

  function handleHarnessChange(runtimeId: string) {
    handleConfigChange(resetConfigForHarnessChange(config, runtimeId));
    setConfigIsValid(false);
    setIsCustomModelEditing(false);
    setIsCustomProvider(false);
  }

  async function handleSave() {
    // Snapshot the config being submitted so we can detect edits that arrive
    // during the IPC round-trip and avoid clobbering the user's newer input.
    const submittedConfig = config;
    onSavingChange?.(true);
    setSaveState("saving");
    setSaveError(null);
    try {
      const result = await setGlobalAgentConfig(submittedConfig);
      // Apply the backend's canonical config ONLY if nothing changed during the
      // IPC window. If the user edited, keep their newer value and leave dirty=true
      // so they can save again. setDirty(false) runs inside the updater so both
      // state updates batch into the same render (React 18 automatic batching).
      const savedCurrentDraft = configRef.current === submittedConfig;
      setConfig((current) => {
        if (!savedCurrentDraft) {
          // Mid-flight edit detected — do not overwrite newer user input.
          return current;
        }
        configRef.current = result.config;
        setDirty(false);
        return result.config;
      });
      setRestartedCount(result.restarted_count);
      setFailedRestartCount(result.failed_restart_count);
      setSaveState("saved");
      // Seed the shared TanStack Query cache with the canonical saved value so
      // all open dialogs (and any that open afterward) see the new config
      // synchronously — no second IPC round-trip needed.
      queryClient.setQueryData(globalAgentConfigQueryKey, result.config);
      if (savedCurrentDraft) onSaveSuccess?.(result);
      if (savedTimerRef.current) clearTimeout(savedTimerRef.current);
      // Partial restart failures require an explicit acknowledgment in the
      // overlay, so keep their result visible instead of fading it away.
      if (result.failed_restart_count === 0) {
        savedTimerRef.current = setTimeout(() => setSaveState("idle"), 2500);
      }
    } catch (err) {
      setSaveState("error");
      setSaveError(typeof err === "string" ? err : "Couldn't save.");
    } finally {
      onSavingChange?.(false);
    }
  }

  const configFields = selectedRuntime ? (
    <AgentConfigFields
      bakedEnv={bakedEnv}
      selectedRuntime={selectedRuntime}
      config={renderedConfig}
      disclosure={flatLayout ? "progressive-defaults" : "full"}
      isCustomModelEditing={isCustomModelEditing}
      isCustomProvider={isCustomProvider}
      onConfigChange={handleConfigChange}
      onCustomModelEditingChange={setIsCustomModelEditing}
      onIsCustomProviderChange={setIsCustomProvider}
      onValidityChange={setConfigIsValid}
      placeholderClassName={flatLayout ? "text-muted-foreground/55" : undefined}
      runtimeFileConfig={runtimeFileConfig}
      key={selectedRuntime.id}
      selectClassName={flatLayout ? PERSONA_SELECT_TRIGGER_CLASS : undefined}
      unstyled={flatLayout}
      useCustomSelect
    />
  ) : null;
  const progressiveFieldsTransition = shouldReduceMotion
    ? { duration: 0 }
    : PROGRESSIVE_FIELDS_TRANSITION;

  return (
    <div className={cn("min-w-0", flatLayout ? "space-y-7" : "space-y-4")}>
      {configSurfaceLoading ? (
        <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
          <Loader className="size-4 animate-spin" />
          Loading…
        </div>
      ) : configSurfaceError ? (
        <div className="flex items-center gap-2 py-4 text-sm text-destructive">
          <AlertCircle className="size-4" />
          Couldn't load agent defaults. Restart the app to try again.
        </div>
      ) : (
        <>
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="global-agent-default-harness"
            >
              Default harness
            </label>
            <AgentDropdownSelect
              className={flatLayout ? PERSONA_SELECT_TRIGGER_CLASS : undefined}
              id="global-agent-default-harness"
              onValueChange={handleHarnessChange}
              options={harnessOptions}
              placeholder="Select a harness"
              placeholderClassName={
                flatLayout ? "text-muted-foreground/55" : undefined
              }
              testId="global-agent-default-harness"
              value={selectedRuntime?.id ?? ""}
            />
          </div>
          {flatLayout ? (
            <AnimatePresence initial={false}>
              {configFields ? (
                <motion.div
                  animate={{ height: "auto", opacity: 1 }}
                  className="overflow-hidden"
                  data-testid="global-agent-runtime-fields-motion"
                  exit={{ height: 0, opacity: 0 }}
                  initial={{ height: 0, opacity: 0 }}
                  key={selectedRuntime?.id}
                  transition={progressiveFieldsTransition}
                >
                  {configFields}
                </motion.div>
              ) : null}
            </AnimatePresence>
          ) : (
            configFields
          )}
        </>
      )}

      {/* Save bar */}
      {!configSurfaceLoading && !configSurfaceError && (
        <div className="mt-4 flex flex-wrap items-center gap-3">
          {saveState === "saved" && (
            <span className="flex min-w-0 items-center gap-1 text-sm text-green-600 dark:text-green-400">
              <Check className="size-3.5 shrink-0" />
              {restartedCount > 0
                ? `Saved. Restarted ${restartedCount} agent${restartedCount === 1 ? "" : "s"}.${failedRestartCount > 0 ? ` ${failedRestartCount} couldn't restart — check the Agents page.` : ""}`
                : failedRestartCount > 0
                  ? `Saved. ${failedRestartCount} agent${failedRestartCount === 1 ? "" : "s"} couldn't restart — check the Agents page.`
                  : "Saved."}
            </span>
          )}
          {saveState === "error" && saveError && (
            <span className="flex min-w-0 items-center gap-1 text-sm text-destructive">
              <AlertCircle className="size-3.5 shrink-0" />
              {saveError}
            </span>
          )}
          <div className="ml-auto flex items-center gap-3">
            {secondaryAction}
            <Button
              disabled={
                !dirty ||
                !configIsValid ||
                selectedRuntime === undefined ||
                saveState === "saving"
              }
              onClick={() => void handleSave()}
              size="sm"
            >
              {saveState === "saving" ? (
                <Loader className="mr-1.5 size-3.5 animate-spin" />
              ) : null}
              Save defaults
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
