import {
  setShowJoinLeaveMessagesEnabled,
  useShowJoinLeaveMessages,
} from "@/features/messages/lib/showJoinLeaveMessages";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

export function MessageDisplaySettingsCard() {
  const showJoinLeaveMessages = useShowJoinLeaveMessages();

  return (
    <section className="min-w-0" data-testid="settings-message-display">
      <SettingsSectionHeader
        title="Messages"
        description="Control which system messages appear in channel timelines on this device."
      />

      <SettingsOptionGroup>
        <SettingsOptionRow>
          <div className="min-w-0">
            <label
              className="text-sm font-medium"
              htmlFor="show-join-leave-messages-switch"
            >
              Show join and leave messages
            </label>
            <p className="text-sm font-normal text-muted-foreground">
              Show "joined", "added", "left", and "removed" messages in channel
              timelines. Member lists stay up to date either way.
            </p>
          </div>
          <Switch
            checked={showJoinLeaveMessages}
            data-testid="show-join-leave-messages-toggle"
            id="show-join-leave-messages-switch"
            onCheckedChange={setShowJoinLeaveMessagesEnabled}
          />
        </SettingsOptionRow>
      </SettingsOptionGroup>
    </section>
  );
}
