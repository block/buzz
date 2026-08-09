import { findReusablePersonaAgent } from "@/features/agents/agentReuse";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import type {
  AgentPersona,
  Channel,
  CreateManagedAgentInput,
  CreateManagedAgentResponse,
  ManagedAgent,
} from "@/shared/api/types";

export interface OpenPersonaConversationDependencies {
  readonly definitions: readonly AgentPersona[];
  readonly managedAgents: readonly ManagedAgent[];
  readonly buildInput: (
    definition: AgentPersona,
  ) => Promise<CreateManagedAgentInput>;
  readonly createAgent: (
    input: CreateManagedAgentInput,
  ) => Promise<CreateManagedAgentResponse>;
  readonly startAgent: (pubkey: string) => Promise<unknown>;
  readonly openDm: (pubkeys: string[]) => Promise<Pick<Channel, "id">>;
  readonly navigate: (channelId: string) => Promise<unknown>;
  readonly refetch: () => Promise<unknown>;
}

export interface OpenPersonaConversationResult {
  readonly pubkey: string;
  readonly channelId: string;
  readonly created: boolean;
}

export async function openPersonaConversation(
  personaId: string,
  dependencies: OpenPersonaConversationDependencies,
): Promise<OpenPersonaConversationResult> {
  const definition = dependencies.definitions.find(
    (candidate) => candidate.id === personaId && candidate.isActive,
  );
  if (!definition) {
    throw new Error("This adviser is not active in My Agents.");
  }

  let selected = findReusablePersonaAgent(
    [...dependencies.managedAgents],
    personaId,
    new Set(),
  );
  let created = false;

  if (!selected) {
    const input = await dependencies.buildInput(definition);
    const response = await dependencies.createAgent(input);
    selected = response.agent;
    created = true;
    await dependencies.refetch();
    if (response.spawnError) {
      throw new Error(response.spawnError);
    }
  } else if (!isManagedAgentActive(selected)) {
    await dependencies.startAgent(selected.pubkey);
    await dependencies.refetch();
  }

  const dm = await dependencies.openDm([selected.pubkey]);
  await dependencies.navigate(dm.id);
  return { pubkey: selected.pubkey, channelId: dm.id, created };
}
