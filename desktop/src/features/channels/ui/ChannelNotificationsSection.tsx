import { Bell, MessagesSquare, Radio, Settings2 } from "lucide-react";

import { useAppShell } from "@/app/AppShellContext";
import {
  CHANNEL_MUTE_PRESETS,
  CHANNEL_NOTIFY_LEVEL_OPTIONS,
  formatMuteUntil,
} from "@/features/notifications/lib/channelNotifyLabels";
import { Button } from "@/shared/ui/button";
import {
  ChoiceFieldRow,
  FieldGroup,
  IngressRow,
  ToggleFieldRow,
} from "./ChannelManagementSheetRows";

/**
 * The channel sheet's member-scoped notification preferences (NIP-CN): level,
 * timed mute, and the three per-channel toggles. Deliberately *not*
 * admin-gated — every member controls their own notifications.
 */
export function ChannelNotificationsSection({
  channelId,
}: {
  channelId: string;
}) {
  const { channelNotify, onOpenSettings } = useAppShell();
  const state = channelNotify.resolveChannelNotify(channelId);

  return (
    <div className="space-y-2" data-testid="channel-notifications-section">
      <h4 className="px-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
        Notifications
      </h4>

      <FieldGroup>
        <div>
          {CHANNEL_NOTIFY_LEVEL_OPTIONS.map((option) => (
            <ChoiceFieldRow
              description={option.description}
              key={option.value}
              label={option.label}
              name={`channel-notify-level-${channelId}`}
              onSelect={() =>
                channelNotify.setChannelNotifyLevel(channelId, option.value)
              }
              // A running timed mute is an overlay, not a level: keep the
              // stored level visible so expiry has an obvious destination.
              selected={!state.timedMuteActive && state.level === option.value}
              testId={`channel-notifications-level-${option.value}`}
            />
          ))}
        </div>
      </FieldGroup>

      <FieldGroup>
        <div className="flex flex-col items-start gap-2 px-4 py-3">
          <span className="text-sm font-medium text-foreground">
            {state.muteUntil !== null
              ? `Muted until ${formatMuteUntil(state.muteUntil)}`
              : "Mute temporarily"}
          </span>
          <div className="flex flex-wrap gap-2">
            {state.muteUntil !== null ? (
              <Button
                data-testid="channel-notifications-unmute"
                onClick={() => channelNotify.clearChannelTimedMute(channelId)}
                size="sm"
                type="button"
                variant="outline"
              >
                Unmute
              </Button>
            ) : (
              CHANNEL_MUTE_PRESETS.map((preset) => (
                <Button
                  data-testid={`channel-notifications-${preset.testId}`}
                  key={preset.label}
                  onClick={() =>
                    channelNotify.muteChannelUntil(
                      channelId,
                      preset.getTimestamp(),
                    )
                  }
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  {preset.label}
                </Button>
              ))
            )}
          </div>
        </div>
      </FieldGroup>

      <FieldGroup>
        <ToggleFieldRow
          checked={state.desktop}
          description="Show OS banners, play sounds, and bounce the dock for this channel."
          icon={Bell}
          label="Desktop notifications"
          onCheckedChange={(checked) =>
            channelNotify.setChannelNotifyAdvanced(channelId, {
              desktop: checked,
            })
          }
          testId="channel-notifications-desktop-toggle"
        />
        <ToggleFieldRow
          checked={state.followAllThreads}
          description="Get replies in every thread here, not just the ones you join."
          icon={MessagesSquare}
          label="Follow every thread"
          onCheckedChange={(checked) =>
            channelNotify.setChannelNotifyAdvanced(channelId, {
              followAllThreads: checked,
            })
          }
          testId="channel-notifications-threads-toggle"
        />
        <ToggleFieldRow
          checked={state.broadcasts}
          description="Alert on @channel and @here announcements in this channel."
          icon={Radio}
          label="Get broadcast messages"
          onCheckedChange={(checked) =>
            channelNotify.setChannelNotifyAdvanced(channelId, {
              broadcasts: checked,
            })
          }
          testId="channel-notifications-broadcasts-toggle"
        />
      </FieldGroup>

      {onOpenSettings ? (
        <IngressRow
          icon={Settings2}
          label="Edit default preferences"
          onClick={() => onOpenSettings("notifications")}
          testId="channel-notifications-edit-defaults"
        />
      ) : null}
    </div>
  );
}
