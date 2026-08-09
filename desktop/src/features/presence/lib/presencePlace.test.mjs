import assert from "node:assert/strict";
import { describe, it } from "node:test";

// Mirror pure helpers (keep in sync with presencePlace.ts)

function placeLookupFromLocationProof(proof) {
  if (!proof) return {};
  const out = {};
  const ingest = (row) => {
    const birth = String(row.birth_cert_id || row.pubkey || "").toLowerCase();
    if (!birth || birth.length < 16) return;
    out[birth] = {
      hostId: row.host_id,
      hostRole: row.host_role,
      surfaceKind: row.surface_kind,
      surfaceId: row.surface_id,
      health: row.health,
    };
  };
  for (const b of proof.bodies || []) ingest(b);
  for (const s of proof.seats || []) ingest(s);
  return out;
}

function getPresenceLabelWithPlace(status, place) {
  const base =
    status === "online"
      ? "Online"
      : status === "away"
        ? "Away"
        : status === "offline"
          ? "Offline"
          : "Unknown";
  if (!place?.hostId && !place?.hostRole) return base;
  return [base, place.hostRole, place.hostId, place.surfaceKind]
    .filter(Boolean)
    .join(" · ");
}

describe("presencePlace R4", () => {
  it("indexes public bodies by birth_cert", () => {
    const lookup = placeLookupFromLocationProof({
      bodies: [
        {
          birth_cert_id: "aa".repeat(32),
          host_id: "asus-g501vw",
          host_role: "home",
          surface_kind: "host-unit",
          surface_id: "bind:x",
          health: "ok",
        },
      ],
    });
    assert.equal(lookup["aa".repeat(32)].hostId, "asus-g501vw");
  });

  it("label includes place without paths", () => {
    const label = getPresenceLabelWithPlace("online", {
      hostRole: "home",
      hostId: "asus",
      surfaceKind: "cli-seat",
    });
    assert.equal(label, "Online · home · asus · cli-seat");
    assert.ok(!label.includes("/home"));
  });

  it("empty proof yields empty lookup", () => {
    assert.deepEqual(placeLookupFromLocationProof(null), {});
  });
});
