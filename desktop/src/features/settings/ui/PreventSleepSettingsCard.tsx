import { usePreventSleepContext } from "@/features/agents/usePreventSleep";
import type { UnaddressedChannelAgentMode } from "@/features/channels/lib/contextualAgentConversationPolicy";
import { useUnaddressedChannelAgentMode } from "@/features/channels/lib/unaddressedChannelAgentMode";
import {
  setPersistentAgentAudienceEnabled,
  usePersistentAgentAudience,
} from "@/features/messages/lib/persistentAgentAudience";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

const UNADDRESSED_MODE_OPTIONS: {
  value: UnaddressedChannelAgentMode;
  label: string;
}[] = [
  { value: "all-channel-agents", label: "Notify all channel agents" },
  { value: "mentions-only", label: "Mentions only" },
];

export function PreventSleepSettingsCard() {
  const { enabled, setEnabled, hasRunningAgents, expired, clearExpired } =
    usePreventSleepContext();
  const persistentAudience = usePersistentAgentAudience(null);
  const { mode: unaddressedMode, setMode: setUnaddressedMode } =
    useUnaddressedChannelAgentMode();

  return (
    <section className="min-w-0" data-testid="settings-agents">
      <SettingsSectionHeader
        title="Agents"
        description="Control how agents behave in conversations and run on this machine."
      />

      <SettingsOptionGroup>
        <SettingsOptionRow className="items-start">
          <div className="min-w-0 flex-1">
            <p
              className="text-sm font-medium"
              id="unaddressed-channel-agents-label"
            >
              Unaddressed channel messages
            </p>
            <p className="text-sm font-normal text-muted-foreground">
              When you post in a channel without @mentioning anyone, choose who
              is notified. Direct messages always address their current agent.
            </p>
            <div
              aria-labelledby="unaddressed-channel-agents-label"
              className="mt-3 flex flex-col gap-2"
              data-testid="unaddressed-channel-agent-mode"
              role="radiogroup"
            >
              {UNADDRESSED_MODE_OPTIONS.map((option) => (
                <label
                  className="flex cursor-pointer items-center gap-2 text-sm"
                  htmlFor={`unaddressed-mode-${option.value}`}
                  key={option.value}
                >
                  <input
                    checked={unaddressedMode === option.value}
                    className="size-4 accent-primary"
                    data-testid={`unaddressed-mode-${option.value}`}
                    id={`unaddressed-mode-${option.value}`}
                    name="unaddressed-channel-agent-mode"
                    onChange={() => setUnaddressedMode(option.value)}
                    type="radio"
                    value={option.value}
                  />
                  <span>{option.label}</span>
                </label>
              ))}
            </div>
          </div>
        </SettingsOptionRow>

        <SettingsOptionRow>
          <div className="min-w-0">
            <label
              className="text-sm font-medium"
              htmlFor="persistent-agent-audience-switch"
            >
              Keep addressed agents active
            </label>
            <p className="text-sm font-normal text-muted-foreground">
              Keep agents you address selected for future messages in the same
              channel or thread. Remove them from the composer at any time.
            </p>
          </div>
          <Switch
            checked={persistentAudience.enabled}
            data-testid="persistent-agent-audience-toggle"
            id="persistent-agent-audience-switch"
            onCheckedChange={setPersistentAgentAudienceEnabled}
          />
        </SettingsOptionRow>

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
              running. Automatically releases when all agents stop or after 1
              hour without agent activity.
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
          Sleep prevention expired after 1 hour without agent activity. It will
          resume on the next agent activity, or toggle off and on to re-enable
          now.
        </p>
      )}
    </section>
  );
}
