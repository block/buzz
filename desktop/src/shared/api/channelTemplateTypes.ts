export type TemplateBackend =
  | { type: "local" }
  | { type: "provider"; id: string };

export type TemplateAgentEntry = {
  personaId: string;
  runtime: string | null;
  model: string | null;
  role: string | null;
  backend: TemplateBackend | null;
};

export type TemplateTeamEntry = {
  teamId: string;
  runtime: string | null;
  model: string | null;
  backend: TemplateBackend | null;
};

export type TemplateWorktreeConfig = {
  /** Directory where new worktrees are created. */
  location: string;
  /** Branch used as the base for each new worktree. */
  baseBranch: string;
};
export type TemplateAgents = {
  personas: TemplateAgentEntry[];
  teams: TemplateTeamEntry[];
};

export type ChannelTemplate = {
  id: string;
  name: string;
  description: string | null;
  channelType: "stream" | "forum";
  visibility: "open" | "private";
  canvasTemplate: string | null;
  projectFolders: string[];
  /** Legacy first-folder alias retained while older saved templates migrate. */
  projectFolder: string | null;
  worktree: TemplateWorktreeConfig | null;
  agents: TemplateAgents;
  isBuiltin: boolean;
  createdAt: string;
  updatedAt: string;
};

export type CreateChannelTemplateInput = {
  name: string;
  description?: string;
  channelType?: string;
  visibility?: string;
  canvasTemplate?: string;
  projectFolders?: string[];
  /** Legacy single-folder input accepted for backward compatibility. */
  projectFolder?: string;
  worktree?: TemplateWorktreeConfig;
  agents?: TemplateAgents;
};

export type UpdateChannelTemplateInput = {
  id: string;
  name: string;
  description?: string;
  channelType?: string;
  visibility?: string;
  canvasTemplate?: string;
  projectFolders?: string[];
  /** Legacy single-folder input accepted for backward compatibility. */
  projectFolder?: string;
  worktree?: TemplateWorktreeConfig;
  agents?: TemplateAgents;
};
