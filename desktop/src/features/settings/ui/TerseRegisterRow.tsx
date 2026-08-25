import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  globalAgentConfigQueryKey,
  useGlobalAgentConfig,
} from "@/features/agents/useGlobalAgentConfig";
import { setGlobalAgentConfig } from "@/shared/api/tauriGlobalAgentConfig";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionRow } from "./SettingsOptionGroup";
import {
  isTerseRegisterEnabled,
  withTerseRegister,
} from "./terseRegisterLogic";

/**
 * Experiments row: terse agent-to-agent register.
 *
 * Unlike manifest preview features (localStorage-backed, desktop-only UI
 * gates), this switch controls harness behavior. Its source of truth is the
 * global agent config's `BUZZ_ACP_TERSE_REGISTER` env var — the exact value
 * managed agents receive at spawn — so the UI state cannot drift from what
 * running agents were started with. Saving through `set_global_agent_config`
 * auto-restarts running local agents whose effective env changed, so the
 * toggle takes effect without manual restarts.
 */
export function TerseRegisterRow() {
  const { globalConfig, isLoading } = useGlobalAgentConfig();
  const queryClient = useQueryClient();

  const mutation = useMutation({
    mutationFn: (enabled: boolean) =>
      setGlobalAgentConfig(withTerseRegister(globalConfig, enabled)),
    onSuccess: (result) => {
      queryClient.setQueryData(globalAgentConfigQueryKey, result.config);
    },
    onError: (error) => {
      console.error("Failed to toggle terse agent register:", error);
    },
  });

  const enabled = isTerseRegisterEnabled(globalConfig);
  const switchId = "feature-toggle-terseRegister";

  return (
    <SettingsOptionRow>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium" id={`${switchId}-label`}>
          Terse agent-to-agent messages
        </p>
        <p className="text-xs text-muted-foreground/70" data-settings-subcopy>
          Agents drop social padding and write telegraphic English in agent-only
          conversations. Messages to humans keep the normal tone. Applies to
          managed agents and restarts running ones.
        </p>
      </div>
      <Switch
        aria-labelledby={`${switchId}-label`}
        checked={enabled}
        data-testid={switchId}
        disabled={isLoading || mutation.isPending}
        onCheckedChange={(value) => {
          mutation.mutate(value);
        }}
      />
    </SettingsOptionRow>
  );
}
