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
import {
  GlobalAgentConfigFields,
  EMPTY_GLOBAL_CONFIG,
} from "@/features/agents/ui/GlobalAgentConfigFields";
import { Button } from "@/shared/ui/button";

type SaveState = "idle" | "saving" | "saved" | "error";

export type GlobalAgentConfigSaveResult = Awaited<
  ReturnType<typeof setGlobalAgentConfig>
>;

type GlobalAgentConfigEditorProps = {
  onDirtyChange?: (dirty: boolean) => void;
  onSaveSuccess?: (result: GlobalAgentConfigSaveResult) => void;
  onSavingChange?: (saving: boolean) => void;
  secondaryAction?: React.ReactNode;
};

export function GlobalAgentConfigEditor({
  onDirtyChange,
  onSaveSuccess,
  onSavingChange,
  secondaryAction,
}: GlobalAgentConfigEditorProps) {
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

  // Resolve the buzz-agent runtime catalog entry for model discovery.
  const runtimesQuery = useAcpRuntimesQuery();
  const buzzAgentRuntime = React.useMemo(
    () => (runtimesQuery.data ?? []).find((r) => r.id === "buzz-agent"),
    [runtimesQuery.data],
  );

  function handleConfigChange(next: GlobalAgentConfig) {
    configRef.current = next;
    setConfig(next);
    setDirty(true);
    setSaveState("idle");
    setSaveError(null);
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
      setSaveError(typeof err === "string" ? err : "Failed to save.");
    } finally {
      onSavingChange?.(false);
    }
  }

  return (
    <div className="min-w-0 space-y-4">
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
        <GlobalAgentConfigFields
          bakedEnv={bakedEnv}
          buzzAgentRuntime={buzzAgentRuntime}
          config={config}
          isCustomModelEditing={isCustomModelEditing}
          isCustomProvider={isCustomProvider}
          onConfigChange={handleConfigChange}
          onCustomModelEditingChange={setIsCustomModelEditing}
          onIsCustomProviderChange={setIsCustomProvider}
          onValidityChange={setConfigIsValid}
        />
      )}

      {/* Save bar */}
      {!isLoading && !loadError && (
        <div className="mt-4 flex flex-wrap items-center gap-3">
          {saveState === "saved" && (
            <span className="flex min-w-0 items-center gap-1 text-sm text-green-600 dark:text-green-400">
              <Check className="size-3.5 shrink-0" />
              {restartedCount > 0
                ? `Saved. Restarted ${restartedCount} agent${restartedCount === 1 ? "" : "s"}.${failedRestartCount > 0 ? ` ${failedRestartCount} failed to restart — check the Agents tab.` : ""}`
                : failedRestartCount > 0
                  ? `Saved. ${failedRestartCount} agent${failedRestartCount === 1 ? "" : "s"} failed to restart — check the Agents tab.`
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
              disabled={!dirty || !configIsValid || saveState === "saving"}
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
