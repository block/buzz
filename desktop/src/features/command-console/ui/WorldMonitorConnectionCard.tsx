import { Globe2, Loader2, ShieldCheck } from "lucide-react";

import { Button } from "@/shared/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/shared/ui/card";

import { useWorldMonitorConnection } from "../hooks/useWorldMonitorConnection";

const STATUS_LABELS = {
  not_connected: "Not connected",
  connected: "Connected",
  unavailable: "Unavailable",
  reauthorise: "Reconnect required",
  quota_limited: "Provider quota limited",
} as const;

export function WorldMonitorConnectionCard() {
  const worldMonitor = useWorldMonitorConnection();
  const connection = worldMonitor.connection;
  const connected = connection !== null && connection.status === "connected";

  return (
    <Card data-testid="world-monitor-connection">
      <CardHeader className="pb-3">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-3">
            <span className="flex h-10 w-10 items-center justify-center rounded-full border border-primary/40 bg-[#071a2f] text-primary">
              <Globe2 aria-hidden="true" className="h-5 w-5" />
            </span>
            <div>
              <CardTitle className="text-base">World Monitor OSINT</CardTitle>
              <p className="mt-1 text-xs text-muted-foreground">
                Curated intelligence for the Maritime N2
              </p>
            </div>
          </div>
          <span className="flex items-center gap-2 text-sm">
            <ShieldCheck aria-hidden="true" className="h-4 w-4 text-primary" />
            {connection ? STATUS_LABELS[connection.status] : "Checking"}
          </span>
        </div>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex flex-wrap gap-x-5 gap-y-1 text-sm text-muted-foreground">
          <span>
            Brief {connection?.briefUsed ?? 0}/{connection?.briefLimit ?? 25}
          </span>
          <span>
            Direct questions {connection?.directUsed ?? 0}/
            {connection?.directLimit ?? 25}
          </span>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            disabled={worldMonitor.busy || connected}
            onClick={() => void worldMonitor.connect()}
            type="button"
          >
            {connection?.status === "reauthorise"
              ? "Reconnect World Monitor"
              : "Connect World Monitor"}
          </Button>
          <Button
            disabled={worldMonitor.busy || !connected}
            onClick={() => void worldMonitor.test()}
            type="button"
            variant="secondary"
          >
            Test connection
          </Button>
          <Button
            disabled={worldMonitor.busy || !connected}
            onClick={() => void worldMonitor.disconnect()}
            type="button"
            variant="ghost"
          >
            Disconnect
          </Button>
        </div>
        {worldMonitor.busy ? (
          <p className="flex items-center gap-2 text-xs text-muted-foreground">
            <Loader2 aria-hidden="true" className="h-3 w-3 animate-spin" />
            Updating World Monitor connection…
          </p>
        ) : null}
        {worldMonitor.error ? (
          <p className="text-sm text-destructive">{worldMonitor.error}</p>
        ) : null}
        <p className="text-xs text-muted-foreground">
          Uses your World Monitor Pro account through OAuth. No API key is
          required.
        </p>
      </CardContent>
    </Card>
  );
}
