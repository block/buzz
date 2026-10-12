import type * as React from "react";
import { RuntimeIcon } from "@/features/onboarding/ui/RuntimeIcon";
import { AgentDefaultsEditor } from "@/features/agents/ui/AgentDefaultsEditor";
import { useAcpRuntimesQueryForced } from "@/features/agents/acpRuntimesQuery";
import { useGlobalAgentConfig } from "@/features/agents/useGlobalAgentConfig";
import { useDesktopData } from "@/features/bridge/ui/DesktopData";
import { SettingsOptionGroup } from "./SettingsOptionGroup";

/** Web: the desktop's defaults as plain text (read-only; they are changed on the desktop). */
function WebAgentDefaults() {
  const { globalConfig, isLoading } = useGlobalAgentConfig();
  const runtimes = useAcpRuntimesQueryForced().data ?? [];
  const rt = globalConfig.preferred_runtime;
  const entry = rt ? runtimes.find((r) => r.id === rt) : undefined;
  const harness: React.ReactNode = rt ? (
    <span className="inline-flex items-center gap-2">
      {entry ? <RuntimeIcon className="h-5 w-5" runtime={entry} /> : null}
      {entry?.label ?? rt}
    </span>
  ) : null;
  const rows: [string, React.ReactNode, string][] = [
    ["Default harness", harness, "agent-defaults-harness"],
    ["Model", globalConfig.model, "agent-defaults-model"],
  ];
  return (
    <div data-testid="agent-defaults-readonly">
      <dl className="divide-y divide-border/55 text-sm">
        {rows.map(([label, value, id]) => (
          <div key={id} className="flex items-center justify-between gap-4 py-2">
            <dt className="text-muted-foreground">{label}</dt>
            <dd className="truncate font-medium" data-testid={id}>
              {isLoading ? "…" : value || "Not set"}
            </dd>
          </div>
        ))}
      </dl>
      <p className="mt-3 text-xs text-muted-foreground" data-testid="agent-defaults-web-note">
        These are your desktop's defaults. Change them on your desktop in Settings → Agents.
      </p>
    </div>
  );
}

export function AgentDefaultsSettingsCard() {
  const { web, live } = useDesktopData();
  return (
    <SettingsOptionGroup
      data-testid="settings-global-agent-config"
      description="Provider, model, effort, and environment settings inherited by local agents. Agent-specific settings always take priority."
      title="Agent defaults"
    >
      <div className="px-4 py-4">
        {web && !live ? (
          <p className="text-sm text-muted-foreground" data-testid="agent-defaults-needs-direct">
            Your desktop's agent defaults appear here once this device has a Direct connection to it.
          </p>
        ) : web ? (
          <WebAgentDefaults />
        ) : (
          <AgentDefaultsEditor layout="flat" />
        )}
      </div>
    </SettingsOptionGroup>
  );
}
