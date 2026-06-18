import { usePreventSleepContext } from "@/features/agents/usePreventSleep";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

export function PreventSleepSettingsCard() {
  const { enabled, setEnabled, hasRunningAgents, expired, clearExpired } =
    usePreventSleepContext();

  return (
    <section className="min-w-0" data-testid="settings-agents">
      <div className="mb-12 min-w-0">
        <h2 className="text-2xl font-semibold tracking-tight">Agents</h2>
        <p className="text-base font-normal text-muted-foreground">
          Settings that affect how local managed agents run on this machine.
        </p>
      </div>

      <SettingsOptionGroup>
        <SettingsOptionRow>
          <div className="min-w-0">
            <label
              className="text-sm font-medium"
              htmlFor="prevent-sleep-switch"
            >
              Keep awake while agents are active
            </label>
            <p className="text-sm font-normal text-muted-foreground">
              Prevents your computer from sleeping while local agents are
              running. Automatically releases when all agents stop or after 4
              hours.
            </p>
          </div>
          <Switch
            checked={enabled}
            data-testid="prevent-sleep-toggle"
            id="prevent-sleep-switch"
            onCheckedChange={(checked) => {
              if (expired) {
                clearExpired();
              }
              setEnabled(checked);
            }}
          />
        </SettingsOptionRow>
      </SettingsOptionGroup>

      {enabled && !hasRunningAgents && (
        <p className="mt-3 text-sm text-muted-foreground">
          Waiting for agents to start
        </p>
      )}

      {expired && (
        <p className="mt-3 rounded-xl border border-yellow-500/30 bg-yellow-500/10 px-3 py-2 text-sm text-yellow-700 dark:text-yellow-400">
          Sleep prevention expired after 4 hours. Toggle off and on to
          re-enable.
        </p>
      )}
    </section>
  );
}
