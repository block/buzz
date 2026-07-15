import { ArrowUp } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import type { OnboardingAgent } from "../agents";
import {
  type ChatMessage,
  FIRST_CHAT_PROMPT,
  requestAgentReply,
} from "../firstChat";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import type { FirstChatStepActions } from "./types";

/**
 * Onboarding first-chat (step 5).
 *
 * The user sends one (forced-haiku) prompt to the agent they picked in step 3
 * and receives a reply, proving the chat works. The reply is scripted for now
 * (see firstChat.ts `requestAgentReply` seam). Once a reply lands, a Continue
 * affordance appears to finish onboarding.
 */
export function FirstChatStep({
  actions,
  agent,
  direction,
}: {
  actions: FirstChatStepActions;
  agent: OnboardingAgent;
  direction: OnboardingTransitionDirection;
}) {
  const [draft, setDraft] = React.useState(FIRST_CHAT_PROMPT);
  const [messages, setMessages] = React.useState<ChatMessage[]>([]);
  const [isAwaitingReply, setIsAwaitingReply] = React.useState(false);
  const [replyError, setReplyError] = React.useState(false);
  const hasReply = messages.some((m) => m.role === "agent");
  const cancelRef = React.useRef<(() => void) | null>(null);

  React.useEffect(() => {
    return () => cancelRef.current?.();
  }, []);

  const send = React.useCallback(() => {
    const text = draft.trim();
    if (text.length === 0 || isAwaitingReply || hasReply) {
      return;
    }
    setReplyError(false);
    setMessages((prev) => [
      ...prev,
      { id: `user-${Date.now()}`, role: "user", text, at: Date.now() },
    ]);
    setDraft("");
    setIsAwaitingReply(true);

    const { promise, cancel } = requestAgentReply(agent, text);
    cancelRef.current = cancel;
    promise
      .then((reply) => {
        setMessages((prev) => [
          ...prev,
          {
            id: `agent-${Date.now()}`,
            role: "agent",
            text: reply,
            at: Date.now(),
          },
        ]);
      })
      .catch(() => {
        setReplyError(true);
      })
      .finally(() => {
        setIsAwaitingReply(false);
        cancelRef.current = null;
      });
  }, [agent, draft, hasReply, isAwaitingReply]);

  return (
    <OnboardingSlideTransition
      className="flex w-full flex-col items-center text-center"
      direction={direction}
      transitionKey={`first-chat-${direction}`}
    >
      <div className="flex w-full max-w-[640px] flex-col items-center">
        <h1 className="text-3xl font-semibold tracking-tight">
          {hasReply
            ? `Nice — you met ${agent.name}`
            : "Say hello to your agent"}
        </h1>
        <p className="mt-3 text-base leading-6 text-muted-foreground">
          {hasReply
            ? "That's it — you can keep chatting or finish setting up."
            : `Send your first message to ${agent.name}. Anything you send here goes straight to your agent.`}
        </p>

        {/* Conversation card */}
        <div className="mt-8 flex w-full flex-col overflow-hidden rounded-xl border border-border/60 bg-card">
          <div className="flex min-h-[16rem] flex-col gap-4 p-5">
            {messages.length === 0 ? (
              <div className="flex flex-1 flex-col items-center justify-center gap-3 text-center">
                <img
                  src={agent.avatarUrl}
                  alt=""
                  className="h-14 w-14 rounded-full object-cover"
                />
                <p className="text-sm text-muted-foreground">
                  No messages yet — start the conversation below.
                </p>
              </div>
            ) : (
              <div className="flex flex-col gap-4">
                {messages.map((message) => (
                  <ChatBubble
                    key={message.id}
                    message={message}
                    agent={agent}
                  />
                ))}
                {isAwaitingReply ? <TypingIndicator agent={agent} /> : null}
                {replyError ? (
                  <p className="text-sm text-destructive">
                    {agent.name} didn't respond. You can try again or skip for
                    now.
                  </p>
                ) : null}
              </div>
            )}
          </div>

          {/* Composer */}
          <div className="flex items-center gap-2 border-t border-border/60 p-3">
            <input
              type="text"
              value={draft}
              disabled={isAwaitingReply || hasReply}
              placeholder={`Message ${agent.name}…`}
              data-testid="onboarding-first-chat-input"
              onChange={(event) => setDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.shiftKey) {
                  event.preventDefault();
                  send();
                }
              }}
              className="flex-1 bg-transparent px-2 text-base outline-none placeholder:text-muted-foreground disabled:opacity-60"
            />
            <Button
              size="icon"
              disabled={
                draft.trim().length === 0 || isAwaitingReply || hasReply
              }
              data-testid="onboarding-first-chat-send"
              onClick={send}
              aria-label="Send message"
            >
              <ArrowUp className="h-4 w-4" strokeWidth={2.5} />
            </Button>
          </div>
        </div>

        {/* Footer */}
        <div className="mt-8 flex w-full items-center justify-between">
          <Button
            variant="ghost"
            data-testid="onboarding-back"
            onClick={actions.back}
          >
            Back
          </Button>
          {hasReply ? (
            <Button data-testid="onboarding-next" onClick={actions.submit}>
              Continue
            </Button>
          ) : (
            <Button
              variant="ghost"
              data-testid="onboarding-skip"
              onClick={actions.submit}
            >
              Skip for now
            </Button>
          )}
        </div>
      </div>
    </OnboardingSlideTransition>
  );
}

function ChatBubble({
  message,
  agent,
}: {
  message: ChatMessage;
  agent: OnboardingAgent;
}) {
  const isUser = message.role === "user";
  return (
    <div
      className={cn(
        "flex items-start gap-2",
        isUser ? "flex-row-reverse" : "flex-row",
      )}
    >
      {isUser ? null : (
        <img
          src={agent.avatarUrl}
          alt=""
          className="mt-0.5 h-8 w-8 shrink-0 rounded-full object-cover"
        />
      )}
      <div
        className={cn(
          "max-w-[80%] whitespace-pre-line rounded-2xl px-3.5 py-2 text-left text-base",
          isUser
            ? "bg-primary/15 text-foreground"
            : "border border-border/60 bg-background text-foreground",
        )}
      >
        {message.text}
      </div>
    </div>
  );
}

function TypingIndicator({ agent }: { agent: OnboardingAgent }) {
  return (
    <div
      className="flex items-center gap-2"
      data-testid="onboarding-first-chat-typing"
    >
      <img
        src={agent.avatarUrl}
        alt=""
        className="h-8 w-8 shrink-0 rounded-full object-cover"
      />
      <div className="flex gap-1 rounded-2xl border border-border/60 bg-background px-3.5 py-3">
        <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-muted-foreground [animation-delay:-0.3s]" />
        <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-muted-foreground [animation-delay:-0.15s]" />
        <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-muted-foreground" />
      </div>
    </div>
  );
}
