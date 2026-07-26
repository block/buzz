import type { SpawnerAiProvider } from "@/shared/api/spawnerRelay";

/**
 * Context for editing an agent on a server spawner.
 *
 * Returned by `resolveServerAgentEditContext` when the agent is server-resident
 * (either relocated or deployed to a spawner).
 */
export type ServerAgentEditContext = {
  spawnerPubkey: string;
  specSlug: string;
  agentPubkey: string;
  spawnerName: string;
};

/**
 * Resolve whether the agent being edited lives on a spawner.
 *
 * Returns non-null when EITHER `relocatedToSpawner` is a non-empty string OR
 * `deployedSpawnerPubkey` is non-null — preferring `relocatedToSpawner` when
 * both exist. Requires both `agentPubkey` and `slug` to be non-empty to build
 * the context; a server context without those fields is unusable for the
 * prompt-update frame and will return null.
 */
export function resolveServerAgentEditContext(input: {
  relocatedToSpawner: string | null | undefined;
  deployedSpawnerPubkey: string | null;
  agentPubkey: string | null;
  slug: string | null;
  spawnerNameFor: (pubkey: string) => string;
}): ServerAgentEditContext | null {
  // Check that both required fields are present and non-empty
  if (!input.agentPubkey?.trim() || !input.slug?.trim()) {
    return null;
  }

  // Prefer relocatedToSpawner, fall back to deployedSpawnerPubkey
  const spawnerPubkey =
    input.relocatedToSpawner?.trim() || input.deployedSpawnerPubkey;

  if (!spawnerPubkey) {
    return null;
  }

  return {
    spawnerPubkey,
    specSlug: input.slug.trim(),
    agentPubkey: input.agentPubkey.trim(),
    spawnerName: input.spawnerNameFor(spawnerPubkey),
  };
}

/**
 * Model options for a server agent: the spawner's catalog, or null → free-text fallback.
 *
 * Returns null when `ai` is undefined or empty; otherwise returns all provider
 * IDs and the models of the provider matching `provider`. If `provider` is null
 * or not found, uses the first provider's models.
 */
export function serverModelOptions(
  ai: SpawnerAiProvider[] | undefined,
  provider: string | null,
): { providers: string[]; models: string[] } | null {
  if (!ai || ai.length === 0) {
    return null;
  }

  const providers = ai.map((p) => p.id);

  // Find the provider matching the given id, or use the first provider
  const selectedProvider = ai.find((p) => p.id === provider) || ai[0];

  return {
    providers,
    models: selectedProvider.models,
  };
}
