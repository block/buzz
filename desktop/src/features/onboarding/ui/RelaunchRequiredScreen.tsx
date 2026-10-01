import { RecoveryScreen } from "./RecoveryScreen";

export function RelaunchRequiredScreen() {
  return (
    <RecoveryScreen
      testId="relaunch-required"
      title="Restart Orbit to finish recovery"
      body="Your identity was updated. Orbit needs to restart so syncing and agents run under it."
    />
  );
}
