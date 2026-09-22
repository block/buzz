import { useTranslation } from "@/i18n";

import { RecoveryScreen } from "./RecoveryScreen";

export function RelaunchRequiredScreen() {
  const { t } = useTranslation();
  return (
    <RecoveryScreen
      testId="relaunch-required"
      title={t("onboarding.recovery.title-restart")}
      body={t("onboarding.recovery.body-restart")}
    />
  );
}
