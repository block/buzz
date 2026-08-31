// Persona (agent definition) wire types, split out of `types.ts` to keep that
// file inside the repo-wide size ratchet. Consumers import these through
// `@/shared/api/types`, which re-exports everything here.
import type { RespondToMode } from "./types";

export type AgentPersona = {
  id: string;
  displayName: string;
  avatarUrl: string | null;
  /**
   * Optional short, PUBLIC description (max 280 chars), shown on the agent's
   * card and profile. Excluded from the persona content hash (no restart
   * badge). Null means no owner-authored description.
   */
  description: string | null;
  systemPrompt: string;
  /** Preferred ACP runtime ID (e.g. "goose", "claude"). */
  runtime: string | null;
  /** Opaque, harness-specific model identifier string. Buzz stores and passes through without interpretation. */
  model: string | null;
  /** LLM inference provider (e.g. "databricks", "anthropic"). Injected as the runtime's provider env var at spawn time. */
  provider: string | null;
  namePool: string[];
  isBuiltIn: boolean;
  isActive: boolean;
  /** Whether this persona is discoverable in the active community catalog. */
  shared: boolean;
  /** Team ID if this persona was imported from a team directory. Team personas are non-editable. */
  sourceTeam?: string | null;
  /**
   * Set only on a local copy of another owner's shared catalog entry. A copy
   * carries a fresh local `id`, so this coordinate is the only thing that can
   * answer "is this catalog entry already added" without minting a duplicate.
   */
  catalogSource?: CatalogSourceCoordinate | null;
  /** Agent environment variables, layered after desktop parent and persona values. */
  envVars: Record<string, string>;
  /** NIP-AP behavioral defaults (wire shape). Null/empty = unset. */
  respondTo: RespondToMode | null;
  respondToAllowlist: string[];
  parallelism: number | null;
  createdAt: string;
  updatedAt: string;
};

/**
 * A catalog publication's coordinate: the owner who published it and the
 * `d`-tag identifying the persona within that owner's catalog. Mirrors the
 * backend `CatalogSource`.
 */
export type CatalogSourceCoordinate = {
  ownerPubkey: string;
  personaId: string;
};

/**
 * NIP-AP behavioral group for a definition: absent preserves the stored group
 * for legacy callers; present replaces it as a unit. Mirrors `PersonaBehaviorRequest`.
 */
export type PersonaBehaviorInput = {
  respondTo?: RespondToMode;
  respondToAllowlist?: string[];
  parallelism?: number;
};

export type CreatePersonaInput = {
  displayName: string;
  avatarUrl?: string;
  /** Optional short, PUBLIC description (max 280 chars). Empty string clears. */
  description?: string | null;
  systemPrompt: string;
  runtime?: string;
  model?: string;
  provider?: string;
  namePool?: string[];
  envVars?: Record<string, string>;
  behavior?: PersonaBehaviorInput;
  /**
   * Set when this persona is a copy of another owner's shared catalog entry,
   * so the catalog can tell an already-added foreign entry from a new one.
   */
  catalogSource?: CatalogSourceCoordinate;
};

export type UpdatePersonaInput = {
  id: string;
  displayName: string;
  avatarUrl?: string;
  /** Optional short, PUBLIC description (max 280 chars). Empty string clears. */
  description?: string | null;
  systemPrompt: string;
  runtime?: string;
  model?: string;
  provider?: string;
  namePool?: string[];
  envVars?: Record<string, string>;
  behavior?: PersonaBehaviorInput;
};
