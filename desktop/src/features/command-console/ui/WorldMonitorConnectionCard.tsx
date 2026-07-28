import { Globe2, Loader2, ShieldCheck } from "lucide-react";
import { useState } from "react";

import { Button } from "@/shared/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/shared/ui/card";
import { Input } from "@/shared/ui/input";

import { useWorldMonitorConnection } from "../hooks/useWorldMonitorConnection";

const STATUS_LABELS = {
  not_configured: "Not configured",
  configured: "Configured",
  connected: "Connected",
  unavailable: "Unavailable",
  unauthorised: "Key rejected",
  quota_limited: "Provider quota limited",
} as const;

export function WorldMonitorConnectionCard() {
  const worldMonitor = useWorldMonitorConnection();
  const [apiKey, setApiKey] = useState("");
  const connection = worldMonitor.connection;
  const configured =
    connection !== null && connection.status !== "not_configured";

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
          <Input
            aria-label="World Monitor API key"
            autoComplete="off"
            className="min-w-64 flex-1"
            onChange={(event) => setApiKey(event.target.value)}
            placeholder="wm_live_…"
            type="password"
            value={apiKey}
          />
          <Button
            disabled={worldMonitor.busy || apiKey.length === 0}
            onClick={() => {
              void worldMonitor.save(apiKey).then((saved) => {
                if (saved) setApiKey("");
              });
            }}
            type="button"
          >
            Save
          </Button>
          <Button
            disabled={worldMonitor.busy || !configured}
            onClick={() => void worldMonitor.test()}
            type="button"
            variant="secondary"
          >
            Test connection
          </Button>
          <Button
            disabled={worldMonitor.busy || !configured}
            onClick={() => void worldMonitor.remove()}
            type="button"
            variant="ghost"
          >
            Remove
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
          The key is stored in macOS Keychain and is never returned to this
          screen.
        </p>
      </CardContent>
    </Card>
  );
}
