import * as React from "react";

import {
  parseCommandKnowledgeStatus,
  type AppleKnowledgeStatus,
  type CommandKnowledgeStatus,
  type KnowledgeValidation,
} from "@/features/command-console/domain/knowledgeStatus";
import type { ConnectionState } from "@/shared/api/relayClientShared";
import { useMeshNodeStatus } from "@/features/mesh-compute/hooks/useMeshNodeStatus";
import { getCommandKnowledgeStatus } from "@/shared/api/tauriCommandServices";
import type { MeshNodeStatus } from "@/shared/api/tauriMesh";
import {
  getLmStudioReadiness,
  type LmStudioReadiness,
} from "@/shared/api/tauriLmStudio";
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
  readonly facts?: readonly {
    readonly label: string;
    readonly value: string;
  }[];
  readonly diagnostics?: readonly string[];
};

export type CommandConsoleStatusViewModel = {
  readonly degradedSections: readonly string[];
  readonly liveServices: readonly CommandServiceStatus[];
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
  readonly lmStudio?: {
    readonly status: LmStudioReadiness | null;
    readonly error: string | null;
  };
  readonly knowledge?: {
    readonly status: CommandKnowledgeStatus | null;
    readonly error: string | null;
  };
};

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

function lmStudioStatus(probe: {
  status: LmStudioReadiness | null;
  error: string | null;
}): CommandServiceStatus {
  const base = { id: "lm-studio" as const, label: "LM Studio" };
  if (probe.error !== null) {
    return {
      ...base,
      detail: `Status probe failed: ${probe.error}`,
      state: "unavailable",
      statusLabel: "Unavailable",
    };
  }
  if (probe.status === null) {
    return {
      ...base,
      detail: "Waiting for the native LM Studio readiness probe.",
      state: "unavailable",
      statusLabel: "Unavailable",
    };
  }
  const status = probe.status;
  const securityWarnings = [...status.securityWarnings];
  if (
    status.bindExposure === "unknown" &&
    !securityWarnings.includes("LM Studio listener exposure is unverified.")
  ) {
    securityWarnings.push("LM Studio listener exposure is unverified.");
  }
  const detail =
    securityWarnings.length === 0
      ? status.detail
      : `${status.detail} ${securityWarnings.join(" ")}`;
  if (status.status === "app_missing") {
    return {
      ...base,
      detail: status.detail,
      state: "offline",
      statusLabel: "Offline",
    };
  }
  if (status.status === "api_unreachable") {
    return {
      ...base,
      detail: status.detail,
      state: "unavailable",
      statusLabel: "Unavailable",
    };
  }
  if (status.status === "ready" && securityWarnings.length === 0) {
    return {
      ...base,
      detail,
      state: "connected",
      statusLabel: "Connected",
    };
  }
  return {
    ...base,
    detail,
    state: "degraded",
    statusLabel: "Degraded",
  };
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

function titleCase(value: string): string {
  return value
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function validationFact(value: KnowledgeValidation) {
  return { label: "Validation", value: titleCase(value) };
}

function unavailableKnowledgeStatus(
  id: "memory" | "rag" | "apple-inputs",
  label: string,
): CommandServiceStatus {
  return {
    id,
    label,
    state: "unavailable",
    statusLabel: "Unavailable",
    detail:
      "Native knowledge status probe failed. Check the local service and its protected configuration.",
    diagnostics: [
      "Retry the local status probe after checking the service and Keychain-backed credential.",
    ],
  };
}

function memoryStatus(status: CommandKnowledgeStatus): CommandServiceStatus {
  const memory = status.memory;
  const base = { id: "memory" as const, label: "Memory" };
  if (memory.status === "not_configured") {
    return {
      ...base,
      detail: "Command Memory has no protected local configuration.",
      state: "not_configured",
      statusLabel: "Not configured",
    };
  }
  if (memory.status === "unavailable") {
    return {
      ...base,
      detail: "Authenticated local Memory readiness was not verified.",
      state: "unavailable",
      statusLabel: "Unavailable",
      facts: [
        { label: "Freshness", value: titleCase(memory.freshness) },
        validationFact(memory.validation),
      ],
      diagnostics: [
        "Check the local Memory service, protected node identity, and Keychain credential.",
      ],
    };
  }
  const conflicts = memory.conflictCount;
  const degraded = conflicts > 0;
  return {
    ...base,
    detail: degraded
      ? `${conflicts} unresolved conflicts are excluded from unattended adviser context.`
      : "Authenticated local Memory is ready for approved adviser reads.",
    state: degraded ? "degraded" : "connected",
    statusLabel: degraded ? "Degraded" : "Connected",
    facts: [
      { label: "Node", value: memory.nodeId ?? "Unknown" },
      {
        label: "Replication cursor",
        value: memory.replicationCursor?.toString() ?? "Unknown",
      },
      {
        label: "Last successful sync",
        value: memory.lastSuccessfulSync ?? "Unknown",
      },
      { label: "Revisions", value: memory.revisionCount.toString() },
      { label: "Conflicts", value: conflicts.toString() },
      { label: "Freshness", value: titleCase(memory.freshness) },
      validationFact(memory.validation),
      {
        label: "Permissions",
        value:
          memory.toolAllowlist.length > 0
            ? memory.toolAllowlist.join(", ")
            : "None",
      },
    ],
    diagnostics: degraded
      ? ["Resolve Memory conflicts before unattended brief generation."]
      : [],
  };
}

function ragStatus(status: CommandKnowledgeStatus): CommandServiceStatus {
  const rag = status.rag;
  const base = { id: "rag" as const, label: "RAG" };
  if (rag.status === "not_configured") {
    return {
      ...base,
      detail: "Command RAG has no protected local configuration.",
      state: "not_configured",
      statusLabel: "Not configured",
    };
  }
  if (
    rag.status === "unavailable" ||
    rag.validation !== "verified" ||
    rag.freshness !== "fresh"
  ) {
    return {
      ...base,
      detail:
        "The local RAG service did not prove a fresh signed active snapshot.",
      state: "unavailable",
      statusLabel: "Unavailable",
      facts: [
        {
          label: "Active snapshot",
          value: rag.activeSnapshotId ?? "Unknown",
        },
        { label: "Freshness", value: titleCase(rag.freshness) },
        validationFact(rag.validation),
      ],
      diagnostics: [
        "Restore the expected signed snapshot or re-run its staging validation.",
      ],
    };
  }
  return {
    ...base,
    detail:
      "A fresh signed active snapshot is verified for read-only retrieval.",
    state: "connected",
    statusLabel: "Connected",
    facts: [
      { label: "Active snapshot", value: rag.activeSnapshotId ?? "Unknown" },
      {
        label: "Signer fingerprint",
        value: rag.signatureFingerprint ?? "Unknown",
      },
      { label: "Snapshot time", value: rag.snapshotTime ?? "Unknown" },
      {
        label: "Last activation",
        value: rag.lastSuccessfulActivation ?? "Unknown",
      },
      { label: "Freshness", value: titleCase(rag.freshness) },
      validationFact(rag.validation),
      {
        label: "Permissions",
        value:
          rag.toolAllowlist.length > 0 ? rag.toolAllowlist.join(", ") : "None",
      },
    ],
  };
}

const APPLE_LABELS: Record<AppleKnowledgeStatus["source"], string> = {
  calendar: "Calendar",
  reminders: "Reminders",
  notes: "Notes",
  files: "Files",
};

function appleStatus(status: CommandKnowledgeStatus): CommandServiceStatus {
  const base = { id: "apple-inputs" as const, label: "Apple inputs" };
  const sources = status.appleInputs;
  if (sources.length === 0) {
    return {
      ...base,
      detail: "No Apple input permission status is available.",
      state: "not_configured",
      statusLabel: "Not configured",
    };
  }
  const facts = sources.map((source) => ({
    label: APPLE_LABELS[source.source],
    value: titleCase(source.permission),
  }));
  if (sources.every(({ permission }) => permission === "unavailable")) {
    return {
      ...base,
      detail: "The signed Apple input helper is unavailable.",
      state: "unavailable",
      statusLabel: "Unavailable",
      facts,
      diagnostics: ["Check the bundled helper and macOS privacy settings."],
    };
  }
  const degraded = sources.some(
    ({ permission, error }) => permission !== "authorized" || error !== null,
  );
  return {
    ...base,
    detail: degraded
      ? "Some read-only Apple input sections are unavailable and will degrade independently."
      : "All configured read-only Apple input permissions are authorized.",
    state: degraded ? "degraded" : "connected",
    statusLabel: degraded ? "Degraded" : "Connected",
    facts,
    diagnostics: degraded
      ? [
          "Review denied or restricted sources in macOS System Settings; other sources remain usable.",
        ]
      : [],
  };
}

export function createCommandConsoleStatusViewModel({
  knowledge,
  lmStudio,
  localCompute,
  relayConnection,
}: CommandConsoleStatusSources): CommandConsoleStatusViewModel {
  const knowledgeServices =
    knowledge === undefined
      ? []
      : knowledge.error !== null || knowledge.status === null
        ? [
            unavailableKnowledgeStatus("memory", "Memory"),
            unavailableKnowledgeStatus("rag", "RAG"),
            unavailableKnowledgeStatus("apple-inputs", "Apple inputs"),
          ]
        : [
            memoryStatus(knowledge.status),
            ragStatus(knowledge.status),
            appleStatus(knowledge.status),
          ];
  return {
    degradedSections:
      knowledge === undefined
        ? []
        : knowledge.error !== null || knowledge.status === null
          ? ["knowledge-status"]
          : knowledge.status.degradedSections,
    liveServices: [
      relayStatus(relayConnection),
      localComputeStatus(localCompute),
      ...(lmStudio ? [lmStudioStatus(lmStudio)] : []),
      ...knowledgeServices,
    ],
  };
}

function useLmStudioReadiness(): {
  status: LmStudioReadiness | null;
  error: string | null;
} {
  const [status, setStatus] = React.useState<LmStudioReadiness | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    const probe = async () => {
      try {
        const value = await getLmStudioReadiness();
        if (!cancelled) {
          setStatus(value);
          setError(null);
        }
      } catch (cause) {
        if (!cancelled) {
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      }
    };
    void probe();
    const interval = window.setInterval(() => void probe(), 5_000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, []);

  return { status, error };
}

function useCommandKnowledgeStatus(): {
  status: CommandKnowledgeStatus | null;
  error: string | null;
} {
  const [status, setStatus] = React.useState<CommandKnowledgeStatus | null>(
    null,
  );
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    let retry: number | undefined;
    const probe = async () => {
      try {
        const value = parseCommandKnowledgeStatus(
          await getCommandKnowledgeStatus(),
        );
        if (!value) throw new Error("invalid_knowledge_status");
        if (!cancelled) {
          setStatus(value);
          setError(null);
        }
      } catch {
        if (!cancelled) {
          setStatus(null);
          setError("knowledge_status_unavailable");
        }
      } finally {
        if (!cancelled) {
          retry = window.setTimeout(() => void probe(), 15_000);
        }
      }
    };
    void probe();
    return () => {
      cancelled = true;
      if (retry !== undefined) window.clearTimeout(retry);
    };
  }, []);

  return { status, error };
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
  const lmStudio = useLmStudioReadiness();
  const knowledge = useCommandKnowledgeStatus();

  return React.useMemo(
    () =>
      createCommandConsoleStatusViewModel({
        localCompute,
        lmStudio,
        knowledge,
        relayConnection,
      }),
    [knowledge, lmStudio, localCompute, relayConnection],
  );
}
