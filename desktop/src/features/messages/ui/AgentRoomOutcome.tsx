import * as React from "react";
import { Check, FileCheck2, Loader2 } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { agentMemoryQueryKey } from "@/features/agent-memory/hooks";
import type { TimelineMessage } from "@/features/messages/types";
import { saveThreadOutcomeMemory } from "@/shared/api/tauriEngrams";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import type { AgentRoomAudienceAgent } from "./MessageComposer.types";

export type AgentSourceReceipt = {
  label: string;
  location: string;
};

function addSource(
  sources: AgentSourceReceipt[],
  seen: Set<string>,
  location: string,
  label = location,
) {
  const cleanLocation = location.trim();
  if (!cleanLocation || seen.has(cleanLocation)) return;
  seen.add(cleanLocation);
  sources.push({
    label: label.trim() || cleanLocation,
    location: cleanLocation,
  });
}

/** Extract the citation shapes Buzz already asks agents to publish. */
export function extractAgentSourceReceipt(
  message: Pick<TimelineMessage, "body" | "tags">,
): AgentSourceReceipt[] {
  const sources: AgentSourceReceipt[] = [];
  const seen = new Set<string>();

  for (const tag of message.tags ?? []) {
    if (tag[0] === "source" && tag[1]) {
      addSource(sources, seen, tag[1], tag[2]);
    }
  }

  // ponytail: parse the citation forms in Buzz's agent prompt; use a Markdown
  // AST only if nested-link syntax becomes a real source format.
  const markdownLinks =
    /(?<!!)\[([^\]]+)]\((https?:\/\/[^\s)]+|(?:\.{0,2}\/)[^\s)]+)\)/g;
  for (const match of message.body.matchAll(markdownLinks)) {
    addSource(sources, seen, match[2] ?? "", match[1]);
  }

  const bareUrls = /https?:\/\/[^\s<>)\]]+/g;
  for (const match of message.body.matchAll(bareUrls)) {
    addSource(sources, seen, (match[0] ?? "").replace(/[.,;:!?]+$/, ""));
  }

  const citedPaths = /`((?:\/|\.{1,2}\/|[\w.-]+\/)[^`\n]+)`/g;
  for (const match of message.body.matchAll(citedPaths)) {
    addSource(sources, seen, match[1] ?? "");
  }

  return sources;
}

function uniqueSources(messages: readonly TimelineMessage[]) {
  const sources: AgentSourceReceipt[] = [];
  const seen = new Set<string>();
  for (const message of messages) {
    for (const source of extractAgentSourceReceipt(message)) {
      addSource(sources, seen, source.location, source.label);
    }
  }
  return sources;
}

export function getThreadMemoryTargets(
  agents: readonly AgentRoomAudienceAgent[],
  initialAgentPubkeys: readonly string[],
  messages: readonly TimelineMessage[],
): AgentRoomAudienceAgent[] {
  const participants = new Set(initialAgentPubkeys.map(normalizePubkey));
  for (const message of messages) {
    if (message.isAgent && message.pubkey) {
      participants.add(normalizePubkey(message.pubkey));
    }
  }

  return agents.filter(
    (agent) =>
      agent.agentSource === "managed" &&
      participants.has(normalizePubkey(agent.pubkey)),
  );
}

export function findLatestAgentOutcome(
  messages: readonly TimelineMessage[],
): TimelineMessage | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message?.isAgent && message.body.trim()) return message;
  }
  return null;
}

export function buildThreadMemoryBody({
  channelId,
  channelName,
  outcome,
  sources,
  threadHead,
}: {
  channelId: string;
  channelName: string;
  outcome: TimelineMessage;
  sources: readonly AgentSourceReceipt[];
  threadHead: TimelineMessage;
}): string {
  const sourceLines =
    sources.length > 0
      ? sources
          .map((source) =>
            source.label === source.location
              ? `- ${source.location}`
              : `- ${source.label}: ${source.location}`,
          )
          .join("\n")
      : "- No sources cited";

  return `# Thread outcome

Conversation: buzz://message?channel=${encodeURIComponent(channelId)}&id=${threadHead.id}
Channel: #${channelName}
Saved from: ${outcome.author} (${outcome.id})

## Context

${threadHead.body.trim()}

## Agreed outcome

${outcome.body.trim()}

## Source receipt

${sourceLines}`;
}

export function AgentMessageSourceReceipt({
  message,
}: {
  message: TimelineMessage;
}) {
  const sources = extractAgentSourceReceipt(message);
  if (!message.isAgent || message.pending) return null;

  if (sources.length === 0) {
    return (
      <p
        className="mt-1.5 flex items-center gap-1.5 text-2xs text-muted-foreground/75"
        data-testid="agent-source-receipt-empty"
      >
        <FileCheck2 className="size-3" />
        Source receipt · none cited
      </p>
    );
  }

  return (
    <details
      className="group mt-1.5 text-2xs text-muted-foreground"
      data-testid="agent-source-receipt"
    >
      <summary className="flex w-fit cursor-pointer list-none items-center gap-1.5 rounded focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring">
        <FileCheck2 className="size-3 text-emerald-600 dark:text-emerald-400" />
        Context used · {sources.length}{" "}
        {sources.length === 1 ? "source" : "sources"}
      </summary>
      <ul className="ml-4 mt-1 space-y-1 border-l border-border/60 pl-3">
        {sources.map((source) => (
          <li className="min-w-0" key={source.location}>
            <span className="font-medium text-foreground/80">
              {source.label}
            </span>
            {source.label !== source.location ? (
              <span className="ml-1 break-all">{source.location}</span>
            ) : null}
          </li>
        ))}
      </ul>
    </details>
  );
}

export function ThreadDoneToMemoryButton({
  agents,
  channelId,
  channelName,
  initialAgentPubkeys,
  threadHead,
  threadMessages,
}: {
  agents: readonly AgentRoomAudienceAgent[];
  channelId: string;
  channelName: string;
  initialAgentPubkeys: readonly string[];
  threadHead: TimelineMessage;
  threadMessages: readonly TimelineMessage[];
}) {
  const queryClient = useQueryClient();
  const [state, setState] = React.useState<"idle" | "saving" | "saved">("idle");
  const messages = React.useMemo(
    () => [threadHead, ...threadMessages],
    [threadHead, threadMessages],
  );
  const outcome = findLatestAgentOutcome(messages);
  const targets = getThreadMemoryTargets(agents, initialAgentPubkeys, messages);
  const canSave =
    /^[0-9a-f]{64}$/i.test(threadHead.id) && outcome && targets.length > 0;

  const handleDone = async () => {
    if (!canSave || !outcome || state !== "idle") return;
    setState("saving");
    const body = buildThreadMemoryBody({
      channelId,
      channelName,
      outcome,
      sources: uniqueSources(messages),
      threadHead,
    });
    const results = await Promise.allSettled(
      targets.map((agent) =>
        saveThreadOutcomeMemory(agent.pubkey, threadHead.id, body),
      ),
    );
    const savedTargets = targets.filter(
      (_, index) => results[index]?.status === "fulfilled",
    );
    const failedTargets = targets.filter(
      (_, index) => results[index]?.status === "rejected",
    );

    for (const agent of savedTargets) {
      void queryClient.invalidateQueries({
        queryKey: agentMemoryQueryKey(agent.pubkey),
      });
    }

    if (failedTargets.length > 0) {
      setState("idle");
      toast.error(
        `Saved to ${savedTargets.length} of ${targets.length} agents. Couldn't update ${new Intl.ListFormat().format(failedTargets.map((agent) => agent.name))}.`,
      );
      return;
    }

    setState("saved");
    toast.success(
      `Saved this outcome and its source receipt to ${new Intl.ListFormat().format(savedTargets.map((agent) => agent.name))}.`,
    );
  };

  if (targets.length === 0) return null;

  return (
    <Button
      aria-label="Save thread outcome to agent memory"
      className="h-7 gap-1.5 px-2 text-xs"
      data-testid="thread-done-to-memory"
      disabled={!canSave || state !== "idle"}
      onClick={() => void handleDone()}
      title={
        outcome
          ? "Save the latest agent outcome and source receipt to participating agents"
          : "Ask an agent to synthesize the outcome first"
      }
      type="button"
      variant={state === "saved" ? "ghost" : "outline"}
    >
      {state === "saving" ? <Loader2 className="animate-spin" /> : <Check />}
      {state === "saved" ? "Saved" : state === "saving" ? "Saving…" : "Done"}
    </Button>
  );
}
