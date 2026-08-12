/**
 * Wrapper that handles the current-viewer identity check for the
 * `PermissionRequestCard`, keeping React hooks out of the memo-heavy
 * `MessageRow` component.
 *
 * Renders nothing when `computePermissionRequest` returns null (no trusted
 * sentinel, wrong signer, or non-interactive surface).
 */
import * as React from "react";

import { useIdentityQuery } from "@/shared/api/hooks";
import { computePermissionRequest } from "@/shared/lib/computePermissionRequest";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { AttachmentGroup } from "@/shared/ui/attachment";
import { PermissionRequestCard } from "@/shared/ui/permission-request-card";

export type PermissionRequestCardBlockProps = {
  /** Message body content — may contain the sentinel or may not. */
  content: string;
  /** Whether this is an interactive render surface. Non-interactive → no card. */
  interactive: boolean;
  /**
   * Normalized hex pubkey of the known agent that signed the kind-9 event.
   * Undefined → card disabled (no agent signer on this message).
   */
  agentPubkey: string | undefined;
  /** Raw signer pubkey from the signed event envelope. */
  signerPubkey: string | undefined;
  /**
   * Signer pubkey of the most recent authorized kind-40003 edit, if any.
   * Used to enforce edit authenticity: only agent-signed edits resolve the card.
   */
  editSignerPubkey?: string;
  /** Verified owner pubkey for the agent (from the agent's profile). */
  ownerPubkey?: string | null;
  /** Channel ID for routing the permission decision click. */
  channelId: string;
};

export const PermissionRequestCardBlock = React.memo(
  function PermissionRequestCardBlock({
    content,
    interactive,
    agentPubkey,
    signerPubkey,
    editSignerPubkey,
    ownerPubkey,
    channelId,
  }: PermissionRequestCardBlockProps) {
    const identityQuery = useIdentityQuery();
    const viewerPubkey = identityQuery.data?.pubkey;

    const request = computePermissionRequest(
      content,
      interactive,
      agentPubkey,
      signerPubkey,
      editSignerPubkey,
    );

    if (request === null || !agentPubkey) return null;

    const isOwner =
      !!viewerPubkey &&
      !!ownerPubkey &&
      normalizePubkey(viewerPubkey) === normalizePubkey(ownerPubkey);

    return (
      <AttachmentGroup
        className="max-w-full flex-wrap overflow-visible pb-0"
        data-permission-request=""
      >
        <PermissionRequestCard
          agentPubkey={agentPubkey}
          channelId={channelId}
          isOwner={isOwner}
          request={request}
        />
      </AttachmentGroup>
    );
  },
  (prev, next) =>
    prev.content === next.content &&
    prev.interactive === next.interactive &&
    prev.agentPubkey === next.agentPubkey &&
    prev.signerPubkey === next.signerPubkey &&
    prev.editSignerPubkey === next.editSignerPubkey &&
    prev.ownerPubkey === next.ownerPubkey &&
    prev.channelId === next.channelId,
);
