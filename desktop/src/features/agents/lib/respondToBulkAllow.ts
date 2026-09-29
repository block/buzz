/**
 * Bulk-allow helpers for the inbound author gate UI.
 *
 * The three dropdown shortcuts fill `respondToAllowlist` with concrete
 * pubkeys and leave `respondTo` as `"allowlist"`, so they persist on the
 * same path as a hand-picked Selected people list. No new harness modes.
 *
 * Semantics:
 * - `all-local-agents` — every managed agent on this computer
 *   (`backend.type === "local"`).
 * - `all-community-bots` — every installed community (catalog) bot.
 * - `all-bots` — the union of both sets (local agents and community bots).
 */

export const RESPOND_TO_BULK_ALLOW_OPTIONS = [
  "all-local-agents",
  "all-community-bots",
  "all-bots",
] as const;

export type RespondToBulkAllowOption =
  (typeof RESPOND_TO_BULK_ALLOW_OPTIONS)[number];

export type BulkAllowAgent = {
  pubkey: string;
  backend: { type: string };
};

export type BulkAllowBot = {
  pubkey: string;
};

function normalizePubkey(pubkey: string): string {
  return pubkey.trim().toLowerCase();
}

export function isRespondToBulkAllowOption(
  value: string,
): value is RespondToBulkAllowOption {
  return (RESPOND_TO_BULK_ALLOW_OPTIONS as readonly string[]).includes(value);
}

/** Local managed-agent pubkeys, normalized and deduped. */
export function localAgentAllowlistPubkeys(
  agents: readonly BulkAllowAgent[],
  options?: { excludePubkey?: string | null },
): string[] {
  const exclude = options?.excludePubkey
    ? normalizePubkey(options.excludePubkey)
    : null;
  const out: string[] = [];
  const seen = new Set<string>();
  for (const agent of agents) {
    if (agent.backend.type !== "local") continue;
    const pubkey = normalizePubkey(agent.pubkey);
    if (!pubkey || (exclude && pubkey === exclude) || seen.has(pubkey)) {
      continue;
    }
    seen.add(pubkey);
    out.push(pubkey);
  }
  return out;
}

/** Installed community-bot pubkeys, normalized and deduped. */
export function communityBotAllowlistPubkeys(
  bots: readonly BulkAllowBot[],
): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const bot of bots) {
    const pubkey = normalizePubkey(bot.pubkey);
    if (!pubkey || seen.has(pubkey)) continue;
    seen.add(pubkey);
    out.push(pubkey);
  }
  return out;
}

/** Pubkeys for a bulk-allow dropdown choice. */
export function bulkAllowPubkeys(
  option: RespondToBulkAllowOption,
  input: {
    localAgents: readonly BulkAllowAgent[];
    communityBots: readonly BulkAllowBot[];
    excludePubkey?: string | null;
  },
): string[] {
  const local = localAgentAllowlistPubkeys(input.localAgents, {
    excludePubkey: input.excludePubkey,
  });
  const community = communityBotAllowlistPubkeys(input.communityBots);
  switch (option) {
    case "all-local-agents":
      return local;
    case "all-community-bots":
      return community;
    case "all-bots": {
      const seen = new Set(local);
      const out = [...local];
      for (const pubkey of community) {
        if (seen.has(pubkey)) continue;
        seen.add(pubkey);
        out.push(pubkey);
      }
      return out;
    }
  }
}

/** One-line helper under the dropdown after a bulk choice. */
export function bulkAllowHelperText(
  option: RespondToBulkAllowOption,
  count: number,
): string {
  switch (option) {
    case "all-local-agents":
      return count === 0
        ? "No local agents are available to allow yet."
        : `Allows all ${count} local agent${count === 1 ? "" : "s"} on this computer.`;
    case "all-community-bots":
      return count === 0
        ? "No community bots are installed to allow yet."
        : `Allows all ${count} installed community bot${count === 1 ? "" : "s"}.`;
    case "all-bots":
      return count === 0
        ? "No local agents or community bots are available to allow yet."
        : `Allows all ${count} local agents and community bots.`;
  }
}
