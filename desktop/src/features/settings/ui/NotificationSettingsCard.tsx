import { Suspense, useState } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";

import type {
  DesktopNotificationPermissionState,
  NotificationSettings,
} from "@/features/notifications/hooks";
import {
  COMING_SOON_SLOTS,
  EVENT_ALERT_DESCRIPTIONS,
  EVENT_ALERT_LABELS,
  type NotificationToggleKey,
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
import {
  SettingsOptionGroup,
  SettingsOptionGroupList,
  SettingsOptionRow,
} from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";
import { SoundPicker } from "./SoundPicker";
import { lazyWithReload } from "@/shared/lib/lazyWithReload";
import { playNotificationSound } from "@/features/notifications/lib/sound";
import { sendTestNotification } from "@/features/notifications/lib/testNotification";

const IS_WEB = import.meta.env?.VITE_BUZZ_PLATFORM === "web";

/** Per-type switches, in display order. Remote terminal alerts exist only on desktop. */
const NOTIFY_TYPE_ROWS: NotificationToggleKey[] = [
  "dm",
  "thread_reply",
  "mention",
  "needs_action",
  "reminder",
  "community_join",
  ...(IS_WEB ? [] : (["remote_terminal"] as const)),
];

function toggleLabel(key: NotificationToggleKey) {
  return key in EVENT_ALERT_LABELS
    ? EVENT_ALERT_LABELS[key as keyof typeof EVENT_ALERT_LABELS]
    : SLOT_LABELS[key as SoundSlot];
}
function toggleDescription(key: NotificationToggleKey) {
  return key in EVENT_ALERT_DESCRIPTIONS
    ? EVENT_ALERT_DESCRIPTIONS[key as keyof typeof EVENT_ALERT_DESCRIPTIONS]
    : SLOT_DESCRIPTIONS[key as SoundSlot];
}
function toggleEnabled(
  settings: NotificationSettings,
  key: NotificationToggleKey,
) {
  return key in settings.eventAlertsEnabled
    ? settings.eventAlertsEnabled[
        key as keyof NotificationSettings["eventAlertsEnabled"]
      ]
    : settings.slotAlertsEnabled[key as SoundSlot];
}

/** Shown in place of the push row when its code chunk is from a replaced build and an auto-reload already ran. */
function StalePushRow() {
  return (
    <SettingsOptionRow>
      <div className="min-w-0">
        <p className="text-sm font-medium">Phone notifications</p>
        <p className="text-sm font-normal text-muted-foreground/70">
          A new version is available.{" "}
          <button className="underline" onClick={() => location.reload()} type="button">
            Reload
          </button>{" "}
          to show these settings.
        </p>
      </div>
    </SettingsOptionRow>
  );
}

// Web Push row: web build only, and only once the push sender is live (VITE_BUZZ_WEB_PUSH_URL set at build time).
const WebPushSettingsRow =
  import.meta.env?.VITE_BUZZ_PLATFORM === "web" &&
  import.meta.env?.VITE_BUZZ_WEB_PUSH_URL
    ? lazyWithReload(
        () => import("@/platform/web/push/WebPushSettingsRow"),
        StalePushRow,
      )
    : null;

function TestNotificationRow({ settings }: { settings: NotificationSettings }) {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; message: string } | null>(
    null,
  );
  const run = async () => {
    setBusy(true);
    const r = await sendTestNotification();
    if (r.ok && settings.slotAlertsEnabled.dm)
      playNotificationSound(settings.sounds.dm);
    setResult(r);
    setBusy(false);
  };
  return (
    <SettingsOptionRow>
      <div className="min-w-0">
        <p className="text-sm font-medium">Test notification</p>
        <p
          className={cn(
            "text-sm font-normal",
            result && !result.ok
              ? "text-destructive"
              : "text-muted-foreground/70",
          )}
          data-settings-subcopy
          data-testid="notifications-test-result"
        >
          {result?.message ??
            "Send a notification to check that alerts reach this device."}
        </p>
      </div>
      <Button
        data-testid="notifications-test-button"
        disabled={busy}
        onClick={() => void run()}
        type="button"
        variant="outline"
      >
        {busy ? "Sending…" : "Send test notification"}
      </Button>
    </SettingsOptionRow>
  );
}

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
  onSetSlotAlertsEnabled: (
    slot: NotificationToggleKey,
    enabled: boolean,
  ) => void;
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
  const visibleSlots = SOUND_SLOTS.filter((slot) =>
    COMING_SOON_SLOTS.has(slot)
      ? showComingSoon
      : notificationSettings.slotAlertsEnabled[slot],
  );

  return (
    <section className="min-w-0" data-testid="settings-notifications">
      <SettingsSectionHeader
        title="Notifications"
        description="Desktop alerts are on by default. Fine-tune what gets through below."
      />

      <span className="sr-only" data-testid="notifications-desktop-state">
        {notificationPermission === "unsupported"
          ? "Unavailable"
          : notificationPermission === "denied"
            ? "Blocked"
            : notificationSettings.desktopEnabled
              ? "On"
              : "Off"}
      </span>

      <SettingsOptionGroupList>
        <SettingsOptionGroup title="Desktop">
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
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
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
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
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
          {WebPushSettingsRow ? (
            <Suspense fallback={null}>
              <WebPushSettingsRow />
            </Suspense>
          ) : null}
          <TestNotificationRow settings={notificationSettings} />
        </SettingsOptionGroup>

        <SettingsOptionGroup title="Notify me about">
          {NOTIFY_TYPE_ROWS.map((key) => (
            <SettingsOptionRow key={key}>
              <div className="min-w-0">
                <label
                  className="text-sm font-medium"
                  htmlFor={`notify-type-${key}-switch`}
                >
                  {toggleLabel(key)}
                </label>
                <p
                  className="text-sm font-normal text-muted-foreground/70"
                  data-settings-subcopy
                >
                  {toggleDescription(key)}
                </p>
              </div>
              <Switch
                checked={toggleEnabled(notificationSettings, key)}
                data-testid={`notifications-type-enabled-${key}`}
                id={`notify-type-${key}-switch`}
                onCheckedChange={(checked) => {
                  onSetSlotAlertsEnabled(key, checked);
                }}
              />
            </SettingsOptionRow>
          ))}
          <p
            className="px-3 pb-2 text-xs text-muted-foreground/70"
            data-settings-subcopy
          >
            Synced to your other devices and to phone push. Anything you are
            notified about also shows as unread in the Inbox.
          </p>
        </SettingsOptionGroup>

        {notificationSettings.desktopEnabled ? (
          <>
            <SettingsOptionGroup title="Sound">
              <SettingsOptionRow>
                <div className="min-w-0">
                  <label
                    className="text-sm font-medium"
                    htmlFor="notification-sound-switch"
                  >
                    Sound
                  </label>
                  <p
                    className="text-sm font-normal text-muted-foreground/70"
                    data-settings-subcopy
                  >
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
              <div className="space-y-4">
                <SettingsOptionGroup title="Alert sounds">
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
                          <p
                            className="text-sm font-normal text-muted-foreground/70"
                            data-settings-subcopy
                          >
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
                          {comingSoon ? (
                            <Switch
                              checked={false}
                              data-testid={`notifications-alerts-enabled-${slot}`}
                              disabled
                              id={`alerts-enabled-${slot}-switch`}
                            />
                          ) : null}
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
              </div>
            ) : null}
          </>
        ) : null}

        <SettingsOptionGroup title="Badges">
          <SettingsOptionRow>
            <div className="min-w-0">
              <label
                className="text-sm font-medium"
                htmlFor="home-badge-switch"
              >
                Home badge
              </label>
              <p
                className="text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
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
      </SettingsOptionGroupList>

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
