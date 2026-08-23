import { invokeTauri } from "./tauri";

export type OllamaOwnershipMode =
  | "connect_only"
  | "external_managed_models"
  | "managed";

export type OllamaMachineConfig = {
  endpoint: string;
  mode: OllamaOwnershipMode;
  selectedModel?: string;
};

export type OllamaModel = {
  name: string;
  model: string;
  modifiedAt: string;
  size: number;
  digest: string;
  details: unknown;
};

export type OllamaStatus = {
  config: OllamaMachineConfig;
  reachable: boolean;
  version: string | null;
  models: OllamaModel[];
  error: string | null;
  managedRuntimeInstalled: boolean;
  managedRuntimeRunning: boolean;
  managedInstallSupported: boolean;
};

export type OllamaModelInfo = {
  model: string;
  capabilities: string[];
  supportsTools: boolean;
  details: unknown;
  modelInfo: unknown;
};

export type OllamaPullProgress = {
  model: string;
  status: string;
  digest: string | null;
  completed: number | null;
  total: number | null;
  done: boolean;
};

export const OLLAMA_PULL_PROGRESS_EVENT = "ollama-pull-progress";

export function getOllamaConfig(): Promise<OllamaMachineConfig> {
  return invokeTauri("get_ollama_config");
}

export function setOllamaConfig(
  config: OllamaMachineConfig,
): Promise<OllamaMachineConfig> {
  return invokeTauri("set_ollama_config", { config });
}

export function getOllamaStatus(): Promise<OllamaStatus> {
  return invokeTauri("get_ollama_status");
}

export function detectOllama(endpoint?: string): Promise<OllamaStatus> {
  return invokeTauri("detect_ollama", { endpoint });
}

export function showOllamaModel(model: string): Promise<OllamaModelInfo> {
  return invokeTauri("show_ollama_model", { model });
}

export function pullOllamaModel(model: string): Promise<void> {
  return invokeTauri("pull_ollama_model", { model });
}

export function deleteOllamaModel(model: string): Promise<void> {
  return invokeTauri("delete_ollama_model", {
    input: { model, confirmed: true },
  });
}

export function installManagedOllama(): Promise<void> {
  return invokeTauri("install_managed_ollama");
}

export function startManagedOllama(): Promise<OllamaStatus> {
  return invokeTauri("start_managed_ollama");
}

export function stopManagedOllama(): Promise<void> {
  return invokeTauri("stop_managed_ollama");
}
