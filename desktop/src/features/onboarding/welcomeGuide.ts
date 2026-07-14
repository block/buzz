import {
  addChannelMembers,
  createManagedAgent,
  getChannelMembers,
  listManagedAgents,
} from "@/shared/api/tauri";
import { sendManagedAgentChannelMessage } from "@/shared/api/tauriManagedAgentMessages";
import { listPersonas, setPersonaActive } from "@/shared/api/tauriPersonas";
import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export const WELCOME_GUIDE_AGENT_NAME = "Fizz";
export const WELCOME_GUIDE_PERSONA_ID = "builtin:fizz";
export const WELCOME_GUIDE_INTRO_MARKER = "buzz-welcome-intro.v1";
const LEGACY_WELCOME_GUIDE_AGENT_NAME = "Kit";
export const LEGACY_WELCOME_GUIDE_SYSTEM_PROMPT =
  "You are Kit, Sprout's friendly welcome guide. Help new users understand the workspace, channels, messages, and agents. Keep introductions concise, practical, and warm.";
export const WELCOME_GUIDE_INTRO_MESSAGE =
  "Hi, I'm Fizz. Welcome to Buzz.\n\nI can help you get oriented, answer questions, and make the first few steps feel less mysterious.\n\nFeel free to ask me what else you can do in Buzz, or just talk through what you want to build.";

function normalizeRelayUrl(relayUrl: string | null | undefined) {
  return relayUrl?.trim().replace(/\/+$/, "") ?? null;
}

function isAgentScopedToRelay(agent: ManagedAgent, relayUrl?: string | null) {
  const targetRelayUrl = normalizeRelayUrl(relayUrl);
  if (!targetRelayUrl) {
    return true;
  }
  return normalizeRelayUrl(agent.relayUrl) === targetRelayUrl;
}

function isNamedWelcomeGuideAgent(agent: ManagedAgent) {
  return (
    agent.name.trim().toLowerCase() === WELCOME_GUIDE_AGENT_NAME.toLowerCase()
  );
}

function isBuiltInWelcomeGuideAgent(agent: ManagedAgent) {
  return (
    agent.personaId === WELCOME_GUIDE_PERSONA_ID &&
    isNamedWelcomeGuideAgent(agent)
  );
}

function isLegacyKitWelcomeGuideAgent(agent: ManagedAgent) {
  return (
    agent.name.trim().toLowerCase() ===
      LEGACY_WELCOME_GUIDE_AGENT_NAME.toLowerCase() &&
    agent.systemPrompt?.trim() === LEGACY_WELCOME_GUIDE_SYSTEM_PROMPT
  );
}

function isWelcomeGuideAgent(agent: ManagedAgent) {
  return (
    isBuiltInWelcomeGuideAgent(agent) || isLegacyKitWelcomeGuideAgent(agent)
  );
}

function pickAgentByStatus(agents: ManagedAgent[]) {
  return (
    agents.find((agent) => agent.status === "running") ??
    agents.find((agent) => agent.status === "deployed") ??
    agents[0] ??
    null
  );
}

export function pickWelcomeGuideAgent(agents: ManagedAgent[]) {
  return pickAgentByStatus(agents.filter(isWelcomeGuideAgent));
}

export function pickWelcomeGuideAgentForRelay(
  agents: ManagedAgent[],
  relayUrl?: string | null,
) {
  return pickAgentByStatus(
    agents.filter(
      (agent) =>
        isWelcomeGuideAgent(agent) && isAgentScopedToRelay(agent, relayUrl),
    ),
  );
}

export async function getWelcomeGuideAgentPubkeys(relayUrl?: string | null) {
  return (await listManagedAgents())
    .filter(
      (agent) =>
        isWelcomeGuideAgent(agent) && isAgentScopedToRelay(agent, relayUrl),
    )
    .map((agent) => agent.pubkey);
}

async function ensureWelcomeGuidePersonaActive() {
  const guidePersona = (await listPersonas()).find(
    (persona) => persona.id === WELCOME_GUIDE_PERSONA_ID,
  );
  if (!guidePersona) {
    throw new Error(`${WELCOME_GUIDE_AGENT_NAME} agent not found.`);
  }
  if (!guidePersona.isActive) {
    await setPersonaActive(WELCOME_GUIDE_PERSONA_ID, true);
  }
}

async function ensureWelcomeGuideAgent(relayUrl?: string | null) {
  const agents = await listManagedAgents();
  const existing = pickWelcomeGuideAgentForRelay(agents, relayUrl);
  if (existing) {
    return existing;
  }

  await ensureWelcomeGuidePersonaActive();

  const created = await createManagedAgent({
    name: WELCOME_GUIDE_AGENT_NAME,
    personaId: WELCOME_GUIDE_PERSONA_ID,
    relayUrl: relayUrl ?? undefined,
    spawnAfterCreate: false,
    startOnAppLaunch: false,
    respondTo: "owner-only",
  });

  return created.agent;
}

async function ensureWelcomeGuideMembership(
  channelId: string,
  agent: ManagedAgent,
) {
  const agentPubkey = normalizePubkey(agent.pubkey);
  const members = await getChannelMembers(channelId).catch(() => []);
  if (
    members.some((member) => normalizePubkey(member.pubkey) === agentPubkey)
  ) {
    return;
  }

  const result = await addChannelMembers({
    channelId,
    pubkeys: [agent.pubkey],
    role: "bot",
  });
  const error = result.errors.find(
    (entry) => normalizePubkey(entry.pubkey) === agentPubkey,
  );
  if (error && !error.error.toLowerCase().includes("already")) {
    throw new Error(error.error);
  }
}

export async function ensureWelcomeGuideIntro(
  channelId: string,
  relayUrl?: string | null,
) {
  const agent = await ensureWelcomeGuideAgent(relayUrl);
  await ensureWelcomeGuideMembership(channelId, agent);
  await sendManagedAgentChannelMessage({
    agentPubkey: agent.pubkey,
    channelId,
    content: WELCOME_GUIDE_INTRO_MESSAGE,
    marker: WELCOME_GUIDE_INTRO_MARKER,
    markerScope: "channel",
  });
  return agent;
}
