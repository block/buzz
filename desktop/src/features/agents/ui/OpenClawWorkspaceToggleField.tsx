import * as React from "react";
import { invokeTauri } from "@/shared/api/tauri";
import { useFeatureEnabled } from "@/shared/features";
import { Switch } from "@/shared/ui/switch";

type OpenClawWorkspaceToggleFieldProps = {
  checked: boolean;
  disabled?: boolean;
  id: string;
  onCheckedChange: (value: boolean) => void;
  /**
   * When true (create dialog), the Switch can only turn on while the OpenClaw
   * workspace grant is Connected. Disconnected → forced off + helper text.
   */
  requireConnected?: boolean;
  /** Optional test id override (defaults to the field id). */
  testId?: string;
};

type OpenClawWorkspaceConnection = {
  connected: boolean;
  loading: boolean;
};

async function readOpenClawConnected(): Promise<boolean> {
  try {
    const status = await invokeTauri<{ connected: boolean }>(
      "get_openclaw_workspace_mcp_status",
    );
    return status.connected === true;
  } catch {
    return false;
  }
}

/**
 * Live OpenClaw workspace connection for gating the create Switch.
 * Soft-fails when the Tauri command is unavailable (OSS builds).
 */
function useOpenClawWorkspaceConnection(
  enabled: boolean,
): OpenClawWorkspaceConnection {
  const [connected, setConnected] = React.useState(false);
  const [loading, setLoading] = React.useState(enabled);

  React.useEffect(() => {
    if (!enabled) {
      setConnected(false);
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);

    async function load() {
      const next = await readOpenClawConnected();
      if (!cancelled) {
        setConnected(next);
        setLoading(false);
      }
    }

    void load();
    const interval = window.setInterval(() => {
      void load();
    }, 8_000);
    function onChanged() {
      void load();
    }
    window.addEventListener("buzz:openclaw-workspace-status", onChanged);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
      window.removeEventListener("buzz:openclaw-workspace-status", onChanged);
    };
  }, [enabled]);

  return { connected, loading };
}

/** Notify create/edit toggles that OpenClaw workspace status changed. */
export function emitOpenClawWorkspaceStatusChanged(): void {
  window.dispatchEvent(new Event("buzz:openclaw-workspace-status"));
}

/**
 * Top-level “OpenClaw workspace” Switch for create/edit agent forms.
 * Gated by the Hula `openclaw-workspace-mcp` feature flag (same as Settings card).
 * Always uses the shared Switch (slider) — never a checkbox.
 */
export function OpenClawWorkspaceToggleField({
  checked,
  disabled = false,
  id,
  onCheckedChange,
  requireConnected = false,
  testId,
}: OpenClawWorkspaceToggleFieldProps) {
  const featureEnabled = useFeatureEnabled("openclaw-workspace-mcp");
  const { connected, loading } = useOpenClawWorkspaceConnection(
    featureEnabled && requireConnected,
  );

  React.useEffect(() => {
    if (!requireConnected || loading) return;
    if (!connected && checked) {
      onCheckedChange(false);
    }
  }, [checked, connected, loading, onCheckedChange, requireConnected]);

  if (!featureEnabled) return null;

  const blockedByConnection = requireConnected && !connected;
  const switchDisabled =
    disabled || blockedByConnection || (requireConnected && loading);
  const switchChecked = blockedByConnection ? false : checked;

  return (
    <section
      className="space-y-2 rounded-lg border border-border/60 bg-muted/20 p-3"
      data-testid={testId ?? id}
    >
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0 space-y-0.5">
          <p className="text-sm font-semibold text-foreground">
            OpenClaw workspace
          </p>
          <label className="text-sm font-medium text-foreground" htmlFor={id}>
            Use OpenClaw workspace (MCP)
          </label>
        </div>
        <Switch
          checked={switchChecked}
          disabled={switchDisabled}
          id={id}
          onCheckedChange={onCheckedChange}
        />
      </div>
      {blockedByConnection ? (
        <p className="text-xs text-muted-foreground">
          Connect OpenClaw workspace in Settings → Agents first.
        </p>
      ) : (
        <p className="text-xs text-muted-foreground">
          {switchChecked
            ? "Remote OpenClaw FS + skills via MCP (skills_list / skills_get). Local Mac disk is not the workspace."
            : "Local Mac filesystem and local skills. OpenClaw MCP is not attached."}
        </p>
      )}
      {switchChecked && !blockedByConnection ? (
        <p className="text-xs text-amber-600 dark:text-amber-400">
          Requires a connected OpenClaw workspace grant (Settings → Agents →
          OpenClaw workspace). Without a grant, start will fail with a clear
          error instead of falling back to local disk.
        </p>
      ) : null}
    </section>
  );
}
