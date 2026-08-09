import { invokeTauri } from "@/shared/api/tauri";

export type LmStudioReadiness = {
  status:
    | "app_missing"
    | "api_unreachable"
    | "auth_required"
    | "no_loaded_model"
    | "configured_model_unavailable"
    | "ready";
  detail: string;
  configuredModel: string | null;
  loadedModels: string[];
  securityWarnings: string[];
  /** The native API does not attest its listener bind address. */
  bindExposure: "unknown";
};

export function getLmStudioReadiness(): Promise<LmStudioReadiness> {
  return invokeTauri<LmStudioReadiness>("get_lmstudio_readiness");
}
