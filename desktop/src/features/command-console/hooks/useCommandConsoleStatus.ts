import * as React from "react";

import { useMeshNodeStatus } from "@/features/mesh-compute/hooks/useMeshNodeStatus";
import type { ConnectionState } from "@/shared/api/relayClientShared";
import type { MeshNodeStatus } from "@/shared/api/tauriMesh";
import { useRelayConnection } from "@/shared/api/useRelayConnection";

export type CommandServiceState =
  | "connected"
  | "degraded"
  | "unavailable"
  | "offline"
  | "not_configured";

export type CommandServiceStatus = {
  readonly id:
    | "relay"
    | "local-compute"
    | "lm-studio"
    | "memory"
    | "rag"
    | "apple-inputs";
  readonly label: string;
  readonly state: CommandServiceState;
  readonly statusLabel:
    | "Connected"
    | "Degraded"
    | "Unavailable"
    | "Offline"
    | "Not configured";
  readonly detail: string;
};

export type CommandConsoleStatusViewModel = {
  readonly liveServices: readonly CommandServiceStatus[];
  readonly laterCapabilities: readonly CommandServiceStatus[];
};

type LocalComputeProbe = {
  readonly status: MeshNodeStatus | null;
  readonly error: string | null;
};

type LocalComputeFreshnessOptions = {
  readonly freshnessMs?: number;
};

type CommandConsoleStatusSources = {
  readonly relayConnection: ConnectionState;
  readonly localCompute: LocalComputeProbe;
};

const LATER_CAPABILITIES: readonly CommandServiceStatus[] = [
  {
    detail: "No runtime integration is configured in Phase 1.",
    id: "lm-studio",
    label: "LM Studio",
    state: "not_configured",
    statusLabel: "Not configured",
  },
  {
    detail: "No memory integration is configured in Phase 1.",
    id: "memory",
    label: "Memory",
    state: "not_configured",
    statusLabel: "Not configured",
  },
  {
    detail: "No retrieval integration is configured in Phase 1.",
    id: "rag",
    label: "RAG",
    state: "not_configured",
    statusLabel: "Not configured",
  },
  {
    detail: "No Calendar, Reminders, or Notes access is configured.",
    id: "apple-inputs",
    label: "Apple inputs",
    state: "not_configured",
    statusLabel: "Not configured",
  },
];

const LOCAL_COMPUTE_FRESHNESS_MS = 10_000;

function relayStatus(connection: ConnectionState): CommandServiceStatus {
  const base = {
    id: "relay" as const,
    label: "Buzz relay",
  };

  switch (connection) {
    case "connected":
      return {
        ...base,
        detail: "Authenticated relay connection is active.",
        state: "connected",
        statusLabel: "Connected",
      };
    case "reconnecting":
      return {
        ...base,
        detail: "Relay connection was interrupted and is reconnecting.",
        state: "degraded",
        statusLabel: "Degraded",
      };
    case "stalled":
      return {
        ...base,
        detail: "Relay connection is open but has stopped receiving data.",
        state: "degraded",
        statusLabel: "Degraded",
      };
    case "disconnected":
      return {
        ...base,
        detail: "No relay connection is active.",
        state: "offline",
        statusLabel: "Offline",
      };
    case "connecting":
      return {
        ...base,
        detail: "Relay connection has not completed a successful handshake.",
        state: "unavailable",
        statusLabel: "Unavailable",
      };
    case "idle":
      return {
        ...base,
        detail: "A relay connection has not been established.",
        state: "unavailable",
        statusLabel: "Unavailable",
      };
  }
}

function localComputeStatus({
  error,
  status,
}: LocalComputeProbe): CommandServiceStatus {
  const base = {
    id: "local-compute" as const,
    label: "Local compute",
  };

  if (error !== null) {
    return {
      ...base,
      detail: `Status probe failed: ${error}`,
      state: "unavailable",
      statusLabel: "Unavailable",
    };
  }

  if (status === null) {
    return {
      ...base,
      detail: "Waiting for a successful local-compute status probe.",
      state: "unavailable",
      statusLabel: "Unavailable",
    };
  }

  if (status.state === "off") {
    return {
      ...base,
      detail: "Local compute is not running.",
      state: "offline",
      statusLabel: "Offline",
    };
  }

  if (status.mode === "client") {
    return {
      ...base,
      detail:
        "This Mac is operating in mesh client mode, not serving local compute.",
      state: "unavailable",
      statusLabel: "Unavailable",
    };
  }

  if (status.mode !== "serve") {
    return {
      ...base,
      detail:
        "The status probe did not verify this Mac as a local-compute server.",
      state: "unavailable",
      statusLabel: "Unavailable",
    };
  }

  if (status.state === "starting" || status.state === "stopping") {
    return {
      ...base,
      detail: `Local compute is ${status.state}.`,
      state: "degraded",
      statusLabel: "Degraded",
    };
  }

  if (status.state === "failed" || status.health.status === "failed") {
    return {
      ...base,
      detail:
        status.health.status === "failed"
          ? status.health.reason
          : "Local compute reported a failed state.",
      state: "unavailable",
      statusLabel: "Unavailable",
    };
  }

  if (status.health.status === "degraded") {
    return {
      ...base,
      detail: status.health.reason,
      state: "degraded",
      statusLabel: "Degraded",
    };
  }

  return {
    ...base,
    detail: status.modelName
      ? `${status.modelName} is running on this Mac.`
      : "Local compute is running on this Mac.",
    state: "connected",
    statusLabel: "Connected",
  };
}

export function createCommandConsoleStatusViewModel({
  localCompute,
  relayConnection,
}: CommandConsoleStatusSources): CommandConsoleStatusViewModel {
  return {
    laterCapabilities: LATER_CAPABILITIES,
    liveServices: [
      relayStatus(relayConnection),
      localComputeStatus(localCompute),
    ],
  };
}

export function useFreshCommandConsoleLocalCompute(
  localCompute: LocalComputeProbe,
  options?: LocalComputeFreshnessOptions,
): LocalComputeProbe {
  const freshnessMs = options?.freshnessMs ?? LOCAL_COMPUTE_FRESHNESS_MS;
  const [freshnessExpired, setFreshnessExpired] = React.useState(false);

  React.useEffect(() => {
    setFreshnessExpired(false);

    if (localCompute.error !== null || localCompute.status === null) {
      return;
    }

    const timeout = window.setTimeout(
      () => setFreshnessExpired(true),
      Math.max(0, freshnessMs),
    );
    return () => window.clearTimeout(timeout);
  }, [freshnessMs, localCompute.error, localCompute.status]);

  if (freshnessExpired) {
    return {
      error:
        "Local-compute status is stale: no successful probe completed before the freshness deadline.",
      status: null,
    };
  }

  return localCompute;
}

export function useCommandConsoleStatus(): CommandConsoleStatusViewModel {
  const relayConnection = useRelayConnection({ degradedAfterMs: 0 });
  const localComputeProbe = useMeshNodeStatus();
  const localCompute = useFreshCommandConsoleLocalCompute(localComputeProbe);

  return React.useMemo(
    () =>
      createCommandConsoleStatusViewModel({
        localCompute,
        relayConnection,
      }),
    [localCompute, relayConnection],
  );
}
