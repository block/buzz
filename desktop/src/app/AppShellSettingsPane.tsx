import * as React from "react";

import { LazySettingsScreen } from "@/app/LazySettingsScreen";
import type { useNotificationSettings } from "@/features/notifications/hooks";
import type { SettingsSection } from "@/features/settings/ui/SettingsPanels";

type AppShellSettingsPaneProps = {
  currentPubkey?: string;
  fallbackDisplayName?: string;
  /**
   * The whole `useNotificationSettings` result. `SettingsScreen` takes its ten
   * fields as ten separate props; spreading them here keeps that fan-out out of
   * `AppShell`, which is up against the 1000-line file-size gate.
   */
  notificationSettings: ReturnType<typeof useNotificationSettings>;
  onClose: () => void;
  onSectionChange: (section: SettingsSection) => void;
  section: SettingsSection;
};

/** The settings screen branch of the app shell, including its Suspense gate. */
export function AppShellSettingsPane({
  currentPubkey,
  fallbackDisplayName,
  notificationSettings,
  onClose,
  onSectionChange,
  section,
}: AppShellSettingsPaneProps) {
  return (
    <div className="flex min-h-0 flex-1 overflow-hidden">
      <React.Suspense fallback={null}>
        <LazySettingsScreen
          currentPubkey={currentPubkey}
          fallbackDisplayName={fallbackDisplayName}
          isUpdatingDesktopNotifications={
            notificationSettings.isUpdatingDesktopEnabled
          }
          notificationErrorMessage={notificationSettings.errorMessage}
          notificationPermission={notificationSettings.permission}
          notificationSettings={notificationSettings.settings}
          onClose={onClose}
          onSectionChange={onSectionChange}
          onSetAllSlotAlertsEnabled={
            notificationSettings.setAllSlotAlertsEnabled
          }
          onSetDesktopNotificationsEnabled={
            notificationSettings.setDesktopEnabled
          }
          onSetHomeBadgeEnabled={notificationSettings.setHomeBadgeEnabled}
          onSetNotifyWhileViewing={notificationSettings.setNotifyWhileViewing}
          onSetSlotAlertsEnabled={notificationSettings.setSlotAlertsEnabled}
          onSetSoundForSlot={notificationSettings.setSoundForSlot}
          section={section}
        />
      </React.Suspense>
    </div>
  );
}
