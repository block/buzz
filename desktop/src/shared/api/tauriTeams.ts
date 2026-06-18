import { invokeTauri } from "@/shared/api/tauri";
import type {
  AgentTeam,
  CreateTeamInput,
  UpdateTeamInput,
} from "@/shared/api/types";

type RawTeam = {
  id: string;
  name: string;
  description: string | null;
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
        personaIds: input.personaIds,
      },
    }),
  );
}

export async function deleteTeam(id: string): Promise<void> {
  await invokeTauri("delete_team", { id });
}

export type ParsedTeamPreview = {
  name: string;
  description: string | null;
  personas: Array<{
    display_name: string;
    system_prompt: string;
    avatar_url: string | null;
  }>;
};

export async function exportTeamToJson(id: string): Promise<boolean> {
  return invokeTauri<boolean>("export_team_to_json", { id });
}

export async function parseTeamFile(
  fileBytes: number[],
  fileName: string,
): Promise<ParsedTeamPreview> {
  return invokeTauri<ParsedTeamPreview>("parse_team_file", {
    fileBytes,
    fileName,
  });
}

export type SyncResult = {
  personas_added: string[];
  personas_removed: string[];
  personas_updated: string[];
  metadata_changed: boolean;
};

export async function pickTeamDirectory(): Promise<string | null> {
  return invokeTauri<string | null>("pick_team_directory");
}

export async function installTeamFromDirectory(
  path: string,
  symlink?: boolean,
): Promise<AgentTeam> {
  return fromRawTeam(
    await invokeTauri<RawTeam>("install_team_from_directory", {
      path,
      symlink: symlink ?? false,
    }),
  );
}

export async function syncTeamDirectory(teamId: string): Promise<SyncResult> {
  return invokeTauri<SyncResult>("sync_team_directory", { teamId });
}
