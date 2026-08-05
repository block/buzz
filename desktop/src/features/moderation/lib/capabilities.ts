/**
 * Relay-role–keyed moderation capability map.
 *
 * Single source of truth for which moderation actions each relay role may
 * perform. Both `MessageModerationMenuItems` and the moderation queue consume
 * this helper so the capability contract is never duplicated across surfaces.
 *
 * Capability grid (plan v3.2):
 *   - owner / admin: all actions (Delete, Kick, Timeout, Untimeout, Ban, Unban,
 *     Resolve, ViewQueue)
 *   - moderator: Delete, Kick, Timeout, Untimeout, Resolve, ViewQueue — NOT
 *     Ban/Unban; guard rails (cannot Kick/Timeout owner/admin) are enforced by
 *     the relay seam, not this map.
 *   - member / null: none
 */

import type { RelayMemberRole } from "@/shared/api/types";

export type ModerationCapabilities = {
  /** May delete messages (9005 path for non-owned messages). */
  canDelete: boolean;
  /** May kick a user from a channel (9001). */
  canKick: boolean;
  /** May time out a user (9042). */
  canTimeout: boolean;
  /** May lift a timeout (9043). */
  canUntimeout: boolean;
  /** May ban a user from the community (9040). Admin+ only. */
  canBan: boolean;
  /** May lift a community ban (9041). Admin+ only. */
  canUnban: boolean;
  /** May view the moderation queue. */
  canViewQueue: boolean;
  /** May resolve/dismiss/escalate reports (9044). */
  canResolve: boolean;
};

const ADMIN_CAPABILITIES: ModerationCapabilities = {
  canDelete: true,
  canKick: true,
  canTimeout: true,
  canUntimeout: true,
  canBan: true,
  canUnban: true,
  canViewQueue: true,
  canResolve: true,
};

const MODERATOR_CAPABILITIES: ModerationCapabilities = {
  canDelete: true,
  canKick: true,
  canTimeout: true,
  canUntimeout: true,
  canBan: false,
  canUnban: false,
  canViewQueue: true,
  canResolve: true,
};

const NO_CAPABILITIES: ModerationCapabilities = {
  canDelete: false,
  canKick: false,
  canTimeout: false,
  canUntimeout: false,
  canBan: false,
  canUnban: false,
  canViewQueue: false,
  canResolve: false,
};

/**
 * Returns the moderation capabilities for the given relay role.
 *
 * Pass `null` or `undefined` for unauthenticated/non-member callers.
 */
export function moderationCapabilities(
  role: RelayMemberRole | null | undefined,
): ModerationCapabilities {
  if (role === "owner" || role === "admin") return ADMIN_CAPABILITIES;
  if (role === "moderator") return MODERATOR_CAPABILITIES;
  return NO_CAPABILITIES;
}
