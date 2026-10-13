import { ProtectedOpenClawWorkspaceSettingsCard } from "@protected-feature-components";
import { useDirectSession } from "@/features/bridge/ui/WebRemoteSettings";
import {
  DesktopDataNotice,
  isWebBuild,
  WebOpenClawStatusNote,
} from "@/features/bridge/ui/DesktopData";
import { AgentDefaultsSettingsCard } from "./AgentDefaultsSettingsCard";
import { AgentPermissionPromptsCard } from "./AgentPermissionPromptsCard";
import {
  setKeepMentionedAgentsPinned,
  useKeepMentionedAgentsPinned,
} from "@/features/messages/lib/autoPinMentionedAgentsPreference";
import {
  setShowAgentThinking,
  useShowAgentThinking,
} from "@/features/messages/lib/showAgentThinkingPreference";
import { Switch } from "@/shared/ui/switch";
import { HarnessesSettingsPanel } from "./HarnessesSettingsPanel";
import { PreventSleepSettingsCard } from "./PreventSleepSettingsCard";
import { RemoteAgentManagementCard } from "./RemoteAccessSettingsCard";
import {
  SettingsOptionGroup,
  SettingsOptionGroupList,
  SettingsOptionRow,
} from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

export function AgentsSettingsPanel() {
  const automaticallyMentionAgents = useKeepMentionedAgentsPinned();
  const showAgentThinking = useShowAgentThinking();
  const web = isWebBuild();
  // Same direct session as the Agents page (shared client, one signature, Via relay → Direct, Retry on decline), so opening
  // Settings → Agents directly loads the desktop's runtimes and defaults.
  if (web) useDirectSession(); // biome-ignore lint: web is constant for the app lifetime

  return (
    <section className="min-w-0" data-testid="settings-agents">
      <SettingsSectionHeader
        title="Agents"
        description="Control how agents behave in conversations and run on this machine."
      />

      <DesktopDataNotice />
      {web ? (
        <WebOpenClawStatusNote />
      ) : (
        <div className="mb-4">
          <ProtectedOpenClawWorkspaceSettingsCard />
        </div>
      )}

      <SettingsOptionGroupList>
        <SettingsOptionGroup title="Conversations">
          <SettingsOptionRow data-testid="settings-automatic-agent-mentions">
            <div className="min-w-0">
              <label
                className="font-medium text-foreground"
                htmlFor="settings-automatic-agent-mentions-switch"
              >
                Automatically mention agents
              </label>
              <p
                className="mt-0.5 text-sm text-muted-foreground/70"
                data-settings-subcopy
              >
                Address selected agents in thread replies
              </p>
            </div>
            <Switch
              aria-label="Automatically mention agents"
              checked={automaticallyMentionAgents}
              id="settings-automatic-agent-mentions-switch"
              onCheckedChange={setKeepMentionedAgentsPinned}
            />
          </SettingsOptionRow>
          <SettingsOptionRow data-testid="settings-show-agent-thinking">
            <div className="min-w-0">
              <label
                className="font-medium text-foreground"
                htmlFor="settings-show-agent-thinking-switch"
              >
                Show agent thinking
              </label>
              <p
                className="mt-0.5 text-sm text-muted-foreground/70"
                data-settings-subcopy
              >
                Duration, tokens, and thinking under each agent reply
              </p>
            </div>
            <Switch
              aria-label="Show agent thinking"
              checked={showAgentThinking}
              id="settings-show-agent-thinking-switch"
              onCheckedChange={setShowAgentThinking}
            />
          </SettingsOptionRow>
        </SettingsOptionGroup>
        {web ? null : <PreventSleepSettingsCard />}
        <RemoteAgentManagementCard />
        <HarnessesSettingsPanel />
        <AgentDefaultsSettingsCard />
        <AgentPermissionPromptsCard />
      </SettingsOptionGroupList>
    </section>
  );
}
