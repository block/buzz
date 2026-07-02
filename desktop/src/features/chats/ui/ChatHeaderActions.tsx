import * as React from "react";
import {
  Bot,
  Check,
  FileText,
  Link2,
  MoreVertical,
  Share2,
} from "lucide-react";
import { toast } from "sonner";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { ChannelCanvas } from "@/features/channels/ui/ChannelCanvas";
import { useUpdateChatMetadataMutation } from "@/features/chats/hooks";
import { buildChatLink } from "@/features/chats/lib/chatLink";
import {
  cleanAssistantMessageText,
  isHumanFacingAssistantText,
} from "@/features/chats/ui/chatActivityText";
import { addChannelMembers, sendChannelMessage } from "@/shared/api/tauri";
import type {
  Channel,
  ChatMetadata,
  ManagedAgent,
  RelayEvent,
} from "@/shared/api/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Spinner } from "@/shared/ui/spinner";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type ChatHeaderActionsProps = {
  chat: Channel;
  defaultAgentPubkey?: string | null;
  messages: RelayEvent[];
  metadata: ChatMetadata | null;
};

export function ChatHeaderActions({
  chat,
  defaultAgentPubkey,
  messages,
  metadata,
}: ChatHeaderActionsProps) {
  const [isCanvasOpen, setIsCanvasOpen] = React.useState(false);
  const [isDefaultAgentOpen, setIsDefaultAgentOpen] = React.useState(false);

  return (
    <div className="flex items-center gap-1">
      <ChatCanvasDialog
        chat={chat}
        onOpenChange={setIsCanvasOpen}
        open={isCanvasOpen}
      />
      <DefaultAgentDialog
        chat={chat}
        defaultAgentPubkey={metadata?.defaultAgentPubkey ?? defaultAgentPubkey}
        metadata={metadata}
        onOpenChange={setIsDefaultAgentOpen}
        open={isDefaultAgentOpen}
      />
      <ChatShareMenu
        chat={chat}
        defaultAgentPubkey={metadata?.defaultAgentPubkey ?? defaultAgentPubkey}
        messages={messages}
        metadata={metadata}
      />
      <ChatSettingsMenu
        onOpenCanvas={() => setIsCanvasOpen(true)}
        onOpenDefaultAgent={() => setIsDefaultAgentOpen(true)}
      />
    </div>
  );
}

function ChatSettingsMenu({
  onOpenCanvas,
  onOpenDefaultAgent,
}: {
  onOpenCanvas: () => void;
  onOpenDefaultAgent: () => void;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="icon" type="button" variant="ghost">
          <MoreVertical className="h-4 w-4" />
          <span className="sr-only">Chat settings</span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuItem onSelect={onOpenDefaultAgent}>
          <Bot className="h-4 w-4" />
          Default agent
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={onOpenCanvas}>
          <FileText className="h-4 w-4" />
          Canvas settings
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function ChatCanvasDialog({
  chat,
  onOpenChange,
  open,
}: {
  chat: Channel;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-h-[80vh] max-w-2xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Canvas</DialogTitle>
          <DialogDescription>{chat.name}</DialogDescription>
        </DialogHeader>
        <ChannelCanvas
          canEdit
          channelId={chat.id}
          emptyMessage="No canvas set for this chat."
          isArchived={Boolean(chat.archivedAt)}
        />
      </DialogContent>
    </Dialog>
  );
}

function DefaultAgentDialog({
  chat,
  defaultAgentPubkey,
  metadata,
  onOpenChange,
  open,
}: {
  chat: Channel;
  defaultAgentPubkey?: string | null;
  metadata: ChatMetadata | null;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const managedAgentsQuery = useManagedAgentsQuery({ enabled: open });
  const updateMetadataMutation = useUpdateChatMetadataMutation();
  const currentDefault = defaultAgentPubkey
    ? normalizePubkey(defaultAgentPubkey)
    : null;
  const agents = React.useMemo(
    () =>
      [...(managedAgentsQuery.data ?? [])].sort((left, right) =>
        left.name.localeCompare(right.name),
      ),
    [managedAgentsQuery.data],
  );

  const handleSelectAgent = React.useCallback(
    async (agent: ManagedAgent) => {
      try {
        await addBotToChat(chat.id, agent.pubkey);
        await updateMetadataMutation.mutateAsync({
          channelId: chat.id,
          defaultAgentPubkey: agent.pubkey,
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
          templateId: metadata?.templateId ?? undefined,
          title: metadata?.title ?? chat.name,
        });
        toast.success(`Default agent set to ${agent.name}`);
        onOpenChange(false);
      } catch (error) {
        console.error("Failed to update default chat agent", error);
        toast.error("Could not update default agent", {
          description: error instanceof Error ? error.message : undefined,
        });
      }
    },
    [chat.id, chat.name, metadata, onOpenChange, updateMetadataMutation],
  );

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-h-[80vh] max-w-lg overflow-hidden p-0">
        <div className="flex min-h-0 flex-col">
          <DialogHeader className="border-b border-border/60 px-6 py-5 pr-14">
            <DialogTitle>Default agent</DialogTitle>
            <DialogDescription>
              Choose who replies when you send a message without an @mention.
            </DialogDescription>
          </DialogHeader>

          <div className="min-h-0 flex-1 overflow-y-auto p-3">
            {managedAgentsQuery.isLoading ? (
              <div className="flex items-center gap-2 px-3 py-4 text-sm text-muted-foreground">
                <Spinner className="h-4 w-4" />
                Loading agents
              </div>
            ) : agents.length === 0 ? (
              <div className="px-3 py-6 text-sm text-muted-foreground">
                No managed agents are available.
              </div>
            ) : (
              <div className="space-y-1">
                {agents.map((agent) => {
                  const isSelected =
                    currentDefault === normalizePubkey(agent.pubkey);
                  const isSaving =
                    updateMetadataMutation.isPending &&
                    updateMetadataMutation.variables?.defaultAgentPubkey ===
                      agent.pubkey;

                  return (
                    <button
                      className={cn(
                        "flex w-full items-center gap-3 rounded-md px-3 py-2 text-left text-sm transition-colors hover:bg-accent hover:text-accent-foreground",
                        isSelected && "bg-secondary text-secondary-foreground",
                      )}
                      disabled={updateMetadataMutation.isPending}
                      key={agent.pubkey}
                      onClick={() => void handleSelectAgent(agent)}
                      type="button"
                    >
                      <UserAvatar
                        avatarUrl={agent.avatarUrl}
                        displayName={agent.name}
                        size="sm"
                      />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate font-medium">
                          {agent.name}
                        </span>
                        <span className="block truncate text-xs text-muted-foreground">
                          {agent.status.replace("_", " ")}
                        </span>
                      </span>
                      {isSaving ? (
                        <Spinner className="h-4 w-4 text-muted-foreground" />
                      ) : (
                        <Check
                          className={cn(
                            "h-4 w-4 text-primary",
                            !isSelected && "invisible",
                          )}
                        />
                      )}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function ChatShareMenu({
  chat,
  defaultAgentPubkey,
  messages,
  metadata,
}: ChatHeaderActionsProps) {
  const [isSharingSummary, setIsSharingSummary] = React.useState(false);
  const chatLink = React.useMemo(
    () =>
      buildChatLink({
        chatId: chat.id,
        title: metadata?.title?.trim() || chat.name,
      }),
    [chat.id, chat.name, metadata?.title],
  );
  const canShareSummary = Boolean(metadata?.sourceChannelId);

  const handleCopyLink = React.useCallback(async () => {
    try {
      await navigator.clipboard.writeText(chatLink);
      toast.success("Chat link copied");
    } catch (error) {
      console.error("Failed to copy chat link", error);
      toast.error("Could not copy link");
    }
  }, [chatLink]);

  const handleShareSummary = React.useCallback(async () => {
    const sourceChannelId = metadata?.sourceChannelId;
    if (!sourceChannelId) {
      return;
    }

    setIsSharingSummary(true);
    try {
      const parentEventId =
        metadata.sourceThreadRootId ?? metadata.sourceEventId ?? null;
      await sendChannelMessage(
        sourceChannelId,
        buildSharedChatSummaryMessage({
          defaultAgentPubkey,
          link: chatLink,
          messages,
        }),
        parentEventId,
      );
      toast.success("Summary shared");
    } catch (error) {
      console.error("Failed to share chat summary", error);
      toast.error("Could not share summary");
    } finally {
      setIsSharingSummary(false);
    }
  }, [chatLink, defaultAgentPubkey, messages, metadata]);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          className="shadow-none"
          size="sm"
          type="button"
          variant="secondary"
        >
          <Share2 className="h-4 w-4" />
          Share
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        <DropdownMenuItem onSelect={() => void handleCopyLink()}>
          <Link2 className="h-4 w-4" />
          Copy link
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          disabled={!canShareSummary || isSharingSummary}
          onSelect={() => void handleShareSummary()}
        >
          {isSharingSummary ? (
            <Spinner className="h-4 w-4" />
          ) : (
            <FileText className="h-4 w-4" />
          )}
          Share summary to source
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
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

function buildSharedChatSummaryMessage({
  defaultAgentPubkey,
  link,
  messages,
}: {
  defaultAgentPubkey?: string | null;
  link: string;
  messages: RelayEvent[];
}) {
  const summary = summarizeChat(messages, defaultAgentPubkey);
  return [
    link,
    summary
      ? `> ${summary.replace(/\n+/g, "\n> ")}`
      : "> No summary is available yet.",
  ].join("\n");
}

function summarizeChat(
  messages: RelayEvent[],
  defaultAgentPubkey?: string | null,
) {
  const normalizedAgentPubkey = defaultAgentPubkey
    ? normalizePubkey(defaultAgentPubkey)
    : null;
  const latestAssistantMessage = [...messages].reverse().find((message) => {
    if (!normalizedAgentPubkey) {
      return false;
    }
    if (message.kind === KIND_SYSTEM_MESSAGE) {
      return false;
    }
    if (normalizePubkey(message.pubkey) !== normalizedAgentPubkey) {
      return false;
    }
    return isHumanFacingAssistantText(message.content);
  });

  const text = latestAssistantMessage
    ? cleanAssistantMessageText(latestAssistantMessage.content)
    : messages
        .filter(
          (message) =>
            message.kind !== KIND_SYSTEM_MESSAGE &&
            !eventHasTag(message, "chat_context", "source") &&
            message.content.trim().length > 0,
        )
        .slice(-3)
        .map((message) => message.content.trim())
        .join("\n");

  return truncateSummary(text);
}

function eventHasTag(event: RelayEvent, name: string, value?: string) {
  return event.tags.some(
    (tag) => tag[0] === name && (value === undefined || tag[1] === value),
  );
}

function truncateSummary(value: string) {
  const normalized = value.trim().replace(/\n{3,}/g, "\n\n");
  if (normalized.length <= 900) {
    return normalized;
  }
  return `${normalized.slice(0, 897).trimEnd()}...`;
}
