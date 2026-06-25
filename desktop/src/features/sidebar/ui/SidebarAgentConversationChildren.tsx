import type { AgentConversation } from "@/features/agents/agentConversations";
import { cn } from "@/shared/lib/cn";
import { X } from "lucide-react";
import * as React from "react";
import { SidebarMenuButton, SidebarMenuItem } from "@/shared/ui/sidebar";

const COLLAPSED_CONVERSATION_LIMIT = 4;

type SidebarAgentConversationChildrenProps = {
  channelId: string;
  conversations?: readonly AgentConversation[];
  isConversationViewActive: boolean;
  onHideConversation?: (conversationId: string) => void;
  onSelectConversation?: (conversationId: string) => void;
  selectedConversationId?: string | null;
};

export function SidebarAgentConversationChildren({
  channelId,
  conversations,
  isConversationViewActive,
  onHideConversation,
  onSelectConversation,
  selectedConversationId,
}: SidebarAgentConversationChildrenProps) {
  const [isExpanded, setIsExpanded] = React.useState(false);

  if (!conversations || conversations.length === 0) {
    return null;
  }

  const hasOverflow = conversations.length > COLLAPSED_CONVERSATION_LIMIT;
  const visibleConversations = isExpanded
    ? conversations
    : conversations.slice(0, COLLAPSED_CONVERSATION_LIMIT);
  const toggleLabel = isExpanded ? "Show less" : "Show more";

  return (
    <>
      {visibleConversations.map((conversation) => {
        const isActive =
          isConversationViewActive &&
          selectedConversationId === conversation.id;

        return (
          <SidebarMenuItem
            className="group-data-[collapsible=icon]:hidden"
            data-testid={`sidebar-channel-agent-conversations-${channelId}`}
            key={conversation.id}
          >
            <div className="relative">
              <SidebarMenuButton
                className="pr-8"
                data-testid={`sidebar-agent-conversation-${conversation.id}`}
                isActive={isActive}
                onClick={(event) => {
                  event.stopPropagation();
                  onSelectConversation?.(conversation.id);
                }}
                tooltip={conversation.title}
                type="button"
              >
                <span className="min-w-0 flex-1 truncate pl-6">
                  {conversation.title}
                </span>
              </SidebarMenuButton>
              {onHideConversation ? (
                <button
                  aria-label="Close conversation"
                  className={cn(
                    "absolute right-1 top-1/2 z-10 flex size-6 -translate-y-1/2 items-center justify-center p-1 text-sidebar-foreground/45 opacity-0 transition-[color,opacity] hover:text-sidebar-foreground focus-visible:opacity-100 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring group-hover/menu-item:opacity-100 group-focus-within/menu-item:opacity-100",
                    isActive &&
                      "text-sidebar-active-foreground/70 hover:text-sidebar-active-foreground",
                  )}
                  data-testid={`hide-agent-conversation-${conversation.id}`}
                  onClick={(event) => {
                    event.preventDefault();
                    event.stopPropagation();
                    onHideConversation(conversation.id);
                  }}
                  title="Close conversation"
                  type="button"
                >
                  <X className="h-4 w-4" />
                </button>
              ) : null}
            </div>
          </SidebarMenuItem>
        );
      })}
      {hasOverflow ? (
        <SidebarMenuItem
          className="group-data-[collapsible=icon]:hidden"
          data-testid={`sidebar-channel-agent-conversations-toggle-${channelId}`}
        >
          <SidebarMenuButton
            aria-expanded={isExpanded}
            className="text-sidebar-foreground/65 hover:text-sidebar-foreground"
            onClick={(event) => {
              event.stopPropagation();
              setIsExpanded((current) => !current);
            }}
            tooltip={toggleLabel}
            type="button"
          >
            <span className="min-w-0 flex-1 truncate pl-6">{toggleLabel}</span>
          </SidebarMenuButton>
        </SidebarMenuItem>
      ) : null}
    </>
  );
}
