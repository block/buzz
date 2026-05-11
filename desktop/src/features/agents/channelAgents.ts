import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  addChannelMembers,
  createManagedAgent,
  getChannelMembers,
  listManagedAgents,
  startManagedAgent,
  stopManagedAgent,
  uploadMediaBytes,
} from "@/shared/api/tauri";
import type {
  AcpProvider,
  ChannelRole,
  ManagedAgent,
  ManagedAgentBackend,
} from "@/shared/api/types";

type ChannelAgentProvider = Pick<
  AcpProvider,
  "id" | "label" | "command" | "defaultArgs" | "mcpCommand"
>;

export type AttachManagedAgentToChannelInput = {
  agent: ManagedAgent;
  role?: Exclude<ChannelRole, "owner">;
  ensureRunning?: boolean;
};

export type AttachManagedAgentToChannelResult = {
  agent: ManagedAgent;
  membershipAdded: boolean;
  restarted: boolean;
  started: boolean;
};

export type EnsureChannelAgentPresetInput = {
  provider: ChannelAgentProvider;
  role?: Exclude<ChannelRole, "owner">;
  ensureRunning?: boolean;
};

export type EnsureChannelAgentPresetResult =
  AttachManagedAgentToChannelResult & {
    created: boolean;
    providerId: string;
  };

export type CreateChannelManagedAgentInput = {
  provider: ChannelAgentProvider;
  name: string;
  systemPrompt?: string;
  avatarUrl?: string;
  personaId?: string | null;
  /** Preferred model ID from the persona. Passed to createManagedAgent. */
  model?: string;
  role?: Exclude<ChannelRole, "owner">;
  ensureRunning?: boolean;
  backend?: ManagedAgentBackend;
};

export type CreateChannelManagedAgentResult =
  AttachManagedAgentToChannelResult & {
    created: true;
    providerId: string;
  };

export type CreateChannelManagedAgentBatchFailure = {
  kind: "generic" | "persona";
  name: string;
  personaId: string | null;
  error: string;
};

export type CreateChannelManagedAgentsResult = {
  successes: CreateChannelManagedAgentResult[];
  failures: CreateChannelManagedAgentBatchFailure[];
};

function commandBasename(command: string) {
  const normalized = command.trim().replace(/\\/g, "/");
  const parts = normalized.split("/");
  return parts[parts.length - 1] ?? normalized;
}

function normalizeCommandIdentity(command: string) {
  const lower = commandBasename(command).toLowerCase();
  if (lower === "claude-code-acp" || lower === "claude-agent-acp") {
    return "claude-acp";
  }
  return lower;
}

function commandsMatch(left: string, right: string) {
  return normalizeCommandIdentity(left) === normalizeCommandIdentity(right);
}

function parseTimestamp(value: string | null | undefined) {
  if (!value) {
    return 0;
  }

  const timestamp = Date.parse(value);
  return Number.isNaN(timestamp) ? 0 : timestamp;
}

export async function attachManagedAgentToChannel(
  channelId: string,
  input: AttachManagedAgentToChannelInput,
) {
  const role = input.role ?? "bot";
  const ensureRunning = input.ensureRunning ?? true;
  const agentPubkey = normalizePubkey(input.agent.pubkey);
  const membershipResult = await addChannelMembers({
    channelId,
    pubkeys: [input.agent.pubkey],
    role,
  });
  const membershipError = membershipResult.errors.find(
    (error) => normalizePubkey(error.pubkey) === agentPubkey,
  );
  if (membershipError) {
    throw new Error(membershipError.error);
  }
  const membershipAdded = membershipResult.added.some(
    (pubkey) => normalizePubkey(pubkey) === agentPubkey,
  );

  let agent = input.agent;
  let started = false;
  let restarted = false;

  if (ensureRunning) {
    // Remote (provider-backed) agents don't need restart — the harness
    // auto-discovers new channels via membership notifications.
    const isRemote = input.agent.backend.type === "provider";
    if (isRemote) {
      // No-op: remote agents pick up channel membership changes automatically.
    } else if (
      membershipAdded &&
      (input.agent.status === "running" || input.agent.status === "deployed")
    ) {
      await stopManagedAgent(input.agent.pubkey);
      agent = await startManagedAgent(input.agent.pubkey);
      restarted = true;
    } else if (
      input.agent.status !== "running" &&
      input.agent.status !== "deployed"
    ) {
      agent = await startManagedAgent(input.agent.pubkey);
      started = true;
    }
  }

  return {
    agent,
    membershipAdded,
    restarted,
    started,
  } satisfies AttachManagedAgentToChannelResult;
}

function pickPreferredManagedAgent(agents: ManagedAgent[]) {
  return [...agents].sort((left, right) => {
    const leftRunningScore =
      left.status === "running" || left.status === "deployed" ? 1 : 0;
    const rightRunningScore =
      right.status === "running" || right.status === "deployed" ? 1 : 0;
    if (leftRunningScore !== rightRunningScore) {
      return rightRunningScore - leftRunningScore;
    }

    return parseTimestamp(right.updatedAt) - parseTimestamp(left.updatedAt);
  })[0];
}

function buildChannelAgentName(providerId: string, providerLabel: string) {
  const normalizedProviderId = providerId.trim().toLowerCase();
  if (normalizedProviderId.length > 0) {
    return normalizedProviderId;
  }

  return providerLabel.trim().toLowerCase() || "agent";
}

function pickPreferredChannelPresetAgent(
  agents: ManagedAgent[],
  memberPubkeys: ReadonlySet<string>,
  providerCommand: string,
  expectedName: string,
) {
  const inChannelAgent = pickPreferredManagedAgent(
    agents.filter(
      (agent) =>
        commandsMatch(agent.agentCommand, providerCommand) &&
        memberPubkeys.has(normalizePubkey(agent.pubkey)),
    ),
  );
  if (inChannelAgent) {
    return inChannelAgent;
  }

  return pickPreferredManagedAgent(
    agents.filter(
      (agent) =>
        commandsMatch(agent.agentCommand, providerCommand) &&
        agent.name.trim().toLowerCase() === expectedName.trim().toLowerCase(),
    ),
  );
}

export async function ensureChannelAgentPresetInChannel(
  channelId: string,
  input: EnsureChannelAgentPresetInput,
): Promise<EnsureChannelAgentPresetResult> {
  const role = input.role ?? "bot";
  const ensureRunning = input.ensureRunning ?? true;
  const members = await getChannelMembers(channelId);
  const memberPubkeys = new Set(
    members.map((member) => normalizePubkey(member.pubkey)),
  );
  const managedAgents = await listManagedAgents();
  const expectedName = buildChannelAgentName(
    input.provider.id,
    input.provider.label,
  );
  const existingAgent = pickPreferredChannelPresetAgent(
    managedAgents,
    memberPubkeys,
    input.provider.command,
    expectedName,
  );

  if (existingAgent) {
    const attached = await attachManagedAgentToChannel(channelId, {
      agent: existingAgent,
      role,
      ensureRunning,
    });
    return {
      ...attached,
      created: false,
      providerId: input.provider.id,
    };
  }

  const created = await createManagedAgent({
    name: expectedName,
    acpCommand: "sprout-acp",
    agentCommand: input.provider.command,
    agentArgs: input.provider.defaultArgs,
    mcpCommand: input.provider.mcpCommand ?? "sprout-mcp-server",
    spawnAfterCreate: false,
  });
  const attached = await attachManagedAgentToChannel(channelId, {
    agent: created.agent,
    role,
    ensureRunning,
  });

  return {
    ...attached,
    created: true,
    providerId: input.provider.id,
  };
}

export async function createChannelManagedAgent(
  channelId: string,
  input: CreateChannelManagedAgentInput,
): Promise<CreateChannelManagedAgentResult> {
  const role = input.role ?? "bot";
  const ensureRunning = input.ensureRunning ?? true;
  const trimmedName = input.name.trim();

  if (trimmedName.length === 0) {
    throw new Error("Agent name is required.");
  }

  // If the avatar is a data URI (e.g. from a persona PNG card import),
  // upload it to get a hosted URL the relay can serve.
  let resolvedAvatarUrl = input.avatarUrl?.trim() || undefined;
  if (resolvedAvatarUrl?.startsWith("data:image/")) {
    try {
      const [, b64] = resolvedAvatarUrl.split(",", 2);
      if (!b64) throw new Error("empty data URI payload");
      const bytes = Array.from(atob(b64), (c) => c.charCodeAt(0));
      const blob = await uploadMediaBytes(bytes);
      resolvedAvatarUrl = blob.url;
    } catch (err) {
      console.warn("Avatar upload failed, proceeding without avatar:", err);
      resolvedAvatarUrl = undefined;
    }
  }

  const isProviderMode = input.backend?.type === "provider";

  const created = await createManagedAgent({
    name: trimmedName,
    acpCommand: "sprout-acp",
    agentCommand: input.provider.command,
    agentArgs: input.provider.defaultArgs,
    mcpCommand: input.provider.mcpCommand ?? "sprout-mcp-server",
    personaId: input.personaId ?? undefined,
    systemPrompt: input.systemPrompt?.trim() || undefined,
    avatarUrl: resolvedAvatarUrl,
    model: input.model?.trim() || undefined,
    spawnAfterCreate: isProviderMode,
    startOnAppLaunch: isProviderMode ? false : undefined,
    backend: input.backend,
  });

  // Tauri returns Ok() even on deploy failure — spawnError carries the message.
  if (created.spawnError) {
    throw new Error(created.spawnError);
  }

  const attached = await attachManagedAgentToChannel(channelId, {
    agent: created.agent,
    role,
    ensureRunning,
  });

  return {
    ...attached,
    created: true,
    providerId: input.provider.id,
  };
}

export async function createChannelManagedAgents(
  channelId: string,
  inputs: readonly CreateChannelManagedAgentInput[],
): Promise<CreateChannelManagedAgentsResult> {
  const successes: CreateChannelManagedAgentResult[] = [];
  const failures: CreateChannelManagedAgentBatchFailure[] = [];

  for (const input of inputs) {
    try {
      successes.push(await createChannelManagedAgent(channelId, input));
    } catch (error) {
      failures.push({
        kind: input.personaId ? "persona" : "generic",
        name: input.name.trim() || "agent",
        personaId: input.personaId ?? null,
        error: error instanceof Error ? error.message : "Failed to add agent.",
      });
    }
  }

  return {
    successes,
    failures,
  };
}
