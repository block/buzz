import { useState } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";

import type {
  DesktopNotificationPermissionState,
  NotificationSettings,
} from "@/features/notifications/hooks";
import {
  COMING_SOON_SLOTS,
  RECOMMENDED_SOUND_BY_SLOT,
  SLOT_DESCRIPTIONS,
  SLOT_LABELS,
  SOUND_SLOTS,
  type SoundName,
  type SoundSlot,
} from "@/features/notifications/lib/sound";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SoundPicker } from "./SoundPicker";

export function NotificationSettingsCard({
  isUpdatingDesktopNotifications,
  notificationErrorMessage,
  notificationPermission,
  notificationSettings,
  onSetDesktopNotificationsEnabled,
  onSetAllSlotAlertsEnabled,
  onSetHomeBadgeEnabled,
  onSetSlotAlertsEnabled,
  onSetNotifyWhileViewing,
  onSetSoundForSlot,
}: {
  isUpdatingDesktopNotifications: boolean;
  notificationErrorMessage: string | null;
  notificationPermission: DesktopNotificationPermissionState;
  notificationSettings: NotificationSettings;
  onSetDesktopNotificationsEnabled: (enabled: boolean) => Promise<boolean>;
  onSetAllSlotAlertsEnabled: (enabled: boolean) => void;
  onSetHomeBadgeEnabled: (enabled: boolean) => void;
  onSetSlotAlertsEnabled: (slot: SoundSlot, enabled: boolean) => void;
  onSetNotifyWhileViewing: (enabled: boolean) => void;
  onSetSoundForSlot: (slot: SoundSlot, name: SoundName) => void;
}) {
  const permissionBlocked =
    notificationPermission === "denied" ||
    notificationPermission === "unsupported";
  // The parent Sound switch derives from its children: on when any live
  // event row is on, and toggling it bulk-sets every live row.
  const anyAlertsOn = SOUND_SLOTS.some(
    (slot) =>
      !COMING_SOON_SLOTS.has(slot) &&
      notificationSettings.slotAlertsEnabled[slot],
  );
  const [showComingSoon, setShowComingSoon] = useState(false);
  const visibleSlots = SOUND_SLOTS.filter(
    (slot) => showComingSoon || !COMING_SOON_SLOTS.has(slot),
  );

  return (
    <section className="min-w-0" data-testid="settings-notifications">
      <div className="mb-12 min-w-0">
        <h2 className="text-2xl font-semibold tracking-tight">Notifications</h2>
        <p className="text-base font-normal text-muted-foreground">
          Desktop alerts are on by default. Fine-tune what gets through below.
        </p>
      </div>

      <span className="sr-only" data-testid="notifications-desktop-state">
        {notificationPermission === "unsupported"
          ? "Unavailable"
          : notificationPermission === "denied"
            ? "Blocked"
            : notificationSettings.desktopEnabled
              ? "On"
              : "Off"}
      </span>

      <div className="flex flex-col gap-4">
        <SettingsOptionGroup>
          <SettingsOptionRow>
            <div className="min-w-0">
              <label
                className="text-sm font-medium"
                htmlFor="desktop-alerts-switch"
              >
                {isUpdatingDesktopNotifications
                  ? "Requesting..."
                  : "Desktop alerts"}
              </label>
              <p className="text-sm font-normal text-muted-foreground">
                {notificationSettings.desktopEnabled
                  ? "Native desktop alerts are enabled for the categories you have armed below."
                  : "Request OS permission and surface new mentions or needs-action items outside the app."}
              </p>
            </div>
            <Switch
              checked={notificationSettings.desktopEnabled}
              data-testid="notifications-desktop-toggle"
              disabled={isUpdatingDesktopNotifications}
              id="desktop-alerts-switch"
              onCheckedChange={(checked) => {
                void onSetDesktopNotificationsEnabled(checked);
              }}
            />
          </SettingsOptionRow>

          <SettingsOptionRow>
            <div className="min-w-0">
              <label
                className="text-sm font-medium"
                htmlFor="notify-while-viewing-switch"
              >
                Notify while viewing
              </label>
              <p className="text-sm font-normal text-muted-foreground">
                Also alert for direct messages in the conversation you have
                open.
              </p>
            </div>
            <Switch
              checked={
                notificationSettings.desktopEnabled &&
                notificationSettings.notifyWhileViewing
              }
              data-testid="notifications-notify-while-viewing-toggle"
              disabled={!notificationSettings.desktopEnabled}
              id="notify-while-viewing-switch"
              onCheckedChange={(checked) => {
                onSetNotifyWhileViewing(checked);
              }}
            />
          </SettingsOptionRow>
        </SettingsOptionGroup>

        {notificationSettings.desktopEnabled ? (
          <>
            <SettingsOptionGroup>
              <SettingsOptionRow>
                <div className="min-w-0">
                  <label
                    className="text-sm font-medium"
                    htmlFor="notification-sound-switch"
                  >
                    Sound
                  </label>
                  <p className="text-sm font-normal text-muted-foreground">
                    Alert with a sound for the events below.
                  </p>
                </div>
                <Switch
                  checked={anyAlertsOn}
                  data-testid="notifications-sound-toggle"
                  id="notification-sound-switch"
                  onCheckedChange={(checked) => {
                    onSetAllSlotAlertsEnabled(checked);
                  }}
                />
              </SettingsOptionRow>
            </SettingsOptionGroup>

            {anyAlertsOn ? (
              <>
                <SettingsOptionGroup>
                  {visibleSlots.map((slot) => {
                    const comingSoon = COMING_SOON_SLOTS.has(slot);
                    const alertsOn =
                      notificationSettings.slotAlertsEnabled[slot];
                    return (
                      <SettingsOptionRow
                        aria-disabled={comingSoon || undefined}
                        className={cn(
                          comingSoon && "cursor-not-allowed opacity-40",
                        )}
                        key={slot}
                      >
                        <div className="min-w-0">
                          <span className="flex items-center gap-2 text-sm font-medium">
                            {SLOT_LABELS[slot]}
                            {comingSoon ? (
                              <span className="rounded-full bg-muted/70 px-2 py-0.5 text-2xs font-normal uppercase tracking-wide text-muted-foreground">
                                Coming soon
                              </span>
                            ) : null}
                          </span>
                          <p className="text-sm font-normal text-muted-foreground">
                            {SLOT_DESCRIPTIONS[slot]}
                          </p>
                        </div>
                        <span className="flex items-center gap-3">
                          <span
                            className={cn(
                              "transition-opacity duration-200",
                              !alertsOn && "pointer-events-none opacity-40",
                            )}
                          >
                            <SoundPicker
                              disabled={comingSoon || !alertsOn}
                              onChange={(next) => onSetSoundForSlot(slot, next)}
                              recommended={RECOMMENDED_SOUND_BY_SLOT[slot]}
                              value={notificationSettings.sounds[slot]}
                            />
                          </span>
                          <Switch
                            checked={alertsOn && !comingSoon}
                            data-testid={`notifications-alerts-enabled-${slot}`}
                            disabled={comingSoon}
                            id={`alerts-enabled-${slot}-switch`}
                            onCheckedChange={(checked) => {
                              onSetSlotAlertsEnabled(slot, checked);
                            }}
                          />
                        </span>
                      </SettingsOptionRow>
                    );
                  })}
                </SettingsOptionGroup>

                <div className="flex justify-center">
                  <Button
                    data-testid="notifications-toggle-coming-soon"
                    onClick={() => setShowComingSoon((current) => !current)}
                    size="sm"
                    type="button"
                    variant="secondary"
                  >
                    {showComingSoon ? (
                      <>
                        <ChevronUp className="h-4 w-4" />
                        Show less
                      </>
                    ) : (
                      <>
                        <ChevronDown className="h-4 w-4" />
                        View all
                      </>
                    )}
                  </Button>
                </div>
              </>
            ) : null}
          </>
        ) : null}

        <SettingsOptionGroup>
          <SettingsOptionRow>
            <div className="min-w-0">
              <label
                className="text-sm font-medium"
                htmlFor="home-badge-switch"
              >
                Home badge
              </label>
              <p className="text-sm font-normal text-muted-foreground">
                Show a Home badge for mentions and needs-action items in the
                sidebar.
              </p>
            </div>
            <Switch
              checked={notificationSettings.homeBadgeEnabled}
              data-testid="notifications-home-badge-toggle"
              id="home-badge-switch"
              onCheckedChange={(checked) => {
                onSetHomeBadgeEnabled(checked);
              }}
            />
          </SettingsOptionRow>
        </SettingsOptionGroup>
      </div>

      {permissionBlocked && (
        <p className="mt-4 rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {notificationPermission === "unsupported"
            ? "Desktop notifications are not supported in this environment."
            : "Desktop notifications are blocked. Enable them in your system settings."}
        </p>
      )}

      {notificationErrorMessage ? (
        <p className="mt-4 rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {notificationErrorMessage}
        </p>
      ) : null}
    </section>
  );
}
