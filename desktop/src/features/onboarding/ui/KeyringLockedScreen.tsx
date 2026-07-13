import { RecoveryScreen } from "./RecoveryScreen";

export function KeyringLockedScreen() {
  return (
    <RecoveryScreen
      testId="keyring-locked"
      title="Unlock your system keyring"
      body="Your identity is safe in the OS keyring, but it's unreachable this session. Unlock your keyring or sign into your desktop session, then relaunch Buzz."
    />
  );
}
