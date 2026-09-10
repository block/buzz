import "./conversation.css";
import type { ReactNode } from "react";
import { Avatar } from "@/shared/ui/Avatar";
import { Button } from "@/shared/ui/Button";

/** Resolved shared message presentation; no name lookup, sending or persistence. */
export type ConversationMessagePresentation = {
  id: string;
  author: {
    name: string;
    fallback: string;
    avatar?: string;
    kind: "human" | "agent";
  };
  time: string;
  dateTime: string;
  content: ReactNode;
  reply?: { author: string; excerpt: string; onOpen: () => void };
  delivery?: { label: string; failed?: boolean; onRetry?: () => void };
  actions?: ReactNode;
};

/** A shared contribution stays readable independently of private agent work. */
export function ConversationMessage({
  message,
}: {
  message: ConversationMessagePresentation;
}) {
  return (
    <article
      className="conversation-message"
      aria-label={`${message.author.name}, ${message.time}`}
      data-message-id={message.id}
    >
      <span aria-hidden="true">
        <Avatar
          src={message.author.avatar}
          alt=""
          fallback={message.author.fallback}
        />
      </span>
      <div className="conversation-message-body">
        <header className="conversation-message-heading">
          <span className="text-body font-semibold text-primary">
            {message.author.name}
          </span>
          {message.author.kind === "agent" ? (
            <span className="text-body-sm text-tertiary">Agent</span>
          ) : null}
          <time
            className="text-body-sm text-tertiary"
            dateTime={message.dateTime}
          >
            {message.time}
          </time>
        </header>
        {message.reply ? (
          <div className="conversation-reply">
            <Button
              variant="ghost"
              size="compact"
              onClick={message.reply.onOpen}
            >
              Replying to {message.reply.author}: {message.reply.excerpt}
            </Button>
          </div>
        ) : null}
        <div className="conversation-message-content text-body text-primary">
          {message.content}
        </div>
        {message.delivery ? (
          <div
            className="conversation-delivery text-body-sm"
            data-failed={message.delivery.failed || undefined}
            role="status"
          >
            <span>{message.delivery.label}</span>
            {message.delivery.onRetry ? (
              <Button
                size="compact"
                variant="ghost"
                onClick={message.delivery.onRetry}
              >
                Retry send
              </Button>
            ) : null}
          </div>
        ) : null}
        {message.actions ? (
          <div className="conversation-message-actions">{message.actions}</div>
        ) : null}
      </div>
    </article>
  );
}
