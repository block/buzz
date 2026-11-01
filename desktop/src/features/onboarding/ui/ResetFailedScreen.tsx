// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import { RecoveryScreen } from "./RecoveryScreen";

export function ResetFailedScreen() {
  return (
    <RecoveryScreen
      testId="reset-failed"
      title={uiT("Sign out could not complete")}
      body="Buzz was unable to fully clear your local data. Try relaunching — the reset will resume automatically. If this persists, contact support."
    />
  );
}
