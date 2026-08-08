import type {
  AcpRuntimeCatalogEntry,
  ManagedAgent,
  ManagedAgentRuntimeStatus,
} from "@/shared/api/types";
import type {
  ConnectionState,
  ObserverEvent,
} from "@/features/agents/ui/agentSessionTypes";
import { commandsMatch } from "@/features/agents/agentReuse";

const MAX_SAFE_LABEL_LENGTH = 120;
const MAX_MANIFEST_ITEMS = 100;

/** Whether a capability is positively reported, absent, or not yet known. */
export type CapabilityEvidenceState = "reported" | "unavailable" | "unknown";

/** Validity of the local evidence against the current process and connection. */
export type ManifestFreshness = "fresh" | "stale" | "unknown";
/** Summary state for the owner-visible local readiness card. */
export type ManifestOverallStatus =
  | "ready"
  | "attention"
  | "stopped"
  | "unknown";
/** Status of one local readiness prerequisite. */
export type ReadinessStatus = "ready" | "attention" | "pending" | "unknown";
/** Stable identifier for a local readiness prerequisite. */
export type ReadinessCheckId =
  | "installation"
  | "authentication"
  | "credential_persistence"
  | "process"
  | "community"
  | "presence"
  | "observer";
/** Coarse display-only risk class derived from a reported tool name. */
export type ToolRiskClass =
  | "read"
  | "write"
  | "execute"
  | "external"
  | "unknown";

/** One runtime- or catalog-backed feature shown in the manifest. */
export type CapabilityFeature = {
  id: string;
  label: string;
  state: CapabilityEvidenceState;
  source: "runtime" | "buzzCatalog";
};

/** One owner-local readiness check and its supporting detail. */
export type ReadinessCheck = {
  id: ReadinessCheckId;
  label: string;
  status: ReadinessStatus;
  detail: string;
};

/** Sanitized tool evidence safe for the owner-visible manifest. */
export type ManifestTool = {
  name: string;
  source: string | null;
  riskClass: ToolRiskClass;
  availability: CapabilityEvidenceState;
};

/** Requested and locally observed permission behavior. */
export type ManifestPermissionMode = {
  requested: string | null;
  effective: string | null;
  source: "runtime" | "buzzHarness" | "unknown";
};

/** Session identity for the evidence shown in the manifest. */
export type ManifestSessionIdentity = {
  /** ACP session ID, or null when no session has been observed. */
  sessionId: string | null;
  /** Channel UUID the session evidence came from, or null. */
  channelId: string | null;
};

/** Owner-local capability and readiness projection for a managed agent. */
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
    source: "applied" | "reported" | "configured" | "unknown";
    requested: string | null;
    matchesRequested: boolean | null;
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
  /** Session and channel the evidence was drawn from. */
  sessionEvidence: ManifestSessionIdentity;
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
    capabilityEvidence?: AgentCapabilityEvidence;
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

/** Durable observer evidence retained independently of the capped event log. */
export type AgentCapabilityEvidence = {
  initializeEvent: ObserverEvent | null;
  sessionConfigEvent: ObserverEvent | null;
  commandsEvent: ObserverEvent | null;
};

/** Empty durable evidence state for a new or reset observer stream. */
export const EMPTY_AGENT_CAPABILITY_EVIDENCE: AgentCapabilityEvidence = {
  initializeEvent: null,
  sessionConfigEvent: null,
  commandsEvent: null,
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

function eventsHaveConflictingSessions(
  candidate: ObserverEvent,
  minimum: ObserverEvent,
): boolean {
  return (
    candidate.sessionId !== null &&
    minimum.sessionId !== null &&
    candidate.sessionId !== minimum.sessionId
  );
}

function isInitializeEvent(event: ObserverEvent): boolean {
  if (event.kind !== "agent_initialized") return false;
  const payload = asRecord(event.payload);
  return asRecord(payload?.initializeResult) !== null;
}

function isSessionManifestEvent(event: ObserverEvent): boolean {
  if (event.kind !== "session_config_captured") return false;
  const payload = asRecord(event.payload);
  return asRecord(payload?.capabilityManifest) !== null;
}

function isCommandsEvent(event: ObserverEvent): boolean {
  if (event.kind !== "acp_read") return false;
  const payload = asRecord(event.payload);
  const params = asRecord(payload?.params);
  const update = asRecord(params?.update);
  return update?.sessionUpdate === "available_commands_update";
}

/** Fold one observer event into the durable capability evidence state. */
export function reduceAgentCapabilityEvidence(
  current: AgentCapabilityEvidence,
  event: ObserverEvent,
): AgentCapabilityEvidence {
  if (isInitializeEvent(event)) {
    if (
      current.initializeEvent &&
      !eventIsSameOrAfter(event, current.initializeEvent)
    ) {
      return current;
    }
    const sessionConfigEvent =
      current.sessionConfigEvent &&
      eventIsSameOrAfter(current.sessionConfigEvent, event)
        ? current.sessionConfigEvent
        : null;
    const commandsMinimum = sessionConfigEvent ?? event;
    const commandsEvent =
      current.commandsEvent &&
      !eventsHaveConflictingSessions(current.commandsEvent, commandsMinimum) &&
      eventIsSameOrAfter(current.commandsEvent, commandsMinimum)
        ? current.commandsEvent
        : null;
    return {
      initializeEvent: event,
      sessionConfigEvent,
      commandsEvent,
    };
  }

  if (isSessionManifestEvent(event)) {
    if (
      (current.initializeEvent &&
        !eventIsSameOrAfter(event, current.initializeEvent)) ||
      (current.sessionConfigEvent &&
        !eventIsSameOrAfter(event, current.sessionConfigEvent))
    ) {
      return current;
    }
    return {
      ...current,
      sessionConfigEvent: event,
      // Commands are session-scoped. Keep an out-of-order command frame only
      // when it is from this session or later; otherwise wait for a fresh
      // available_commands_update frame.
      commandsEvent:
        current.commandsEvent &&
        !eventsHaveConflictingSessions(current.commandsEvent, event) &&
        eventIsSameOrAfter(current.commandsEvent, event)
          ? current.commandsEvent
          : null,
    };
  }

  if (isCommandsEvent(event)) {
    const minimum =
      current.sessionConfigEvent ?? current.initializeEvent ?? null;
    if (
      (minimum &&
        (eventsHaveConflictingSessions(event, minimum) ||
          !eventIsSameOrAfter(event, minimum))) ||
      (current.commandsEvent &&
        !eventIsSameOrAfter(event, current.commandsEvent))
    ) {
      return current;
    }
    return { ...current, commandsEvent: event };
  }

  return current;
}

/** Reduce an event snapshot into durable capability evidence. */
export function reduceAgentCapabilityEvidenceEvents(
  events: readonly ObserverEvent[],
): AgentCapabilityEvidence {
  return events.reduce(
    reduceAgentCapabilityEvidence,
    EMPTY_AGENT_CAPABILITY_EVIDENCE,
  );
}

function parseInitializeEvent(
  event: ObserverEvent | null,
): ParsedInitialize | null {
  if (!event || !isInitializeEvent(event)) return null;
  const payload = asRecord(event.payload);
  const result = asRecord(payload?.initializeResult);
  return result ? { event, result } : null;
}

function parseSessionManifestEvent(
  event: ObserverEvent | null,
): ParsedSessionManifest | null {
  if (!event || !isSessionManifestEvent(event)) return null;
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

function parseModelApplication(session: ParsedSessionManifest | null): {
  requested: string | null;
  applied: string | null;
} {
  const raw = asRecord(session?.manifest.modelApplication);
  const requested = safeLabel(raw?.requested);
  return {
    requested,
    applied: raw?.applied === true ? requested : null,
  };
}

function parseCommands(event: ObserverEvent | null): {
  commands: string[];
  state: CapabilityEvidenceState;
  event: ObserverEvent | null;
} {
  if (!event || !isCommandsEvent(event)) {
    return { commands: [], state: "unknown", event: null };
  }
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
  const credentialPersistence: ReadinessCheck =
    agent.credentialPersistence === "keyring_verified"
      ? {
          id: "credential_persistence",
          label: "Credential persistence",
          status: "ready",
          detail: "Keyring entry verified",
        }
      : agent.credentialPersistence === "inline_fallback"
        ? {
            id: "credential_persistence",
            label: "Credential persistence",
            status: "ready",
            detail: "Key stored inline (keyring unreachable at last save)",
          }
        : agent.credentialPersistence === "missing"
          ? {
              id: "credential_persistence",
              label: "Credential persistence",
              status: "attention",
              detail: "No key found in keyring or inline storage",
            }
          : {
              id: "credential_persistence",
              label: "Credential persistence",
              status: "unknown",
              detail: "Keyring unavailable — cannot determine persistence",
            };
  const active = isActiveManagedAgent(agent);
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
  return [
    installation,
    authentication,
    credentialPersistence,
    process,
    community,
    presence,
    observer,
  ];
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
  const active = isActiveManagedAgent(agent);
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
  if (!isActiveManagedAgent(agent)) {
    return "stopped";
  }
  if (checks.some((check) => check.status === "attention")) return "attention";
  if (
    freshness === "fresh" &&
    checks.every((check) => check.status === "ready")
  ) {
    return "ready";
  }
  return "unknown";
}

function isActiveManagedAgent(agent: ManagedAgent): boolean {
  return agent.status === "running" || agent.status === "deployed";
}

/** Match a managed agent command to the runtime catalog using shared semantics. */
export function findManifestRuntime(
  agent: ManagedAgent,
  runtimes: readonly AcpRuntimeCatalogEntry[],
): AcpRuntimeCatalogEntry | undefined {
  return runtimes.find(
    (runtime) =>
      commandsMatch(agent.agentCommand, runtime.id) ||
      (runtime.command !== null &&
        commandsMatch(agent.agentCommand, runtime.command)),
  );
}

/** Build the local owner-only manifest from catalog, runtime, and observer facts. */
export function buildAgentCapabilityManifest({
  agent,
  runtime,
  runtimeStatus,
  presenceStatus,
  observer,
  catalogObservedAt,
  runtimeObservedAt,
}: ManifestInputs): AgentCapabilityManifest {
  const capabilityEvidence =
    observer.capabilityEvidence ??
    reduceAgentCapabilityEvidenceEvents(observer.events);
  const initialize = parseInitializeEvent(capabilityEvidence.initializeEvent);
  const session = parseSessionManifestEvent(
    capabilityEvidence.sessionConfigEvent,
  );
  const commands = parseCommands(capabilityEvidence.commandsEvent);
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
  const modelApplication = parseModelApplication(session);
  const reportedModel = parseCurrentModel(session);
  const configuredModel = safeLabel(agent.model);
  const requestedModel = modelApplication.requested ?? configuredModel;
  const appliedModel = modelApplication.applied;
  const modelValue = appliedModel ?? reportedModel ?? configuredModel;
  const modelSource = appliedModel
    ? "applied"
    : reportedModel
      ? "reported"
      : configuredModel
        ? "configured"
        : "unknown";
  const matchesRequested =
    requestedModel && modelValue ? requestedModel === modelValue : null;
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
    model: {
      value: modelValue,
      source: modelSource,
      requested: requestedModel,
      matchesRequested,
    },
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
    sessionEvidence: {
      sessionId: session?.event.sessionId ?? null,
      channelId: session?.event.channelId ?? null,
    },
    limitations,
  };
}
