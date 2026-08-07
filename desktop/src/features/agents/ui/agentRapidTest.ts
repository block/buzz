/**
 * Pure helpers for the in-app rapid agent smoke workflow.
 *
 * The rapid smoke test (Buzz Hermes) lets an owner verify that a managed agent
 * is reachable end-to-end in a channel: Save/restart/test → owner-authored
 * visible message in the chosen channel → agent opens/responds in its thread.
 * This module is intentionally side-effect-free so the same helpers can run in
 * tests, the panel, and the integration that actually posts the owner message.
 *
 * No env values, secrets, tokens, or relay URLs are interpolated into the
 * generated prompt. The only token the agent needs to recognise is the smoke
 * id, which is short, uniquely random, and self-contained.
 */

import type { Channel, ManagedAgent } from "../../../shared/api/types.ts";
import { normalizePubkey } from "../../../shared/lib/pubkey.ts";

/**
 * Sentinel token the agent must echo back to prove the round-trip worked.
 * The trailing `<id>` slot is filled by `createSmokeId()`.
 */
export const BUZZ_HERMES_OK_PREFIX = "BUZZ_HERMES_OK";

/**
 * Length of the random suffix appended to smoke prompts. Eight base36 chars
 * give ~41 bits of entropy — enough to keep collisions vanishingly unlikely
 * across a single device's session without making the prompt unreadable.
 */
const SMOKE_ID_RANDOM_SUFFIX_LENGTH = 8;

/**
 * Alphabet we draw from when minting a smoke id. Lowercase ASCII alphanumerics
 * only, so the id is safe to embed in chat content without escaping and will
 * not collide with whitespace/markdown tokenisation.
 */
const SMOKE_ID_ALPHABET = "0123456789abcdefghijklmnopqrstuvwxyz";

/**
 * Filter a channel list down to the channels that are eligible for a rapid
 * smoke test. A channel is eligible when:
 *
 *   1. The current desktop user is a member (`channel.isMember`).
 *   2. The channel is not archived (`archivedAt` is null).
 *   3. The channel already contains the managed agent's pubkey.
 *      `participantPubkeys` is the canonical surface for "agent is in this
 *      channel" (combines `memberPubkeys` + `participants` server-side); we
 *      fall back to `memberPubkeys` for forward-compat with older relay
 *      payloads that don't populate the merged list.
 *   4. Direct messages are allowed — the smoke test does not require a
 *      multi-member room, and DMs are often the cleanest place to ping a
 *      single agent.
 *
 * The input is treated as readonly; ordering is preserved.
 */
export function filterEligibleRapidTestChannels(
  channels: readonly Channel[] | undefined,
  agent: Pick<ManagedAgent, "pubkey"> | null | undefined,
): Channel[] {
  if (!channels || channels.length === 0 || !agent?.pubkey) {
    return [];
  }

  const targetPubkey = normalizePubkey(agent.pubkey);

  return channels.filter((channel) => {
    if (!channel.isMember) {
      return false;
    }
    // Forum posts use a different composer and navigation surface. Keep the
    // rapid workflow limited to stream/DM roots that can open the normal
    // thread panel and be handled by the normal message mutation.
    if (channel.channelType === "forum") {
      return false;
    }
    if (channel.archivedAt) {
      return false;
    }

    const memberPubkeys = [
      ...(channel.participantPubkeys ?? []),
      ...(channel.memberPubkeys ?? []),
    ];
    if (!Array.isArray(memberPubkeys) || memberPubkeys.length === 0) {
      return false;
    }

    for (const candidate of memberPubkeys) {
      if (normalizePubkey(candidate) === targetPubkey) {
        return true;
      }
    }
    return false;
  });
}

/**
 * Pick a default channel id from the eligible list, preserving the caller's
 * existing selection when it is still valid.
 *
 *   - Returns `null` when nothing is eligible yet, so the caller can render
 *     a "no channel" state instead of a misleading empty selection.
 *   - Returns the existing id when it appears in the eligible list, so the
 *     panel keeps the user's choice across channel-query refetches.
 *   - Otherwise returns the first eligible channel id, which is deterministic
 *     because `filterEligibleRapidTestChannels` preserves input order.
 */
export function pickDefaultRapidTestChannelId(
  eligibleChannels: readonly Channel[],
  currentChannelId: string | null,
): string | null {
  if (eligibleChannels.length === 0) {
    return null;
  }

  if (currentChannelId) {
    const stillEligible = eligibleChannels.some(
      (channel) => channel.id === currentChannelId,
    );
    if (stillEligible) {
      return currentChannelId;
    }
  }

  return eligibleChannels[0].id;
}

/**
 * Generate a short, unique smoke id suffix. The id is purely random (drawn from
 * `crypto.getRandomValues` when available, otherwise `Math.random`) and
 * carries no embedded metadata — agents match on the literal string, not
 * on the encoding.
 *
 * The function is deterministic on shape only: callers who need the id
 * observable across the panel and the posting path must capture the return
 * value rather than re-rolling.
 */
export function createSmokeId(
  randomSource: { randomValues?: (buffer: Uint8Array) => Uint8Array } = {},
): string {
  const buffer = new Uint8Array(SMOKE_ID_RANDOM_SUFFIX_LENGTH);
  if (randomSource.randomValues) {
    randomSource.randomValues(buffer);
  } else if (globalThis.crypto?.getRandomValues) {
    globalThis.crypto.getRandomValues(buffer);
  } else {
    for (let i = 0; i < buffer.length; i += 1) {
      buffer[i] = Math.floor(Math.random() * 256);
    }
  }

  let suffix = "";
  for (let i = 0; i < buffer.length; i += 1) {
    suffix += SMOKE_ID_ALPHABET[buffer[i] % SMOKE_ID_ALPHABET.length];
  }

  return suffix;
}

export type RapidTestPrompt = {
  /** Stable identifier for the smoke run; embedded in the prompt body. */
  smokeId: string;
  /** Human-readable relative timestamp suffix for log readability. */
  generatedAt: string;
  /** The full message body the owner should post. */
  body: string;
};

export type RapidTestSelection = {
  channel: Channel;
  channelId: string;
  prompt: RapidTestPrompt;
};

export type RapidSaveMode = "save" | "restart" | "smoke";

export type RapidPostSaveOutcome =
  | { kind: "restarted" }
  | {
      kind: "smoke-posted";
      channelId: string;
      eventId: string;
      threadOpened: boolean;
    };

type RapidPostSaveRouteAgent = Pick<
  ManagedAgent,
  "agentCommand" | "acpCommand"
>;

/** Fail-closed route mismatch detected after Save and before restart/send. */
export class RapidPostSaveRouteError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "RapidPostSaveRouteError";
  }
}

function normalizeCommandIdentity(command: string): string {
  const normalized = command.trim().replaceAll("\\", "/");
  return /^[a-zA-Z]:\//.test(normalized)
    ? normalized.toLowerCase()
    : normalized;
}

function commandsMatch(left: string, right: string): boolean {
  return (
    normalizeCommandIdentity(left) !== "" &&
    normalizeCommandIdentity(left) === normalizeCommandIdentity(right)
  );
}

/**
 * Fail closed when Save changed the runtime route that the owner reviewed.
 * This check runs before restart or message posting and intentionally reports
 * only route classes, never command paths or environment values.
 */
export function assertRapidPostSaveRoute(input: {
  savedAgent: RapidPostSaveRouteAgent;
  savedRuntimeId: string | null | undefined;
  expectedRuntimeId: string;
  expectedAgentCommand: string;
  expectedAcpCommand: string;
  catalogRuntimeCommand: string | null | undefined;
  catalogRequired?: boolean;
}): void {
  const expectedRuntimeId = input.expectedRuntimeId.trim();
  if (
    !expectedRuntimeId ||
    input.savedRuntimeId?.trim() !== expectedRuntimeId
  ) {
    throw new RapidPostSaveRouteError(
      "The persisted runtime changed after save.",
    );
  }

  const catalogRequired = input.catalogRequired ?? true;
  if (
    !commandsMatch(input.savedAgent.agentCommand, input.expectedAgentCommand) ||
    (catalogRequired &&
      (!input.catalogRuntimeCommand ||
        !commandsMatch(
          input.savedAgent.agentCommand,
          input.catalogRuntimeCommand,
        )))
  ) {
    throw new RapidPostSaveRouteError(
      "The saved harness route changed after save.",
    );
  }

  if (!commandsMatch(input.savedAgent.acpCommand, input.expectedAcpCommand)) {
    throw new RapidPostSaveRouteError(
      "The saved ACP sidecar changed after save.",
    );
  }
}

type RapidRouteRefetchResult<T> = {
  data: readonly T[] | undefined;
  isError: boolean;
};

/**
 * Re-read the persisted route after Save, then fail before restart/send if the
 * reviewed runtime, harness command, or ACP sidecar no longer matches.
 */
export async function revalidateRapidPostSaveRoute(input: {
  savedAgent: RapidPostSaveRouteAgent &
    Pick<ManagedAgent, "runtime" | "personaId">;
  expectedRuntimeId: string;
  expectedAgentCommand: string;
  expectedAcpCommand: string;
  refetchRuntimes: () => Promise<
    RapidRouteRefetchResult<{ id: string; command: string | null }>
  >;
  refetchPersonas: () => Promise<
    RapidRouteRefetchResult<{ id: string; runtime: string | null }>
  >;
}): Promise<void> {
  const catalogRequired = input.expectedRuntimeId.trim() !== "custom";
  const personaRequired =
    !input.savedAgent.runtime?.trim() && input.savedAgent.personaId != null;
  const [runtimes, personas] = await Promise.all([
    catalogRequired
      ? input.refetchRuntimes()
      : Promise.resolve({ data: [], isError: false }),
    personaRequired
      ? input.refetchPersonas()
      : Promise.resolve({ data: [], isError: false }),
  ]);
  if (runtimes.isError || personas.isError) {
    throw new RapidPostSaveRouteError(
      "The saved runtime route could not be revalidated.",
    );
  }

  const inheritedRuntimeId = personaRequired
    ? personas.data?.find(
        (persona) => persona.id === input.savedAgent.personaId,
      )?.runtime
    : null;
  const savedRuntimeId =
    input.savedAgent.runtime?.trim() ||
    inheritedRuntimeId?.trim() ||
    (catalogRequired ? null : "custom");
  const savedRuntime = runtimes.data?.find(
    (runtime) => runtime.id === savedRuntimeId,
  );
  assertRapidPostSaveRoute({
    savedAgent: input.savedAgent,
    savedRuntimeId,
    expectedRuntimeId: input.expectedRuntimeId,
    expectedAgentCommand: input.expectedAgentCommand,
    expectedAcpCommand: input.expectedAcpCommand,
    catalogRuntimeCommand: savedRuntime?.command,
    catalogRequired,
  });
}

/**
 * Execute the deterministic post-save portion of the rapid workflow.
 * Dependencies are injected so ordering is testable without React or Tauri.
 */
export async function runRapidAgentPostSaveAction(input: {
  mode: RapidSaveMode;
  pubkey: string;
  relayUrl: string | null | undefined;
  selection: RapidTestSelection | null;
  restart: (pubkey: string, relayUrl: string) => Promise<unknown>;
  waitForReady?: () => Promise<void>;
  sendOwnerMessage: (
    channel: Channel,
    content: string,
    mentionPubkeys: string[],
  ) => Promise<{ eventId: string }>;
  openThread: (channelId: string, eventId: string) => Promise<unknown>;
}): Promise<RapidPostSaveOutcome | null> {
  if (input.mode === "save") {
    return null;
  }

  const relayUrl = input.relayUrl?.trim();
  if (!relayUrl) {
    throw new Error("No active community relay is available for restart.");
  }

  await input.restart(input.pubkey, relayUrl);
  await input.waitForReady?.();
  if (input.mode === "restart") {
    return { kind: "restarted" };
  }

  if (!input.selection) {
    throw new Error("Choose a channel where this agent is already a member.");
  }

  const sent = await input.sendOwnerMessage(
    input.selection.channel,
    input.selection.prompt.body,
    [input.pubkey],
  );
  let threadOpened = true;
  try {
    await input.openThread(input.selection.channelId, sent.eventId);
  } catch {
    threadOpened = false;
  }
  return {
    kind: "smoke-posted",
    channelId: input.selection.channelId,
    eventId: sent.eventId,
    threadOpened,
  };
}

/**
 * Build the deterministic, clearly labelled prompt the owner-authored smoke
 * message must contain. The prompt:
 *
 *   - Starts with a visible `[buzz-hermes-smoke]` label so anyone watching the
 *     channel (including the agent) can recognise it as a test, not real
 *     work.
 *   - Echoes the full `BUZZ_HERMES_OK <smokeId>` directive on its own line so
 *     agents can match on the literal token without parsing surrounding copy.
 *   - Includes the smoke id and a generated-at timestamp for log correlation.
 *   - Never embeds env values, secrets, relay URLs, or pubkey material.
 *
 * The function is pure: the same `smokeId` and `generatedAt` always produce
 * the same body, and the body shape is stable enough that contract tests can
 * assert against it.
 */
export function buildRapidTestPrompt(input: {
  smokeId: string;
  generatedAt?: Date | string;
}): RapidTestPrompt {
  const generatedAt = formatGeneratedAt(input.generatedAt ?? new Date());
  const body = [
    "[buzz-hermes-smoke] rapid agent test",
    "",
    `Please reply in this thread with the exact token: ${BUZZ_HERMES_OK_PREFIX} ${input.smokeId}`,
    "",
    `smoke id: ${input.smokeId}`,
    `generated at: ${generatedAt}`,
  ].join("\n");

  return {
    smokeId: input.smokeId,
    generatedAt,
    body,
  };
}

/**
 * Compact ISO-8601 (seconds, UTC) used inside the prompt body. We avoid
 * locale formatting so the same id always renders the same timestamp —
 * agents and humans reading logs shouldn't have to square a timezone.
 */
function formatGeneratedAt(value: Date | string): string {
  const date = typeof value === "string" ? new Date(value) : value;
  if (Number.isNaN(date.getTime())) {
    return "unknown";
  }
  return date.toISOString().replace(/\.\d{3}Z$/, "Z");
}
