import * as React from "react";
import { listen } from "@tauri-apps/api/event";

import {
  deleteOllamaModel,
  detectOllama,
  getOllamaConfig,
  getOllamaStatus,
  installManagedOllama,
  OLLAMA_PULL_PROGRESS_EVENT,
  type OllamaMachineConfig,
  type OllamaModelInfo,
  type OllamaOwnershipMode,
  type OllamaPullProgress,
  type OllamaStatus,
  pullOllamaModel,
  setOllamaConfig,
  showOllamaModel,
  startManagedOllama,
  stopManagedOllama,
} from "@/shared/api/tauriOllama";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { ollamaEndpointSecurityWarning } from "../lib/ollamaEndpointSecurity";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

const DEFAULT_CONFIG: OllamaMachineConfig = {
  endpoint: "http://127.0.0.1:11434",
  mode: "connect_only",
};

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 ** 2) return `${(value / 1024).toFixed(1)} KB`;
  if (value < 1024 ** 3) return `${(value / 1024 ** 2).toFixed(1)} MB`;
  return `${(value / 1024 ** 3).toFixed(1)} GB`;
}

function modeDescription(mode: OllamaOwnershipMode): string {
  switch (mode) {
    case "connect_only":
      return "Use an existing Ollama daemon without changing its models or process.";
    case "external_managed_models":
      return "Use an existing daemon and allow Buzz to pull or remove models.";
    case "managed":
      return "Use Buzz's private runtime, process, and model directory. It starts when you click Start or a local Ollama agent needs it, and stops when Buzz exits; app-launch startup and runtime removal are not available yet.";
  }
}

/** Machine-level Ollama lifecycle settings, independent of any one agent. */
export function OllamaSettingsCard() {
  const [config, setConfig] =
    React.useState<OllamaMachineConfig>(DEFAULT_CONFIG);
  const [status, setStatus] = React.useState<OllamaStatus | null>(null);
  const [model, setModel] = React.useState("");
  const [progress, setProgress] = React.useState<OllamaPullProgress | null>(
    null,
  );
  const [modelInfo, setModelInfo] = React.useState<
    Record<string, OllamaModelInfo>
  >({});
  const [busy, setBusy] = React.useState(false);
  const [message, setMessage] = React.useState<string | null>(null);

  const refresh = React.useCallback(async () => {
    const next = await getOllamaStatus();
    setStatus(next);
    setConfig(next.config);
  }, []);

  React.useEffect(() => {
    let cancelled = false;
    void getOllamaConfig()
      .then((value) => {
        if (!cancelled) setConfig(value);
      })
      .then(() => refresh())
      .catch((error) => {
        if (!cancelled) setMessage(errorMessage(error));
      });
    return () => {
      cancelled = true;
    };
  }, [refresh]);

  React.useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<OllamaPullProgress>(OLLAMA_PULL_PROGRESS_EVENT, ({ payload }) =>
      setProgress(payload),
    ).then((stop) => {
      unlisten = stop;
    });
    return () => unlisten?.();
  }, []);

  const run = React.useCallback(async (action: () => Promise<unknown>) => {
    setBusy(true);
    setMessage(null);
    try {
      await action();
    } catch (error) {
      setMessage(errorMessage(error));
    } finally {
      setBusy(false);
    }
  }, []);

  async function saveAndDetect() {
    const saved = await setOllamaConfig(config);
    setConfig(saved);
    const next = await detectOllama(saved.endpoint);
    setStatus(next);
    setMessage(
      next.reachable
        ? `Connected to Ollama ${next.version ?? ""}`.trim()
        : next.error,
    );
  }

  async function pullModel() {
    setProgress(null);
    await pullOllamaModel(model);
    setModel("");
    await refresh();
  }

  const canManageModels =
    config.mode === "external_managed_models" || config.mode === "managed";
  const endpointSecurityWarning = ollamaEndpointSecurityWarning(
    config.endpoint,
  );

  return (
    <SettingsOptionGroup
      data-testid="ollama-settings-card"
      description="Connect to your installation, let Buzz manage its models, or use a private Buzz runtime."
      title="Ollama"
    >
      <SettingsOptionRow className="items-start">
        <div className="min-w-0 flex-1 space-y-2">
          <label className="text-sm font-medium" htmlFor="ollama-mode">
            Ownership
          </label>
          <select
            className="h-9 w-full rounded-lg border border-input/40 bg-background px-3 text-sm"
            disabled={busy}
            id="ollama-mode"
            onChange={(event) =>
              setConfig((current) => ({
                ...current,
                endpoint:
                  event.target.value === "managed"
                    ? DEFAULT_CONFIG.endpoint
                    : current.endpoint,
                mode: event.target.value as OllamaOwnershipMode,
              }))
            }
            value={config.mode}
          >
            <option value="connect_only">Connect only</option>
            <option value="external_managed_models">
              Connect and manage models
            </option>
            <option
              disabled={
                status != null &&
                !status.managedInstallSupported &&
                !status.managedRuntimeInstalled
              }
              value="managed"
            >
              {status != null &&
              !status.managedInstallSupported &&
              !status.managedRuntimeInstalled
                ? "Fully managed (unavailable in this build)"
                : "Fully managed by Buzz"}
            </option>
          </select>
          <p className="text-sm text-muted-foreground/70">
            {modeDescription(config.mode)}
          </p>
        </div>
      </SettingsOptionRow>

      <SettingsOptionRow className="items-end">
        <div className="min-w-0 flex-1 space-y-2">
          <label className="text-sm font-medium" htmlFor="ollama-endpoint">
            Server address
          </label>
          <Input
            disabled={busy || config.mode === "managed"}
            id="ollama-endpoint"
            onChange={(event) =>
              setConfig((current) => ({
                ...current,
                endpoint: event.target.value,
              }))
            }
            placeholder="http://127.0.0.1:11434"
            value={config.endpoint}
          />
          {endpointSecurityWarning ? (
            <p className="text-sm text-yellow-700 dark:text-yellow-400">
              {endpointSecurityWarning}
            </p>
          ) : null}
        </div>
        <Button
          disabled={busy}
          onClick={() => void run(saveAndDetect)}
          variant="outline"
        >
          Save and test
        </Button>
      </SettingsOptionRow>

      {config.mode === "managed" ? (
        <SettingsOptionRow>
          <div className="min-w-0">
            <p className="text-sm font-medium">Private runtime</p>
            <p className="text-sm text-muted-foreground/70">
              {status?.managedInstallSupported
                ? status.managedRuntimeInstalled
                  ? status.managedRuntimeRunning
                    ? "Running on this computer"
                    : "Installed but stopped"
                  : "Ready to install"
                : "This build has no verified Ollama runtime artifact. You can still connect to an existing installation."}
            </p>
          </div>
          <div className="flex gap-2">
            {!status?.managedRuntimeInstalled ? (
              <Button
                disabled={busy || !status?.managedInstallSupported}
                onClick={() =>
                  void run(async () => {
                    await installManagedOllama();
                    await refresh();
                  })
                }
                variant="outline"
              >
                Install
              </Button>
            ) : status.managedRuntimeRunning ? (
              <Button
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await stopManagedOllama();
                    await refresh();
                  })
                }
                variant="outline"
              >
                Stop
              </Button>
            ) : (
              <Button
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await startManagedOllama();
                    await refresh();
                  })
                }
              >
                Start
              </Button>
            )}
          </div>
        </SettingsOptionRow>
      ) : null}

      {canManageModels ? (
        <SettingsOptionRow className="items-end">
          <div className="min-w-0 flex-1 space-y-2">
            <label className="text-sm font-medium" htmlFor="ollama-pull-model">
              Add a model
            </label>
            <Input
              disabled={busy}
              id="ollama-pull-model"
              onChange={(event) => setModel(event.target.value)}
              placeholder="qwen3:8b"
              value={model}
            />
            {progress ? (
              <p className="text-sm text-muted-foreground/70">
                {progress.status}
                {progress.completed != null && progress.total
                  ? ` · ${Math.round((progress.completed / progress.total) * 100)}%`
                  : ""}
              </p>
            ) : null}
          </div>
          <Button
            disabled={busy || model.trim() === ""}
            onClick={() => void run(pullModel)}
          >
            Pull
          </Button>
        </SettingsOptionRow>
      ) : null}

      {status?.reachable ? (
        <div className="divide-y divide-border/55">
          {status.models.length === 0 ? (
            <SettingsOptionRow>
              <p className="text-sm text-muted-foreground">
                No models installed
              </p>
            </SettingsOptionRow>
          ) : (
            status.models.map((installed) => (
              <SettingsOptionRow key={installed.digest || installed.name}>
                <div className="min-w-0">
                  <p className="truncate text-sm font-medium">
                    {installed.name}
                  </p>
                  <p className="text-sm text-muted-foreground/70">
                    {formatBytes(installed.size)}
                    {modelInfo[installed.name]
                      ? modelInfo[installed.name].supportsTools
                        ? " · Agent tools supported"
                        : " · Agent tools not reported"
                      : ""}
                  </p>
                </div>
                <div className="flex gap-2">
                  <Button
                    disabled={busy}
                    onClick={() =>
                      void run(async () => {
                        const info = await showOllamaModel(installed.name);
                        setModelInfo((current) => ({
                          ...current,
                          [installed.name]: info,
                        }));
                      })
                    }
                    size="sm"
                    variant="outline"
                  >
                    Inspect
                  </Button>
                  {canManageModels ? (
                    <Button
                      disabled={busy}
                      onClick={() => {
                        if (
                          !window.confirm(
                            `Remove ${installed.name} from this Ollama installation?`,
                          )
                        ) {
                          return;
                        }
                        void run(async () => {
                          await deleteOllamaModel(installed.name);
                          await refresh();
                        });
                      }}
                      size="sm"
                      variant="destructive"
                    >
                      Remove
                    </Button>
                  ) : null}
                </div>
              </SettingsOptionRow>
            ))
          )}
        </div>
      ) : null}

      {message ? (
        <div className="px-4 py-3 text-sm text-muted-foreground" role="status">
          {message}
        </div>
      ) : null}
    </SettingsOptionGroup>
  );
}
