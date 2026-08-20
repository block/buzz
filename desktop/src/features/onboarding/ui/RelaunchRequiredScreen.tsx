import { RecoveryScreen } from "./RecoveryScreen";

export function RelaunchRequiredScreen() {
  return (
    <RecoveryScreen
      testId="relaunch-required"
      title="Restart Mesh to finish recovery"
      body="Your identity was updated. Mesh needs to restart so syncing and agents run under it."
    />
  );
}
