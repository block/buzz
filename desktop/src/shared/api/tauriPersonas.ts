import { invokeTauri } from "@/shared/api/tauri";
import type {
  AgentPersona,
  CreatePersonaInput,
  RespondToMode,
  UpdatePersonaInput,
} from "@/shared/api/types";

export type RawPersona = {
  id: string;
  display_name: string;
  avatar_url: string | null;
  /** Optional short, PUBLIC description (max 280 chars). */
  description?: string | null;
  system_prompt: string;
  runtime?: string | null;
  model?: string | null;
  provider?: string | null;
  name_pool?: string[];
  is_builtin: boolean;
  is_active?: boolean;
  shared?: boolean;
  source_team?: string | null;
  /**
   * Provenance of a local copy of another owner's catalog entry. Serialized by
   * the backend `CatalogSource` in snake_case; the create payload sends the
   * camelCase aliases it accepts.
   */
  catalog_source?: { owner_pubkey: string; persona_id: string } | null;
  env_vars?: Record<string, string>;
  respond_to?: string | null;
  respond_to_allowlist?: string[];
  parallelism?: number | null;
  created_at: string;
  updated_at: string;
  /** Non-null when the pack `.persona.md` write-back failed (non-fatal). */
  writeback_warning?: string | null;
};

export function fromRawPersona(persona: RawPersona): AgentPersona {
  return {
    id: persona.id,
    displayName: persona.display_name,
    avatarUrl: persona.avatar_url,
    description: persona.description ?? null,
    systemPrompt: persona.system_prompt,
    runtime: persona.runtime ?? null,
    model: persona.model ?? null,
    provider: persona.provider ?? null,
    namePool: persona.name_pool ?? [],
    isBuiltIn: persona.is_builtin,
    isActive: persona.is_active ?? true,
    shared: persona.shared ?? false,
    sourceTeam: persona.source_team ?? null,
    catalogSource: persona.catalog_source
      ? {
          ownerPubkey: persona.catalog_source.owner_pubkey,
          personaId: persona.catalog_source.persona_id,
        }
      : null,
    envVars: persona.env_vars ?? {},
    respondTo: (persona.respond_to as RespondToMode | undefined) ?? null,
    respondToAllowlist: persona.respond_to_allowlist ?? [],
    parallelism: persona.parallelism ?? null,
    createdAt: persona.created_at,
    updatedAt: persona.updated_at,
  };
}

/**
 * Normalize only the unambiguous empty/absent cases for the wire. The trusted
 * Rust boundary validates the authored bytes before applying trim/empty
 * storage normalization.
 */
function normalizeDescription(
  description: string | null | undefined,
): string | null {
  if (description === null || description === undefined || description === "") {
    return null;
  }
  // Preserve the authored bytes for the Rust boundary to validate. Trimming
  // here could turn a prohibited edge control into apparently valid text.
  return description;
}

export async function listPersonas(): Promise<AgentPersona[]> {
  return (await invokeTauri<RawPersona[]>("list_personas")).map(fromRawPersona);
}

export async function createPersona(
  input: CreatePersonaInput,
): Promise<AgentPersona> {
  return fromRawPersona(
    await invokeTauri<RawPersona>("create_persona", {
      input: {
        displayName: input.displayName,
        avatarUrl: input.avatarUrl,
        description: normalizeDescription(input.description),
        systemPrompt: input.systemPrompt,
        runtime: input.runtime,
        model: input.model,
        provider: input.provider,
        namePool: input.namePool ?? [],
        envVars: input.envVars ?? {},
        behavior: input.behavior,
        catalogSource: input.catalogSource,
      },
    }),
  );
}

/** The `UpdatePersonaRequest` payload shared by both edit commands. */
function updatePersonaPayload(input: UpdatePersonaInput) {
  return {
    id: input.id,
    displayName: input.displayName,
    avatarUrl: input.avatarUrl,
    description: normalizeDescription(input.description),
    systemPrompt: input.systemPrompt,
    runtime: input.runtime,
    model: input.model,
    provider: input.provider,
    namePool: input.namePool ?? [],
    // Send envVars only when caller explicitly provided it; omitting
    // tells the backend "don't touch the stored env vars" so editing
    // unrelated fields can't silently wipe saved credentials.
    envVars: input.envVars,
    // Same absent-vs-present contract as envVars for the behavioral quad.
    behavior: input.behavior,
  };
}

export async function updatePersona(
  input: UpdatePersonaInput,
): Promise<AgentPersona> {
  const raw = await invokeTauri<RawPersona>("update_persona", {
    input: updatePersonaPayload(input),
  });
  if (raw.writeback_warning) {
    console.warn(
      `[updatePersona] pack write-back failed (edit saved locally): ${raw.writeback_warning}`,
    );
  }
  return fromRawPersona(raw);
}

/**
 * Save an edit AND publish the persona's catalog head, reporting whether the
 * relay accepted it.
 *
 * `updatePersona` only enqueues the head best-effort, so it cannot tell the UI
 * whether the community catalog actually received the change. Use this for the
 * "Save and publish" affordance, which promises exactly that.
 */
export async function updatePersonaAndPublish(
  input: UpdatePersonaInput,
): Promise<PersonaSharePublicationResult> {
  return fromRawPublicationResult(
    await invokeTauri<RawPersonaSharePublicationResult>(
      "update_persona_and_publish",
      { input: updatePersonaPayload(input) },
    ),
  );
}

type RawPersonaSharePublicationResult = {
  persona: RawPersona;
  publicationStatus: "published" | "queued";
  relayMessage?: string;
};

function fromRawPublicationResult(
  raw: RawPersonaSharePublicationResult,
): PersonaSharePublicationResult {
  return {
    persona: fromRawPersona(raw.persona),
    publicationStatus: raw.publicationStatus,
    relayMessage: raw.relayMessage ?? null,
  };
}

export async function deletePersona(id: string): Promise<void> {
  await invokeTauri("delete_persona", { id });
}

export async function setPersonaActive(
  id: string,
  active: boolean,
): Promise<AgentPersona> {
  return fromRawPersona(
    await invokeTauri<RawPersona>("set_persona_active", { id, active }),
  );
}

export async function setPersonaShared(
  id: string,
  shared: boolean,
): Promise<PersonaSharePublicationResult> {
  return fromRawPublicationResult(
    await invokeTauri<RawPersonaSharePublicationResult>("set_persona_shared", {
      id,
      shared,
    }),
  );
}

export type PersonaSharePublicationResult = {
  persona: AgentPersona;
  publicationStatus: "published" | "queued";
  relayMessage: string | null;
};

export type SnapshotMemoryLevel = "none" | "core" | "everything";
export type SnapshotFormat = "json" | "png";

export async function exportAgentSnapshot(
  id: string,
  memoryLevel: SnapshotMemoryLevel,
  format: SnapshotFormat,
  memorySourcePubkey?: string | null,
  avatarPngDataUrl?: string,
): Promise<boolean> {
  return invokeTauri<boolean>("export_agent_snapshot", {
    id,
    memorySourcePubkey: memorySourcePubkey ?? null,
    memoryLevel,
    format,
    avatarPngDataUrl: avatarPngDataUrl ?? null,
  });
}

/** The byte payload returned by `encode_agent_snapshot_for_send`. */
export type EncodedSnapshotPayload = {
  /** Raw snapshot bytes — pass directly to `uploadMediaBytes`. */
  fileBytes: number[];
  /** Suggested filename (e.g. `my-agent.agent.json`). */
  fileName: string;
};

/**
 * Encode a snapshot in memory and return the bytes to the frontend without
 * opening any file dialog.  Use this for the native-send path; use
 * `exportAgentSnapshot` for the local save-to-disk path.
 *
 * Both commands call the same shared Rust encoder, so byte output is
 * identical for identical inputs.
 */
export async function encodeAgentSnapshotForSend(
  id: string,
  memoryLevel: SnapshotMemoryLevel,
  format: SnapshotFormat,
  memorySourcePubkey?: string | null,
  avatarPngDataUrl?: string,
): Promise<EncodedSnapshotPayload> {
  return invokeTauri<EncodedSnapshotPayload>("encode_agent_snapshot_for_send", {
    id,
    memorySourcePubkey: memorySourcePubkey ?? null,
    memoryLevel,
    format,
    avatarPngDataUrl: avatarPngDataUrl ?? null,
  });
}

// ── Agent trading cards ───────────────────────────────────────────────────────

/** Result of `mint_agent_card`. */
export type MintedAgentCard = {
  /** Final `.agent.png` bytes (chunk-injected, verified), base64-encoded. */
  cardPngBase64: string;
  /** Suggested filename, e.g. `eva.agent.png`. */
  fileName: string;
  /** Designer commentary emitted alongside the image (may be empty). */
  designerNotes: string;
  /** True when the embedded manifest is encrypted to the (owner, agent) pair. */
  locked: boolean;
  /** How much memory is embedded in the card's snapshot. */
  memoryLevel: SnapshotMemoryLevel;
};

/** Error prefix Rust returns when no OpenAI key is configured. */
export const NO_OPENAI_KEY_PREFIX = "NO_OPENAI_KEY:";

/**
 * Which env layer resolves the OpenAI key for a card mint of this agent.
 *
 * - `"none"` — no key configured anywhere; show the first-time setup panel.
 * - `"global"` — key comes from global Agent Defaults env; writable from the
 *   dialog via `cardMintSaveOpenaiKey`.
 * - `"persona"` — key is set on the linked persona; cannot be updated from
 *   the mint dialog (would be shadowed by the higher-priority layer).
 * - `"agent"` — key is set directly on the agent record; same restriction.
 * - `"process"` — key comes from the process environment (dev fallback);
 *   same restriction.
 */
export type CardMintKeyLayer =
  | "none"
  | "global"
  | "persona"
  | "agent"
  | "process";

/**
 * Report which env layer resolves the OpenAI key for a card mint of this agent
 * (same layering as the mint itself). Returns a layer discriminant — never the
 * key value itself. Use to decide whether to show a writable key input (none /
 * global) or a read-only redirect (persona / agent / process).
 */
export async function cardMintKeyStatus(id: string): Promise<CardMintKeyLayer> {
  return invokeTauri<CardMintKeyLayer>("card_mint_key_status", { id });
}

/**
 * Save an OPENAI_API_KEY into the global Agent Defaults env for card minting.
 *
 * Narrow seam: validated read-modify-write of the single key against the
 * latest on-disk config. Unlike the general set_global_agent_config, this
 * NEVER restarts running agents — the mint re-reads config per call, and a
 * card setup must not disrupt unrelated agents.
 */
export async function cardMintSaveOpenaiKey(key: string): Promise<void> {
  return invokeTauri<void>("card_mint_save_openai_key", { key });
}

/**
 * Mint a trading card for an agent. One long API call (~2–3 minutes).
 * Reroll = call again; the backend holds no session state.
 *
 * When `lock` is true, the embedded manifest is NIP-44-encrypted to the
 * (owner, agent) pair: only those two keys can import the card. Requires a
 * linked agent instance.
 *
 * `memoryLevel` (default `"none"`) embeds the owner's decrypted memory in
 * the card's snapshot — same levels as snapshot export. Levels other than
 * `"none"` require a linked agent instance; Rust derives the memory source
 * from the resolved instance itself.
 */
export async function mintAgentCard(
  id: string,
  styleNotes?: string,
  lock?: boolean,
  memoryLevel?: SnapshotMemoryLevel,
): Promise<MintedAgentCard> {
  return invokeTauri<MintedAgentCard>("mint_agent_card", {
    id,
    styleNotes: styleNotes || null,
    lock: lock ?? null,
    memoryLevel: memoryLevel ?? null,
  });
}

/** Save minted card bytes to disk via the OS save dialog. */
export async function saveAgentCard(
  cardPngBase64: string,
  fileName: string,
): Promise<boolean> {
  return invokeTauri<boolean>("save_agent_card", { cardPngBase64, fileName });
}

/** One archived (previously minted) card, as listed by `list_agent_cards`. */
export type ArchivedAgentCard = {
  /** On-disk archive key — pass to `loadAgentCard`. */
  storedFileName: string;
  /** Suggested save-as name, e.g. `eva.agent.png`. */
  fileName: string;
  agentId: string;
  agentName: string;
  designerNotes: string;
  locked: boolean;
  /** Memory embedded in the card's snapshot ("none" for pre-field archives). */
  memoryLevel: SnapshotMemoryLevel;
  /** ISO-8601 mint timestamp. */
  mintedAt: string;
  /** Small JPEG grid preview, base64 (absent if the thumb write failed). */
  thumbJpegBase64: string | null;
};

/** List all archived cards, newest first. */
export async function listAgentCards(): Promise<ArchivedAgentCard[]> {
  return invokeTauri<ArchivedAgentCard[]>("list_agent_cards");
}

/** Load one archived card's full PNG bytes (base64) by its archive key. */
export async function loadAgentCard(storedFileName: string): Promise<string> {
  return invokeTauri<string>("load_agent_card", { storedFileName });
}

// ── Snapshot import ───────────────────────────────────────────────────────────

/** Preview returned by `preview_agent_snapshot_import` before any write. */
export type AgentSnapshotImportPreview = {
  displayName: string;
  /** Source classification shown in the preview; imports remain custom. */
  isBuiltIn: boolean;
  model: string | null;
  runtime: string | null;
  systemPrompt: string | null;
  /** Effective avatar: data URL if present, source URL fallback otherwise. */
  avatarUrl: string | null;
  /** "none" | "core" | "everything" */
  memoryLevel: string;
  memoryEntryCount: number;
  /** True when the snapshot's respond_to_allowlist is non-empty. */
  hasSourceAllowlist: boolean;
  sourceAllowlistCount: number;
  /** Full source pubkeys shown before import, not only their count. */
  sourceAllowlist: string[];
  /** Validated, pretty-printed manifest for full payload disclosure. */
  manifestJson: string;
  /**
   * True when the snapshot came from a locked (encrypted) card this machine
   * unlocked. Cards that cannot be unlocked fail with a refusal error and
   * never reach a preview.
   */
  locked: boolean;
};

/** Confirmation sent to `confirm_agent_snapshot_import`. */
export type AgentSnapshotImportConfirm = {
  fileBytes: number[];
  /** When true, copy source allowlist to the new agent. Default: false (Clear). */
  keepAllowlist: boolean;
};

/** Structured result from a confirmed import. */
export type AgentSnapshotImportResult = {
  displayName: string;
  /** Hex pubkey of the newly minted agent. */
  newPubkey: string;
  /** Persona ID created for the agent. */
  personaId: string;
  memoryWritten: number;
  memoryTotal: number;
  /** Non-empty when some memory entries failed to publish. */
  memoryErrors: string[];
  /** Non-empty when profile sync encountered a non-fatal relay error. */
  profileSyncError: string | null;
};

/**
 * Decode and validate a snapshot file, returning a preview for the
 * confirmation UI. No writes of any kind are performed.
 */
export async function previewAgentSnapshotImport(
  fileBytes: number[],
  fileName: string,
): Promise<AgentSnapshotImportPreview> {
  return invokeTauri<AgentSnapshotImportPreview>(
    "preview_agent_snapshot_import",
    { fileBytes, fileName },
  );
}

/**
 * Import a `buzz-agent-snapshot v1` file as a brand-new agent with fresh
 * keys. Returns a structured result describing what was created and whether
 * memory restoration was complete.
 */
export async function confirmAgentSnapshotImport(
  input: AgentSnapshotImportConfirm,
): Promise<AgentSnapshotImportResult> {
  return invokeTauri<AgentSnapshotImportResult>(
    "confirm_agent_snapshot_import",
    { input },
  );
}

// Patches a single inbound persona/team/agent projection event into the local
// store (personas.json). The backend resolves the match key and the
// pending-edit race; the frontend forwards the raw Nostr event JSON plus the
// relay it arrived on, so a workspace switch mid-flight cannot retain the event
// into the newly active community's scoped store.
export async function reconcileInboundPersonaEvent(
  eventJson: string,
  arrivalRelayUrl: string,
): Promise<void> {
  await invokeTauri("reconcile_inbound_persona_event", {
    eventJson,
    arrivalRelayUrl,
  });
}
