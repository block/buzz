import type * as React from "react";
import { AgentManagementMarker } from "@/features/agents/ui/OtherSetupAgentMarker";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { InlineChip } from "@/shared/ui/InlineChip";
import { cn } from "@/shared/lib/cn";
import { useMarkdownRuntime } from "./runtimeContext";

/** Exact-identity mention chip and its shared management provenance. */
export function MarkdownMention({
  children,
  interactive,
}: {
  children?: React.ReactNode;
  interactive: boolean;
}) {
  const { agentMentionPubkeysByName, mentionPubkeysByName } =
    useMarkdownRuntime();
  const mentionText = String(children ?? "");
  const mentionName = mentionText.replace(/^@/, "").trim().toLowerCase();
  const pubkey = mentionPubkeysByName?.[mentionName];
  const isAgentMention =
    pubkey !== undefined && agentMentionPubkeysByName?.[mentionName] === pubkey;
  const mentionLabel = mentionText.replace(/^@/, "");
  // Only chips that actually open a profile get the clickable affordance.
  // A mention whose pubkey didn't resolve stays a plain chip — a pointer
  // cursor there promises a click that does nothing.
  const opensProfile = interactive && pubkey !== undefined;
  const mentionNode = (
    <InlineChip
      data-mention=""
      className={cn(isAgentMention && "agent-mention-highlight")}
      icon={isAgentMention ? "agent" : "human"}
      interactive={opensProfile}
    >
      {mentionLabel}
      {isAgentMention ? <AgentManagementMarker pubkey={pubkey} /> : null}
    </InlineChip>
  );

  return opensProfile ? (
    <UserProfilePopover
      botIdenticonValue={mentionLabel}
      pubkey={pubkey}
      role={isAgentMention ? "bot" : undefined}
      triggerElement="span"
    >
      {mentionNode}
    </UserProfilePopover>
  ) : (
    mentionNode
  );
}
