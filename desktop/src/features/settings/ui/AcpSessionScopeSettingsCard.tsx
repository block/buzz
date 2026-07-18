import { useEffect, useState } from "react";
import { invokeTauri, listManagedAgents } from "@/shared/api/tauri";
import {
  startManagedAgent,
  stopManagedAgent,
} from "@/shared/api/tauriManagedAgents";
import { Switch } from "@/shared/ui/switch";
import { SettingsSectionHeader } from "./SettingsSectionHeader";
import { applyAcpSessionScopeSetting } from "./acpSessionScopeSetting";

type SessionScope = "thread" | "channel";

export function AcpSessionScopeSettingsCard() {
  const [scope, setScope] = useState<SessionScope>("thread");
  const [pending, setPending] = useState(true);

  useEffect(() => {
    let cancelled = false;
    void invokeTauri<SessionScope>("get_acp_session_scope")
      .then((persisted) => {
        if (!cancelled) setScope(persisted);
      })
      .catch((error) =>
        console.error("Failed to hydrate ACP session scope", error),
      )
      .finally(() => {
        if (!cancelled) setPending(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const setThreadScoped = async (threadScoped: boolean) => {
    setPending(true);
    try {
      await applyAcpSessionScopeSetting(scope === "thread", threadScoped, {
        setBackend: (next) =>
          invokeTauri("set_acp_session_scope", { scope: next }),
        listAgents: listManagedAgents,
        stopAgent: stopManagedAgent,
        startAgent: startManagedAgent,
        setUi: (enabled) => setScope(enabled ? "thread" : "channel"),
      });
    } catch (error) {
      console.error("Failed to apply ACP session scope", error);
    } finally {
      setPending(false);
    }
  };

  return (
    <section className="min-w-0" data-testid="settings-acp-session-scope">
      <SettingsSectionHeader
        title="Agent session scope"
        description="Choose how local ACP agents isolate ongoing conversations."
      />
      <div className="flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-background/70 px-4 py-3">
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium" id="acp-session-scope-label">
            Thread-scoped sessions
          </p>
          <p className="text-xs text-muted-foreground">
            Run separate threads concurrently. Turn this off for one legacy
            session per channel.
          </p>
        </div>
        <Switch
          aria-labelledby="acp-session-scope-label"
          checked={scope === "thread"}
          data-testid="acp-session-scope-toggle"
          disabled={pending}
          onCheckedChange={(value) => void setThreadScoped(value)}
        />
      </div>
    </section>
  );
}
