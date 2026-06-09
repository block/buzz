import { invokeTauri } from "@/shared/api/tauri";
import type {
  AgentPersona,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";

// Raw types matching Rust snake_case output
type RawParsedPersonaPreview = {
  display_name: string;
  system_prompt: string;
  avatar_data_url: string | null;
  runtime: string | null;
  model: string | null;
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
  runtime: string | null;
  model: string | null;
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

type RawPersona = {
  id: string;
  display_name: string;
  avatar_url: string | null;
  system_prompt: string;
  runtime?: string | null;
  model?: string | null;
  name_pool?: string[];
  is_builtin: boolean;
  is_active?: boolean;
  env_vars?: Record<string, string>;
  created_at: string;
  updated_at: string;
};

function fromRawPersona(persona: RawPersona): AgentPersona {
  return {
    id: persona.id,
    displayName: persona.display_name,
    avatarUrl: persona.avatar_url,
    systemPrompt: persona.system_prompt,
    runtime: persona.runtime ?? null,
    model: persona.model ?? null,
    namePool: persona.name_pool ?? [],
    isBuiltIn: persona.is_builtin,
    isActive: persona.is_active ?? true,
    envVars: persona.env_vars ?? {},
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
        namePool: input.namePool ?? [],
        envVars: input.envVars ?? {},
      },
    }),
  );
}

export async function updatePersona(
  input: UpdatePersonaInput,
): Promise<AgentPersona> {
  return fromRawPersona(
    await invokeTauri<RawPersona>("update_persona", {
      input: {
        id: input.id,
        displayName: input.displayName,
        avatarUrl: input.avatarUrl,
        systemPrompt: input.systemPrompt,
        runtime: input.runtime,
        model: input.model,
        namePool: input.namePool ?? [],
        // Send envVars only when caller explicitly provided it; omitting
        // tells the backend "don't touch the stored env vars" so editing
        // unrelated fields can't silently wipe saved credentials.
        envVars: input.envVars,
      },
    }),
  );
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
      avatarDataUrl: p.avatar_data_url,
      runtime: p.runtime,
      model: p.model,
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
