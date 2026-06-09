import {
  commandsMatch,
  findReusableGenericAgent,
  findReusablePersonaAgent,
  pickPreferredManagedAgent,
} from "@/features/agents/agentReuse";
export { findReusableAgent } from "@/features/agents/agentReuse";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  addChannelMembers,
  createManagedAgent,
  getChannelMembers,
  listManagedAgents,
  startManagedAgent,
  stopManagedAgent,
  updateManagedAgent,
  uploadMediaBytes,
} from "@/shared/api/tauri";
import type {
  AcpRuntime,
  ChannelRole,
  ManagedAgent,
  ManagedAgentBackend,
  RespondToMode,
} from "@/shared/api/types";

type ChannelAgentRuntime = Pick<
  AcpRuntime,
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
  runtime: ChannelAgentRuntime;
  role?: Exclude<ChannelRole, "owner">;
  ensureRunning?: boolean;
};

export type EnsureChannelAgentPresetResult =
  AttachManagedAgentToChannelResult & {
    created: boolean;
    runtimeId: string;
  };

export type CreateChannelManagedAgentInput = {
  runtime: ChannelAgentRuntime;
  name: string;
  systemPrompt?: string;
  avatarUrl?: string;
  personaId?: string | null;
  /** Preferred model ID from the persona. Passed to createManagedAgent. */
  model?: string;
  role?: Exclude<ChannelRole, "owner">;
  ensureRunning?: boolean;
  backend?: ManagedAgentBackend;
  /** Inbound author gate mode. Omitted = server default ("owner-only"). */
  respondTo?: RespondToMode;
  /** Hex pubkeys for allowlist mode. */
  respondToAllowlist?: string[];
  /** Skip reuse logic and always create a fresh agent instance. */
  forceNewInstance?: boolean;
};

export type CreateChannelManagedAgentResult =
  AttachManagedAgentToChannelResult & {
    created: boolean;
    runtimeId: string;
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

function buildChannelAgentName(runtimeId: string, runtimeLabel: string) {
  const normalizedRuntimeId = runtimeId.trim().toLowerCase();
  if (normalizedRuntimeId.length > 0) {
    return normalizedRuntimeId;
  }

  return runtimeLabel.trim().toLowerCase() || "agent";
}

function pickPreferredChannelPresetAgent(
  agents: ManagedAgent[],
  memberPubkeys: ReadonlySet<string>,
  runtimeCommand: string,
  expectedName: string,
) {
  const inChannelAgent = pickPreferredManagedAgent(
    agents.filter(
      (agent) =>
        commandsMatch(agent.agentCommand, runtimeCommand) &&
        memberPubkeys.has(normalizePubkey(agent.pubkey)),
    ),
  );
  if (inChannelAgent) {
    return inChannelAgent;
  }

  return pickPreferredManagedAgent(
    agents.filter(
      (agent) =>
        commandsMatch(agent.agentCommand, runtimeCommand) &&
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
    input.runtime.id,
    input.runtime.label,
  );
  const existingAgent = pickPreferredChannelPresetAgent(
    managedAgents,
    memberPubkeys,
    input.runtime.command,
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
      runtimeId: input.runtime.id,
    };
  }

  const created = await createManagedAgent({
    name: expectedName,
    acpCommand: "sprout-acp",
    agentCommand: input.runtime.command,
    agentArgs: input.runtime.defaultArgs,
    mcpCommand: input.runtime.mcpCommand ?? "",
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
    runtimeId: input.runtime.id,
  };
}

export async function createChannelManagedAgent(
  channelId: string,
  input: CreateChannelManagedAgentInput,
  context?: {
    managedAgents?: ManagedAgent[];
    channelMemberPubkeys?: ReadonlySet<string>;
  },
): Promise<CreateChannelManagedAgentResult> {
  const role = input.role ?? "bot";
  const ensureRunning = input.ensureRunning ?? true;
  const trimmedName = input.name.trim();

  if (trimmedName.length === 0) {
    throw new Error("Agent name is required.");
  }

  // Smart reuse: if a managed agent with the same personaId already exists
  // and is not already in this channel, attach it instead of creating a new one.
  if (
    input.personaId &&
    !input.forceNewInstance &&
    context?.managedAgents &&
    context.channelMemberPubkeys
  ) {
    const reusable = findReusablePersonaAgent(
      context.managedAgents,
      input.personaId,
      context.channelMemberPubkeys,
    );
    if (reusable) {
      // Apply the caller's respondTo settings so the user's permission
      // choice in the dialog is always honored, even when reusing.
      const needsRespondToUpdate =
        input.respondTo && input.respondTo !== "owner-only";
      const updatedAgent = needsRespondToUpdate
        ? (
            await updateManagedAgent({
              pubkey: reusable.pubkey,
              respondTo: input.respondTo,
              respondToAllowlist:
                input.respondTo === "allowlist"
                  ? input.respondToAllowlist
                  : undefined,
            })
          ).agent
        : reusable;

      const attached = await attachManagedAgentToChannel(channelId, {
        agent: updatedAgent,
        role,
        ensureRunning,
      });
      return {
        ...attached,
        created: false,
        runtimeId: input.runtime.id,
      };
    }
  }

  // Generic agent reuse: if no persona is set and the system prompt is blank,
  // look for an existing agent with the same command and no custom prompt.
  if (
    !input.personaId &&
    !input.systemPrompt?.trim() &&
    !input.forceNewInstance &&
    context?.managedAgents &&
    context.channelMemberPubkeys
  ) {
    const reusable = findReusableGenericAgent(
      context.managedAgents,
      input.runtime.command,
      context.channelMemberPubkeys,
    );
    if (reusable) {
      const needsRespondToUpdate =
        input.respondTo && input.respondTo !== "owner-only";
      const updatedAgent = needsRespondToUpdate
        ? (
            await updateManagedAgent({
              pubkey: reusable.pubkey,
              respondTo: input.respondTo,
              respondToAllowlist:
                input.respondTo === "allowlist"
                  ? input.respondToAllowlist
                  : undefined,
            })
          ).agent
        : reusable;

      const attached = await attachManagedAgentToChannel(channelId, {
        agent: updatedAgent,
        role,
        ensureRunning,
      });
      return {
        ...attached,
        created: false,
        runtimeId: input.runtime.id,
      };
    }
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
    agentCommand: input.runtime.command,
    agentArgs: input.runtime.defaultArgs,
    mcpCommand: input.runtime.mcpCommand ?? "",
    personaId: input.personaId ?? undefined,
    systemPrompt: input.systemPrompt?.trim() || undefined,
    avatarUrl: resolvedAvatarUrl,
    model: input.model?.trim() || undefined,
    spawnAfterCreate: isProviderMode,
    startOnAppLaunch: isProviderMode ? false : undefined,
    backend: input.backend,
    respondTo: input.respondTo,
    respondToAllowlist: input.respondToAllowlist,
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
    runtimeId: input.runtime.id,
  };
}

export async function createChannelManagedAgents(
  channelId: string,
  inputs: readonly CreateChannelManagedAgentInput[],
): Promise<CreateChannelManagedAgentsResult> {
  // Fetch managed agents and channel members once for smart reuse checks.
  const [managedAgents, members] = await Promise.all([
    listManagedAgents(),
    getChannelMembers(channelId),
  ]);
  const channelMemberPubkeys = new Set(
    members.map((m) => normalizePubkey(m.pubkey)),
  );
  const context = { managedAgents, channelMemberPubkeys };

  // Sequential loop: each agent must be fully created and its relay membership
  // written before the next starts. Concurrent writes to the replaceable
  // kind:39002 membership event cause last-write-wins data loss.
  const successes: CreateChannelManagedAgentResult[] = [];
  const failures: CreateChannelManagedAgentBatchFailure[] = [];

  for (let i = 0; i < inputs.length; i++) {
    const input = inputs[i];
    try {
      const result = await createChannelManagedAgent(channelId, input, context);
      successes.push(result);
    } catch (error) {
      failures.push({
        kind: input.personaId ? "persona" : "generic",
        name: input.name.trim() || "agent",
        personaId: input.personaId ?? null,
        error: error instanceof Error ? error.message : "Failed to add agent.",
      });
    }
  }

  return { successes, failures };
}
