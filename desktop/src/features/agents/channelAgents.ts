import {
  commandsMatch,
  findReusableGenericAgent,
  findReusablePersonaAgent,
  pickPreferredManagedAgent,
  resolveReusableAgentAccessPolicy,
} from "@/features/agents/agentReuse";
export { findReusableAgent } from "@/features/agents/agentReuse";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { resolveManagedAgentAvatarUrl } from "@/features/agents/ui/managedAgentAvatar";
import {
  addChannelMembers,
  createManagedAgent,
  getChannelMembers,
  listManagedAgents,
  updateManagedAgent,
} from "@/shared/api/tauri";
import { listPersonas } from "@/shared/api/tauriPersonas";
import { startManagedAgent } from "@/shared/api/tauriManagedAgents";
import type {
  AcpRuntime,
  AgentPersona,
  ChannelRole,
  CreateManagedAgentInput,
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
  /**
   * When set, a needed start/deploy is handed to this callback instead of
   * being awaited: the attach resolves as soon as the membership write lands
   * and the callback owns the start, including surfacing its failure. The
   * message-send path passes a queue collector here — the wake it records is
   * flushed fire-and-forget only after the relay accepts the publish, with a
   * replay floor stamped at queue time, so the spawned harness replays the
   * published message and an aborted send leaves no orphan wake.
   */
  detachedStart?: (agent: ManagedAgent) => void;
};

export type AttachManagedAgentToChannelResult = {
  agent: ManagedAgent;
  membershipAdded: boolean;
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
  /** Team this instance is deployed from; prevents cross-team reuse. */
  teamId?: string | null;
  /**
   * True when `runtime` is a runtime the user deliberately picked to override
   * the persona (a deploy-dialog runtime selector), as opposed to a
   * missing-runtime fallback. Forwarded to the backend so a persona-backed
   * create only pins the harness for a deliberate override.
   */
  harnessOverride?: boolean;
  /** Preferred model ID from the persona. Passed to createManagedAgent. */
  model?: string;
  role?: Exclude<ChannelRole, "owner">;
  ensureRunning?: boolean;
  backend?: ManagedAgentBackend;
  /**
   * Inbound author gate mode. Omitted = linked persona default, then
   * `"owner-only"` when the persona leaves it unset or no persona is linked.
   */
  respondTo?: RespondToMode;
  /** Hex pubkeys for allowlist mode. */
  respondToAllowlist?: string[];
  /** Skip reuse logic and always create a fresh agent instance. */
  forceNewInstance?: boolean;
  /** Detached start hook forwarded to the channel attach — see
   * `AttachManagedAgentToChannelInput.detachedStart`. */
  detachedStart?: (agent: ManagedAgent) => void;
};

export type CreateChannelManagedAgentResult =
  AttachManagedAgentToChannelResult & {
    created: boolean;
    runtimeId: string;
  };

export type ProvisionChannelManagedAgentResult = {
  agent: ManagedAgent;
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

type ChannelAgentReuseContext = {
  managedAgents: ManagedAgent[];
  channelMemberPubkeys: ReadonlySet<string>;
  personas: readonly Pick<
    AgentPersona,
    "id" | "respondTo" | "respondToAllowlist"
  >[];
};

export type ApplyReusableAgentAccessPolicyResult = {
  agent: ManagedAgent;
  /**
   * True when reconciling the policy required a relay write. Callers that
   * sequence authorization around this call — the message-send path revalidates
   * mention authorization at the publish boundary whenever an awaited relay
   * round-trip separated it from its earlier pass — depend on this flag rather
   * than on comparing the returned record's identity against the input, so the
   * signal survives any future change to whether an update returns a fresh
   * object.
   */
  wrote: boolean;
};

export async function applyReusableAgentAccessPolicy(
  agent: ManagedAgent,
  request: Pick<CreateManagedAgentInput, "respondTo" | "respondToAllowlist">,
  persona?: Pick<AgentPersona, "respondTo" | "respondToAllowlist">,
): Promise<ApplyReusableAgentAccessPolicyResult> {
  const policy = resolveReusableAgentAccessPolicy(request, persona);
  const matches =
    agent.respondTo === policy.respondTo &&
    agent.respondToAllowlist.length === policy.respondToAllowlist.length &&
    agent.respondToAllowlist.every(
      (pubkey, index) => pubkey === policy.respondToAllowlist[index],
    );
  if (matches) return { agent, wrote: false };

  const { agent: updatedAgent } = await updateManagedAgent({
    pubkey: agent.pubkey,
    ...policy,
  });
  return { agent: updatedAgent, wrote: true };
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

  if (ensureRunning) {
    // Running agents (local or provider) auto-discover new channel membership
    // via the harness's membership notifications — no restart needed. Only
    // not-yet-running agents need a start/deploy call before the first mention
    // can reach them. For a local agent the status check and the start are both
    // pair-scoped to the active community: `agent.status` reflects that
    // community's (agent, relay) pair, and `startManagedAgent` spawns that same
    // pair — so this ensures the pair the caller is attaching to, never
    // another community's.
    const isRemote = input.agent.backend.type === "provider";
    const needsStart = isRemote
      ? input.agent.status !== "deployed"
      : input.agent.status !== "running" && input.agent.status !== "deployed";
    if (needsStart) {
      if (input.detachedStart) {
        input.detachedStart(input.agent);
      } else {
        agent = await startManagedAgent(input.agent.pubkey);
        started = true;
      }
    }
  }

  return {
    agent,
    membershipAdded,
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
    acpCommand: "buzz-acp",
    agentCommand: input.runtime.command,
    // Do NOT seed agentArgs from runtime.defaultArgs (see instanceInputForDefinition.ts
    // for the rationale — empty args let spawn resolve definition args live).
    agentArgs: [],
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

export async function provisionChannelManagedAgent(
  input: CreateChannelManagedAgentInput,
  context?: ChannelAgentReuseContext,
): Promise<ProvisionChannelManagedAgentResult> {
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
      const definition = context.personas.find(
        (persona) => persona.id === input.personaId,
      );
      const { agent: updatedAgent } = await applyReusableAgentAccessPolicy(
        reusable,
        input,
        definition,
      );

      return {
        agent: updatedAgent,
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
      const { agent: updatedAgent } = await applyReusableAgentAccessPolicy(
        reusable,
        input,
      );

      return {
        agent: updatedAgent,
        created: false,
        runtimeId: input.runtime.id,
      };
    }
  }

  // Resolve the avatar for the channel-managed agent. Base64 data URIs (e.g.
  // from a persona PNG card import) are uploaded to a hosted URL the relay can
  // serve; percent-encoded emoji SVG data URLs pass through unchanged so the
  // selected emoji survives deployment. Shared with agent creation so both
  // paths handle emoji avatars identically.
  const resolvedAvatarUrl = await resolveManagedAgentAvatarUrl(input.avatarUrl);

  const isProviderMode = input.backend?.type === "provider";

  const created = await createManagedAgent({
    name: trimmedName,
    acpCommand: "buzz-acp",
    agentCommand: input.runtime.command,
    harnessOverride: input.harnessOverride ?? false,
    // Do NOT seed agentArgs from runtime.defaultArgs (see instanceInputForDefinition.ts
    // for the rationale — empty args let spawn resolve definition args live).
    agentArgs: [],
    mcpCommand: input.runtime.mcpCommand ?? "",
    personaId: input.personaId ?? undefined,
    teamId: input.teamId ?? undefined,
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

  return {
    agent: created.agent,
    created: true,
    runtimeId: input.runtime.id,
  };
}

export async function createChannelManagedAgent(
  channelId: string,
  input: CreateChannelManagedAgentInput,
  context?: ChannelAgentReuseContext,
): Promise<CreateChannelManagedAgentResult> {
  const provisioned = await provisionChannelManagedAgent(input, context);
  const attached = await attachManagedAgentToChannel(channelId, {
    agent: provisioned.agent,
    role: input.role ?? "bot",
    ensureRunning: input.ensureRunning ?? true,
    detachedStart: input.detachedStart,
  });

  return {
    ...attached,
    created: provisioned.created,
    runtimeId: provisioned.runtimeId,
  };
}

export async function createChannelManagedAgents(
  channelId: string,
  inputs: readonly CreateChannelManagedAgentInput[],
): Promise<CreateChannelManagedAgentsResult> {
  // Fetch managed agents and channel members once for smart reuse checks.
  const needsPersonaPolicy = inputs.some(
    (input) =>
      Boolean(input.personaId) &&
      !input.forceNewInstance &&
      input.respondTo === undefined,
  );
  const [managedAgents, members, personas] = await Promise.all([
    listManagedAgents(),
    getChannelMembers(channelId),
    needsPersonaPolicy ? listPersonas() : Promise.resolve([]),
  ]);
  const channelMemberPubkeys = new Set(
    members.map((m) => normalizePubkey(m.pubkey)),
  );
  const context = { managedAgents, channelMemberPubkeys, personas };

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
