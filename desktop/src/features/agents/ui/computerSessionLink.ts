export type ComputerSessionDescriptor = {
  version: 1;
  sessionId: string;
  endpoint: string;
};

const SESSION_ID = /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,119}$/;

export function parseComputerSessionDescriptor(
  value: unknown,
): ComputerSessionDescriptor | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (
    Object.keys(record).sort().join("\0") !==
      ["endpoint", "sessionId", "version"].join("\0") ||
    record.version !== 1 ||
    typeof record.sessionId !== "string" ||
    !SESSION_ID.test(record.sessionId) ||
    typeof record.endpoint !== "string"
  )
    return null;

  let endpoint: URL;
  try {
    endpoint = new URL(record.endpoint);
  } catch {
    return null;
  }
  if (
    endpoint.protocol !== "https:" ||
    !endpoint.hostname.endsWith(".ts.net") ||
    !/^[a-z0-9.-]+$/i.test(endpoint.hostname) ||
    endpoint.username ||
    endpoint.password ||
    endpoint.search ||
    endpoint.hash ||
    (endpoint.pathname !== "" && endpoint.pathname !== "/")
  )
    return null;

  return {
    version: 1,
    sessionId: record.sessionId,
    endpoint: endpoint.origin,
  };
}

export function buildComputerSessionLink(
  descriptor: ComputerSessionDescriptor,
): string {
  const validated = parseComputerSessionDescriptor(descriptor);
  if (!validated)
    throw new Error("Invalid remote computer session descriptor.");
  const url = new URL(
    `mesh-computer://session/${encodeURIComponent(validated.sessionId)}`,
  );
  url.searchParams.set("endpoint", validated.endpoint);
  return url.toString();
}

export function extractComputerSessionDescriptor(
  update: Record<string, unknown>,
): ComputerSessionDescriptor | null {
  if (
    !update.rawOutput ||
    typeof update.rawOutput !== "object" ||
    Array.isArray(update.rawOutput)
  ) {
    return null;
  }
  return parseComputerSessionDescriptor(
    (update.rawOutput as Record<string, unknown>).computerSession,
  );
}
