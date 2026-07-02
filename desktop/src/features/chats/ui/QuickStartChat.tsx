import * as React from "react";
import {
  Check,
  ChevronDown,
  Folder,
  FolderPlus,
  FolderX,
  Search,
} from "lucide-react";
import { toast } from "sonner";

import { useChannelTemplatesQuery } from "@/features/channel-templates/hooks";
import { useApplyTemplate } from "@/features/channel-templates/useApplyTemplate";
import { ChatHeader } from "@/features/chat/ui/ChatHeader";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  useCreateChatMutation,
  useSendChatContextMessageMutation,
  useUpdateChatMetadataMutation,
} from "@/features/chats/hooks";
import {
  buildProjectSetupContext,
  deriveChatTitle,
  type ChatProject,
  NO_PROJECT_SELECTION_ID,
  uniqueMentionPubkeys,
} from "@/features/chats/lib/chatSetup";
import { ChatProjectDialog } from "@/features/chats/ui/ChatProjectDialog";
import { splitOutgoingTags } from "@/features/messages/lib/imetaMediaMarkdown";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { ensureWelcomeGuideAgentInChannel } from "@/features/onboarding/welcomeGuide";
import { useIdentityQuery } from "@/shared/api/hooks";
import { addChannelMembers, sendChannelMessage } from "@/shared/api/tauri";
import type { Channel, ChannelTemplate } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";

type QuickStartChatProps = {
  initialProjectId?: string | null;
  onCreated: (chat: Channel) => void;
  onProjectCreated?: (project: ChatProject) => void;
  projects: ChatProject[];
  relayUrl?: string | null;
};

function errorMessage(error: unknown) {
  if (error instanceof Error && error.message.trim()) {
    return error.message.trim();
  }
  if (typeof error === "string" && error.trim()) {
    return error.trim();
  }
  return "Unknown error";
}

function nonAgentMentionPubkeys({
  defaultAgentPubkey,
  identityPubkey,
  managedAgentPubkeys,
  mentionPubkeys,
}: {
  defaultAgentPubkey?: string | null;
  identityPubkey?: string | null;
  managedAgentPubkeys: string[];
  mentionPubkeys: string[];
}) {
  const blockedPubkeys = new Set(
    [identityPubkey, defaultAgentPubkey, ...managedAgentPubkeys]
      .map((pubkey) => normalizePubkey(pubkey ?? ""))
      .filter(Boolean),
  );
  return [
    ...new Set(mentionPubkeys.map((pubkey) => normalizePubkey(pubkey))),
  ].filter((pubkey) => pubkey && !blockedPubkeys.has(pubkey));
}

export function QuickStartChat({
  initialProjectId,
  onCreated,
  onProjectCreated,
  projects,
  relayUrl,
}: QuickStartChatProps) {
  const [selectedProjectId, setSelectedProjectId] = React.useState<
    string | null
  >(() => initialProjectSelection(initialProjectId, projects));
  const [isCreating, setIsCreating] = React.useState(false);
  const identityQuery = useIdentityQuery();
  const createChatMutation = useCreateChatMutation();
  const updateMetadataMutation = useUpdateChatMetadataMutation();
  const sendContextMutation = useSendChatContextMessageMutation();
  const templatesQuery = useChannelTemplatesQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const { applyAgents, applyCanvas } = useApplyTemplate();
  const templates = templatesQuery.data ?? [];
  const allProjects = projects;
  const selectedProject =
    allProjects.find((project) => project.id === selectedProjectId) ?? null;
  const selectedTemplate =
    templates.find((template) => template.id === selectedProject?.templateId) ??
    null;

  React.useEffect(() => {
    if (initialProjectId === undefined) {
      return;
    }
    const nextSelection = initialProjectSelection(
      initialProjectId,
      allProjects,
    );
    setSelectedProjectId(nextSelection);
  }, [allProjects, initialProjectId]);

  React.useEffect(() => {
    if (
      selectedProjectId !== null ||
      initialProjectId !== undefined ||
      allProjects.length === 0
    ) {
      return;
    }
    setSelectedProjectId(allProjects[0].id);
  }, [allProjects, initialProjectId, selectedProjectId]);

  const handleCreateProject = React.useCallback(
    (project: ChatProject) => {
      onProjectCreated?.(project);
      setSelectedProjectId(project.id);
    },
    [onProjectCreated],
  );

  const handleCreate = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
    ) => {
      const trimmed = content.trim();
      if (!trimmed && !mediaTags?.length) {
        return;
      }
      if (isCreating) {
        return;
      }

      setIsCreating(true);
      try {
        const title = deriveChatTitle(trimmed);
        const templateId = selectedProject?.templateId ?? undefined;
        const chat = await createChatMutation.mutateAsync({
          title,
          templateId,
          projectId: selectedProject?.id,
          projectName: selectedProject?.name,
          projectPath: selectedProject?.path ?? undefined,
          projectTemplateId: selectedProject?.templateId ?? undefined,
        });

        const projectCanvasContext = buildProjectSetupContext({
          project: selectedProject,
          templateName: selectedTemplate?.name,
        });
        await applyCanvas(templateId, chat.id, title, projectCanvasContext);
        void applyAgents(templateId, chat.id);

        const agent = await ensureWelcomeGuideAgentInChannel(chat.id, relayUrl);
        const setupContext = buildProjectSetupContext({
          agent,
          project: selectedProject,
          templateName: selectedTemplate?.name,
        });

        await updateMetadataMutation.mutateAsync({
          channelId: chat.id,
          title,
          defaultAgentPubkey: agent.pubkey,
          templateId,
          projectId: selectedProject?.id,
          projectName: selectedProject?.name,
          projectPath: selectedProject?.path ?? undefined,
          projectTemplateId: selectedProject?.templateId ?? undefined,
        });

        if (setupContext) {
          await sendContextMutation.mutateAsync({
            channelId: chat.id,
            content: setupContext,
          });
        }

        const memberMentionPubkeys = nonAgentMentionPubkeys({
          defaultAgentPubkey: agent.pubkey,
          identityPubkey: identityQuery.data?.pubkey,
          managedAgentPubkeys:
            managedAgentsQuery.data?.map(
              (managedAgent) => managedAgent.pubkey,
            ) ?? [],
          mentionPubkeys,
        });
        if (memberMentionPubkeys.length > 0) {
          const result = await addChannelMembers({
            channelId: chat.id,
            pubkeys: memberMentionPubkeys,
            role: "member",
          });
          if (result.errors.length > 0) {
            throw new Error(
              `Could not share chat: ${result.errors
                .map((memberError) => memberError.error)
                .join("; ")}`,
            );
          }
        }

        const {
          mediaTags: imetaTags,
          emojiTags,
          mentionTags,
        } = splitOutgoingTags(mediaTags);
        await sendChannelMessage(
          chat.id,
          content,
          null,
          imetaTags,
          uniqueMentionPubkeys(
            identityQuery.data?.pubkey,
            mentionPubkeys,
            agent.pubkey,
          ),
          undefined,
          emojiTags,
          mentionTags,
        );
        if (selectedProject) {
          onProjectCreated?.({
            ...selectedProject,
            chatCount: selectedProject.chatCount + 1,
            updatedAt: Math.floor(Date.now() / 1_000),
          });
        }
        onCreated(chat);
      } catch (error) {
        console.error("Failed to create chat", error);
        toast.error("Could not start chat", {
          description: errorMessage(error),
        });
        throw error;
      } finally {
        setIsCreating(false);
      }
    },
    [
      applyAgents,
      applyCanvas,
      createChatMutation,
      identityQuery.data?.pubkey,
      isCreating,
      managedAgentsQuery.data,
      onCreated,
      onProjectCreated,
      relayUrl,
      selectedProject,
      selectedTemplate?.name,
      sendContextMutation,
      updateMetadataMutation,
    ],
  );

  const projectPicker = (
    <ProjectPicker
      onCreateProject={handleCreateProject}
      onSelectProject={setSelectedProjectId}
      isNoProjectSelected={selectedProjectId === NO_PROJECT_SELECTION_ID}
      projects={allProjects}
      selectedProject={selectedProject}
      templates={templates}
    />
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-background">
      <ChatHeader
        description="Describe a task or ask a question."
        mode="chats"
        title="New chat"
        transparentChrome
      />

      <div className="mx-auto flex min-h-0 w-full max-w-4xl flex-1 items-center px-4 py-6 sm:px-6 lg:px-8">
        <div className="w-full text-center">
          <h2 className="text-xl font-semibold tracking-tight">Start a chat</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Describe a task or ask a question.
          </p>
        </div>
      </div>

      <div className="shrink-0 bg-background">
        <MessageComposer
          autoInviteNonMemberMentions
          channelId={null}
          channelName="Chat"
          channelType="chat"
          containerClassName="mx-auto w-full max-w-4xl px-4 pb-3 sm:px-6 lg:px-8"
          disabled={isCreating}
          draftKey="chat:new"
          isSending={isCreating}
          onSend={handleCreate}
          placeholder="Message..."
          toolbarControls={{ emoji: false, formatting: false, spoiler: false }}
          toolbarExtraActions={projectPicker}
        />
      </div>
    </div>
  );
}

type SetupPillProps = React.ComponentPropsWithoutRef<typeof Button> & {
  testId?: string;
};

const SetupPill = React.forwardRef<HTMLButtonElement, SetupPillProps>(
  function SetupPill({ children, className, testId, ...props }, ref) {
    return (
      <Button
        className={cn(
          "h-9 gap-2 rounded-lg px-3 text-sm font-medium text-muted-foreground hover:bg-muted/60 hover:text-foreground",
          className,
        )}
        data-testid={testId}
        ref={ref}
        type="button"
        variant="ghost"
        {...props}
      >
        {children}
        <ChevronDown className="h-4 w-4 text-muted-foreground/70" />
      </Button>
    );
  },
);

export function ProjectPicker({
  isNoProjectSelected,
  onCreateProject,
  onSelectProject,
  projects,
  selectedProject,
  templates,
}: {
  isNoProjectSelected: boolean;
  onCreateProject: (project: ChatProject) => void;
  onSelectProject: (projectId: string | null) => void;
  projects: ChatProject[];
  selectedProject: ChatProject | null;
  templates: ChannelTemplate[];
}) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [isCreateOpen, setIsCreateOpen] = React.useState(false);
  const filteredProjects = React.useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    if (!normalizedQuery) {
      return projects;
    }
    return projects.filter((project) =>
      [project.name, project.path ?? ""]
        .join(" ")
        .toLowerCase()
        .includes(normalizedQuery),
    );
  }, [projects, query]);

  return (
    <>
      <Popover onOpenChange={setOpen} open={open}>
        <PopoverTrigger asChild>
          <SetupPill className="max-w-64" testId="chat-project-picker">
            <Folder className="h-4 w-4 shrink-0" />
            <span className="truncate">
              {selectedProject?.name ||
                (isNoProjectSelected ? "No project" : "Project")}
            </span>
          </SetupPill>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-80 p-2">
          <div className="relative mb-2">
            <Search className="absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              className="h-9 pl-8"
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search projects"
              value={query}
            />
          </div>
          <div className="max-h-64 overflow-y-auto">
            {filteredProjects.length > 0 ? (
              filteredProjects.map((project) => (
                <ProjectPickerRow
                  checked={selectedProject?.id === project.id}
                  key={project.id}
                  onSelect={() => {
                    onSelectProject(project.id);
                    setOpen(false);
                  }}
                  project={project}
                />
              ))
            ) : (
              <div className="px-2 py-3 text-sm text-muted-foreground">
                {projects.length === 0
                  ? "No projects yet"
                  : "No projects found"}
              </div>
            )}
          </div>
          <div className="my-2 border-t border-border/60" />
          <ProjectModeRow
            icon={<FolderPlus className="h-4 w-4" />}
            label="New project"
            onSelect={() => {
              setIsCreateOpen(true);
              setOpen(false);
            }}
          />
          <ProjectModeRow
            checked={isNoProjectSelected}
            icon={<FolderX className="h-4 w-4" />}
            label="No project"
            onSelect={() => {
              onSelectProject(NO_PROJECT_SELECTION_ID);
              setOpen(false);
            }}
          />
        </PopoverContent>
      </Popover>
      <ChatProjectDialog
        onSaveProject={(project) => {
          onCreateProject(project);
          void onSelectProject(project.id);
        }}
        onOpenChange={setIsCreateOpen}
        open={isCreateOpen}
        templates={templates}
      />
    </>
  );
}

function ProjectPickerRow({
  checked,
  onSelect,
  project,
}: {
  checked: boolean;
  onSelect: () => void;
  project: ChatProject;
}) {
  return (
    <button
      className="flex w-full items-center gap-3 rounded-lg px-2 py-2 text-left text-sm outline-hidden transition-colors hover:bg-muted/60 focus-visible:bg-muted/60"
      onClick={onSelect}
      type="button"
    >
      <Folder className="h-4 w-4 shrink-0 text-muted-foreground" />
      <span className="min-w-0 flex-1">
        <span className="block truncate font-medium">{project.name}</span>
        {project.path ? (
          <span className="block truncate text-xs text-muted-foreground">
            {project.path}
          </span>
        ) : null}
      </span>
      {checked ? <Check className="h-4 w-4 shrink-0" /> : null}
    </button>
  );
}

function ProjectModeRow({
  checked,
  icon,
  label,
  onSelect,
}: {
  checked?: boolean;
  icon: React.ReactNode;
  label: string;
  onSelect: () => void;
}) {
  return (
    <button
      className="flex w-full items-center gap-3 rounded-lg px-2 py-2 text-left text-sm outline-hidden transition-colors hover:bg-muted/60 focus-visible:bg-muted/60"
      onClick={onSelect}
      type="button"
    >
      {icon}
      <span className="min-w-0 flex-1 truncate font-medium">{label}</span>
      {checked ? <Check className="h-4 w-4 shrink-0" /> : null}
    </button>
  );
}

function initialProjectSelection(
  initialProjectId: string | null | undefined,
  projects: ChatProject[],
) {
  if (initialProjectId === null) {
    return NO_PROJECT_SELECTION_ID;
  }
  if (initialProjectId) {
    return initialProjectId;
  }
  return projects[0]?.id ?? null;
}
