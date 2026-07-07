// @ts-nocheck
import * as React from "react";
import {
  ChevronDown,
  ChevronRight,
  MoreVertical,
  Notebook,
  Plus,
} from "lucide-react";

import { useActiveAgentTurnsByChannel } from "@/features/agents/activeAgentTurnsStore";
import type { buildChatProjects } from "@/features/chats/lib/chatProjects";
import { ChatListHeader, ChatListItem } from "@/features/chats/ui/ChatListItem";
import { ChatListSectionHeader } from "@/features/chats/ui/ChatListSectionHeader";
import { ChatListSkeleton } from "@/features/chats/ui/ChatListSkeleton";
import { ChatProjectDialog } from "@/features/chats/ui/ChatProjectDialog";
import { isSharedChatMetadata } from "@/features/chats/lib/chatShared";
import type {
  Channel,
  ChannelTemplate,
  ChatMetadata,
} from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

export function ChatList({
  archivingChatId,
  chats,
  getChannelReadAt,
  identityPubkey,
  isLoading,
  metadataByChatId,
  onArchiveChat,
  onCreateChat,
  onCreateProjectChat,
  onRenameChat,
  onSelectChat,
  onTogglePin,
  onUpdateProject,
  pinnedChatIds,
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
  onRenameChat: (chatId: string) => void;
  onSelectChat: (chatId: string) => void;
  onTogglePin: (chatId: string) => void;
  pinnedChatIds: ReadonlySet<string>;
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
    const pinnedFirst = (list: Channel[]) =>
      [...list].sort(
        (left, right) =>
          Number(pinnedChatIds.has(right.id)) -
          Number(pinnedChatIds.has(left.id)),
      );
    return {
      groups: new Map(
        [...groups.entries()].map(([projectId, group]) => [
          projectId,
          pinnedFirst(group),
        ]),
      ),
      shared: pinnedFirst(shared),
      unprojected: pinnedFirst(unprojected),
    };
  }, [chats, identityPubkey, metadataByChatId, pinnedChatIds, projects]);

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
                  <Notebook className="h-3.5 w-3.5 shrink-0" />
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
                          displayName={metadataByChatId.get(chat.id)?.title}
                          getChannelReadAt={getChannelReadAt}
                          isAgentRunning={activeChatIds.has(chat.id)}
                          isArchiving={archivingChatId === chat.id}
                          isPinned={pinnedChatIds.has(chat.id)}
                          key={chat.id}
                          onArchiveChat={onArchiveChat}
                          onRenameChat={onRenameChat}
                          onSelectChat={onSelectChat}
                          onTogglePin={onTogglePin}
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
                displayName={metadataByChatId.get(chat.id)?.title}
                getChannelReadAt={getChannelReadAt}
                isAgentRunning={activeChatIds.has(chat.id)}
                isArchiving={archivingChatId === chat.id}
                isPinned={pinnedChatIds.has(chat.id)}
                key={chat.id}
                onArchiveChat={onArchiveChat}
                onRenameChat={onRenameChat}
                onSelectChat={onSelectChat}
                onTogglePin={onTogglePin}
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
                canRename={false}
                chat={chat}
                displayName={metadataByChatId.get(chat.id)?.title}
                getChannelReadAt={getChannelReadAt}
                isAgentRunning={activeChatIds.has(chat.id)}
                isArchiving={archivingChatId === chat.id}
                isPinned={pinnedChatIds.has(chat.id)}
                key={chat.id}
                onArchiveChat={onArchiveChat}
                onRenameChat={onRenameChat}
                onSelectChat={onSelectChat}
                onTogglePin={onTogglePin}
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
