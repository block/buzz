import * as React from "react";
import { invoke } from "@tauri-apps/api/core";

export type ContributionSummary = {
  contributionId: string;
  title: string;
};

export type InstalledPlugin = {
  pluginId: string;
  name: string;
  version: string;
  publisher: string;
  enabled: boolean;
  contributions: ContributionSummary[];
};

function commandErrorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

/**
 * Lists installed plugins and exposes install/enable/disable/uninstall
 * mutations against the `plugin_*` Tauri commands. Consent and the
 * trusted-native-code disclosure both live inside `plugin_install` on the
 * host side — this hook calls `plugin_pick_directory` then
 * `plugin_install(directory)` and nothing else; a `null` result from either
 * is a neutral cancellation, not an error.
 */
export function usePluginRegistry() {
  const [plugins, setPlugins] = React.useState<InstalledPlugin[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);
  const [installing, setInstalling] = React.useState(false);
  const [busyPluginId, setBusyPluginId] = React.useState<string | null>(null);

  const refresh = React.useCallback(async () => {
    setError(null);
    try {
      const list = await invoke<InstalledPlugin[]>("plugin_list");
      setPlugins(list);
    } catch (cause) {
      setError(commandErrorMessage(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  const install = React.useCallback(async () => {
    setError(null);
    setInstalling(true);
    try {
      const directory = await invoke<string | null>("plugin_pick_directory");
      if (directory === null) {
        // Cancelling the native directory picker is neutral — no error.
        return;
      }
      const installed = await invoke<InstalledPlugin | null>("plugin_install", {
        directory,
      });
      if (installed === null) {
        // Declining the consent dialog is also neutral.
        return;
      }
      await refresh();
    } catch (cause) {
      setError(commandErrorMessage(cause));
    } finally {
      setInstalling(false);
    }
  }, [refresh]);

  const setEnabled = React.useCallback(
    async (pluginId: string, enabled: boolean) => {
      setBusyPluginId(pluginId);
      let mutationError: string | null = null;
      try {
        await invoke("plugin_set_enabled", { pluginId, enabled });
      } catch (cause) {
        mutationError = commandErrorMessage(cause);
      }
      try {
        // The command may have partially committed the registry mutation
        // even though its own promise rejected (e.g. a cleanup step failed
        // after the mutation landed) — refresh regardless of outcome so the
        // list never goes stale.
        await refresh();
      } finally {
        if (mutationError) {
          // The mutation's own error is what the user needs to see, not
          // whatever refresh() left in place (it clears/replaces `error`
          // unconditionally on its own success/failure).
          setError(mutationError);
        }
        setBusyPluginId(null);
      }
    },
    [refresh],
  );

  const uninstall = React.useCallback(
    async (pluginId: string) => {
      setBusyPluginId(pluginId);
      let mutationError: string | null = null;
      try {
        await invoke("plugin_uninstall", { pluginId });
      } catch (cause) {
        mutationError = commandErrorMessage(cause);
      }
      try {
        // Same reasoning as setEnabled above: refresh regardless of the
        // mutation's own outcome so a partial commit is never left stale.
        await refresh();
      } finally {
        if (mutationError) {
          setError(mutationError);
        }
        setBusyPluginId(null);
      }
    },
    [refresh],
  );

  return {
    plugins,
    loading,
    error,
    installing,
    busyPluginId,
    install,
    setEnabled,
    uninstall,
    refresh,
  };
}
