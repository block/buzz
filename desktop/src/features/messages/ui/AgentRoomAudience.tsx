import * as React from "react";
import { Bot, Check, ChevronDown, Sparkles } from "lucide-react";

import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import type { UseRichTextEditorResult } from "@/features/messages/lib/useRichTextEditor";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { AgentRoomAudienceAgent } from "@/features/messages/ui/MessageComposer.types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export const AGENT_ROOM_ROUNDTABLE_PROMPT =
  "Roundtable: give independent recommendations grounded in this conversation. Cite the evidence you used, flag assumptions, and disagree where needed. ";

export function getUnaddressedAgentRoomAgents(
  agents: readonly AgentRoomAudienceAgent[],
  addressedPubkeys: Iterable<string>,
): AgentRoomAudienceAgent[] {
  const addressed = new Set([...addressedPubkeys].map(normalizePubkey));
  const seen = new Set<string>();
  return agents.filter((agent) => {
    const pubkey = normalizePubkey(agent.pubkey);
    if (addressed.has(pubkey) || seen.has(pubkey)) return false;
    seen.add(pubkey);
    return true;
  });
}

export function getReadyAgentRoomAgents(
  agents: readonly AgentRoomAudienceAgent[],
): AgentRoomAudienceAgent[] {
  return agents.filter(
    (agent) => agent.status === "running" || agent.status === "deployed",
  );
}

export function AgentRoomAudience({
  agents,
  disabled,
  mentions,
  profiles,
  richText,
}: {
  agents: readonly AgentRoomAudienceAgent[];
  disabled: boolean;
  mentions: UseMentionsResult;
  profiles?: UserProfileLookup;
  richText: UseRichTextEditorResult;
}) {
  const uniqueAgents = React.useMemo(
    () => getUnaddressedAgentRoomAgents(agents, []),
    [agents],
  );
  const readyAgents = React.useMemo(
    () => getReadyAgentRoomAgents(uniqueAgents),
    [uniqueAgents],
  );
  const currentText = richText.getPlainTextAndCursor().text;
  const addressedPubkeys = new Set(
    mentions.extractMentionPubkeys(currentText).map(normalizePubkey),
  );
  const unaddressedAgents = getUnaddressedAgentRoomAgents(
    readyAgents,
    addressedPubkeys,
  );
  const addressedAgentCount = readyAgents.length - unaddressedAgents.length;

  const roundtablePromptAdded = currentText.includes(
    AGENT_ROOM_ROUNDTABLE_PROMPT.trim(),
  );

  const addAgents = React.useCallback(
    (targets: readonly AgentRoomAudienceAgent[], trailingText: string = "") => {
      const current = richText.getPlainTextAndCursor();
      const missing = getUnaddressedAgentRoomAgents(
        targets,
        mentions.extractMentionPubkeys(current.text),
      );
      const shouldAddTrailingText =
        trailingText.length > 0 && !current.text.includes(trailingText.trim());
      if (missing.length === 0 && !shouldAddTrailingText) return;

      let cursor = current.cursor;
      if (cursor > 0 && !/\s/.test(current.text[cursor - 1] ?? "")) {
        richText.replacePlainTextRange(cursor, cursor, " ");
        cursor += 1;
      }
      for (const agent of missing) {
        const edit = mentions.insertResolvedMention({
          displayName: agent.name,
          pubkey: agent.pubkey,
          replaceFromOffset: cursor,
          replaceToOffset: cursor,
          isAgent: true,
        });
        richText.replacePlainTextRange(
          edit.replaceFromOffset,
          edit.replaceToOffset,
          edit.insertText,
        );
        cursor += edit.insertText.length;
      }
      if (shouldAddTrailingText) {
        richText.replacePlainTextRange(cursor, cursor, trailingText);
      }
      mentions.cancelMentionAutocomplete();
      richText.focusPreserve();
    },
    [mentions, richText],
  );

  if (uniqueAgents.length === 0) return null;

  return (
    <div
      className="-mx-1 mb-2 flex items-center justify-between gap-2 border-b border-border/40 px-1 pb-2"
      data-testid="agent-room-audience"
    >
      <Popover>
        <PopoverTrigger asChild>
          <Button
            className="h-7 min-w-0 gap-2 rounded-full px-2 text-xs"
            data-testid="agent-room-picker"
            disabled={disabled}
            size="sm"
            type="button"
            variant="ghost"
          >
            <Bot className="text-primary" />
            <span className="font-semibold">Agent room</span>
            <span className="flex items-center -space-x-1.5">
              {uniqueAgents.slice(0, 3).map((agent) => (
                <UserAvatar
                  accent
                  avatarUrl={
                    profiles?.[normalizePubkey(agent.pubkey)]?.avatarUrl ?? null
                  }
                  className="!h-4 !w-4 border border-background text-3xs"
                  displayName={agent.name}
                  fallbackDelayMs={0}
                  key={agent.pubkey}
                  size="xs"
                />
              ))}
            </span>
            <span className="hidden text-muted-foreground sm:inline">
              {addressedAgentCount > 0
                ? `${addressedAgentCount} addressed`
                : `${readyAgents.length} ready`}
            </span>
            <ChevronDown className="text-muted-foreground" />
          </Button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          className="w-72 p-1.5"
          onOpenAutoFocus={(event) => event.preventDefault()}
          side="top"
          sideOffset={8}
        >
          <div className="px-2 pb-2 pt-1">
            <p className="text-sm font-semibold">Direct the room</p>
            <p className="mt-0.5 text-xs text-muted-foreground">
              Everyone shares the channel context and replies in their own
              voice.
            </p>
          </div>
          <div className="flex flex-col gap-1">
            {uniqueAgents.map((agent) => {
              const pubkey = normalizePubkey(agent.pubkey);
              const isAddressed = addressedPubkeys.has(pubkey);
              const isReady =
                agent.status === "running" || agent.status === "deployed";
              return (
                <button
                  aria-label={
                    isAddressed
                      ? `${agent.name} is addressed`
                      : `Address ${agent.name}`
                  }
                  className="flex w-full items-center gap-2 rounded-lg px-2 py-2 text-left text-sm transition-colors hover:bg-accent focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-70"
                  data-testid={`agent-room-agent-${pubkey}`}
                  disabled={disabled || isAddressed || !isReady}
                  key={pubkey}
                  onClick={() => addAgents([agent])}
                  type="button"
                >
                  <UserAvatar
                    accent
                    avatarUrl={profiles?.[pubkey]?.avatarUrl ?? null}
                    className="shrink-0"
                    displayName={agent.name}
                    fallbackDelayMs={0}
                    size="sm"
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium">
                      {agent.name}
                    </span>
                    <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
                      <span
                        className={
                          isReady
                            ? "size-1.5 rounded-full bg-emerald-500"
                            : "size-1.5 rounded-full bg-muted-foreground/40"
                        }
                      />
                      {isReady ? "Ready" : "Offline"}
                    </span>
                  </span>
                  {isAddressed ? (
                    <Check className="text-primary" />
                  ) : isReady ? (
                    <span className="text-xs font-medium text-primary">
                      Add
                    </span>
                  ) : (
                    <span className="text-xs text-muted-foreground">
                      Unavailable
                    </span>
                  )}
                </button>
              );
            })}
          </div>
          <div className="mt-1 border-t border-border/50 p-1 pt-2">
            <Button
              className="h-auto w-full justify-start gap-2 px-2 py-2 text-left"
              data-testid="agent-room-roundtable"
              disabled={
                disabled || readyAgents.length === 0 || roundtablePromptAdded
              }
              onClick={() =>
                addAgents(readyAgents, AGENT_ROOM_ROUNDTABLE_PROMPT)
              }
              size="sm"
              type="button"
              variant="secondary"
            >
              {roundtablePromptAdded ? <Check /> : <Sparkles />}
              <span>
                <span className="block text-xs font-semibold">
                  {roundtablePromptAdded
                    ? "Roundtable ready"
                    : "Start roundtable"}
                </span>
                <span className="block text-2xs font-normal text-muted-foreground">
                  Independent, evidence-backed recommendations
                </span>
              </span>
            </Button>
          </div>
          <p className="px-2 pb-1 pt-1 text-xs text-muted-foreground">
            Signed replies stay separate. Ask one agent to synthesize, then mark
            the thread Done to save it with its sources.
          </p>
        </PopoverContent>
      </Popover>

      <Button
        className="h-7 shrink-0 rounded-full px-2.5 text-xs"
        data-testid="agent-room-ask-all"
        disabled={disabled || unaddressedAgents.length === 0}
        onClick={() => addAgents(readyAgents)}
        onMouseDown={(event) => event.preventDefault()}
        size="sm"
        type="button"
        variant={unaddressedAgents.length > 0 ? "secondary" : "ghost"}
      >
        {unaddressedAgents.length > 0 ? <Sparkles /> : <Check />}
        {unaddressedAgents.length > 0 ? "Ask all" : "All addressed"}
      </Button>
    </div>
  );
}
