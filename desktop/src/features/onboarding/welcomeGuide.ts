import {
  buildInstanceInputForDefinition,
  resolveStartRuntimeForDefinition,
} from "@/features/agents/lib/instanceInputForDefinition";
import { canonicalRelayUrl } from "@/features/agents/managedAgentRuntimeStatus";
import {
  addChannelMembers,
  createManagedAgent,
  getChannelMembers,
  listManagedAgents,
  updateManagedAgent,
} from "@/shared/api/tauri";
import { discoverAcpRuntimes } from "@/shared/api/tauriAcpDiscovery";
import { getAgentAccessOwnerOnly } from "@/shared/api/tauriAgentAccess";
import { getGlobalAgentConfig } from "@/shared/api/tauriGlobalAgentConfig";
import { listPersonas, setPersonaActive } from "@/shared/api/tauriPersonas";
import type {
  AcpRuntime,
  AgentPersona,
  CreateManagedAgentInput,
  ManagedAgent,
  UpdateManagedAgentInput,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export const WELCOME_GUIDE_AGENT_NAME = "Fizz";
export const WELCOME_GUIDE_PERSONA_ID = "builtin:fizz";
export const WELCOME_TEAM_ID = "builtin-team:welcome";
/**
 * Team id of the retired built-in single-member Fizz team (#1718). Agent
 * records provisioned while that team existed still carry this `teamId`;
 * they are the same built-in Welcome identity and must be reused rather
 * than re-minted.
 */
export const RETIRED_WELCOME_FIZZ_TEAM_ID = "builtin-team:fizz";
export const WELCOME_GUIDE_INTRO_MARKER = "buzz-welcome-intro.v1";
const LEGACY_WELCOME_GUIDE_AGENT_NAME = "Kit";
export const LEGACY_WELCOME_GUIDE_SYSTEM_PROMPT =
  "You are Kit, Sprout's friendly welcome guide. Help new users understand the community, channels, messages, and agents. Keep introductions concise, practical, and warm.";
export const WELCOME_GUIDE_INTRO_MESSAGE =
  "Hi, I'm Fizz. Welcome to Buzz.\n\nI can help you get oriented, answer questions, and make the first few steps feel less mysterious.\n\nFeel free to ask me what else you can do in Buzz, or just talk through what you want to build.";

export type WelcomeTeamRole = "lead" | "teammate";

export type WelcomeTeamStarterDefinition = Readonly<{
  name: string;
  personaId: string;
  role: WelcomeTeamRole;
}>;

/** Stable identities used to provision the Rust-seeded Welcome Team. */
export const WELCOME_TEAM_STARTERS = [
  { name: "Fizz", personaId: "builtin:fizz", role: "lead" },
  { name: "Honey", personaId: "builtin:honey", role: "teammate" },
  { name: "Pollen", personaId: "builtin:bumble", role: "teammate" },
] as const satisfies readonly WelcomeTeamStarterDefinition[];

export type WelcomeTeamAgents = [ManagedAgent, ManagedAgent, ManagedAgent];

const welcomeTeamPromises = new Map<string, Promise<WelcomeTeamAgents>>();

/**
 * Comparison key for a relay pin.
 *
 * Reuses the backend-compatible `canonicalRelayUrl` rather than trimming,
 * because the pair key the backend matches on already folds host case,
 * loopback aliases (`localhost` / `[::1]` / `127.*`) and default ports.
 * Trimming alone ranked `ws://localhost:3000` and `ws://127.0.0.1:3000` as two
 * different communities, so on an install that already carries duplicate
 * Welcome records the picker could reuse the wrong identity while the backend
 * considered both pins the same relay.
 *
 * A legacy or malformed pin canonicalizes to `null`; it falls back to a stable
 * lowercased form so two records carrying the same bad pin still match each
 * other instead of each becoming its own community. An unbound pin (`""`, what
 * the backend stores when none was supplied) stays `null` and is ranked as
 * eligible-everywhere by `relayPinRank`.
 */
function normalizeRelayUrl(relayUrl: string | null | undefined) {
  const raw = relayUrl?.trim() ?? "";
  if (!raw) return null;
  return canonicalRelayUrl(raw) ?? raw.toLowerCase().replace(/\/+$/, "");
}

/**
 * How well a record's stored relay pin matches the community being provisioned.
 * Lower is better.
 *
 * The pin is advisory only. The backend resolves an agent's relay purely from
 * the active workspace and deliberately ignores `relay_url`
 * (`effective_agent_relay_url` in `src-tauri/src/relay.rs`, agents-everywhere
 * #2122): every agent is eligible on every community. Treating a pin miss as a
 * hard *filter* is what made joining a second community mint a second keypair
 * for a starter that already existed — and an unbound record (`relayUrl === ""`,
 * which is what the backend stores when no pin was supplied) matched no
 * community at all. Ranking instead of filtering keeps the existing local
 * record for the community that pinned it while still reusing that same
 * identity everywhere else.
 */
function relayPinRank(agent: ManagedAgent, targetRelayUrl: string | null) {
  if (!targetRelayUrl) return 0;
  const pinnedRelayUrl = normalizeRelayUrl(agent.relayUrl);
  if (pinnedRelayUrl === targetRelayUrl) return 0;
  // Unbound records are eligible everywhere, so they outrank a record another
  // community already claimed.
  return pinnedRelayUrl ? 2 : 1;
}

const RELAY_PIN_RANKS = [0, 1, 2] as const;

function isBuiltInWelcomeGuideAgent(agent: ManagedAgent) {
  return agent.personaId === WELCOME_GUIDE_PERSONA_ID;
}

/**
 * The exact retired built-in Fizz identity: the record the retired
 * single-member built-in team (#1718) provisioned. Team id + persona + the
 * stock name together pin that one record, so a user-created agent that
 * merely shares the Fizz persona — or a customized agent left behind in a
 * demoted copy of the retired team — is never absorbed into the Welcome
 * Team and silently reconfigured.
 */
function isRetiredWelcomeFizzAgent(agent: ManagedAgent) {
  return (
    agent.teamId === RETIRED_WELCOME_FIZZ_TEAM_ID &&
    agent.personaId === WELCOME_GUIDE_PERSONA_ID &&
    agent.name === WELCOME_GUIDE_AGENT_NAME
  );
}

/**
 * True when `agent` is a Welcome Team record for `starter` — either a
 * current `builtin-team:welcome` record or the exact retired built-in Fizz
 * record for the lead starter.
 */
function isWelcomeTeamStarterAgent(
  agent: ManagedAgent,
  starter: WelcomeTeamStarterDefinition,
) {
  return (
    agent.personaId === starter.personaId &&
    (agent.teamId === WELCOME_TEAM_ID || isRetiredWelcomeFizzAgent(agent))
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

/**
 * Pick one already-provisioned agent out of `agents`, preferring the community
 * that pinned it but never rejecting a candidate over its pin alone. A miss
 * here is what mints a brand new keypair, so this must fall through to any
 * surviving record rather than return null.
 */
function pickAgentForRelay(agents: ManagedAgent[], relayUrl?: string | null) {
  const targetRelayUrl = normalizeRelayUrl(relayUrl);
  for (const rank of RELAY_PIN_RANKS) {
    const picked = pickAgentByStatus(
      agents.filter((agent) => relayPinRank(agent, targetRelayUrl) === rank),
    );
    if (picked) return picked;
  }
  return null;
}

export function pickWelcomeGuideAgent(agents: ManagedAgent[]) {
  return pickAgentByStatus(agents.filter(isWelcomeGuideAgent));
}

export function pickWelcomeGuideAgentForRelay(
  agents: ManagedAgent[],
  relayUrl?: string | null,
) {
  return pickAgentForRelay(agents.filter(isWelcomeGuideAgent), relayUrl);
}

/**
 * Find the preferred managed instance for one starter persona, preferring the
 * instance pinned to `relayUrl` but reusing an existing instance from any
 * community rather than reporting the starter as missing.
 */
export function pickWelcomeTeamStarterAgentForRelay(
  agents: ManagedAgent[],
  starter: WelcomeTeamStarterDefinition,
  relayUrl?: string | null,
) {
  return pickAgentForRelay(
    agents.filter((agent) => isWelcomeTeamStarterAgent(agent, starter)),
    relayUrl,
  );
}

/**
 * Pubkeys belonging to any managed Welcome Team persona. Relay-agnostic, to
 * match how the backend resolves an agent's relay — see {@link relayPinRank}.
 * Shares the team-scope predicate with
 * {@link pickWelcomeTeamStarterAgentForRelay} so the retired built-in Fizz
 * record counts here exactly when it is eligible for reuse there.
 */
export async function getWelcomeTeamAgentPubkeys() {
  return (await listManagedAgents())
    .filter((agent) =>
      WELCOME_TEAM_STARTERS.some((starter) =>
        isWelcomeTeamStarterAgent(agent, starter),
      ),
    )
    .map((agent) => agent.pubkey);
}

/** Legacy Fizz/Kit lookup retained for existing channel reuse checks. */
export async function getWelcomeGuideAgentPubkeys() {
  return (await listManagedAgents())
    .filter(isWelcomeGuideAgent)
    .map((agent) => agent.pubkey);
}

export async function activateWelcomeTeamPersonasSequentially(
  inactivePersonaIds: readonly string[],
  activate: (personaId: string) => Promise<unknown>,
) {
  for (const personaId of inactivePersonaIds) {
    await activate(personaId);
  }
}

async function ensureWelcomeTeamPersonasActive() {
  const personas = await listPersonas();
  const personasById = new Map(
    personas.map((persona) => [persona.id, persona]),
  );

  for (const starter of WELCOME_TEAM_STARTERS) {
    if (!personasById.has(starter.personaId)) {
      throw new Error(`${starter.name} agent not found.`);
    }
  }

  // Persona activation is a read-modify-write operation over one shared file.
  // Run these sequentially so concurrent writes cannot lose a teammate's
  // activation and leave Welcome provisioning permanently partial.
  await activateWelcomeTeamPersonasSequentially(
    WELCOME_TEAM_STARTERS.filter(
      ({ personaId }) => !personasById.get(personaId)?.isActive,
    ).map(({ personaId }) => personaId),
    (personaId) => setPersonaActive(personaId, true),
  );
}

async function ensureWelcomeTeamMembership(
  channelId: string,
  agents: WelcomeTeamAgents,
) {
  const members = await getChannelMembers(channelId).catch(() => []);
  const memberPubkeys = new Set(
    members.map((member) => normalizePubkey(member.pubkey)),
  );
  const missingAgents = agents.filter(
    (agent) => !memberPubkeys.has(normalizePubkey(agent.pubkey)),
  );
  if (missingAgents.length === 0) {
    return;
  }

  const result = await addChannelMembers({
    channelId,
    pubkeys: missingAgents.map((agent) => agent.pubkey),
    role: "bot",
  });
  const unexpectedError = result.errors.find(
    ({ error }) => !error.toLowerCase().includes("already"),
  );
  if (unexpectedError) {
    throw new Error(unexpectedError.error);
  }
}

export async function buildWelcomeStarterCreateInput(
  starter: WelcomeTeamStarterDefinition,
  persona: AgentPersona,
  runtimes: readonly AcpRuntime[],
  preferredRuntimeId: string | null,
  relayUrl?: string | null,
): Promise<CreateManagedAgentInput> {
  const { runtime } = resolveStartRuntimeForDefinition(
    persona,
    runtimes,
    preferredRuntimeId,
  );
  return {
    ...(await buildInstanceInputForDefinition(persona, runtime)),
    name: starter.name,
    teamId: WELCOME_TEAM_ID,
    relayUrl: relayUrl ?? undefined,
    spawnAfterCreate: false,
    startOnAppLaunch: false,
    respondTo: "owner-only",
  };
}

export function welcomeStarterRuntimeUpdate(
  existing: ManagedAgent,
  desired: CreateManagedAgentInput,
) {
  if (!desired.agentCommand) return null;

  const desiredArgs = desired.agentArgs ?? [];
  const desiredModel = desired.model ?? null;
  const desiredProvider = desired.provider ?? null;
  const desiredMcpCommand = desired.mcpCommand ?? "";
  if (
    existing.agentCommand === desired.agentCommand &&
    existing.agentArgs.join(",") === desiredArgs.join(",") &&
    existing.model === desiredModel &&
    existing.provider === desiredProvider &&
    existing.mcpCommand === desiredMcpCommand
  ) {
    return null;
  }

  return {
    pubkey: existing.pubkey,
    agentCommand: desired.agentCommand,
    harnessOverride: true,
    agentArgs: desiredArgs,
    mcpCommand: desiredMcpCommand,
    model: desiredModel,
    provider: desiredProvider,
  };
}

export function welcomeTeammateHasExpectedAccess(
  teammate: ManagedAgent,
  leadPubkey: string,
  agentAccessOwnerOnly: boolean,
) {
  if (agentAccessOwnerOnly) {
    // Welcome teammates are created owner-only, and the lead remains authorized
    // as a NIP-OA-verified sibling because every Welcome agent shares one owner.
    return (
      teammate.respondTo === "owner-only" &&
      teammate.respondToAllowlist.length === 0
    );
  }
  return (
    teammate.respondTo === "allowlist" &&
    teammate.respondToAllowlist.some(
      (pubkey) => normalizePubkey(pubkey) === normalizePubkey(leadPubkey),
    )
  );
}

/**
 * The access write that moves a Welcome teammate to the state this build
 * expects, or null when it is already there. The remediation target must track
 * {@link welcomeTeammateHasExpectedAccess}: writing `allowlist:[lead]` in an
 * owner-only build would fail the predicate again on the next provisioning
 * pass, so an upgraded install with pre-existing allowlisted teammates would
 * rewrite the same rejected state forever and keep restarting them.
 */
export function welcomeTeammateAccessUpdate(
  teammate: ManagedAgent,
  leadPubkey: string,
  agentAccessOwnerOnly: boolean,
): UpdateManagedAgentInput | null {
  if (
    welcomeTeammateHasExpectedAccess(teammate, leadPubkey, agentAccessOwnerOnly)
  ) {
    return null;
  }
  return agentAccessOwnerOnly
    ? {
        pubkey: teammate.pubkey,
        respondTo: "owner-only",
        respondToAllowlist: [],
      }
    : {
        pubkey: teammate.pubkey,
        respondTo: "allowlist",
        respondToAllowlist: [leadPubkey],
      };
}

/**
 * Ensure the complete built-in Welcome Team is ready for kickoff.
 * The team itself is Rust-seeded; this only activates personas, creates any
 * missing relay-scoped instances, and adds all three to Welcome as bots.
 */
async function provisionWelcomeTeam(
  channelId: string,
  relayUrl?: string | null,
): Promise<WelcomeTeamAgents> {
  const existingAgents = await listManagedAgents();
  await ensureWelcomeTeamPersonasActive();
  const [personas, runtimeCatalog, globalConfig, agentAccessOwnerOnly] =
    await Promise.all([
      listPersonas(),
      discoverAcpRuntimes(),
      getGlobalAgentConfig(),
      getAgentAccessOwnerOnly(),
    ]);
  const personasById = new Map(
    personas.map((persona) => [persona.id, persona]),
  );
  const runtimes = runtimeCatalog.filter(
    (runtime): runtime is AcpRuntime => runtime.availability === "available",
  );

  const agents: ManagedAgent[] = [];
  for (const starter of WELCOME_TEAM_STARTERS) {
    const persona = personasById.get(starter.personaId);
    if (!persona) {
      throw new Error(`${starter.name} agent not found.`);
    }
    const desired = await buildWelcomeStarterCreateInput(
      starter,
      persona,
      runtimes,
      globalConfig.preferred_runtime,
      relayUrl,
    );
    const existing = pickWelcomeTeamStarterAgentForRelay(
      existingAgents,
      starter,
      relayUrl,
    );
    if (existing) {
      const runtimeUpdate = welcomeStarterRuntimeUpdate(existing, desired);
      agents.push(
        runtimeUpdate
          ? (await updateManagedAgent(runtimeUpdate)).agent
          : existing,
      );
      continue;
    }

    const created = await createManagedAgent(desired);
    agents.push(created.agent);
  }
  const [lead, honey, pollen] = agents;
  if (!lead || !honey || !pollen) {
    throw new Error("Welcome Team provisioning did not return every starter.");
  }
  const welcomeAgents: WelcomeTeamAgents = [lead, honey, pollen];
  const leadPubkey = lead.pubkey;
  for (const index of [1, 2] as const) {
    const teammate = welcomeAgents[index];
    const accessUpdate = welcomeTeammateAccessUpdate(
      teammate,
      leadPubkey,
      agentAccessOwnerOnly,
    );
    if (accessUpdate) {
      const updated = await updateManagedAgent(accessUpdate);
      welcomeAgents[index] = updated.agent;
    }
  }
  await ensureWelcomeTeamMembership(channelId, welcomeAgents);
  return welcomeAgents;
}

export function ensureWelcomeTeam(
  channelId: string,
  relayUrl?: string | null,
): Promise<WelcomeTeamAgents> {
  const key = `${normalizeRelayUrl(relayUrl) ?? ""}:${channelId}`;
  const current = welcomeTeamPromises.get(key);
  if (current) return current;

  const promise = provisionWelcomeTeam(channelId, relayUrl).finally(() =>
    welcomeTeamPromises.delete(key),
  );
  welcomeTeamPromises.set(key, promise);
  return promise;
}
