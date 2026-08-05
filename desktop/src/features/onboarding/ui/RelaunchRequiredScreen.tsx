import { RecoveryScreen } from "./RecoveryScreen";

export function RelaunchRequiredScreen() {
  return (
    <RecoveryScreen
      testId="relaunch-required"
      title="Restart Zorro to finish recovery"
      body="Your identity was updated. Zorro needs to restart so syncing and agents run under it."
    />
  );
}
