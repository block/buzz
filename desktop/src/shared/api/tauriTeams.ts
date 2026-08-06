import { invokeTauri } from "@/shared/api/tauri";
import type {
  AgentTeam,
  CreateTeamInput,
  TeamCatalogSourceCoordinate,
  UpdateTeamInput,
} from "@/shared/api/types";

/** Wire shape of `TeamCatalogSource` — snake_case, like its parent record. */
type RawTeamCatalogSource = {
  owner_pubkey: string;
  team_d_tag: string;
};

type RawTeam = {
  id: string;
  name: string;
  description: string | null;
  instructions?: string | null;
  persona_ids: string[];
  is_builtin?: boolean;
  shared?: boolean;
  catalog_source?: RawTeamCatalogSource | null;
  source_dir?: string | null;
  is_symlink?: boolean;
  symlink_target?: string | null;
  version?: string | null;
  created_at: string;
  updated_at: string;
};

function fromRawCatalogSource(
  source: RawTeamCatalogSource | null | undefined,
): TeamCatalogSourceCoordinate | null {
  return source
    ? { ownerPubkey: source.owner_pubkey, teamDTag: source.team_d_tag }
    : null;
}

function fromRawTeam(team: RawTeam): AgentTeam {
  return {
    id: team.id,
    name: team.name,
    description: team.description,
    instructions: team.instructions ?? null,
    personaIds: team.persona_ids,
    isBuiltin: team.is_builtin ?? false,
    shared: team.shared ?? false,
    catalogSource: fromRawCatalogSource(team.catalog_source),
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

// ── Team catalog commands ────────────────────────────────────────────────────

export type TeamSharePublicationResult = {
  team: AgentTeam;
  /** `queued` means the head is durably enqueued but the relay has not yet
   *  accepted it, so catalog visibility lags the toggle. */
  publicationStatus: "published" | "queued";
  relayMessage: string | null;
};

type RawTeamSharePublicationResult = {
  team: RawTeam;
  publicationStatus: "published" | "queued";
  relayMessage?: string;
};

/** Publish this team's catalog head, or replace it with an untagged one. */
export async function setTeamShared(
  id: string,
  shared: boolean,
): Promise<TeamSharePublicationResult> {
  const raw = await invokeTauri<RawTeamSharePublicationResult>(
    "set_team_shared",
    { id, shared },
  );
  return {
    team: fromRawTeam(raw.team),
    publicationStatus: raw.publicationStatus,
    relayMessage: raw.relayMessage ?? null,
  };
}

export type AddTeamFromCatalogResult = {
  team: AgentTeam;
  /** True when the team was already added and nothing was written. */
  alreadyPresent: boolean;
};

type RawAddTeamFromCatalogResult = {
  team: RawTeam;
  alreadyPresent: boolean;
};

/**
 * Copy a published team into the local stores.
 *
 * Only the coordinate crosses the boundary — never the projection the UI is
 * displaying. The backend re-fetches the current head at
 * `30178:<owner>:<d-tag>` and rejects the add if it is not `eventId` or has
 * stopped being shared, so a catalog entry that moved or was retracted while
 * the dialog sat open cannot be copied.
 */
export async function addTeamFromCatalog(
  source: TeamCatalogSourceCoordinate & { eventId: string },
): Promise<AddTeamFromCatalogResult> {
  const raw = await invokeTauri<RawAddTeamFromCatalogResult>(
    "add_team_from_catalog",
    {
      input: {
        ownerPubkey: source.ownerPubkey,
        teamDTag: source.teamDTag,
        eventId: source.eventId,
      },
    },
  );
  return { team: fromRawTeam(raw.team), alreadyPresent: raw.alreadyPresent };
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

/** Raw wire shape of the import result — outer struct is camelCase,
 *  but the nested `team` field is snake_case (no `rename_all` on TeamRecord). */
type RawTeamSnapshotImportResult = {
  team: RawTeam;
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
