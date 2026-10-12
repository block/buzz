import { AgentDefaultsEditor } from "@/features/agents/ui/AgentDefaultsEditor";
import { useDesktopData } from "@/features/bridge/ui/DesktopData";
import { SettingsOptionGroup } from "./SettingsOptionGroup";

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
        ) : (
          <>
            {web ? (
              <p className="mb-3 text-xs text-muted-foreground" data-testid="agent-defaults-web-note">
                These are your desktop's defaults. Provider, model and harness can be changed here; environment variable values stay on your desktop.
              </p>
            ) : null}
            {/* Remount when the connection goes live so the editor loads the desktop's values, not the empty ones. */}
            <AgentDefaultsEditor key={live ? "live" : "idle"} layout="flat" />
          </>
        )}
      </div>
    </SettingsOptionGroup>
  );
}
