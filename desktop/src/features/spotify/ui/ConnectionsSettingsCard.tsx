import { SpotifySettingsCard } from "@/features/spotify/ui/SpotifySettingsCard";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";

export function ConnectionsSettingsCard() {
  return (
    <section className="min-w-0" data-testid="settings-connections">
      <SettingsSectionHeader
        title="Connections"
        description="Connect apps on this device so Buzz can bring their context into your workspace."
      />

      <SpotifySettingsCard />
    </section>
  );
}
