import type {
  AcpRuntimeCatalogEntry,
  ManagedAgent,
  ManagedAgentRuntimeStatus,
} from "@/shared/api/types";
import type {
  ConnectionState,
  ObserverEvent,
} from "@/features/agents/ui/agentSessionTypes";

const MAX_SAFE_LABEL_LENGTH = 120;
const MAX_MANIFEST_ITEMS = 100;

export type CapabilityEvidenceState = "reported" | "unavailable" | "unknown";

export type ManifestFreshness = "fresh" | "stale" | "unknown";
export type ManifestOverallStatus =
  | "ready"
  | "attention"
  | "stopped"
  | "unknown";
export type ReadinessStatus = "ready" | "attention" | "pending" | "unknown";
export type ReadinessCheckId =
  | "installation"
  | "authentication"
  | "process"
  | "community"
  | "presence"
  | "observer";
export type ToolRiskClass =
  | "read"
  | "write"
  | "execute"
  | "external"
  | "unknown";

export type CapabilityFeature = {
  id: string;
  label: string;
  state: CapabilityEvidenceState;
  source: "runtime" | "buzzCatalog";
};

export type ReadinessCheck = {
  id: ReadinessCheckId;
  label: string;
  status: ReadinessStatus;
  detail: string;
};

const readinessGate: Record<ReadinessCheckId, boolean> = {
  installation: true,
  authentication: true,
  process: true,
  community: true,
  presence: true,
  observer: true,
};

export type ManifestTool = {
  name: string;
  source: string | null;
  riskClass: ToolRiskClass;
  availability: CapabilityEvidenceState;
};

export type ManifestPermissionMode = {
  requested: string | null;
  effective: string | null;
  source: "runtime" | "buzzHarness" | "unknown";
};

export type AgentCapabilityManifest = {
  overallStatus: ManifestOverallStatus;
  freshness: ManifestFreshness;
  lastVerifiedAt: string | null;
  runtime: {
    id: string | null;
    label: string;
    version: string | null;
  };
  protocolVersion: string | null;
  model: {
    value: string | null;
    source: "observed" | "configured" | "unknown";
  };
  provider: {
    value: string | null;
    source: "configured" | "unknown";
  };
  readiness: ReadinessCheck[];
  features: CapabilityFeature[];
  commands: string[];
  commandsState: CapabilityEvidenceState;
  toolSources: string[];
  toolSourcesState: CapabilityEvidenceState;
  tools: ManifestTool[];
  toolsState: CapabilityEvidenceState;
  permissionMode: ManifestPermissionMode;
  limitations: string[];
};

type ManifestInputs = {
  agent: ManagedAgent;
  runtime: AcpRuntimeCatalogEntry | undefined;
  runtimeStatus: ManagedAgentRuntimeStatus | undefined;
  presenceStatus: "online" | "away" | "offline" | undefined;
  observer: {
    connectionState: ConnectionState;
    events: ObserverEvent[];
  };
  catalogObservedAt?: string | null;
  runtimeObservedAt?: string | null;
};

type ParsedInitialize = {
  event: ObserverEvent;
  result: Record<string, unknown>;
};

type ParsedSessionManifest = {
  event: ObserverEvent;
  payload: Record<string, unknown>;
  manifest: Record<string, unknown>;
};

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function safeLabel(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  const containsControlCharacter = Array.from(trimmed).some((character) => {
    const code = character.charCodeAt(0);
    return code < 32 || code === 127;
  });
  if (
    trimmed.length === 0 ||
    trimmed.length > MAX_SAFE_LABEL_LENGTH ||
    containsControlCharacter
  ) {
    return null;
  }
  return trimmed;
}

function safeCapabilityName(value: unknown): string | null {
  const label = safeLabel(value);
  if (
    !label ||
    label.startsWith("/") ||
    label.includes("\\") ||
    label.includes("..") ||
    /^[a-z][a-z0-9+.-]*:\/\//i.test(label) ||
    label.includes("=")
  ) {
    return null;
  }
  return label;
}

function latestEvent(
  events: readonly ObserverEvent[],
  predicate: (event: ObserverEvent) => boolean,
  minimum?: ObserverEvent | null,
): ObserverEvent | null {
  let latest: ObserverEvent | null = null;
  for (const event of events) {
    if (
      !predicate(event) ||
      (minimum !== undefined &&
        minimum !== null &&
        !eventIsSameOrAfter(event, minimum))
    ) {
      continue;
    }
    if (
      !latest ||
      Date.parse(event.timestamp) > Date.parse(latest.timestamp) ||
      (event.timestamp === latest.timestamp && event.seq > latest.seq)
    ) {
      latest = event;
    }
  }
  return latest;
}

function eventIsSameOrAfter(
  candidate: ObserverEvent,
  minimum: ObserverEvent,
): boolean {
  const candidateTime = Date.parse(candidate.timestamp);
  const minimumTime = Date.parse(minimum.timestamp);
  if (!Number.isFinite(candidateTime) || !Number.isFinite(minimumTime)) {
    return candidate.seq >= minimum.seq;
  }
  return (
    candidateTime > minimumTime ||
    (candidateTime === minimumTime && candidate.seq >= minimum.seq)
  );
}

function parseLatestInitialize(
  events: readonly ObserverEvent[],
): ParsedInitialize | null {
  const event = latestEvent(events, (candidate) => {
    if (candidate.kind !== "agent_initialized") return false;
    const payload = asRecord(candidate.payload);
    return asRecord(payload?.initializeResult) !== null;
  });
  if (!event) return null;
  const payload = asRecord(event.payload);
  const result = asRecord(payload?.initializeResult);
  return result ? { event, result } : null;
}

function parseLatestSessionManifest(
  events: readonly ObserverEvent[],
  minimum?: ObserverEvent | null,
): ParsedSessionManifest | null {
  const event = latestEvent(
    events,
    (candidate) => {
      if (candidate.kind !== "session_config_captured") return false;
      const payload = asRecord(candidate.payload);
      return asRecord(payload?.capabilityManifest) !== null;
    },
    minimum,
  );
  if (!event) return null;
  const payload = asRecord(event.payload);
  const manifest = asRecord(payload?.capabilityManifest);
  return payload && manifest ? { event, payload, manifest } : null;
}

function readBooleanCapability(
  container: Record<string, unknown> | null,
  key: string,
): CapabilityEvidenceState {
  if (!container || !(key in container)) return "unknown";
  const value = container[key];
  if (value === true) return "reported";
  if (value === false) return "unavailable";
  return "unknown";
}

function liveFeatures(
  initialize: ParsedInitialize | null,
): CapabilityFeature[] {
  const capabilities = asRecord(initialize?.result.agentCapabilities);
  const prompt = asRecord(capabilities?.promptCapabilities);
  const output = asRecord(capabilities?.outputCapabilities);
  return [
    {
      id: "image-input",
      label: "Image input",
      state: readBooleanCapability(prompt, "image"),
      source: "runtime",
    },
    {
      id: "audio-input",
      label: "Audio input",
      state: readBooleanCapability(prompt, "audio"),
      source: "runtime",
    },
    {
      id: "embedded-context",
      label: "Embedded context",
      state: readBooleanCapability(prompt, "embeddedContext"),
      source: "runtime",
    },
    {
      id: "image-output",
      label: "Image output",
      state: readBooleanCapability(output, "image"),
      source: "runtime",
    },
    {
      id: "audio-output",
      label: "Audio output",
      state: readBooleanCapability(output, "audio"),
      source: "runtime",
    },
  ];
}

function catalogFeatures(
  runtime: AcpRuntimeCatalogEntry | undefined,
): CapabilityFeature[] {
  if (!runtime) {
    return [
      {
        id: "native-config",
        label: "Native ACP config",
        state: "unknown",
        source: "buzzCatalog",
      },
      {
        id: "model-switching",
        label: "Native model switching",
        state: "unknown",
        source: "buzzCatalog",
      },
      {
        id: "buzz-mcp-hooks",
        label: "Buzz MCP hooks",
        state: "unknown",
        source: "buzzCatalog",
      },
    ];
  }
  return [
    {
      id: "native-config",
      label: "Native ACP config",
      state: catalogEvidence(runtime.supportsAcpNativeConfig),
      source: "buzzCatalog",
    },
    {
      id: "model-switching",
      label: "Native model switching",
      state: catalogEvidence(runtime.supportsAcpModelSwitching),
      source: "buzzCatalog",
    },
    {
      id: "buzz-mcp-hooks",
      label: "Buzz MCP hooks",
      state: catalogEvidence(runtime.mcpHooks),
      source: "buzzCatalog",
    },
  ];
}

function catalogEvidence(value: boolean | null): CapabilityEvidenceState {
  if (value === true) return "reported";
  if (value === false) return "unavailable";
  return "unknown";
}

function parseCurrentModel(
  session: ParsedSessionManifest | null,
): string | null {
  const models = session ? session.payload.models : null;
  const object = asRecord(models);
  const objectModel = safeLabel(object?.currentModelId);
  if (objectModel) return objectModel;
  if (!Array.isArray(models)) return null;
  for (const candidate of models.slice(0, MAX_MANIFEST_ITEMS)) {
    const model = asRecord(candidate);
    if (model?.isCurrent !== true) continue;
    const id = safeLabel(model.modelId) ?? safeLabel(model.id);
    if (id) return id;
  }
  return null;
}

function parseCommands(
  events: readonly ObserverEvent[],
  minimum?: ObserverEvent | null,
): {
  commands: string[];
  state: CapabilityEvidenceState;
  event: ObserverEvent | null;
} {
  const event = latestEvent(
    events,
    (candidate) => {
      if (candidate.kind !== "acp_read") return false;
      const payload = asRecord(candidate.payload);
      const params = asRecord(payload?.params);
      const update = asRecord(params?.update);
      return update?.sessionUpdate === "available_commands_update";
    },
    minimum,
  );
  if (!event) return { commands: [], state: "unknown", event: null };
  const payload = asRecord(event.payload);
  const params = asRecord(payload?.params);
  const update = asRecord(params?.update);
  const rawCommands = update?.availableCommands;
  if (!Array.isArray(rawCommands)) {
    return { commands: [], state: "unknown", event };
  }
  const commands = rawCommands
    .slice(0, MAX_MANIFEST_ITEMS)
    .map((item) => safeCapabilityName(asRecord(item)?.name))
    .filter((name): name is string => name !== null);
  return {
    commands: [...new Set(commands)].sort(),
    state:
      commands.length > 0
        ? "reported"
        : rawCommands.length === 0
          ? "unavailable"
          : "unknown",
    event,
  };
}

function parseToolSources(session: ParsedSessionManifest | null): {
  sources: string[];
  state: CapabilityEvidenceState;
} {
  if (!session) return { sources: [], state: "unknown" };
  const rawSources = session.manifest.toolSources;
  if (!Array.isArray(rawSources)) return { sources: [], state: "unknown" };
  const sources = rawSources
    .slice(0, MAX_MANIFEST_ITEMS)
    .map((item) => safeCapabilityName(asRecord(item)?.name))
    .filter((name): name is string => name !== null);
  return {
    sources: [...new Set(sources)].sort(),
    state:
      sources.length > 0
        ? "reported"
        : rawSources.length === 0
          ? "unavailable"
          : "unknown",
  };
}

function parseTools(initialize: ParsedInitialize | null): {
  tools: ManifestTool[];
  state: CapabilityEvidenceState;
} {
  const capabilities = asRecord(initialize?.result.agentCapabilities);
  const rawTools = capabilities?.tools;
  if (!Array.isArray(rawTools)) return { tools: [], state: "unknown" };
  const tools = rawTools
    .slice(0, MAX_MANIFEST_ITEMS)
    .map((item): ManifestTool | null => {
      const tool = asRecord(item);
      const name = safeCapabilityName(tool?.name);
      if (!tool || !name) return null;
      const source = safeCapabilityName(tool.source);
      const rawRisk = safeLabel(tool.riskClass);
      const riskClass: ToolRiskClass =
        rawRisk === "read" ||
        rawRisk === "write" ||
        rawRisk === "execute" ||
        rawRisk === "external"
          ? rawRisk
          : "unknown";
      const availability =
        tool.available === true
          ? "reported"
          : tool.available === false
            ? "unavailable"
            : "unknown";
      return { name, source, riskClass, availability };
    })
    .filter((tool): tool is ManifestTool => tool !== null);
  return {
    tools,
    state:
      tools.length > 0
        ? "reported"
        : rawTools.length === 0
          ? "unavailable"
          : "unknown",
  };
}

function parsePermissionMode(
  session: ParsedSessionManifest | null,
): ManifestPermissionMode {
  const raw = asRecord(session?.manifest.permissionMode);
  const requested = safeCapabilityName(raw?.requested);
  const effective = safeCapabilityName(raw?.effective);
  const source =
    raw?.source === "runtime" || raw?.source === "buzzHarness"
      ? raw.source
      : "unknown";
  return { requested, effective, source };
}

function runtimeIdentity(
  initialize: ParsedInitialize | null,
  runtime: AcpRuntimeCatalogEntry | undefined,
) {
  const liveInfo =
    asRecord(initialize?.result.agentInfo) ??
    asRecord(initialize?.result.serverInfo);
  const liveName = safeCapabilityName(liveInfo?.name);
  return {
    id: runtime?.id ?? null,
    label: liveName ?? runtime?.label ?? "Unknown runtime",
    version: safeCapabilityName(liveInfo?.version),
  };
}

function readinessChecks(
  agent: ManagedAgent,
  runtime: AcpRuntimeCatalogEntry | undefined,
  runtimeStatus: ManagedAgentRuntimeStatus | undefined,
  presenceStatus: "online" | "away" | "offline" | undefined,
  observerState: ConnectionState,
): ReadinessCheck[] {
  const installation: ReadinessCheck = runtime
    ? runtime.availability === "available"
      ? {
          id: "installation",
          label: "Installation",
          status: "ready",
          detail: "Runtime and adapter available",
        }
      : {
          id: "installation",
          label: "Installation",
          status: "attention",
          detail: runtime.availability.replaceAll("_", " "),
        }
    : {
        id: "installation",
        label: "Installation",
        status: "unknown",
        detail: "Runtime not matched to the catalog",
      };
  const authStatus = runtime?.authStatus.status;
  const authentication: ReadinessCheck =
    authStatus === "logged_in" || authStatus === "not_applicable"
      ? {
          id: "authentication",
          label: "Authentication",
          status: "ready",
          detail:
            authStatus === "not_applicable" ? "Not required" : "Authenticated",
        }
      : authStatus === "logged_out" || authStatus === "config_invalid"
        ? {
            id: "authentication",
            label: "Authentication",
            status: "attention",
            detail:
              authStatus === "config_invalid"
                ? "Configuration invalid"
                : "Sign-in required",
          }
        : {
            id: "authentication",
            label: "Authentication",
            status: "unknown",
            detail: "Not verified",
          };
  const active = agent.status === "running" || agent.status === "deployed";
  const process: ReadinessCheck = {
    id: "process",
    label: "Process",
    status: active ? "ready" : "attention",
    detail: active ? "Running" : agent.status.replaceAll("_", " "),
  };
  const community: ReadinessCheck = !runtimeStatus
    ? {
        id: "community",
        label: "Community",
        status: "unknown",
        detail: "No lifecycle observation",
      }
    : runtimeStatus.lifecycle === "ready"
      ? {
          id: "community",
          label: "Community",
          status: "ready",
          detail: "Connected",
        }
      : runtimeStatus.lifecycle === "failed" ||
          runtimeStatus.lifecycle === "stopped"
        ? {
            id: "community",
            label: "Community",
            status: "attention",
            detail: runtimeStatus.lifecycle === "failed" ? "Failed" : "Stopped",
          }
        : {
            id: "community",
            label: "Community",
            status: "pending",
            detail: runtimeStatus.lifecycle,
          };
  const presence: ReadinessCheck =
    presenceStatus === "online"
      ? {
          id: "presence",
          label: "Presence",
          status: "ready",
          detail: "Online",
        }
      : presenceStatus === "away"
        ? {
            id: "presence",
            label: "Presence",
            status: "pending",
            detail: "Away",
          }
        : presenceStatus === "offline"
          ? {
              id: "presence",
              label: "Presence",
              status: "attention",
              detail: "Offline",
            }
          : {
              id: "presence",
              label: "Presence",
              status: "unknown",
              detail: "Not reported",
            };
  const observer: ReadinessCheck =
    observerState === "open"
      ? {
          id: "observer",
          label: "Observer",
          status: "ready",
          detail: "Connected",
        }
      : observerState === "error" || observerState === "closed"
        ? {
            id: "observer",
            label: "Observer",
            status: "attention",
            detail: observerState === "error" ? "Error" : "Disconnected",
          }
        : {
            id: "observer",
            label: "Observer",
            status: observerState === "connecting" ? "pending" : "unknown",
            detail:
              observerState === "connecting" ? "Connecting" : "Not connected",
          };
  return [installation, authentication, process, community, presence, observer];
}

function toIsoTimestamp(value: string | null | undefined): string | null {
  if (!value) return null;
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) ? new Date(timestamp).toISOString() : null;
}

function newestTimestamp(
  values: Array<string | null | undefined>,
): string | null {
  let newest: string | null = null;
  for (const value of values) {
    const normalized = toIsoTimestamp(value);
    if (!normalized) continue;
    if (!newest || Date.parse(normalized) > Date.parse(newest)) {
      newest = normalized;
    }
  }
  return newest;
}

function manifestFreshness(
  agent: ManagedAgent,
  runtimeStatus: ManagedAgentRuntimeStatus | undefined,
  presenceStatus: "online" | "away" | "offline" | undefined,
  observerState: ConnectionState,
  initialize: ParsedInitialize | null,
): ManifestFreshness {
  if (!initialize) return "unknown";
  const active = agent.status === "running" || agent.status === "deployed";
  if (!active || observerState !== "open") return "stale";
  if (runtimeStatus && runtimeStatus.lifecycle !== "ready") return "stale";
  if (presenceStatus && presenceStatus !== "online") return "stale";
  if (!runtimeStatus || !presenceStatus) return "unknown";
  const startedAt = toIsoTimestamp(agent.lastStartedAt);
  const initializedAt = toIsoTimestamp(initialize.event.timestamp);
  if (
    startedAt &&
    initializedAt &&
    Date.parse(initializedAt) < Date.parse(startedAt)
  ) {
    return "stale";
  }
  return "fresh";
}

function overallStatus(
  agent: ManagedAgent,
  checks: readonly ReadinessCheck[],
  freshness: ManifestFreshness,
): ManifestOverallStatus {
  if (agent.status !== "running" && agent.status !== "deployed") {
    return "stopped";
  }
  const gating = checks.filter((check) => readinessGate[check.id]);
  if (gating.some((check) => check.status === "attention")) return "attention";
  if (
    freshness === "fresh" &&
    gating.every((check) => check.status === "ready")
  ) {
    return "ready";
  }
  return "unknown";
}

export function findManifestRuntime(
  agent: ManagedAgent,
  runtimes: readonly AcpRuntimeCatalogEntry[],
): AcpRuntimeCatalogEntry | undefined {
  const command = agent.agentCommand.trim();
  const basename = command.split(/[\\/]/).at(-1) ?? command;
  return runtimes.find(
    (runtime) =>
      runtime.id === command ||
      runtime.id === basename ||
      runtime.command === command ||
      runtime.command === basename,
  );
}

export function buildAgentCapabilityManifest({
  agent,
  runtime,
  runtimeStatus,
  presenceStatus,
  observer,
  catalogObservedAt,
  runtimeObservedAt,
}: ManifestInputs): AgentCapabilityManifest {
  const initialize = parseLatestInitialize(observer.events);
  const session = parseLatestSessionManifest(
    observer.events,
    initialize?.event,
  );
  const commands = parseCommands(observer.events, initialize?.event);
  const toolSources = parseToolSources(session);
  const tools = parseTools(initialize);
  const permissionMode = parsePermissionMode(session);
  const freshness = manifestFreshness(
    agent,
    runtimeStatus,
    presenceStatus,
    observer.connectionState,
    initialize,
  );
  const readiness = readinessChecks(
    agent,
    runtime,
    runtimeStatus,
    presenceStatus,
    observer.connectionState,
  );
  const identity = runtimeIdentity(initialize, runtime);
  const protocolVersionValue = initialize?.result.protocolVersion;
  const protocolVersion =
    typeof protocolVersionValue === "number" &&
    Number.isFinite(protocolVersionValue)
      ? String(protocolVersionValue)
      : safeLabel(protocolVersionValue);
  const observedModel = parseCurrentModel(session);
  const configuredModel = safeLabel(agent.model);
  const configuredProvider = safeLabel(agent.provider);
  const limitations: string[] = [];
  if (!initialize) {
    limitations.push("Runtime capabilities have not been reported.");
  }
  if (!identity.version) limitations.push("Runtime version is unreported.");
  if (!protocolVersion) limitations.push("ACP protocol version is unreported.");
  if (tools.state === "unknown") {
    limitations.push("Tool descriptors and risk classes are unreported.");
  }
  if (!permissionMode.effective) {
    limitations.push("Effective permission behavior is unreported.");
  }
  if (runtime?.requiresExternalCli) {
    limitations.push(
      "This runtime depends on a separately installed vendor CLI.",
    );
  }
  if (runtime?.nodeRequired) {
    limitations.push(
      "Node.js and npm are required before the ACP adapter can be installed.",
    );
  }

  return {
    overallStatus: overallStatus(agent, readiness, freshness),
    freshness,
    lastVerifiedAt: newestTimestamp([
      initialize?.event.timestamp,
      session?.event.timestamp,
      commands.event?.timestamp,
      catalogObservedAt,
      runtimeObservedAt,
    ]),
    runtime: identity,
    protocolVersion,
    model: observedModel
      ? { value: observedModel, source: "observed" }
      : configuredModel
        ? { value: configuredModel, source: "configured" }
        : { value: null, source: "unknown" },
    provider: configuredProvider
      ? { value: configuredProvider, source: "configured" }
      : { value: null, source: "unknown" },
    readiness,
    features: [...liveFeatures(initialize), ...catalogFeatures(runtime)],
    commands: commands.commands,
    commandsState: commands.state,
    toolSources: toolSources.sources,
    toolSourcesState: toolSources.state,
    tools: tools.tools,
    toolsState: tools.state,
    permissionMode,
    limitations,
  };
}
