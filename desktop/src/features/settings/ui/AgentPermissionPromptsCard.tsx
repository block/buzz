import * as React from "react";

import {
  useManagedAgentsQuery,
  useUpdateManagedAgentMutation,
} from "@/features/agents/hooks";
import { isWebBuild } from "@/features/bridge/ui/DesktopData";
import type { ManagedAgent } from "@/shared/api/types";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

const PERMISSION_MODE_ENV = "BUZZ_ACP_PERMISSION_MODE";

/**
 * Per-agent "Ask before using tools" switch. Sets the agent's BUZZ_ACP_PERMISSION_MODE=ask env var
 * (desktop: directly; web: via Direct with `permissionAsk`, which the desktop maps to the same var).
 * Takes effect the next time the agent starts.
 */
export function AgentPermissionPromptsCard() {
  const agentsQuery = useManagedAgentsQuery();
  const update = useUpdateManagedAgentMutation();
  const [pending, setPending] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const agents = (agentsQuery.data ?? []).filter(
    (a) => a.backend.type === "local",
  );
  if (agents.length === 0) return null;

  const toggle = async (agent: ManagedAgent, ask: boolean) => {
    setPending(agent.pubkey);
    setError(null);
    try {
      if (isWebBuild()) {
        await update.mutateAsync({ pubkey: agent.pubkey, permissionAsk: ask });
      } else {
        const envVars = { ...agent.envVars };
        if (ask) envVars[PERMISSION_MODE_ENV] = "ask";
        else delete envVars[PERMISSION_MODE_ENV];
        await update.mutateAsync({ pubkey: agent.pubkey, envVars });
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : "Couldn't update the agent.");
    } finally {
      setPending(null);
    }
  };

  return (
    <SettingsOptionGroup title="Permission prompts">
      <p
        className="px-4 pb-1 pt-2 text-sm text-muted-foreground/70"
        data-settings-subcopy
      >
        Agents with this on ask you before using tools, and you can approve from
        desktop or web. Changes apply the next time the agent starts.
      </p>
      {agents.map((agent) => {
        const id = `settings-permission-ask-${agent.pubkey.slice(0, 12)}`;
        // Web + a desktop without the Direct permissionAsk support: state unknown, can't change it from here.
        const unsupported = agent.permissionAsk === undefined;
        return (
          <SettingsOptionRow
            data-testid="settings-agent-permission-ask"
            key={agent.pubkey}
          >
            <div className="min-w-0">
              <label className="font-medium text-foreground" htmlFor={id}>
                {agent.name}
              </label>
              <p
                className="mt-0.5 text-sm text-muted-foreground/70"
                data-settings-subcopy
              >
                {unsupported
                  ? "Update Buzz desktop to change this from web"
                  : agent.permissionAsk
                    ? "Asks before using tools"
                    : "Uses tools without asking"}
                {agent.status === "running" && agent.needsRestart
                  ? " · restart to apply"
                  : ""}
              </p>
            </div>
            <Switch
              aria-label={`Ask before using tools: ${agent.name}`}
              checked={agent.permissionAsk === true}
              disabled={unsupported || pending === agent.pubkey}
              id={id}
              onCheckedChange={(checked) => {
                void toggle(agent, checked);
              }}
            />
          </SettingsOptionRow>
        );
      })}
      {error ? (
        <p className="px-4 pb-2 text-sm text-muted-foreground">{error}</p>
      ) : null}
    </SettingsOptionGroup>
  );
}
