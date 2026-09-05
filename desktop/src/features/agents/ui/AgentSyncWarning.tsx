import { useQuery } from "@tanstack/react-query";
import { invokeTauri } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";

/** Persistent bootstrap warning; independent of the agent list so cached rows
 * and local Stop controls remain available even when history is incomplete. */
export function AgentSyncWarning({ onReconnect }: { onReconnect: () => void }) {
  const sync = useQuery({
    queryKey: ["managed-agent-sync-error"],
    queryFn: () => invokeTauri<string | null>("get_managed_agent_sync_error"),
  });
  const error = sync.error ? String(sync.error) : sync.data;
  if (!error) return null;
  return (
    <div
      role="alert"
      className="space-y-2 rounded-md border border-destructive p-3 text-sm"
    >
      <p>
        Agent sync is incomplete. Saved agents are still shown, but startup
        changes have not been published.
      </p>
      <p className="break-words">{error}</p>
      <p>
        Reconnect to retry. If this continues, share the details above with your
        community operator.
      </p>
      <Button size="sm" variant="outline" onClick={onReconnect}>
        Reconnect community
      </Button>
    </div>
  );
}
