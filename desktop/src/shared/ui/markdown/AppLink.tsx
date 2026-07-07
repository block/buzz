import type * as React from "react";

import {
  parseChatLink,
  resolveChatLinkRenderTarget,
} from "@/features/chats/lib/chatLink";
import {
  parseMessageLink,
  resolveMessageLinkRenderTarget,
} from "@/features/messages/lib/messageLink";

import { ChatLinkCard } from "./ChatLinkCard";
import { MessageLinkPill } from "./MessageLinkPill";
import type { MarkdownRuntime } from "./types";

const APP_LINK_CLASS =
  "font-medium text-primary underline underline-offset-4 transition-colors hover:text-primary/80 cursor-pointer";

type RuntimeRef = React.RefObject<MarkdownRuntime>;

type RenderMarkdownAppLinkInput = {
  children: React.ReactNode;
  href: string;
  interactive: boolean;
  label: string;
  props: React.ComponentProps<"a">;
  runtimeRef: RuntimeRef;
};

export function renderMarkdownAppLink({
  children,
  href,
  interactive,
  label,
  props,
  runtimeRef,
}: RenderMarkdownAppLinkInput) {
  const { channels, onOpenChatLink, onOpenMessageLink } = runtimeRef.current;
  const chatLinkTarget = resolveChatLinkRenderTarget({ href, label });
  if (chatLinkTarget.kind !== "none") {
    if (chatLinkTarget.kind === "card") {
      return (
        <ChatLinkCard
          channels={channels}
          href={href}
          interactive={interactive}
          link={chatLinkTarget.link}
          onOpenChatLink={onOpenChatLink}
        />
      );
    }

    return (
      <a
        {...props}
        className={APP_LINK_CLASS}
        href={href}
        onClick={(event) => {
          event.preventDefault();
          onOpenChatLink(chatLinkTarget.link);
        }}
      >
        {children}
      </a>
    );
  }

  const messageLinkTarget = resolveMessageLinkRenderTarget({ href, label });
  if (messageLinkTarget.kind === "none") {
    return null;
  }
  if (messageLinkTarget.kind === "pill") {
    return (
      <MessageLinkPill
        channels={channels}
        href={href}
        interactive={interactive}
        link={messageLinkTarget.link}
        onOpenMessageLink={onOpenMessageLink}
      />
    );
  }

  return (
    <a
      {...props}
      className={APP_LINK_CLASS}
      href={href}
      onClick={(event) => {
        event.preventDefault();
        onOpenMessageLink(messageLinkTarget.link);
      }}
    >
      {children}
    </a>
  );
}

export function MarkdownChatLinkNode({
  children,
  interactive,
  runtimeRef,
}: {
  children?: React.ReactNode;
  interactive: boolean;
  runtimeRef: RuntimeRef;
}) {
  const { channels, onOpenChatLink } = runtimeRef.current;
  const href = String(children ?? "");
  const parsed = parseChatLink(href);
  if (!parsed.ok) {
    return <span data-chat-link="">{href}</span>;
  }

  return (
    <ChatLinkCard
      channels={channels}
      href={href}
      interactive={interactive}
      link={parsed.value}
      onOpenChatLink={onOpenChatLink}
    />
  );
}

export function MarkdownMessageLinkNode({
  children,
  interactive,
  runtimeRef,
}: {
  children?: React.ReactNode;
  interactive: boolean;
  runtimeRef: RuntimeRef;
}) {
  const { channels, onOpenMessageLink } = runtimeRef.current;
  const href = String(children ?? "");
  const parsed = parseMessageLink(href);
  if (!parsed.ok) {
    return <span data-message-link="">{href}</span>;
  }

  return (
    <MessageLinkPill
      channels={channels}
      href={href}
      interactive={interactive}
      link={parsed.value}
      onOpenMessageLink={onOpenMessageLink}
    />
  );
}
