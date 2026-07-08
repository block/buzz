import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import type {
  RelayEvent,
  RelayMember,
  RelayMemberRole,
} from "@/shared/api/types";

const KIND_NIP43_MEMBERSHIP_LIST = 13534;
const KIND_RELAY_ADMIN_ADD_MEMBER = 9030;
const KIND_RELAY_ADMIN_REMOVE_MEMBER = 9031;
const KIND_RELAY_ADMIN_CHANGE_ROLE = 9032;

function isRelayMemberRole(
  value: string | undefined,
): value is RelayMemberRole {
  return value === "owner" || value === "admin" || value === "member";
}

function normalizePubkey(pubkey: string): string {
  return pubkey.trim().toLowerCase();
}

function eventCreatedAtIso(event: RelayEvent): string {
  return new Date(event.created_at * 1_000).toISOString();
}

export type RelayMembershipLookup = {
  /**
   * True when the relay returned a NIP-43 membership snapshot.
   *
   * Open relays do not publish kind:13534, so absence of this snapshot must not
   * be treated as a denial by onboarding.
   */
  snapshotFound: boolean;
  membership: RelayMember | null;
};

export function relayMembersFromEvent(event: RelayEvent): RelayMember[] {
  const seen = new Set<string>();
  const members: RelayMember[] = [];
  const createdAt = eventCreatedAtIso(event);

  for (const tag of event.tags) {
    const [name, rawPubkey, maybeRoleOrRelay, maybePTagRole] = tag;
    if (name !== "member" && name !== "p") continue;
    if (!rawPubkey) continue;

    const pubkey = normalizePubkey(rawPubkey);
    if (!/^[0-9a-f]{64}$/.test(pubkey) || seen.has(pubkey)) continue;
    seen.add(pubkey);

    const rawRole = name === "member" ? maybeRoleOrRelay : maybePTagRole;
    const role = isRelayMemberRole(rawRole) ? rawRole : "member";

    members.push({
      pubkey,
      role,
      addedBy: null,
      createdAt,
    });
  }

  return members;
}

export function relayMembershipLookupFromEvent(
  event: RelayEvent | null,
  pubkey: string,
): RelayMembershipLookup {
  if (!event) {
    return { snapshotFound: false, membership: null };
  }

  const normalizedPubkey = normalizePubkey(pubkey);
  return {
    snapshotFound: true,
    membership:
      relayMembersFromEvent(event).find(
        (member) => normalizePubkey(member.pubkey) === normalizedPubkey,
      ) ?? null,
  };
}

async function fetchMembershipListEvent(): Promise<RelayEvent | null> {
  const events = await relayClient.fetchEvents({
    kinds: [KIND_NIP43_MEMBERSHIP_LIST],
    limit: 1,
  });

  return events[events.length - 1] ?? null;
}

export async function listRelayMembers(): Promise<RelayMember[]> {
  const event = await fetchMembershipListEvent();
  return event ? relayMembersFromEvent(event) : [];
}

export async function getMyRelayMembershipLookup(): Promise<RelayMembershipLookup> {
  const [{ pubkey }, event] = await Promise.all([
    getIdentity(),
    fetchMembershipListEvent(),
  ]);
  return relayMembershipLookupFromEvent(event, pubkey);
}

export async function getMyRelayMembership(): Promise<RelayMember | null> {
  return (await getMyRelayMembershipLookup()).membership;
}

async function publishRelayAdminEvent(
  kind: number,
  targetPubkey: string,
  role?: string,
): Promise<void> {
  const tags = [["p", normalizePubkey(targetPubkey)]];
  if (role) {
    tags.push(["role", role]);
  }

  const event = await signRelayEvent({
    kind,
    content: "",
    tags,
  });

  await relayClient.publishEvent(
    event,
    "Timed out while updating relay access.",
    "Failed to update relay access.",
  );
}

export async function addRelayMember(
  targetPubkey: string,
  role: string,
): Promise<void> {
  await publishRelayAdminEvent(KIND_RELAY_ADMIN_ADD_MEMBER, targetPubkey, role);
}

export async function removeRelayMember(targetPubkey: string): Promise<void> {
  await publishRelayAdminEvent(KIND_RELAY_ADMIN_REMOVE_MEMBER, targetPubkey);
}

export async function changeRelayMemberRole(
  targetPubkey: string,
  newRole: string,
): Promise<void> {
  await publishRelayAdminEvent(
    KIND_RELAY_ADMIN_CHANGE_ROLE,
    targetPubkey,
    newRole,
  );
}
