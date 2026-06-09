import type {
  DesktopNotificationPermissionState,
  NotificationSettings,
} from "@/features/notifications/hooks";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

export function NotificationSettingsCard({
  isUpdatingDesktopNotifications,
  notificationErrorMessage,
  notificationPermission,
  notificationSettings,
  onSetDesktopNotificationsEnabled,
  onSetHomeBadgeEnabled,
  onSetMentionNotificationsEnabled,
  onSetNeedsActionNotificationsEnabled,
  onSetSoundEnabled,
}: {
  isUpdatingDesktopNotifications: boolean;
  notificationErrorMessage: string | null;
  notificationPermission: DesktopNotificationPermissionState;
  notificationSettings: NotificationSettings;
  onSetDesktopNotificationsEnabled: (enabled: boolean) => Promise<boolean>;
  onSetHomeBadgeEnabled: (enabled: boolean) => void;
  onSetMentionNotificationsEnabled: (enabled: boolean) => void;
  onSetNeedsActionNotificationsEnabled: (enabled: boolean) => void;
  onSetSoundEnabled: (enabled: boolean) => void;
}) {
  const permissionBlocked =
    notificationPermission === "denied" ||
    notificationPermission === "unsupported";

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
              htmlFor="notification-sound-switch"
            >
              Notification sound
            </label>
            <p className="text-sm font-normal text-muted-foreground">
              Play a sound when a desktop notification fires.
            </p>
          </div>
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
        </SettingsOptionRow>

        <SettingsOptionRow>
          <div className="min-w-0">
            <label className="text-sm font-medium" htmlFor="home-badge-switch">
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

        <SettingsOptionRow>
          <div className="min-w-0">
            <label className="text-sm font-medium" htmlFor="mentions-switch">
              @Mentions
            </label>
            <p className="text-sm font-normal text-muted-foreground">
              Alert when someone tags your pubkey in a channel you can access.
            </p>
          </div>
          <Switch
            checked={notificationSettings.mentions}
            data-testid="notifications-mentions-toggle"
            id="mentions-switch"
            onCheckedChange={(checked) => {
              onSetMentionNotificationsEnabled(checked);
            }}
          />
        </SettingsOptionRow>

        <SettingsOptionRow>
          <div className="min-w-0">
            <label
              className="text-sm font-medium"
              htmlFor="needs-action-switch"
            >
              Needs action
            </label>
            <p className="text-sm font-normal text-muted-foreground">
              Alert for reminders and workflow approvals that are waiting on
              you.
            </p>
          </div>
          <Switch
            checked={notificationSettings.needsAction}
            data-testid="notifications-needs-action-toggle"
            id="needs-action-switch"
            onCheckedChange={(checked) => {
              onSetNeedsActionNotificationsEnabled(checked);
            }}
          />
        </SettingsOptionRow>
      </SettingsOptionGroup>

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
