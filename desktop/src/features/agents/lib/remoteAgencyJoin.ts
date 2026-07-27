import type {
  RemoteAgencyAgent,
  RemoteAgencyBinding,
  RemoteAgencyDescriptor,
  RemoteAgencyProxy,
} from "@/shared/api/remoteAgencyTypes";
import type { CreateManagedAgentInput } from "@/shared/api/types";

function normalizedLoopbackHost(hostname: string): string | null {
  const normalized = hostname
    .trim()
    .toLowerCase()
    .replace(/^\[|\]$/g, "");
  return normalized === "localhost" ||
    normalized === "127.0.0.1" ||
    normalized === "::1"
    ? normalized
    : null;
}

function equivalentLoopbackAgencySource(left: string, right: string): boolean {
  try {
    const leftUrl = new URL(left);
    const rightUrl = new URL(right);
    if (
      !normalizedLoopbackHost(leftUrl.hostname) ||
      !normalizedLoopbackHost(rightUrl.hostname)
    ) {
      return false;
    }
    return (
      leftUrl.protocol === rightUrl.protocol &&
      leftUrl.port === rightUrl.port &&
      leftUrl.pathname === rightUrl.pathname &&
      leftUrl.search === rightUrl.search &&
      leftUrl.hash === rightUrl.hash &&
      leftUrl.username === rightUrl.username &&
      leftUrl.password === rightUrl.password
    );
  } catch {
    return false;
  }
}

export function findRemoteAgencyBinding(
  bindings: RemoteAgencyBinding[],
  descriptor: RemoteAgencyDescriptor,
): RemoteAgencyBinding | undefined {
  const matchingAgency = bindings.filter(
    (binding) => binding.agencyId === descriptor.agencyId,
  );
  return (
    matchingAgency.find(
      (binding) => binding.sourceUrl === descriptor.sourceUrl,
    ) ??
    matchingAgency.find((binding) =>
      equivalentLoopbackAgencySource(binding.sourceUrl, descriptor.sourceUrl),
    )
  );
}

/**
 * Build the exact adapter input for a reviewed Remote Agency participant.
 * The adapter requires a public Agent Record and an explicitly reviewed A2A
 * endpoint. Secrets are supplied by the operator through the local Buzz
 * process environment and never enter this object.
 */
export function buildRemoteAgencyManagedAgentInput(
  descriptor: RemoteAgencyDescriptor,
  agent: RemoteAgencyAgent,
  channelId: string,
  spaceId: string | null,
): CreateManagedAgentInput {
  if (!agent.recordUrl) {
    throw new Error(
      "Remote Agent does not advertise a public OASF Agent Record",
    );
  }
  if (!agent.a2aEndpoint) {
    throw new Error("Remote Agent does not advertise a reviewed A2A endpoint");
  }
  return {
    name: `Remote Agency · proxied by Buzz · ${agent.name}`,
    acpCommand: "buzz-acp",
    agentCommand: "buzz-a2a-acp",
    harnessOverride: true,
    agentArgs: [],
    envVars: {
      BUZZ_A2A_AGENT_RECORD: agent.recordUrl,
      BUZZ_A2A_BEARER_ENDPOINT: agent.a2aEndpoint,
      BUZZ_A2A_AGENCY_REF: descriptor.agencyId,
      BUZZ_A2A_AGENT_REF: agent.id,
      BUZZ_A2A_CHANNEL_REF: channelId,
      ...(spaceId ? { BUZZ_A2A_SPACE_REF: spaceId } : {}),
    },
    parallelism: 1,
    spawnAfterCreate: true,
    startOnAppLaunch: true,
  };
}

export function findRemoteAgencyProxy(
  proxies: RemoteAgencyProxy[],
  agentId: string,
  channelId: string,
  spaceId: string | null,
): RemoteAgencyProxy | undefined {
  return proxies.find(
    (proxy) =>
      proxy.agentId === agentId &&
      proxy.channelId === channelId &&
      proxy.spaceId === spaceId,
  );
}

export function bindingFromRemoteAgencyProxies(
  descriptor: RemoteAgencyDescriptor,
  proxies: RemoteAgencyProxy[],
  joinedAt?: string,
): RemoteAgencyBinding {
  const unique = <T>(values: T[]) => [...new Set(values)];
  return {
    sourceUrl: descriptor.sourceUrl,
    agencyId: descriptor.agencyId,
    agentIds: unique(proxies.map((proxy) => proxy.agentId)).sort(),
    spaceIds: unique(
      proxies.flatMap((proxy) => (proxy.spaceId ? [proxy.spaceId] : [])),
    ).sort(),
    channelIds: unique(proxies.map((proxy) => proxy.channelId)).sort(),
    proxies,
    joinedAt: joinedAt ?? new Date().toISOString(),
  };
}
