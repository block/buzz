import * as React from "react";

import { LazySettingsScreen } from "@/app/LazySettingsScreen";
import type { useNotificationSettings } from "@/features/notifications/hooks";
import type { SettingsSection } from "@/features/settings/ui/SettingsPanels";

type AppSettingsSurfaceProps = {
  currentPubkey?: string;
  fallbackDisplayName?: string;
  notificationSettings: ReturnType<typeof useNotificationSettings>;
  onClose: () => void;
  onSectionChange: (section: SettingsSection) => void;
  section: SettingsSection;
};

export function AppSettingsSurface({
  currentPubkey,
  fallbackDisplayName,
  notificationSettings,
  onClose,
  onSectionChange,
  section,
}: AppSettingsSurfaceProps) {
  return (
    <div className="flex min-h-0 min-w-0 flex-1 overflow-hidden">
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
          onSetDesktopNotificationsEnabled={
            notificationSettings.setDesktopEnabled
          }
          onSetHomeBadgeEnabled={notificationSettings.setHomeBadgeEnabled}
          onSetSlotAlertsEnabled={notificationSettings.setSlotAlertsEnabled}
          onSetNotifyWhileViewing={notificationSettings.setNotifyWhileViewing}
          onSetAllSlotAlertsEnabled={
            notificationSettings.setAllSlotAlertsEnabled
          }
          onSetSoundForSlot={notificationSettings.setSoundForSlot}
          section={section}
        />
      </React.Suspense>
    </div>
  );
}
