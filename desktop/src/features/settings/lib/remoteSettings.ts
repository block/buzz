import type { SettingsSection } from "../ui/SettingsPanels";

/** NIP-AB pairing transfers a human private key, which remote custody cannot export. */
export function remoteSettingsSectionAvailable(
  section: SettingsSection,
): boolean {
  return section !== "mobile";
}
