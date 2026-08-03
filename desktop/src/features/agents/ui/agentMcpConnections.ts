export const MCP_PROFILE_ENV_KEY = "BUZZ_ACP_MCP_SERVERS";
export const MCP_SECRET_PREFIX = "BUZZ_MCP_";

export type AgentMcpTransport =
  | "http-first"
  | "http-only"
  | "sse-first"
  | "sse-only";
export type AgentMcpAuthType = "none" | "bearer";

export type AgentMcpConnection = {
  id: string;
  name: string;
  url: string;
  transport: AgentMcpTransport;
  authType: AgentMcpAuthType;
  bearerToken: string;
  allowedTools: string;
};

type RawMcpServer = {
  name?: unknown;
  command?: unknown;
  args?: unknown;
  env?: unknown;
  inherit_env?: unknown;
  allowed_tools?: unknown;
  _buzz_managed_remote?: unknown;
  [key: string]: unknown;
};

export type ParsedAgentMcpConnections = {
  connections: AgentMcpConnection[];
  unmanaged: RawMcpServer[];
  managedSecretKeys: string[];
  error: string | null;
};

const TRANSPORTS = new Set<AgentMcpTransport>([
  "http-first",
  "http-only",
  "sse-first",
  "sse-only",
]);

function strings(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : [];
}

function transportFromArgs(args: string[]): AgentMcpTransport {
  const index = args.indexOf("--transport");
  const value = index >= 0 ? args[index + 1] : undefined;
  return TRANSPORTS.has(value as AgentMcpTransport)
    ? (value as AgentMcpTransport)
    : "http-first";
}

function headerKeyFromArgs(args: string[]): string | null {
  const index = args.indexOf("--header");
  const header = index >= 0 ? args[index + 1] : undefined;
  const match = header?.match(/^Authorization:\$\{([A-Z][A-Z0-9_]*)\}$/);
  return match?.[1] ?? null;
}

export function parseAgentMcpConnections(
  envVars: Record<string, string>,
): ParsedAgentMcpConnections {
  const raw = envVars[MCP_PROFILE_ENV_KEY]?.trim();
  if (!raw) {
    return {
      connections: [],
      unmanaged: [],
      managedSecretKeys: [],
      error: null,
    };
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return {
      connections: [],
      unmanaged: [],
      managedSecretKeys: [],
      error: "The existing MCP profile is not valid JSON.",
    };
  }
  if (!Array.isArray(parsed)) {
    return {
      connections: [],
      unmanaged: [],
      managedSecretKeys: [],
      error: "The existing MCP profile must be a JSON array.",
    };
  }

  const connections: AgentMcpConnection[] = [];
  const unmanaged: RawMcpServer[] = [];
  const managedSecretKeys: string[] = [];
  parsed.forEach((value, index) => {
    if (typeof value !== "object" || value === null || Array.isArray(value)) {
      unmanaged.push(value as RawMcpServer);
      return;
    }
    const server = value as RawMcpServer;
    if (
      server._buzz_managed_remote !== true ||
      server.command !== "mcp-remote"
    ) {
      unmanaged.push(server);
      return;
    }
    const args = strings(server.args);
    const headerKey = headerKeyFromArgs(args);
    if (headerKey) managedSecretKeys.push(headerKey);
    connections.push({
      id: `mcp-${index}-${typeof server.name === "string" ? server.name : "server"}`,
      name: typeof server.name === "string" ? server.name : "",
      url: args[0] ?? "",
      transport: transportFromArgs(args),
      authType: headerKey ? "bearer" : "none",
      bearerToken: headerKey
        ? (envVars[headerKey] ?? "").replace(/^Bearer\s+/i, "")
        : "",
      allowedTools: strings(server.allowed_tools).join(", "),
    });
  });

  return { connections, unmanaged, managedSecretKeys, error: null };
}

function secretKeyForName(name: string, index: number): string {
  const suffix = name
    .trim()
    .toUpperCase()
    .replace(/[^A-Z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .slice(0, 40);
  return `${MCP_SECRET_PREFIX}${suffix || "SERVER"}_${index + 1}_AUTH_HEADER`;
}

function splitTools(value: string): string[] {
  return [
    ...new Set(
      value
        .split(/[\s,]+/)
        .map((tool) => tool.trim())
        .filter(Boolean),
    ),
  ];
}

export function validateAgentMcpConnections(
  connections: AgentMcpConnection[],
): string | null {
  if (connections.length > 15)
    return "At most 15 custom MCP connections are supported.";
  const names = new Set<string>();
  for (const connection of connections) {
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(connection.name)) {
      return "Each MCP name must use 1–64 letters, numbers, dashes, or underscores.";
    }
    if (names.has(connection.name))
      return `MCP name “${connection.name}” is duplicated.`;
    names.add(connection.name);
    try {
      const url = new URL(connection.url);
      if (url.protocol !== "https:") return "MCP URLs must use HTTPS.";
    } catch {
      return `MCP URL for “${connection.name}” is invalid.`;
    }
    if (connection.authType === "bearer" && !connection.bearerToken.trim()) {
      return `Bearer token for “${connection.name}” is required.`;
    }
    for (const tool of splitTools(connection.allowedTools)) {
      if (!/^[A-Za-z0-9_-]{1,64}$/.test(tool) || tool.includes("__")) {
        return `Allowed tool “${tool}” must use 1–64 letters, numbers, dashes, or underscores.`;
      }
      if (`${connection.name}__${tool}`.length > 64) {
        return `MCP name “${connection.name}” and allowed tool “${tool}” are too long together (maximum 64 characters including the separator).`;
      }
    }
  }
  return null;
}

export function writeAgentMcpConnections({
  baseEnvVars,
  connections,
  previous,
}: {
  baseEnvVars: Record<string, string>;
  connections: AgentMcpConnection[];
  previous: ParsedAgentMcpConnections;
}): Record<string, string> {
  const next = { ...baseEnvVars };
  for (const key of previous.managedSecretKeys) delete next[key];

  const managed: RawMcpServer[] = connections.map((connection, index) => {
    const args = [
      connection.url,
      "--transport",
      connection.transport,
      "--silent",
    ];
    const inheritEnv: string[] = [];
    if (connection.authType === "bearer") {
      const secretKey = secretKeyForName(connection.name, index);
      next[secretKey] = `Bearer ${connection.bearerToken.trim()}`;
      args.push("--header", `Authorization:${"${"}${secretKey}}`);
      inheritEnv.push(secretKey);
    }
    return {
      name: connection.name.trim(),
      command: "mcp-remote",
      args,
      inherit_env: inheritEnv,
      allowed_tools: splitTools(connection.allowedTools),
      _buzz_managed_remote: true,
    };
  });

  const profile = [...previous.unmanaged, ...managed];
  if (profile.length > 0) next[MCP_PROFILE_ENV_KEY] = JSON.stringify(profile);
  else delete next[MCP_PROFILE_ENV_KEY];
  return next;
}

export function newAgentMcpConnection(): AgentMcpConnection {
  return {
    id: crypto.randomUUID(),
    name: "",
    url: "",
    transport: "http-first",
    authType: "none",
    bearerToken: "",
    allowedTools: "",
  };
}
