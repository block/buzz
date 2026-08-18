import type { DesktopNotificationPermissionState } from "@/features/notifications/hooks";
import type { NotificationSettings } from "@/features/notifications/hooks";
import type { SoundName, SoundSlot } from "@/features/notifications/lib/sound";
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
  onSetSlotAlertsEnabled: (slot: SoundSlot, enabled: boolean) => void;
  onSetNotifyWhileViewing: (enabled: boolean) => void;
  onSetAllSlotAlertsEnabled: (enabled: boolean) => void;
  onSetSoundForSlot: (slot: SoundSlot, name: SoundName) => void;
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
  onSetSlotAlertsEnabled,
  onSetNotifyWhileViewing,
  onSetAllSlotAlertsEnabled,
  onSetSoundForSlot,
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
      onSetSlotAlertsEnabled={onSetSlotAlertsEnabled}
      onSetNotifyWhileViewing={onSetNotifyWhileViewing}
      onSetAllSlotAlertsEnabled={onSetAllSlotAlertsEnabled}
      onSetSoundForSlot={onSetSoundForSlot}
      section={section}
    />
  );
}
