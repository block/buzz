/**
 * Wrapper that handles the current-viewer identity check for the
 * `PermissionRequestCard`, keeping React hooks out of the memo-heavy
 * `MessageRow` component.
 *
 * Accepts a pre-computed `permReq` from `selectPermissionRequest` — the trust
 * computation happens once in `MessageRow` and the result is passed down here,
 * so prose is suppressed iff the card renders, by construction.
 *
 * Renders nothing when `permReq` is null (no trusted sentinel, wrong signer,
 * falsy channelId, or non-interactive surface).
 */
import * as React from "react";

import type { startPermissionDecisionDelivery } from "@/features/agents/lib/permissionDecisionDelivery";
import type { TimelineMessage } from "@/features/messages/types";
import type { PermissionRequestSelection } from "@/features/messages/ui/permissionRequestAuthPubkey";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { AttachmentGroup } from "@/shared/ui/attachment";
import { PermissionRequestCard } from "@/shared/ui/permission-request-card";

export type PermissionRequestCardBlockProps = {
  /** The message that may carry a permission-request sentinel. */
  message: Pick<TimelineMessage, "ownerPubkey">;
  /**
   * Pre-computed selection from `selectPermissionRequest`. When null, no card
   * renders. MessageRow computes this once and uses the same result for prose
   * suppression and this block — ensuring they stay in sync.
   */
  permReq: PermissionRequestSelection | null;
  /**
   * Delivery function injected by tests to control the outcome without a real
   * relay. Production callers omit this.
   *
   * @internal — test seam only; not part of the public API.
   */
  _deliveryFn?: typeof startPermissionDecisionDelivery;
  /** Channel ID for routing the decision click. */
  channelId: string | null | undefined;
};

export const PermissionRequestCardBlock = React.memo(
  function PermissionRequestCardBlock({
    message,
    permReq,
    channelId,
    _deliveryFn,
  }: PermissionRequestCardBlockProps) {
    const identityQuery = useIdentityQuery();
    const viewerPubkey = identityQuery.data?.pubkey;

    if (permReq === null || !channelId) return null;

    const { agentPubkey, request } = permReq;
    const ownerPubkey = message.ownerPubkey;
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
          _deliveryFn={_deliveryFn}
        />
      </AttachmentGroup>
    );
  },
  (prev, next) =>
    prev.message === next.message &&
    prev.permReq === next.permReq &&
    prev.channelId === next.channelId &&
    prev._deliveryFn === next._deliveryFn,
);
