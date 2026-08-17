import { RecoveryScreen } from "./RecoveryScreen";

export function RelaunchRequiredScreen() {
  return (
    <RecoveryScreen
      testId="relaunch-required"
      title="Restart Buzz to finish recovery"
      body="Your identity was updated. Buzz needs to restart so syncing and agents run under it."
    />
  );
}
