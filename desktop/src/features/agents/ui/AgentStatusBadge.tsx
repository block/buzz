import * as React from "react";

import { Badge } from "@/shared/ui/badge";
import type { ManagedAgent, PresenceStatus } from "@/shared/api/types";

/** Grace period after mount before treating "running + no presence" as "Starting…" */
const PRESENCE_GRACE_MS = 15_000;

export function AgentStatusBadge({
  isWorking,
  presenceLoaded,
  presenceStatus,
  status,
}: {
  isWorking?: boolean;
  presenceLoaded: boolean;
  presenceStatus: PresenceStatus | undefined;
  status: ManagedAgent["status"];
}) {
  const [inGracePeriod, setInGracePeriod] = React.useState(true);

  React.useEffect(() => {
    const timer = setTimeout(() => setInGracePeriod(false), PRESENCE_GRACE_MS);
    return () => clearTimeout(timer);
  }, []);

  // A deployed remote agent that is not present has shut down; the deployment record survives
  // because v1 has no `undeploy`, so it must not keep reading as live (I3, docs/remote-agents.md).
  const presenceSaysOffline =
    presenceLoaded && (!presenceStatus || presenceStatus === "offline");
  const isActive =
    (status === "running" || status === "deployed") &&
    !(status === "deployed" && !inGracePeriod && presenceSaysOffline);
  const isStarting =
    !inGracePeriod && presenceSaysOffline && status === "running";

  const variant: "default" | "warning" | "secondary" = isWorking
    ? "default"
    : isStarting
      ? "warning"
      : isActive
        ? "default"
        : "secondary";

  const label = isWorking
    ? "Working"
    : isStarting
      ? "Starting\u2026"
      : status.replace(/_/g, " ");

  return (
    <Badge
      className={isWorking ? "motion-safe:animate-pulse" : undefined}
      variant={variant}
    >
      {label}
    </Badge>
  );
}
