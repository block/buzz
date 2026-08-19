import * as React from "react";

import { Badge } from "@/shared/ui/badge";
import type { ManagedAgent, PresenceStatus } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

/** Grace period after mount before treating "running + no presence" as "Starting…" */
const PRESENCE_GRACE_MS = 15_000;

export function AgentStatusBadge({
  className,
  isWorking,
  presenceLoaded,
  presenceStatus,
  sentenceCase = false,
  status,
}: {
  className?: string;
  isWorking?: boolean;
  presenceLoaded: boolean;
  presenceStatus: PresenceStatus | undefined;
  sentenceCase?: boolean;
  status: ManagedAgent["status"];
}) {
  const [inGracePeriod, setInGracePeriod] = React.useState(true);

  React.useEffect(() => {
    const timer = setTimeout(() => setInGracePeriod(false), PRESENCE_GRACE_MS);
    return () => clearTimeout(timer);
  }, []);

  const isRemote = status === "deployed" || status === "not_deployed";
  const isActive = isRemote
    ? presenceStatus === "online" || presenceStatus === "away"
    : status === "running";
  const isStarting =
    !inGracePeriod &&
    presenceLoaded &&
    status === "running" &&
    (!presenceStatus || presenceStatus === "offline");

  const variant: "default" | "warning" | "secondary" = isWorking
    ? "default"
    : isStarting
      ? "warning"
      : isActive
        ? "default"
        : "secondary";

  const remoteLabel =
    status === "not_deployed"
      ? "not deployed"
      : !presenceLoaded
        ? "checking…"
        : (presenceStatus ?? "offline");
  const rawLabel = isWorking
    ? "Working"
    : isStarting
      ? "Starting\u2026"
      : isRemote
        ? remoteLabel
        : status.replace(/_/g, " ");
  const label = sentenceCase
    ? `${rawLabel.charAt(0).toUpperCase()}${rawLabel.slice(1)}`
    : rawLabel;

  return (
    <Badge
      className={cn(className, isWorking && "motion-safe:animate-pulse")}
      variant={variant}
    >
      {label}
    </Badge>
  );
}
