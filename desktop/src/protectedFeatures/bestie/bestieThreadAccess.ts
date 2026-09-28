/**
 * Ensure the Assistant agent can read a channel thread (ACL).
 *
 * Research (see BESTIE_THREADS.md):
 * - Open channels: agent can read without membership.
 * - Private channels: membership required → invite via addChannelMembers.
 */

import { canAddChannelMembers } from "@/features/channels/lib/channelMemberAdmission";
import {
  addChannelMembers,
  getChannelDetails,
  getChannelMembers,
} from "@/shared/api/tauri";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type BestieThreadAccessResult =
  | { ok: true; invited: boolean; visibility: string }
  | { ok: false; error: string };

export async function ensureBestieAgentThreadAccess(input: {
  agentPubkey: string;
  channelId: string;
  /** Signed-in user pubkey — used to resolve selfRole for private invites. */
  ownerPubkey: string;
}): Promise<BestieThreadAccessResult> {
  const channelId = input.channelId.trim();
  const agentPubkey = normalizePubkey(input.agentPubkey);
  const ownerPubkey = normalizePubkey(input.ownerPubkey);
  if (!channelId || !agentPubkey || !ownerPubkey) {
    return { ok: false, error: "Missing channel, Assistant agent, or identity." };
  }

  let detail;
  try {
    detail = await getChannelDetails(channelId);
  } catch (error) {
    return {
      ok: false,
      error:
        error instanceof Error
          ? error.message
          : "Could not load channel details.",
    };
  }

  if (detail.channelType === "dm") {
    return { ok: false, error: "Pick a channel thread, not a DM." };
  }

  const visibility = detail.visibility ?? "private";

  let members;
  try {
    members = await getChannelMembers(channelId);
  } catch (error) {
    return {
      ok: false,
      error:
        error instanceof Error
          ? error.message
          : "Could not load channel members.",
    };
  }

  const agentIsMember = members.some(
    (member) => normalizePubkey(member.pubkey) === agentPubkey,
  );
  if (agentIsMember) {
    return { ok: true, invited: false, visibility };
  }

  // Open: accessible-channel set includes all open channels — no invite.
  if (visibility === "open") {
    return { ok: true, invited: false, visibility };
  }

  const selfRole =
    members.find((member) => normalizePubkey(member.pubkey) === ownerPubkey)
      ?.role ?? null;

  if (
    !canAddChannelMembers({
      channelType: detail.channelType,
      visibility,
      selfRole,
    })
  ) {
    return {
      ok: false,
      error: "Only channel members can add Assistant to a private channel.",
    };
  }

  try {
    const result = await addChannelMembers({
      channelId,
      pubkeys: [agentPubkey],
    });
    const inviteError = result.errors.find(
      (entry) => normalizePubkey(entry.pubkey) === agentPubkey,
    );
    if (inviteError) {
      return { ok: false, error: inviteError.error };
    }
    const invited = result.added.some(
      (pubkey) => normalizePubkey(pubkey) === agentPubkey,
    );
    return { ok: true, invited, visibility };
  } catch (error) {
    return {
      ok: false,
      error:
        error instanceof Error
          ? error.message
          : "Failed to invite Assistant to the channel.",
    };
  }
}
