import type {
  DesktopNotificationPermissionState,
  NotificationSettings,
} from "@/features/notifications/hooks";
import {
  COMING_SOON_SLOTS,
  RECOMMENDED_SINGLE_SOUND,
  RECOMMENDED_SOUND_BY_SLOT,
  SLOT_DESCRIPTIONS,
  SLOT_LABELS,
  SOUND_SLOTS,
  type SoundName,
  type SoundSlot,
} from "@/features/notifications/lib/sound";
import { cn } from "@/shared/lib/cn";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SoundPicker } from "./SoundPicker";

export function NotificationSettingsCard({
  isUpdatingDesktopNotifications,
  notificationErrorMessage,
  notificationPermission,
  notificationSettings,
  onSetDesktopNotificationsEnabled,
  onSetHomeBadgeEnabled,
  onSetSlotAlertsEnabled,
  onSetNotifyWhileViewing,
  onSetSingleSound,
  onSetSoundEnabled,
  onSetSoundForSlot,
}: {
  isUpdatingDesktopNotifications: boolean;
  notificationErrorMessage: string | null;
  notificationPermission: DesktopNotificationPermissionState;
  notificationSettings: NotificationSettings;
  onSetDesktopNotificationsEnabled: (enabled: boolean) => Promise<boolean>;
  onSetHomeBadgeEnabled: (enabled: boolean) => void;
  onSetSlotAlertsEnabled: (slot: SoundSlot, enabled: boolean) => void;
  onSetNotifyWhileViewing: (enabled: boolean) => void;
  onSetSingleSound: (name: SoundName) => void;
  onSetSoundEnabled: (enabled: boolean) => void;
  onSetSoundForSlot: (slot: SoundSlot, name: SoundName | null) => void;
}) {
  const permissionBlocked =
    notificationPermission === "denied" ||
    notificationPermission === "unsupported";
  const soundControlsVisible =
    notificationSettings.desktopEnabled && notificationSettings.soundEnabled;

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
                Play a sound when a desktop notification fires.
              </p>
            </div>
            <span className="flex items-center gap-3">
              {soundControlsVisible ? (
                <SoundPicker
                  onChange={(next) =>
                    onSetSingleSound(next ?? RECOMMENDED_SINGLE_SOUND)
                  }
                  recommended={RECOMMENDED_SINGLE_SOUND}
                  value={notificationSettings.singleSound}
                />
              ) : null}
              <Switch
                checked={
                  notificationSettings.desktopEnabled &&
                  notificationSettings.soundEnabled
                }
                data-testid="notifications-sound-toggle"
                disabled={!notificationSettings.desktopEnabled}
                id="notification-sound-switch"
                onCheckedChange={(checked) => {
                  onSetSoundEnabled(checked);
                }}
              />
            </span>
          </SettingsOptionRow>

          <div
            className={
              notificationSettings.desktopEnabled
                ? "grid grid-rows-[1fr] transition-[grid-template-rows] duration-200"
                : "grid grid-rows-[0fr] transition-[grid-template-rows] duration-200"
            }
          >
            <div className="min-h-0 overflow-hidden">
              <div className="border-t border-border/55">
                {SOUND_SLOTS.map((slot) => {
                  const comingSoon = COMING_SOON_SLOTS.has(slot);
                  // Cascade: the Sound master switch off renders every row
                  // off and inert; stored per-row values are preserved.
                  const alertsOn =
                    soundControlsVisible &&
                    notificationSettings.slotAlertsEnabled[slot];
                  return (
                    <SettingsOptionRow
                      aria-disabled={comingSoon || undefined}
                      className={cn(
                        comingSoon && "pointer-events-none opacity-40",
                      )}
                      key={slot}
                    >
                      <div className="min-w-0">
                        <span className="flex items-center gap-2 text-sm font-medium">
                          {SLOT_LABELS[slot]}
                          {comingSoon ? (
                            <span className="rounded-full bg-muted/70 px-2 py-0.5 text-[10px] font-normal uppercase tracking-wide text-muted-foreground">
                              Coming soon
                            </span>
                          ) : null}
                        </span>
                        <p className="text-sm font-normal text-muted-foreground">
                          {SLOT_DESCRIPTIONS[slot]}
                        </p>
                      </div>
                      <span className="flex items-center gap-3">
                        {soundControlsVisible ? (
                          <span
                            className={cn(
                              "transition-opacity duration-200",
                              !alertsOn && "pointer-events-none opacity-40",
                            )}
                          >
                            <SoundPicker
                              disabled={comingSoon || !alertsOn}
                              inheritFrom={notificationSettings.singleSound}
                              onChange={(next) => onSetSoundForSlot(slot, next)}
                              recommended={RECOMMENDED_SOUND_BY_SLOT[slot]}
                              value={notificationSettings.sounds[slot]}
                            />
                          </span>
                        ) : null}
                        <Switch
                          checked={alertsOn && !comingSoon}
                          data-testid={`notifications-alerts-enabled-${slot}`}
                          disabled={comingSoon || !soundControlsVisible}
                          id={`alerts-enabled-${slot}-switch`}
                          onCheckedChange={(checked) => {
                            onSetSlotAlertsEnabled(slot, checked);
                          }}
                        />
                      </span>
                    </SettingsOptionRow>
                  );
                })}
              </div>
            </div>
          </div>
        </SettingsOptionGroup>

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
