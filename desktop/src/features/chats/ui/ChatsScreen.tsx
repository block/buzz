// @ts-nocheck
import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  ChevronDown,
  ChevronRight,
  Folder,
  MoreVertical,
  Plus,
} from "lucide-react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useAppShell } from "@/app/AppShellContext";
import { useActiveAgentTurnsByChannel } from "@/features/agents/activeAgentTurnsStore";
import {
  useManagedAgentsQuery,
  useStartManagedAgentMutation,
} from "@/features/agents/hooks";
import { useChannelTemplatesQuery } from "@/features/channel-templates/hooks";
import {
  useArchiveChatMutation,
  useChatMetadataListQuery,
  useChatMetadataQuery,
  useChatsQuery,
  useUpdateChatMetadataMutation,
} from "@/features/chats/hooks";
import { buildChatProjects } from "@/features/chats/lib/chatProjects";
import {
  mergeChatProjects,
  upsertStoredChatProject,
  useStoredChatProjects,
} from "@/features/chats/lib/chatProjectStorage";
import {
  buildChatCanvasContent,
  buildProjectSetupContext,
  uniqueMentionPubkeys,
} from "@/features/chats/lib/chatSetup";
import { ChatDetail } from "@/features/chats/ui/ChatDetail";
import { ChatHeaderActions } from "@/features/chats/ui/ChatHeaderActions";
import { ChatListHeader, ChatListItem } from "@/features/chats/ui/ChatListItem";
import { ChatListSectionHeader } from "@/features/chats/ui/ChatListSectionHeader";
import { ChatListSkeleton } from "@/features/chats/ui/ChatListSkeleton";
import { ChatProjectDialog } from "@/features/chats/ui/ChatProjectDialog";
import { QuickStartChat } from "@/features/chats/ui/QuickStartChat";
import {
  useChannelMessagesQuery,
  useChannelSubscription,
  useSendMessageMutation,
} from "@/features/messages/hooks";
import { ensureWelcomeGuideAgentInChannel } from "@/features/onboarding/welcomeGuide";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";
import { useIdentityQuery } from "@/shared/api/hooks";
import { addChannelMembers, getCanvas, setCanvas } from "@/shared/api/tauri";
import type {
  Channel,
  ChannelTemplate,
  ChatMetadata,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

type ChatsScreenProps = {
  initialProjectId?: string | null;
  selectedChatId?: string | null;
};

async function backfillBlankChatCanvas(channelId: string, content: string) {
  const existing = await getCanvas(channelId);
  if (existing.content?.trim()) {
    return false;
  }
  await setCanvas({ channelId, content });
  return true;
}

async function addBotToChat(channelId: string, pubkey: string) {
  const result = await addChannelMembers({
    channelId,
    pubkeys: [pubkey],
    role: "bot",
  });
  const error = result.errors.find(
    (entry) => normalizePubkey(entry.pubkey) === normalizePubkey(pubkey),
  );
  if (error && !error.error.toLowerCase().includes("already")) {
    throw new Error(error.error);
  }
}

function isSharedChatMetadata(
  metadata: ChatMetadata | null | undefined,
  identityPubkey: string | null | undefined,
) {
  if (!metadata?.authorPubkey || !identityPubkey) {
    return false;
  }
  return (
    normalizePubkey(metadata.authorPubkey) !== normalizePubkey(identityPubkey)
  );
}

export function ChatsScreen({
  initialProjectId,
  selectedChatId = null,
}: ChatsScreenProps) {
  const queryClient = useQueryClient();
  const { activeWorkspace } = useWorkspaces();
  const { goChat, goChats } = useAppNavigation();
  const {
    getChannelReadAt,
    markChannelRead,
    readStateVersion,
    unreadChannelCounts,
    unreadChannelIds,
  } = useAppShell();
  const identityQuery = useIdentityQuery();
  const chatsQuery = useChatsQuery();
  const chats = chatsQuery.data ?? [];
  const metadataListQuery = useChatMetadataListQuery();
  const allMetadata = metadataListQuery.data ?? [];
  const storedChatProjects = useStoredChatProjects(activeWorkspace?.id);
  const templatesQuery = useChannelTemplatesQuery();
  const templates = templatesQuery.data ?? [];
  const identityPubkey = identityQuery.data?.pubkey;
  const ownedMetadata = React.useMemo(
    () =>
      allMetadata.filter(
        (metadata) => !isSharedChatMetadata(metadata, identityPubkey),
      ),
    [allMetadata, identityPubkey],
  );
  const chatProjects = React.useMemo(
    () =>
      mergeChatProjects(storedChatProjects, buildChatProjects(ownedMetadata)),
    [ownedMetadata, storedChatProjects],
  );
  const backfilledCanvasKeysRef = React.useRef(new Set<string>());
  React.useEffect(() => {
    if (
      chatsQuery.isLoading ||
      metadataListQuery.isLoading ||
      templatesQuery.isLoading
    ) {
      return;
    }

    const chatsById = new Map(chats.map((chat) => [chat.id, chat]));
    const templatesById = new Map(
      templates.map((template) => [template.id, template]),
    );

    for (const metadata of ownedMetadata) {
      const projectId = metadata.projectId?.trim();
      const projectName = metadata.projectName?.trim();
      if (!projectId || !projectName) {
        continue;
      }

      const chat = chatsById.get(metadata.channelId);
      if (!chat) {
        continue;
      }

      const templateId =
        metadata.projectTemplateId?.trim() || metadata.templateId?.trim() || "";
      const template = templateId ? templatesById.get(templateId) : null;
      const project = {
        id: projectId,
        name: projectName,
        path: metadata.projectPath?.trim() || null,
        templateId: templateId || null,
        updatedAt: metadata.updatedAt,
        chatCount: 1,
      };
      const leadingContent = buildProjectSetupContext({
        project,
        templateName: template?.name ?? null,
      });
      const content = buildChatCanvasContent({
        channelName: metadata.title?.trim() || chat.name,
        leadingContent,
        template,
      });
      if (!content) {
        continue;
      }

      const backfillKey = [
        chat.id,
        project.id,
        project.path ?? "",
        template?.id ?? "",
        template?.updatedAt ?? "",
        content.length,
      ].join(":");
      if (backfilledCanvasKeysRef.current.has(backfillKey)) {
        continue;
      }
      backfilledCanvasKeysRef.current.add(backfillKey);

      void backfillBlankChatCanvas(chat.id, content)
        .then((didBackfill) => {
          if (didBackfill) {
            void queryClient.invalidateQueries({
              queryKey: ["channel-canvas", chat.id],
            });
          }
        })
        .catch((error) => {
          console.warn("Failed to backfill chat canvas", chat.id, error);
        });
    }
  }, [
    chats,
    chatsQuery.isLoading,
    metadataListQuery.isLoading,
    ownedMetadata,
    queryClient,
    templates,
    templatesQuery.isLoading,
  ]);
  const metadataByChatId = React.useMemo(
    () =>
      new Map(allMetadata.map((metadata) => [metadata.channelId, metadata])),
    [allMetadata],
  );
  const selectedChat =
    selectedChatId !== null
      ? (chats.find((chat) => chat.id === selectedChatId) ?? null)
      : null;
  const metadataQuery = useChatMetadataQuery(selectedChat?.id);
  const metadata = metadataQuery.data ?? null;
  const managedAgentsQuery = useManagedAgentsQuery();
  const metadataDefaultAgentPubkey = metadata?.defaultAgentPubkey ?? null;
  const defaultAgent = React.useMemo(() => {
    if (!metadataDefaultAgentPubkey) {
      return null;
    }
    const normalizedDefaultAgentPubkey = normalizePubkey(
      metadataDefaultAgentPubkey,
    );
    return (
      (managedAgentsQuery.data ?? []).find(
        (agent) =>
          normalizePubkey(agent.pubkey) === normalizedDefaultAgentPubkey,
      ) ?? null
    );
  }, [managedAgentsQuery.data, metadataDefaultAgentPubkey]);

  const messageQuery = useChannelMessagesQuery(selectedChat);
  useChannelSubscription(selectedChat);
  const messages = messageQuery.data ?? [];
  const pubkeys = React.useMemo(
    () => [
      ...new Set(
        [
          identityQuery.data?.pubkey,
          defaultAgent?.pubkey,
          ...messages.map((message) => message.pubkey),
        ]
          .filter((value): value is string => Boolean(value))
          .map((value) => value.toLowerCase()),
      ),
    ],
    [defaultAgent?.pubkey, identityQuery.data?.pubkey, messages],
  );
  const profilesQuery = useUsersBatchQuery(pubkeys, {
    enabled: pubkeys.length > 0,
  });
  const profiles = profilesQuery.data?.profiles;
  const sendMessageMutation = useSendMessageMutation(
    selectedChat,
    identityQuery.data,
  );
  const archiveChatMutation = useArchiveChatMutation();
  const startManagedAgentMutation = useStartManagedAgentMutation();
  const [isEnsuringDefaultAgent, setIsEnsuringDefaultAgent] =
    React.useState(false);
  const handleArchiveChat = React.useCallback(
    async (chatId: string) => {
      try {
        await archiveChatMutation.mutateAsync(chatId);
        toast.success("Chat archived");
        if (selectedChatId === chatId) {
          void goChats({ replace: true });
        }
      } catch (error) {
        toast.error("Could not archive chat", {
          description: error instanceof Error ? error.message : undefined,
        });
      }
    },
    [archiveChatMutation, goChats, selectedChatId],
  );

  const updateMetadataMutation = useUpdateChatMetadataMutation();
  const ensuredChatIdsRef = React.useRef(new Set<string>());
  React.useEffect(() => {
    if (!selectedChat || metadataQuery.isLoading) {
      return;
    }
    if (metadata?.defaultAgentPubkey) {
      return;
    }
    if (ensuredChatIdsRef.current.has(selectedChat.id)) {
      return;
    }
    ensuredChatIdsRef.current.add(selectedChat.id);
    void ensureWelcomeGuideAgentInChannel(
      selectedChat.id,
      activeWorkspace?.relayUrl,
    )
      .then((agent) =>
        updateMetadataMutation.mutateAsync({
          channelId: selectedChat.id,
          defaultAgentPubkey: agent.pubkey,
          title: metadata?.title ?? selectedChat.name,
          templateId: metadata?.templateId ?? undefined,
          projectId: metadata?.projectId ?? undefined,
          projectName: metadata?.projectName ?? undefined,
          projectPath: metadata?.projectPath ?? undefined,
          projectTemplateId: metadata?.projectTemplateId ?? undefined,
          source: metadata?.sourceChannelId
            ? {
                channelId: metadata.sourceChannelId,
                eventId: metadata.sourceEventId ?? undefined,
                threadRootId: metadata.sourceThreadRootId ?? undefined,
              }
            : undefined,
        }),
      )
      .catch((error) => {
        console.error("Failed to ensure Fizz for chat", selectedChat.id, error);
      });
  }, [
    activeWorkspace?.relayUrl,
    metadata,
    metadataQuery.isLoading,
    selectedChat,
    updateMetadataMutation,
  ]);

  React.useEffect(() => {
    if (!selectedChat || messages.length === 0) {
      return;
    }
    const lastMessage = messages[messages.length - 1];
    if (!lastMessage) {
      return;
    }
    markChannelRead(
      selectedChat.id,
      new Date(lastMessage.created_at * 1_000).toISOString(),
    );
  }, [markChannelRead, messages, selectedChat]);

  const handleSend = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
    ) => {
      const defaultAgentPubkey =
        metadata?.defaultAgentPubkey ?? defaultAgent?.pubkey ?? null;
      await sendMessageMutation.mutateAsync({
        content,
        mentionPubkeys: uniqueMentionPubkeys(
          identityQuery.data?.pubkey,
          mentionPubkeys,
          defaultAgentPubkey,
        ),
        mediaTags,
      });
    },
    [
      defaultAgent?.pubkey,
      identityQuery.data?.pubkey,
      metadata?.defaultAgentPubkey,
      sendMessageMutation,
    ],
  );

  const handleActivateAgent = React.useCallback(async () => {
    if (!selectedChat) {
      return;
    }

    setIsEnsuringDefaultAgent(true);
    try {
      if (defaultAgent) {
        await addBotToChat(selectedChat.id, defaultAgent.pubkey);
        if (
          defaultAgent.status !== "running" &&
          defaultAgent.status !== "deployed"
        ) {
          const updatedAgent = await startManagedAgentMutation.mutateAsync(
            defaultAgent.pubkey,
          );
          toast.success(`${updatedAgent.name || defaultAgent.name} activated`);
        } else {
          await managedAgentsQuery.refetch();
          toast.success(`${defaultAgent.name} activated`);
        }
        return;
      }

      const agent = await ensureWelcomeGuideAgentInChannel(
        selectedChat.id,
        activeWorkspace?.relayUrl,
      );
      await updateMetadataMutation.mutateAsync({
        channelId: selectedChat.id,
        defaultAgentPubkey: agent.pubkey,
        title: metadata?.title ?? selectedChat.name,
        templateId: metadata?.templateId ?? undefined,
        projectId: metadata?.projectId ?? undefined,
        projectName: metadata?.projectName ?? undefined,
        projectPath: metadata?.projectPath ?? undefined,
        projectTemplateId: metadata?.projectTemplateId ?? undefined,
        source: metadata?.sourceChannelId
          ? {
              channelId: metadata.sourceChannelId,
              eventId: metadata.sourceEventId ?? undefined,
              threadRootId: metadata.sourceThreadRootId ?? undefined,
            }
          : undefined,
      });
      await managedAgentsQuery.refetch();
      toast.success(`${agent.name || "Fizz"} activated`);
    } catch (error) {
      console.error("Failed to activate chat agent", error);
      toast.error("Could not activate agent", {
        description: error instanceof Error ? error.message : undefined,
      });
    } finally {
      setIsEnsuringDefaultAgent(false);
    }
  }, [
    activeWorkspace?.relayUrl,
    defaultAgent,
    managedAgentsQuery,
    metadata,
    selectedChat,
    startManagedAgentMutation,
    updateMetadataMutation,
  ]);

  return (
    <div className="grid min-h-0 flex-1 grid-cols-[17rem_minmax(0,1fr)] overflow-hidden">
      <aside className="min-h-0 border-r border-border/60 bg-muted/15">
        <ChatList
          chats={chats}
          getChannelReadAt={getChannelReadAt}
          identityPubkey={identityPubkey}
          isLoading={chatsQuery.isLoading || metadataListQuery.isLoading}
          metadataByChatId={metadataByChatId}
          onCreateChat={() => void goChats({ projectId: null })}
          onCreateProjectChat={(projectId) =>
            void goChats({ projectId, replace: true })
          }
          onArchiveChat={(chatId) => void handleArchiveChat(chatId)}
          onSelectChat={(chatId) => void goChat(chatId)}
          onUpdateProject={(project) =>
            upsertStoredChatProject(activeWorkspace?.id, project)
          }
          archivingChatId={
            archiveChatMutation.isPending
              ? (archiveChatMutation.variables ?? null)
              : null
          }
          projects={chatProjects}
          readStateVersion={readStateVersion}
          selectedChatId={selectedChatId}
          templates={templates}
          unreadChannelCounts={unreadChannelCounts}
          unreadChannelIds={unreadChannelIds}
        />
      </aside>

      <main className="flex min-h-0 min-w-0 flex-col bg-background">
        {selectedChat ? (
          <ChatDetail
            chat={selectedChat}
            defaultAgent={defaultAgent}
            identityPubkey={identityPubkey}
            isActivatingAgent={
              isEnsuringDefaultAgent || startManagedAgentMutation.isPending
            }
            isLoadingMessages={messageQuery.isLoading}
            isSending={sendMessageMutation.isPending}
            messages={messages}
            metadata={metadata}
            onActivateAgent={handleActivateAgent}
            onProjectCreated={(project) =>
              upsertStoredChatProject(activeWorkspace?.id, project)
            }
            onSend={handleSend}
            profiles={profiles}
            projects={chatProjects}
            shareAction={
              <ChatHeaderActions
                chat={selectedChat}
                defaultAgentPubkey={defaultAgent?.pubkey}
                messages={messages}
                metadata={metadata}
              />
            }
            templates={templates}
          />
        ) : (
          <QuickStartChat
            initialProjectId={initialProjectId}
            projects={chatProjects}
            relayUrl={activeWorkspace?.relayUrl}
            onProjectCreated={(project) =>
              upsertStoredChatProject(activeWorkspace?.id, project)
            }
            onCreated={(chat) => void goChat(chat.id, { replace: true })}
          />
        )}
      </main>
    </div>
  );
}

function ChatList({
  archivingChatId,
  chats,
  getChannelReadAt,
  identityPubkey,
  isLoading,
  metadataByChatId,
  onArchiveChat,
  onCreateChat,
  onCreateProjectChat,
  onSelectChat,
  onUpdateProject,
  projects,
  readStateVersion: _readStateVersion,
  selectedChatId,
  templates,
  unreadChannelCounts,
  unreadChannelIds,
}: {
  archivingChatId: string | null;
  chats: Channel[];
  getChannelReadAt: (channelId: string) => number | null;
  identityPubkey?: string | null;
  isLoading: boolean;
  metadataByChatId: ReadonlyMap<string, ChatMetadata>;
  onArchiveChat: (chatId: string) => void;
  onCreateChat: () => void;
  onCreateProjectChat: (projectId: string) => void;
  onSelectChat: (chatId: string) => void;
  onUpdateProject: (
    project: ReturnType<typeof buildChatProjects>[number],
  ) => void;
  projects: ReturnType<typeof buildChatProjects>;
  readStateVersion: number;
  selectedChatId: string | null;
  templates: ChannelTemplate[];
  unreadChannelCounts: ReadonlyMap<string, number>;
  unreadChannelIds: ReadonlySet<string>;
}) {
  const [collapsedProjectIds, setCollapsedProjectIds] = React.useState(
    () => new Set<string>(),
  );
  const [editingProject, setEditingProject] = React.useState<
    ReturnType<typeof buildChatProjects>[number] | null
  >(null);
  const [isCreateProjectOpen, setIsCreateProjectOpen] = React.useState(false);
  const activeAgentTurnsByChannel = useActiveAgentTurnsByChannel();
  const activeChatIds = React.useMemo(
    () => new Set(activeAgentTurnsByChannel.map((turn) => turn.channelId)),
    [activeAgentTurnsByChannel],
  );
  const chatsByProject = React.useMemo(() => {
    const groups = new Map<string, Channel[]>();
    const unprojected: Channel[] = [];
    const shared: Channel[] = [];
    const knownProjectIds = new Set(projects.map((project) => project.id));
    for (const chat of chats) {
      const metadata = metadataByChatId.get(chat.id);
      if (isSharedChatMetadata(metadata, identityPubkey)) {
        shared.push(chat);
        continue;
      }
      const projectId = metadata?.projectId;
      if (projectId && knownProjectIds.has(projectId)) {
        const group = groups.get(projectId) ?? [];
        group.push(chat);
        groups.set(projectId, group);
      } else {
        unprojected.push(chat);
      }
    }
    return { groups, shared, unprojected };
  }, [chats, identityPubkey, metadataByChatId, projects]);

  const toggleProject = React.useCallback((projectId: string) => {
    setCollapsedProjectIds((current) => {
      const next = new Set(current);
      if (next.has(projectId)) {
        next.delete(projectId);
      } else {
        next.add(projectId);
      }
      return next;
    });
  }, []);

  if (isLoading) {
    return (
      <div className="flex h-full min-h-0 flex-col">
        <ChatListHeader />
        <ChatListSkeleton />
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ChatListHeader />
      <div className="buzz-sidebar-scrollbar min-h-0 flex-1 overflow-y-auto p-2 pt-3">
        <div className="mb-3 space-y-1">
          <ChatListSectionHeader
            actionLabel="Add project"
            label="Projects"
            onAction={() => setIsCreateProjectOpen(true)}
          />
          {projects.map((project) => {
            const projectChats = chatsByProject.groups.get(project.id) ?? [];
            const isCollapsed = collapsedProjectIds.has(project.id);
            return (
              <div key={project.id} className="mb-1">
                <div className="group/project flex h-8 w-full min-w-0 items-center gap-1.5 rounded-md px-2 text-xs font-semibold text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground">
                  <Folder className="h-3.5 w-3.5 shrink-0" />
                  <button
                    className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
                    onClick={() => toggleProject(project.id)}
                    type="button"
                  >
                    <span className="min-w-0 truncate">{project.name}</span>
                    {isCollapsed ? (
                      <ChevronRight className="h-3.5 w-3.5 shrink-0" />
                    ) : (
                      <ChevronDown className="h-3.5 w-3.5 shrink-0" />
                    )}
                  </button>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        aria-label={`Project settings for ${project.name}`}
                        className="h-6 w-6 text-muted-foreground opacity-0 transition-opacity hover:text-foreground focus-visible:opacity-100 data-[state=open]:opacity-100 group-hover/project:opacity-100"
                        size="icon-xs"
                        type="button"
                        variant="ghost"
                      >
                        <MoreVertical className="h-3.5 w-3.5" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="w-44">
                      <DropdownMenuItem
                        onSelect={() => setEditingProject(project)}
                      >
                        Project settings
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                  <Button
                    aria-label={`New chat in ${project.name}`}
                    className="h-6 w-6 text-muted-foreground opacity-0 transition-opacity hover:text-foreground focus-visible:opacity-100 group-hover/project:opacity-100"
                    onClick={() => onCreateProjectChat(project.id)}
                    size="icon-xs"
                    type="button"
                    variant="ghost"
                  >
                    <Plus className="h-3.5 w-3.5" />
                  </Button>
                </div>
                {!isCollapsed ? (
                  <div className="space-y-1">
                    {projectChats.length > 0 ? (
                      projectChats.map((chat) => (
                        <ChatListItem
                          chat={chat}
                          getChannelReadAt={getChannelReadAt}
                          isAgentRunning={activeChatIds.has(chat.id)}
                          isArchiving={archivingChatId === chat.id}
                          key={chat.id}
                          onArchiveChat={onArchiveChat}
                          onSelectChat={onSelectChat}
                          selectedChatId={selectedChatId}
                          unreadChannelCounts={unreadChannelCounts}
                          unreadChannelIds={unreadChannelIds}
                        />
                      ))
                    ) : (
                      <div className="px-3 py-1.5 text-xs text-muted-foreground">
                        No chats yet
                      </div>
                    )}
                  </div>
                ) : null}
              </div>
            );
          })}
        </div>
        <div className="space-y-1">
          <ChatListSectionHeader
            actionLabel="New chat without a project"
            label="Chats"
            onAction={onCreateChat}
          />
          {chatsByProject.unprojected.length > 0 ? (
            chatsByProject.unprojected.map((chat) => (
              <ChatListItem
                chat={chat}
                getChannelReadAt={getChannelReadAt}
                isAgentRunning={activeChatIds.has(chat.id)}
                isArchiving={archivingChatId === chat.id}
                key={chat.id}
                onArchiveChat={onArchiveChat}
                onSelectChat={onSelectChat}
                selectedChatId={selectedChatId}
                unreadChannelCounts={unreadChannelCounts}
                unreadChannelIds={unreadChannelIds}
              />
            ))
          ) : (
            <div className="px-3 py-1.5 text-xs text-muted-foreground">
              No chats yet
            </div>
          )}
        </div>
        {chatsByProject.shared.length > 0 ? (
          <div className="mt-4 space-y-1">
            <ChatListSectionHeader label="Shared" />
            {chatsByProject.shared.map((chat) => (
              <ChatListItem
                chat={chat}
                getChannelReadAt={getChannelReadAt}
                isAgentRunning={activeChatIds.has(chat.id)}
                isArchiving={archivingChatId === chat.id}
                key={chat.id}
                onArchiveChat={onArchiveChat}
                onSelectChat={onSelectChat}
                selectedChatId={selectedChatId}
                unreadChannelCounts={unreadChannelCounts}
                unreadChannelIds={unreadChannelIds}
              />
            ))}
          </div>
        ) : null}
      </div>
      <ChatProjectDialog
        onOpenChange={setIsCreateProjectOpen}
        onSaveProject={(project) => {
          onUpdateProject(project);
          setIsCreateProjectOpen(false);
        }}
        open={isCreateProjectOpen}
        templates={templates}
      />
      <ChatProjectDialog
        mode="edit"
        onOpenChange={(open) => {
          if (!open) {
            setEditingProject(null);
          }
        }}
        onSaveProject={(project) => {
          onUpdateProject(project);
          setEditingProject(null);
        }}
        open={editingProject !== null}
        project={editingProject}
        templates={templates}
      />
    </div>
  );
}
