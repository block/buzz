// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { useUiLanguage } from "@/shared/i18n/uiTranslation";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { ProfilePanelProvider } from "@/shared/context/ProfilePanelContext";

export function AppProfilePanelProvider({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  useUiLanguage();

  const { goProfile } = useAppNavigation();
  const handleOpenProfilePanel = React.useCallback(
    (pubkey: string) => {
      void goProfile(pubkey);
    },
    [goProfile],
  );

  return (
    <ProfilePanelProvider onOpenProfilePanel={handleOpenProfilePanel}>
      {children}
    </ProfilePanelProvider>
  );
}
