import { invokeTauri } from "@/shared/api/tauri";
import type {
  AgentPersona,
  CreatePersonaInput,
  RespondToMode,
  UpdatePersonaInput,
} from "@/shared/api/types";

// Raw types matching Rust snake_case output
type RawParsedPersonaPreview = {
  display_name: string;
  system_prompt: string;
  avatar_data_url: string | null;
  avatar_ref: string | null;
  runtime: string | null;
  model: string | null;
  provider: string | null;
  name_pool?: string[];
  source_file: string;
};

type RawSkippedFile = {
  source_file: string;
  reason: string;
};

type RawParsePersonaFilesResult = {
  personas: RawParsedPersonaPreview[];
  skipped: RawSkippedFile[];
};

// Public camelCase types
export type ParsedPersonaPreview = {
  displayName: string;
  systemPrompt: string;
  avatarDataUrl: string | null;
  avatarRef: string | null;
  runtime: string | null;
  model: string | null;
  provider: string | null;
  namePool: string[];
  sourceFile: string;
};

export type SkippedFile = {
  sourceFile: string;
  reason: string;
};

export type ParsePersonaFilesResult = {
  personas: ParsedPersonaPreview[];
  skipped: SkippedFile[];
};

export type RawPersona = {
  id: string;
  display_name: string;
  avatar_url: string | null;
  system_prompt: string;
  runtime?: string | null;
  model?: string | null;
  provider?: string | null;
  name_pool?: string[];
  is_builtin: boolean;
  is_active?: boolean;
  source_team?: string | null;
  env_vars?: Record<string, string>;
  respond_to?: string | null;
  respond_to_allowlist?: string[];
  mcp_toolsets?: string | null;
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
    systemPrompt: persona.system_prompt,
    runtime: persona.runtime ?? null,
    model: persona.model ?? null,
    provider: persona.provider ?? null,
    namePool: persona.name_pool ?? [],
    isBuiltIn: persona.is_builtin,
    isActive: persona.is_active ?? true,
    sourceTeam: persona.source_team ?? null,
    envVars: persona.env_vars ?? {},
    respondTo: (persona.respond_to as RespondToMode | undefined) ?? null,
    respondToAllowlist: persona.respond_to_allowlist ?? [],
    mcpToolsets: persona.mcp_toolsets ?? null,
    parallelism: persona.parallelism ?? null,
    createdAt: persona.created_at,
    updatedAt: persona.updated_at,
  };
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
        systemPrompt: input.systemPrompt,
        runtime: input.runtime,
        model: input.model,
        provider: input.provider,
        namePool: input.namePool ?? [],
        envVars: input.envVars ?? {},
        behavior: input.behavior,
      },
    }),
  );
}

export async function updatePersona(
  input: UpdatePersonaInput,
): Promise<AgentPersona> {
  const raw = await invokeTauri<RawPersona>("update_persona", {
    input: {
      id: input.id,
      displayName: input.displayName,
      avatarUrl: input.avatarUrl,
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
    },
  });
  if (raw.writeback_warning) {
    console.warn(
      `[updatePersona] pack write-back failed (edit saved locally): ${raw.writeback_warning}`,
    );
  }
  return fromRawPersona(raw);
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

export async function parsePersonaFiles(
  fileBytes: number[],
  fileName: string,
): Promise<ParsePersonaFilesResult> {
  const raw = await invokeTauri<RawParsePersonaFilesResult>(
    "parse_persona_files",
    { fileBytes, fileName },
  );
  return {
    personas: raw.personas.map((p) => ({
      displayName: p.display_name,
      systemPrompt: p.system_prompt,
      avatarDataUrl: p.avatar_data_url ?? null,
      avatarRef: p.avatar_ref ?? null,
      runtime: p.runtime,
      model: p.model,
      provider: p.provider,
      namePool: p.name_pool ?? [],
      sourceFile: p.source_file,
    })),
    skipped: raw.skipped.map((s) => ({
      sourceFile: s.source_file,
      reason: s.reason,
    })),
  };
}

export async function exportPersonaToJson(id: string): Promise<boolean> {
  return invokeTauri<boolean>("export_persona_to_json", { id });
}

export type SnapshotMemoryLevel = "none" | "core" | "everything";
export type SnapshotFormat = "json" | "png";

export async function exportAgentSnapshot(
  id: string,
  memoryLevel: SnapshotMemoryLevel,
  format: SnapshotFormat,
  memorySourcePubkey?: string | null,
): Promise<boolean> {
  return invokeTauri<boolean>("export_agent_snapshot", {
    id,
    memorySourcePubkey: memorySourcePubkey ?? null,
    memoryLevel,
    format,
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
): Promise<EncodedSnapshotPayload> {
  return invokeTauri<EncodedSnapshotPayload>("encode_agent_snapshot_for_send", {
    id,
    memorySourcePubkey: memorySourcePubkey ?? null,
    memoryLevel,
    format,
  });
}

// ── Snapshot import ───────────────────────────────────────────────────────────

/** Preview returned by `preview_agent_snapshot_import` before any write. */
export type AgentSnapshotImportPreview = {
  displayName: string;
  systemPrompt: string | null;
  /** Effective avatar: data URL if present, source URL fallback otherwise. */
  avatarUrl: string | null;
  /** "none" | "core" | "everything" */
  memoryLevel: string;
  memoryEntryCount: number;
  /** True when the snapshot's respond_to_allowlist is non-empty. */
  hasSourceAllowlist: boolean;
  sourceAllowlistCount: number;
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
// pending-edit race; the frontend only forwards the raw Nostr event JSON.
export async function reconcileInboundPersonaEvent(
  eventJson: string,
): Promise<void> {
  await invokeTauri("reconcile_inbound_persona_event", { eventJson });
}
