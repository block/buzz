import { getIdentity } from "@/shared/api/tauriIdentity";
import { updateProfile } from "@/shared/api/tauriProfiles";
import type { Profile } from "@/shared/api/types";
import { isRelayMembershipDeniedError } from "@/shared/lib/relayError";

/**
 * Deferred profile save for users who set up a profile before they belong to
 * any relay.
 *
 * A kind:0 write requires relay membership, but first-run setup asks for a
 * display name and avatar before the user has joined a community. The write is
 * refused, and historically that dead-ended onboarding (block/buzz#3544).
 *
 * Rather than discard what the user typed, park it here and replay it once
 * membership exists. This mirrors `avatarProfileSync`, with one difference:
 * the value is persisted to `localStorage`, because joining a community
 * remounts the community-scoped React tree (and the user may quit and relaunch
 * before joining), so in-memory state would not survive the gap.
 */

const STORAGE_KEY = "buzz.pendingProfileSave";

export type PendingProfile = {
  /** Identity the values were typed under. A different key must not inherit them. */
  pubkey: string;
  displayName: string;
  avatarUrl: string;
};

type PendingProfileSaveDependencies = {
  read: () => string | null;
  write: (value: string) => void;
  remove: () => void;
  saveProfile: (input: {
    displayName?: string;
    avatarUrl?: string;
  }) => Promise<Profile>;
  getActivePubkey: () => Promise<string | null>;
};

function isPendingProfile(value: unknown): value is PendingProfile {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.pubkey === "string" &&
    candidate.pubkey.length > 0 &&
    typeof candidate.displayName === "string" &&
    typeof candidate.avatarUrl === "string"
  );
}

/**
 * Outcome of a flush attempt.
 *
 * - `saved` — the write landed; the pending value is cleared.
 * - `deferred` — still no membership, or no active identity to compare
 *   against; the pending value is kept for the next attempt.
 * - `discarded` — the pending value belongs to a different identity, or the
 *   write failed for a reason retrying cannot fix; it is cleared.
 * - `empty` — nothing was pending.
 */
export type FlushPendingProfileResult =
  | "saved"
  | "deferred"
  | "discarded"
  | "empty";

export function createPendingProfileSave(
  dependencies: PendingProfileSaveDependencies,
) {
  const read = (): PendingProfile | null => {
    let raw: string | null;
    try {
      raw = dependencies.read();
    } catch {
      // Storage can throw (disabled, quota, private mode). Treat as empty.
      return null;
    }
    if (raw === null) return null;
    try {
      const parsed: unknown = JSON.parse(raw);
      return isPendingProfile(parsed) ? parsed : null;
    } catch {
      return null;
    }
  };

  const clear = (): void => {
    try {
      dependencies.remove();
    } catch {
      // Nothing actionable — a stale entry is re-validated on the next read.
    }
  };

  const save = (pending: PendingProfile): void => {
    // Nothing worth replaying: drop any earlier value rather than leaving a
    // stale one to be applied later.
    if (pending.displayName.trim() === "" && pending.avatarUrl.trim() === "") {
      clear();
      return;
    }
    try {
      dependencies.write(JSON.stringify(pending));
    } catch {
      // Best-effort: losing the deferred value is strictly better than
      // failing the onboarding step the user is trying to get past.
    }
  };

  const flush = async (): Promise<FlushPendingProfileResult> => {
    const pending = read();
    if (pending === null) return "empty";

    let activePubkey: string | null;
    try {
      activePubkey = await dependencies.getActivePubkey();
    } catch {
      return "deferred";
    }

    // No identity to compare against yet — keep waiting rather than writing
    // one user's profile under whatever key happens to be active later.
    if (activePubkey === null) return "deferred";
    if (activePubkey.toLowerCase() !== pending.pubkey.toLowerCase()) {
      clear();
      return "discarded";
    }

    const payload: { displayName?: string; avatarUrl?: string } = {};
    if (pending.displayName.trim() !== "") {
      payload.displayName = pending.displayName.trim();
    }
    if (pending.avatarUrl.trim() !== "") {
      payload.avatarUrl = pending.avatarUrl.trim();
    }
    if (Object.keys(payload).length === 0) {
      clear();
      return "discarded";
    }

    try {
      await dependencies.saveProfile(payload);
    } catch (error) {
      // Still not a member — the exact case this exists for. Keep it parked.
      if (isRelayMembershipDeniedError(error)) return "deferred";
      // Anything else (malformed value, relay rejection) will not resolve by
      // retrying forever; drop it so it cannot wedge every later boot.
      clear();
      return "discarded";
    }

    clear();
    return "saved";
  };

  return { flush, peek: read, save, clear };
}

function browserStorage(): PendingProfileSaveDependencies {
  return {
    read: () => window.localStorage.getItem(STORAGE_KEY),
    write: (value) => window.localStorage.setItem(STORAGE_KEY, value),
    remove: () => window.localStorage.removeItem(STORAGE_KEY),
    saveProfile: (input) => updateProfile(input),
    getActivePubkey: async () => {
      const identity = await getIdentity();
      return identity.pubkey;
    },
  };
}

let singleton: ReturnType<typeof createPendingProfileSave> | null = null;

function instance() {
  singleton ??= createPendingProfileSave(browserStorage());
  return singleton;
}

/** Park profile values that could not be written for lack of membership. */
export function savePendingProfile(pending: PendingProfile): void {
  instance().save(pending);
}

/** Read the parked profile without attempting to write it. */
export function peekPendingProfile(): PendingProfile | null {
  return instance().peek();
}

/**
 * Attempt the parked write. Safe to call on every community-ready transition:
 * it is a no-op when nothing is pending, and keeps the value parked when
 * membership still is not established.
 */
export function flushPendingProfile(): Promise<FlushPendingProfileResult> {
  return instance().flush();
}

/**
 * Drop the in-memory instance on a relay boundary change, per the
 * community-switching contract in CLAUDE.md. The persisted value deliberately
 * survives — it is keyed to an identity, not to a community, and the whole
 * point is to outlive the remount that joining causes.
 */
export function resetPendingProfileSave(): void {
  singleton = null;
}
