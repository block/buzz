import { invokeTauri } from "@/shared/api/tauri";
import type {
  ChannelTemplate,
  CreateChannelTemplateInput,
  UpdateChannelTemplateInput,
} from "@/shared/api/types";

type RawChannelTemplate = {
  id: string;
  name: string;
  template_type?: "channel" | "section";
  description?: string | null;
  channel_type: string;
  visibility: string;
  canvas_template?: string | null;
  project_folders?: string[];
  project_folder?: string | null;
  worktree?: { location: string; baseBranch: string } | null;
  agents?: {
    personas?: Array<{
      personaId: string;
      runtime?: string | null;
      model?: string | null;
      role?: string | null;
      backend?: { type: "local" } | { type: "provider"; id: string } | null;
    }>;
    teams?: Array<{
      teamId: string;
      runtime?: string | null;
      model?: string | null;
      backend?: { type: "local" } | { type: "provider"; id: string } | null;
    }>;
  };
  is_builtin: boolean;
  created_at: string;
  updated_at: string;
};

function fromRawChannelTemplate(raw: RawChannelTemplate): ChannelTemplate {
  const projectFolders = Array.from(
    new Set(
      [
        ...(raw.project_folders ?? []),
        ...(raw.project_folder ? [raw.project_folder] : []),
      ]
        .map((folder) => folder.trim())
        .filter(Boolean),
    ),
  );

  return {
    id: raw.id,
    name: raw.name,
    templateType: raw.template_type ?? "channel",
    description: raw.description ?? null,
    channelType: raw.channel_type as "stream" | "forum",
    visibility: raw.visibility as "open" | "private",
    canvasTemplate: raw.canvas_template ?? null,
    projectFolders,
    projectFolder: projectFolders[0] ?? null,
    worktree: raw.worktree ?? null,
    agents: {
      personas: (raw.agents?.personas ?? []).map((p) => ({
        personaId: p.personaId,
        runtime: p.runtime ?? null,
        model: p.model ?? null,
        role: p.role ?? null,
        backend: p.backend ?? null,
      })),
      teams: (raw.agents?.teams ?? []).map((t) => ({
        teamId: t.teamId,
        runtime: t.runtime ?? null,
        model: t.model ?? null,
        backend: t.backend ?? null,
      })),
    },
    isBuiltin: raw.is_builtin,
    createdAt: raw.created_at,
    updatedAt: raw.updated_at,
  };
}

export async function listChannelTemplates(): Promise<ChannelTemplate[]> {
  return (
    await invokeTauri<RawChannelTemplate[]>("list_channel_templates")
  ).map(fromRawChannelTemplate);
}

export async function pickChannelTemplateProjectFolder(): Promise<string[]> {
  return invokeTauri<string[]>("pick_channel_template_project_folder");
}

export async function createChannelTemplate(
  input: CreateChannelTemplateInput,
): Promise<ChannelTemplate> {
  return fromRawChannelTemplate(
    await invokeTauri<RawChannelTemplate>("create_channel_template", {
      input: {
        name: input.name,
        templateType: input.templateType,
        description: input.description,
        channelType: input.channelType,
        visibility: input.visibility,
        canvasTemplate: input.canvasTemplate,
        projectFolders: input.projectFolders,
        projectFolder: input.projectFolder ?? input.projectFolders?.[0],
        worktree: input.worktree,
        agents: input.agents ?? { personas: [], teams: [] },
      },
    }),
  );
}

export async function updateChannelTemplate(
  input: UpdateChannelTemplateInput,
): Promise<ChannelTemplate> {
  return fromRawChannelTemplate(
    await invokeTauri<RawChannelTemplate>("update_channel_template", {
      input: {
        id: input.id,
        name: input.name,
        templateType: input.templateType,
        description: input.description,
        channelType: input.channelType,
        visibility: input.visibility,
        canvasTemplate: input.canvasTemplate,
        projectFolders: input.projectFolders,
        projectFolder: input.projectFolder ?? input.projectFolders?.[0],
        worktree: input.worktree,
        agents: input.agents ?? { personas: [], teams: [] },
      },
    }),
  );
}

export async function deleteChannelTemplate(id: string): Promise<void> {
  await invokeTauri("delete_channel_template", { id });
}

export async function duplicateChannelTemplate(
  id: string,
): Promise<ChannelTemplate> {
  return fromRawChannelTemplate(
    await invokeTauri<RawChannelTemplate>("duplicate_channel_template", {
      id,
    }),
  );
}
