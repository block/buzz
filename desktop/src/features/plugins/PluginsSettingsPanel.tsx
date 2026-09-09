import { useNavigate } from "@tanstack/react-router";
import { LoaderCircle, Puzzle } from "lucide-react";

import { Button } from "@/shared/ui/button";
import { Switch } from "@/shared/ui/switch";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";
import {
  SettingsOptionGroup,
  SettingsOptionGroupList,
  SettingsOptionRow,
} from "@/features/settings/ui/SettingsOptionGroup";
import { usePluginRegistry, type InstalledPlugin } from "./usePluginRegistry";

export function PluginsSettingsPanel() {
  const registry = usePluginRegistry();
  const navigate = useNavigate();

  return (
    <section className="min-w-0" data-testid="settings-plugins">
      <SettingsSectionHeader
        title="Plugins"
        description="Install a separately built native plugin and browse in its contributed view."
      />

      {registry.error ? (
        <p
          className="mb-4 rounded-lg border border-destructive/40 bg-destructive/5 p-3 text-sm text-destructive"
          role="alert"
        >
          {registry.error}
        </p>
      ) : null}

      <div className="mb-4">
        <Button
          data-testid="plugins-install-button"
          disabled={registry.installing}
          onClick={() => void registry.install()}
        >
          {registry.installing ? (
            <LoaderCircle className="h-4 w-4 animate-spin" />
          ) : (
            <Puzzle className="h-4 w-4" />
          )}
          Install plugin…
        </Button>
      </div>

      {registry.loading ? (
        <p className="text-sm text-muted-foreground">Loading plugins…</p>
      ) : registry.plugins.length === 0 ? (
        <p className="rounded-xl border border-dashed p-5 text-sm text-muted-foreground">
          No plugins installed yet.
        </p>
      ) : (
        <SettingsOptionGroupList>
          <SettingsOptionGroup title="Installed">
            {registry.plugins.map((plugin) => (
              <PluginRow
                busy={registry.busyPluginId === plugin.pluginId}
                key={plugin.pluginId}
                onDisableEnable={() =>
                  void registry.setEnabled(plugin.pluginId, !plugin.enabled)
                }
                onOpen={(contributionId) =>
                  void navigate({
                    to: "/plugins/$pluginId/$contributionId",
                    params: { pluginId: plugin.pluginId, contributionId },
                  })
                }
                onUninstall={() => void registry.uninstall(plugin.pluginId)}
                plugin={plugin}
              />
            ))}
          </SettingsOptionGroup>
        </SettingsOptionGroupList>
      )}
    </section>
  );
}

function PluginRow({
  plugin,
  busy,
  onOpen,
  onDisableEnable,
  onUninstall,
}: {
  plugin: InstalledPlugin;
  busy: boolean;
  onOpen: (contributionId: string) => void;
  onDisableEnable: () => void;
  onUninstall: () => void;
}) {
  return (
    <SettingsOptionRow data-testid="plugin-row">
      <div className="min-w-0">
        <p className="truncate font-medium text-foreground">{plugin.name}</p>
        <p
          className="mt-0.5 text-sm text-muted-foreground/70"
          data-settings-subcopy
        >
          {plugin.publisher} · v{plugin.version}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        {plugin.contributions.map((contribution) => (
          <Button
            disabled={busy || !plugin.enabled}
            key={contribution.contributionId}
            onClick={() => onOpen(contribution.contributionId)}
            size="sm"
            variant="outline"
          >
            Open {contribution.title}
          </Button>
        ))}
        <Switch
          aria-label={plugin.enabled ? "Disable plugin" : "Enable plugin"}
          checked={plugin.enabled}
          disabled={busy}
          onCheckedChange={onDisableEnable}
        />
        <Button
          className="text-destructive hover:text-destructive"
          disabled={busy}
          onClick={onUninstall}
          size="sm"
          variant="ghost"
        >
          Uninstall
        </Button>
      </div>
    </SettingsOptionRow>
  );
}
