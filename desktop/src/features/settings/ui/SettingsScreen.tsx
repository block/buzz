import type { DesktopNotificationPermissionState } from "@/features/notifications/hooks";
import type { NotificationSettings } from "@/features/notifications/hooks";
import type { SettingsSection } from "@/features/settings/ui/SettingsPanels";
import { SettingsView } from "@/features/settings/ui/SettingsView";

type SettingsScreenProps = {
  currentPubkey?: string;
  fallbackDisplayName?: string;
  isUpdatingDesktopNotifications: boolean;
  notificationErrorMessage: string | null;
  notificationPermission: DesktopNotificationPermissionState;
  notificationSettings: NotificationSettings;
  onClose: () => void;
  onSectionChange: (section: SettingsSection) => void;
  onSetDesktopNotificationsEnabled: (enabled: boolean) => Promise<boolean>;
  onSetHomeBadgeEnabled: (enabled: boolean) => void;
  onSetMentionNotificationsEnabled: (enabled: boolean) => void;
  onSetNeedsActionNotificationsEnabled: (enabled: boolean) => void;
  onSetSoundEnabled: (enabled: boolean) => void;
  section: SettingsSection;
};

export function SettingsScreen({
  currentPubkey,
  fallbackDisplayName,
  isUpdatingDesktopNotifications,
  notificationErrorMessage,
  notificationPermission,
  notificationSettings,
  onClose,
  onSectionChange,
  onSetDesktopNotificationsEnabled,
  onSetHomeBadgeEnabled,
  onSetMentionNotificationsEnabled,
  onSetNeedsActionNotificationsEnabled,
  onSetSoundEnabled,
  section,
}: SettingsScreenProps) {
  return (
    <SettingsView
      currentPubkey={currentPubkey}
      fallbackDisplayName={fallbackDisplayName}
      isUpdatingDesktopNotifications={isUpdatingDesktopNotifications}
      notificationErrorMessage={notificationErrorMessage}
      notificationPermission={notificationPermission}
      notificationSettings={notificationSettings}
      onClose={onClose}
      onSectionChange={onSectionChange}
      onSetDesktopNotificationsEnabled={onSetDesktopNotificationsEnabled}
      onSetHomeBadgeEnabled={onSetHomeBadgeEnabled}
      onSetMentionNotificationsEnabled={onSetMentionNotificationsEnabled}
      onSetNeedsActionNotificationsEnabled={
        onSetNeedsActionNotificationsEnabled
      }
      onSetSoundEnabled={onSetSoundEnabled}
      section={section}
    />
  );
}
