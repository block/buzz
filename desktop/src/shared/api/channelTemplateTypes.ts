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

export type TemplateMemberEntry = {
  pubkey: string;
  role: "admin" | "member" | "guest";
};

export type ChannelTemplate = {
  id: string;
  name: string;
  description: string | null;
  channelType: "stream" | "forum";
  visibility: "open" | "private";
  canvasTemplate: string | null;
  agents: {
    personas: TemplateAgentEntry[];
    teams: TemplateTeamEntry[];
  };
  members: TemplateMemberEntry[];
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
  agents?: {
    personas: TemplateAgentEntry[];
    teams: TemplateTeamEntry[];
  };
  members?: TemplateMemberEntry[];
};

export type UpdateChannelTemplateInput = {
  id: string;
  name: string;
  description?: string;
  channelType?: string;
  visibility?: string;
  canvasTemplate?: string;
  agents?: {
    personas: TemplateAgentEntry[];
    teams: TemplateTeamEntry[];
  };
  members?: TemplateMemberEntry[];
};
