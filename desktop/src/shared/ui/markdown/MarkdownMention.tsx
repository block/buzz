import type * as React from "react";
import { AgentManagementMarker } from "@/features/agents/ui/OtherSetupAgentMarker";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { cn } from "@/shared/lib/cn";
import { WRAPPING_INLINE_CHIP_CLASSES } from "@/shared/ui/mentionChip";
import { InlineChip } from "@/shared/ui/InlineChip";
import { useMarkdownRuntime } from "./runtimeContext";

/**
 * Bind interactivity once; names and identity resolution remain runtime inputs.
 * Inert identity attributes let timeline copy restore the sigil and carry the
 * resolved exact key in its HTML flavor without changing display or a11y output.
 */
export function createMarkdownMention(interactive: boolean) {
  return function MarkdownMention({
    children,
  }: {
    children?: React.ReactNode;
  }) {
    const { agentMentionPubkeysByName, mentionPubkeysByName } =
      useMarkdownRuntime();
    const mentionText = String(children ?? "");
    const mentionName = mentionText.replace(/^@/, "").trim().toLowerCase();
    const pubkey = mentionPubkeysByName?.[mentionName];
    // Unbound literal competitors consume their full range, without a chip.
    if (mentionPubkeysByName && !pubkey) return mentionText;
    const isAgentMention =
      pubkey !== undefined &&
      agentMentionPubkeysByName?.[mentionName] === pubkey;
    const mentionLabel = mentionText.replace(/^@/, "");
    // Only chips that actually open a profile get the clickable affordance.
    // A mention whose pubkey didn't resolve stays a plain chip — a pointer
    // cursor there promises a click that does nothing.
    const opensProfile = interactive && pubkey !== undefined;
    const mentionNode = (
      <InlineChip
        data-mention=""
        data-mention-kind={
          pubkey === undefined ? undefined : isAgentMention ? "agent" : "human"
        }
        data-mention-label={mentionLabel}
        data-mention-pubkey={pubkey}
        className={cn(
          WRAPPING_INLINE_CHIP_CLASSES,
          isAgentMention && "agent-mention-highlight",
        )}
        title={mentionLabel}
        aria-label={mentionLabel}
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
  };
}
