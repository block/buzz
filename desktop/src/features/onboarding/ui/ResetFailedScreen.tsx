import { useTranslation } from "@/i18n";

import { RecoveryScreen } from "./RecoveryScreen";

export function ResetFailedScreen() {
  const { t } = useTranslation();
  return (
    <RecoveryScreen
      testId="reset-failed"
      title={t("onboarding.recovery.title-signout-failed")}
      body={t("onboarding.recovery.body-signout-failed")}
    />
  );
}
