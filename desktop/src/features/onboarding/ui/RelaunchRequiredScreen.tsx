// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import { RecoveryScreen } from "./RecoveryScreen";

export function RelaunchRequiredScreen() {
  return (
    <RecoveryScreen
      testId="relaunch-required"
      title={uiT("Restart Buzz to finish recovery")}
      body="Your identity was updated. Buzz needs to restart so syncing and agents run under it."
    />
  );
}
