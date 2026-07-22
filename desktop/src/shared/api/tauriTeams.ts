import { invokeTauri } from "@/shared/api/tauri";
import type {
  AgentTeam,
  CreateTeamInput,
  ManagedAgentBackend,
  UpdateTeamInput,
} from "@/shared/api/types";

type RawTeam = {
  id: string;
  name: string;
  description: string | null;
  instructions?: string | null;
  persona_ids: string[];
  is_builtin?: boolean;
  source_dir?: string | null;
  is_symlink?: boolean;
  symlink_target?: string | null;
  version?: string | null;
  created_at: string;
  updated_at: string;
};

function fromRawTeam(team: RawTeam): AgentTeam {
  return {
    id: team.id,
    name: team.name,
    description: team.description,
    instructions: team.instructions ?? null,
    personaIds: team.persona_ids,
    isBuiltin: team.is_builtin ?? false,
    sourceDir: team.source_dir ?? null,
    isSymlink: team.is_symlink ?? false,
    symlinkTarget: team.symlink_target ?? null,
    version: team.version ?? null,
    createdAt: team.created_at,
    updatedAt: team.updated_at,
  };
}

export async function listTeams(): Promise<AgentTeam[]> {
  return (await invokeTauri<RawTeam[]>("list_teams")).map(fromRawTeam);
}

export async function createTeam(input: CreateTeamInput): Promise<AgentTeam> {
  return fromRawTeam(
    await invokeTauri<RawTeam>("create_team", {
      input: {
        name: input.name,
        description: input.description,
        instructions: input.instructions,
        personaIds: input.personaIds,
      },
    }),
  );
}

export async function updateTeam(input: UpdateTeamInput): Promise<AgentTeam> {
  return fromRawTeam(
    await invokeTauri<RawTeam>("update_team", {
      input: {
        id: input.id,
        name: input.name,
        description: input.description,
        instructions: input.instructions,
        personaIds: input.personaIds,
      },
    }),
  );
}

export async function deleteTeam(id: string): Promise<void> {
  await invokeTauri("delete_team", { id });
}

// ── Team snapshot types ─────────────────────────────────────────────────────

export type SnapshotFormat = "json" | "png";
export type SnapshotMemoryLevel = "none" | "core" | "everything";

export type EncodedTeamSnapshotPayload = {
  fileBytes: number[];
  fileName: string;
};

export type TeamSnapshotMemberPreview = {
  displayName: string;
  systemPrompt: string | null;
  avatarUrl: string | null;
  hasSourceAllowlist: boolean;
  sourceAllowlistCount: number;
};

export type TeamSnapshotImportPreview = {
  name: string;
  description: string | null;
  instructions: string | null;
  members: TeamSnapshotMemberPreview[];
  hasSourceAllowlist: boolean;
};

export type TeamSnapshotImportConfirm = {
  fileBytes: number[];
  keepAllowlist: boolean;
  backend?: ManagedAgentBackend;
};

export type TeamSnapshotImportMemberResult = {
  displayName: string;
  pubkey: string;
  personaId: string;
  memoryWritten: number;
  memoryTotal: number;
  memoryErrors: string[];
  profileSyncError: string | null;
};

/** Wire shape of the nested `TeamRecord` — Rust has no `rename_all` so fields
 *  arrive in snake_case, matching the existing `RawTeam` convention. */
type RawTeamRecord = {
  id: string;
  name: string;
  description: string | null;
  persona_ids: string[];
  instructions: string | null;
  is_builtin: boolean;
  source_dir: string | null;
  is_symlink: boolean;
  symlink_target: string | null;
  version: string | null;
  created_at: string;
  updated_at: string;
};

/** Raw wire shape of the import result — outer struct is camelCase,
 *  but the nested `team` field is snake_case (no `rename_all` on TeamRecord). */
type RawTeamSnapshotImportResult = {
  team: RawTeamRecord;
  personaIds: string[];
  members: TeamSnapshotImportMemberResult[];
};

export type TeamSnapshotImportResult = {
  team: AgentTeam;
  personaIds: string[];
  members: TeamSnapshotImportMemberResult[];
};

// ── Team snapshot commands ───────────────────────────────────────────────────

export async function exportTeamSnapshot(
  id: string,
  memoryLevel: SnapshotMemoryLevel,
  format: SnapshotFormat,
): Promise<boolean> {
  return invokeTauri<boolean>("export_team_snapshot", {
    id,
    memoryLevel,
    format,
  });
}

export async function encodeTeamSnapshotForSend(
  id: string,
  memoryLevel: SnapshotMemoryLevel,
  format: SnapshotFormat,
): Promise<EncodedTeamSnapshotPayload> {
  return invokeTauri<EncodedTeamSnapshotPayload>(
    "encode_team_snapshot_for_send",
    {
      id,
      memoryLevel,
      format,
    },
  );
}

export async function previewTeamSnapshotImport(
  fileBytes: number[],
  fileName: string,
): Promise<TeamSnapshotImportPreview> {
  return invokeTauri<TeamSnapshotImportPreview>(
    "preview_team_snapshot_import",
    {
      fileBytes,
      fileName,
    },
  );
}

export async function confirmTeamSnapshotImport(
  input: TeamSnapshotImportConfirm,
): Promise<TeamSnapshotImportResult> {
  const raw = await invokeTauri<RawTeamSnapshotImportResult>(
    "confirm_team_snapshot_import",
    { input },
  );
  return {
    team: fromRawTeam(raw.team),
    personaIds: raw.personaIds,
    members: raw.members,
  };
}
