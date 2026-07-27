import { Bell, BellOff, Clock, Settings2 } from "lucide-react";

import { useAppShell } from "@/app/AppShellContext";
import {
  CHANNEL_MUTE_PRESETS,
  CHANNEL_NOTIFY_LEVEL_OPTIONS,
  formatMuteUntil,
} from "@/features/notifications/lib/channelNotifyLabels";
import type { ChannelNotifyLevel } from "@/features/sidebar/lib/channelNotifyPrefsStorage";
import {
  ContextMenuIconSlot,
  deferMenuAction,
} from "@/features/sidebar/ui/sidebarMenuHelpers";
import {
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuRadioGroup,
  ContextMenuRadioItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
} from "@/shared/ui/context-menu";

/**
 * The channel context menu's "Notifications" submenu (NIP-CN): the per-channel
 * level radio group, the two timed-mute presets, and a link into the channel
 * sheet's notification preferences. State and mutations come from the single
 * AppShell notification surface, so this component only renders.
 */
export function ChannelNotificationsSubmenu({
  channelId,
}: {
  channelId: string;
}) {
  const { channelNotify, openChannelManagement } = useAppShell();
  const state = channelNotify.resolveChannelNotify(channelId);

  return (
    <ContextMenuSub>
      <ContextMenuSubTrigger data-testid="channel-notify-submenu">
        <ContextMenuIconSlot>
          {state.level === "all" ? (
            <Bell className="h-4 w-4" />
          ) : (
            <BellOff className="h-4 w-4" />
          )}
        </ContextMenuIconSlot>
        <span>Notifications</span>
      </ContextMenuSubTrigger>
      <ContextMenuSubContent>
        <ContextMenuRadioGroup
          onValueChange={(value) =>
            deferMenuAction(() =>
              channelNotify.setChannelNotifyLevel(
                channelId,
                value as ChannelNotifyLevel,
              ),
            )
          }
          // A running timed mute is an overlay, not a level: leave the group
          // unselected so it never looks like the channel was muted and hidden.
          value={state.timedMuteActive ? "" : state.level}
        >
          {CHANNEL_NOTIFY_LEVEL_OPTIONS.map((option) => (
            <ContextMenuRadioItem
              data-testid={`channel-notify-level-${option.value}`}
              key={option.value}
              value={option.value}
            >
              {option.label}
            </ContextMenuRadioItem>
          ))}
        </ContextMenuRadioGroup>
        <ContextMenuSeparator />
        {state.muteUntil !== null ? (
          <>
            <ContextMenuLabel className="text-xs font-normal text-muted-foreground">
              {`Muted until ${formatMuteUntil(state.muteUntil)}`}
            </ContextMenuLabel>
            <ContextMenuItem
              data-testid="channel-notify-unmute"
              onSelect={() =>
                deferMenuAction(() =>
                  channelNotify.clearChannelTimedMute(channelId),
                )
              }
            >
              <ContextMenuIconSlot>
                <Bell className="h-4 w-4" />
              </ContextMenuIconSlot>
              <span>Unmute</span>
            </ContextMenuItem>
          </>
        ) : (
          CHANNEL_MUTE_PRESETS.map((preset) => (
            <ContextMenuItem
              data-testid={preset.testId}
              key={preset.label}
              onSelect={() =>
                deferMenuAction(() =>
                  channelNotify.muteChannelUntil(
                    channelId,
                    preset.getTimestamp(),
                  ),
                )
              }
            >
              <ContextMenuIconSlot>
                <Clock className="h-4 w-4" />
              </ContextMenuIconSlot>
              <span>{preset.label}</span>
            </ContextMenuItem>
          ))
        )}
        <ContextMenuSeparator />
        <ContextMenuItem
          data-testid="channel-notify-preferences"
          onSelect={() =>
            deferMenuAction(() => openChannelManagement(channelId))
          }
        >
          <ContextMenuIconSlot>
            <Settings2 className="h-4 w-4" />
          </ContextMenuIconSlot>
          <span>Notification preferences...</span>
        </ContextMenuItem>
      </ContextMenuSubContent>
    </ContextMenuSub>
  );
}
