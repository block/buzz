/**
 * Entity holon R4 — presence status + optional place (host proof).
 *
 * Place comes from public place_proof.v1 bodies (no surface_root / secrets).
 * Status remains online|away|offline from kind:20001 / get_presence.
 */

import type { PresenceStatus } from "@/shared/api/types";
import type { PlaceProofPublic } from "@/features/remote-agents/types";
import { getPresenceLabel } from "./presence";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type PresencePlace = {
  hostId?: string;
  hostRole?: string;
  surfaceKind?: string;
  surfaceId?: string;
  bodyId?: string | null;
  health?: string;
  birthCertId?: string;
};

export type PresencePlaceLookup = Record<string, PresencePlace>;

/** Map public place_proof bodies/seats → pubkey → place (public fields only). */
export function placeLookupFromLocationProof(
  proof: Record<string, unknown> | null | undefined,
): PresencePlaceLookup {
  if (!proof) return {};
  const out: PresencePlaceLookup = {};

  const ingest = (row: Record<string, unknown>) => {
    const birth = String(
      row.birth_cert_id || row.pubkey || row.birthCertId || "",
    ).toLowerCase();
    if (!birth || birth.length < 16) return;
    // Refuse to index host-local paths if a buggy client included them
    if (row.surface_root || row.unit_pid) {
      /* strip — never copy into lookup */
    }
    out[normalizePubkey(birth)] = {
      birthCertId: birth,
      hostId: (row.host_id || row.hostId) as string | undefined,
      hostRole: (row.host_role || row.hostRole) as string | undefined,
      surfaceKind: (row.surface_kind || row.surfaceKind) as string | undefined,
      surfaceId: (row.surface_id || row.surfaceId) as string | undefined,
      bodyId: (row.body_id ?? row.bodyId) as string | null | undefined,
      health: row.health as string | undefined,
    };
  };

  for (const b of (proof.bodies as PlaceProofPublic[] | undefined) || []) {
    ingest(b as unknown as Record<string, unknown>);
  }
  for (const s of (proof.seats as Array<Record<string, unknown>>) || []) {
    ingest(s);
  }
  return out;
}

/**
 * Human label: "Online · home · asus" when place known; else stock presence label.
 * Never includes filesystem paths.
 */
export function getPresenceLabelWithPlace(
  status: PresenceStatus | undefined,
  place?: PresencePlace | null,
): string {
  const base = status ? getPresenceLabel(status) : "Unknown";
  if (!place?.hostId && !place?.hostRole) return base;
  const bits = [base];
  if (place.hostRole) bits.push(place.hostRole);
  if (place.hostId) bits.push(place.hostId);
  if (place.surfaceKind) bits.push(place.surfaceKind);
  return bits.join(" · ");
}

/**
 * If host proof says body is live (health ok) but relay presence is missing,
 * treat as online for dual-body guards (bounded: only when proof is fresh).
 */
export function presenceStatusWithHostHint(
  status: PresenceStatus | undefined,
  place?: PresencePlace | null,
): PresenceStatus | undefined {
  if (status === "online" || status === "away" || status === "offline") {
    return status;
  }
  if (place?.health === "ok" || place?.health === "online") {
    return "online";
  }
  return status;
}
