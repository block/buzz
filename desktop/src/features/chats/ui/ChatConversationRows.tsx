import * as React from "react";
import { Bot, FolderGit2, MessageCircle, Power } from "lucide-react";

import { cleanAssistantMessageText } from "@/features/chats/ui/chatActivityText";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { RelayEvent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import {
  resolveMentionNames,
  resolveMentionPubkeysByName,
} from "@/shared/lib/resolveMentionNames";
import { Bubble } from "@/shared/ui/bubble";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import { Marker, MarkerContent, MarkerIcon } from "@/shared/ui/marker";
import {
  Message,
  MessageAvatar,
  MessageContent,
  MessageHeader,
} from "@/shared/ui/message";
import { useMessageScroller } from "@/shared/ui/message-scroller";
import { UserAvatar } from "@/shared/ui/UserAvatar";

function profileName(
  pubkey: string,
  profiles: UserProfileLookup | undefined,
  fallback = "Unknown",
) {
  const profile = profiles?.[pubkey.toLowerCase()];
  return (
    profile?.displayName?.trim() ||
    profile?.nip05Handle?.trim() ||
    `${fallback} ${pubkey.slice(0, 8)}`
  );
}

export function ChatMessageRow({
  event,
  isAgent,
  isOwn,
  profiles,
  showAgentIdentity = true,
}: {
  event: RelayEvent;
  isAgent: boolean;
  isOwn: boolean;
  profiles?: UserProfileLookup;
  /**
   * Solo chats (one human, one agent) hide the agent's avatar/name so its
   * replies read as part of the stream.
   */
  showAgentIdentity?: boolean;
}) {
  const hideIdentity = isAgent && !showAgentIdentity;
  const displayName = profileName(
    event.pubkey,
    profiles,
    isOwn ? "You" : isAgent ? "Fizz" : "User",
  );
  const profile = profiles?.[event.pubkey.toLowerCase()];
  const content = isAgent
    ? cleanAssistantMessageText(event.content)
    : event.content;
  // @mentions render as chips, resolved from the message's p/mention tags —
  // same treatment as channel messages, split into people vs agent chips.
  const mentionNames = React.useMemo(
    () => resolveMentionNames(event.tags, profiles),
    [event.tags, profiles],
  );
  const mentionPubkeysByName = React.useMemo(
    () => resolveMentionPubkeysByName(event.tags, profiles),
    [event.tags, profiles],
  );
  const agentMentionPubkeysByName = React.useMemo(() => {
    if (!mentionPubkeysByName) {
      return undefined;
    }
    const values: Record<string, string> = {};
    for (const [name, pubkey] of Object.entries(mentionPubkeysByName)) {
      if (profiles?.[pubkey.toLowerCase()]?.isAgent) {
        values[name] = pubkey;
      }
    }
    return Object.keys(values).length > 0 ? values : undefined;
  }, [mentionPubkeysByName, profiles]);

  return (
    <Message side={isOwn ? "right" : "left"}>
      {!isOwn && !hideIdentity ? (
        <MessageAvatar>
          <UserAvatar
            avatarUrl={profile?.avatarUrl ?? null}
            displayName={displayName}
            size="sm"
          />
        </MessageAvatar>
      ) : null}
      <MessageContent
        className={cn(isOwn && "items-end", isAgent && "w-full max-w-full")}
      >
        {/* Own bubbles need no "You" header — the right-aligned bubble is
            self-explanatory. Other authors keep their name (unless the solo
            chat hides the lone agent's identity). */}
        {!hideIdentity && !isOwn ? (
          <MessageHeader>
            <span className="truncate font-medium">{displayName}</span>
          </MessageHeader>
        ) : null}
        {isAgent ? (
          <Markdown
            agentAuthored
            agentMentionPubkeysByName={agentMentionPubkeysByName}
            className="w-full max-w-none text-sm leading-6"
            content={content || " "}
            mentionNames={mentionNames}
            mentionPubkeysByName={mentionPubkeysByName}
          />
        ) : (
          <Bubble
            className={cn(isOwn && "buzz-own-bubble-links")}
            side={isOwn ? "right" : "left"}
          >
            <Markdown
              agentMentionPubkeysByName={agentMentionPubkeysByName}
              className={cn(
                "min-w-0",
                isOwn &&
                  "[&_*]:text-primary-foreground [&_a]:text-primary-foreground [&_code]:bg-primary-foreground/15 [&_code]:text-primary-foreground",
              )}
              compact
              content={content || " "}
              mentionNames={mentionNames}
              mentionPubkeysByName={mentionPubkeysByName}
            />
          </Bubble>
        )}
      </MessageContent>
    </Message>
  );
}

// Following the stream is owned entirely by the MessageScroller's built-in
// autoScroll (content mutation + resize observers). This anchor only handles
// the one case autoScroll intentionally skips: jumping back to the bottom
// when the user sends a message while scrolled up in history.
export function ChatScrollAnchor({
  forceSignature,
}: {
  forceSignature: string | null;
}) {
  const { scrollToEnd } = useMessageScroller();
  const lastForcedSignatureRef = React.useRef<string | null>(null);

  React.useLayoutEffect(() => {
    if (
      forceSignature === null ||
      forceSignature === lastForcedSignatureRef.current
    ) {
      return;
    }
    lastForcedSignatureRef.current = forceSignature;
    scrollToEnd({ behavior: "auto" });
  }, [forceSignature, scrollToEnd]);

  return null;
}

export function ChatContextRow({ event }: { event: RelayEvent }) {
  const isProjectSetup = event.content.startsWith("Project setup");
  const projectSetupContent = event.content
    .replace(/^Project setup\s*\n?/, "")
    .trim();

  if (isProjectSetup) {
    return (
      <Message side="left">
        <MessageContent className="w-full max-w-full">
          <div className="max-w-2xl">
            <Marker>
              <MarkerIcon>
                <FolderGit2 />
              </MarkerIcon>
              <MarkerContent>Project setup</MarkerContent>
            </Marker>
            <div className="mt-2 rounded-md border border-border/60 bg-muted/20 px-3 py-2 text-sm">
              <Markdown
                compact
                content={projectSetupContent || event.content}
              />
            </div>
          </div>
        </MessageContent>
      </Message>
    );
  }

  return (
    <Message side="center">
      <MessageContent className="max-w-[min(44rem,86%)]">
        <Marker>
          <MarkerIcon>
            <MessageCircle />
          </MarkerIcon>
          <MarkerContent>Source context</MarkerContent>
        </Marker>
        <Bubble className="text-sm" variant="outline">
          <Markdown compact content={event.content} />
        </Bubble>
      </MessageContent>
    </Message>
  );
}

export function AgentActivationCard({
  agentName,
  isActivating,
  onActivate,
}: {
  agentName: string;
  isActivating: boolean;
  onActivate: () => void;
}) {
  return (
    <Message side="center">
      <MessageContent className="w-full max-w-2xl">
        <div className="rounded-lg border border-border/70 bg-muted/20 px-4 py-3">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div className="flex min-w-0 gap-3">
              <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-border/60 bg-background text-muted-foreground">
                <Bot className="h-4 w-4" />
              </div>
              <div className="min-w-0">
                <p className="text-sm font-medium">
                  Activate {agentName} to get a response
                </p>
                <p className="mt-1 text-sm text-muted-foreground">
                  Your message was sent, but this agent is not active in this
                  chat yet.
                </p>
              </div>
            </div>
            <Button
              className="shrink-0 self-start sm:self-auto"
              disabled={isActivating}
              onClick={onActivate}
              size="sm"
              type="button"
              variant="secondary"
            >
              <Power className="h-4 w-4" />
              {isActivating ? "Activating" : "Activate"}
            </Button>
          </div>
        </div>
      </MessageContent>
    </Message>
  );
}
