import type {
  HostAgentHealth,
  HostAgentStatus,
  PlaceProofHealth,
  PlaceProofPublic,
  RemoteAgentCardModel,
} from "./types";

const FRESH_SECS = 60;
const STALE_SECS = 120;
const DEAD_SECS = 300;

export function deriveHealthFromStatus(
  status: HostAgentStatus | null,
  fetchError: boolean,
  nowSecs: number = Math.floor(Date.now() / 1000),
): { health: HostAgentHealth; label: string } {
  if (fetchError || !status) {
    return { health: "unknown", label: "unreachable" };
  }
  if (status.ok === false) {
    return { health: "stopped", label: status.error || "error" };
  }
  const ts = typeof status.ts === "number" ? status.ts : nowSecs;
  const age = Math.max(0, nowSecs - ts);
  const hasUnit =
    (status.watchers?.unit_pids ?? 0) > 0 ||
    (status.watchers?.process_matches ?? 0) > 0;
  const expected = (status.seats ?? []).some((s) => s.expected_online);

  if (age > DEAD_SECS) {
    return { health: "stale", label: `stale ${age}s` };
  }
  if (age > STALE_SECS) {
    return { health: "stale", label: `amber ${age}s` };
  }
  if (expected && !hasUnit && age <= FRESH_SECS) {
    return { health: "stopped", label: "expected online · no unit" };
  }
  if (hasUnit || status.relay?.ok) {
    return {
      health: "online",
      label: age <= FRESH_SECS ? "live" : `ok ${age}s`,
    };
  }
  return { health: "unknown", label: "unknown" };
}

function mapPlaceHealth(
  h: PlaceProofHealth | string | undefined,
): HostAgentHealth {
  switch (h) {
    case "ok":
      return "online";
    case "degraded":
    case "stale":
      return "stale";
    case "down":
      return "stopped";
    case "online":
      return "online";
    case "stopped":
      return "stopped";
    default:
      return "unknown";
  }
}

function shortDna(hex: string | undefined): string | undefined {
  if (!hex || hex.length < 8) return undefined;
  return `${hex.slice(0, 8)}…`;
}

function bodyFromProof(
  proof: Record<string, unknown> | null | undefined,
  seatId: string,
): PlaceProofPublic | undefined {
  if (!proof) return undefined;
  const bodies = (proof.bodies as PlaceProofPublic[] | undefined) || [];
  const fromBodies = bodies.find(
    (b) => b.seat_id === seatId || b.legal_name === seatId,
  );
  if (fromBodies) return fromBodies;
  const seats =
    (proof.seats as Array<Record<string, unknown>> | undefined) || [];
  const seat = seats.find((s) => s.seat_id === seatId);
  if (!seat) return undefined;
  return {
    schema: String(proof.schema || "place_proof.v1"),
    birth_cert_id: (seat.birth_cert_id || seat.pubkey || "") as string,
    seat_id: seatId,
    body_id: (seat.body_id as string) || null,
    host_id: seat.host_id as string | undefined,
    host_role: seat.host_role as string | undefined,
    surface_kind: seat.surface_kind as string | undefined,
    surface_id: seat.surface_id as string | undefined,
    health: seat.health as string | undefined,
    lease_epoch: seat.lease_epoch as number | undefined,
    model: seat.model as string | undefined,
    runtime: seat.runtime as string | undefined,
  };
}

/**
 * Build Remote Agents cards.
 * Prefer public place_proof fields (DNA · body · place). Never require surface_root for UI.
 */
export function deriveRemoteAgentCards(
  status: HostAgentStatus | null,
  fetchError: boolean,
  locationProof?: Record<string, unknown> | null,
): RemoteAgentCardModel[] {
  const hostId = status?.host_id || "unknown-host";
  const hostRole = status?.host_role || "home";
  const { health, label } = deriveHealthFromStatus(status, fetchError);
  const seats = status?.seats ?? [];

  if (seats.length === 0 && status && !fetchError) {
    return [
      {
        seatId: "(no seats in registry)",
        hostId,
        hostRole,
        model: "",
        runtimes: [],
        channels: [],
        expectedOnline: false,
        health,
        healthLabel: label,
        relayOk: Boolean(status.relay?.ok),
        ollamaOk: Boolean(status.ollama?.ok),
        bodyLive: false,
      },
    ];
  }

  return seats.map((seat) => {
    const proofBody = bodyFromProof(locationProof, seat.seat_id);
    const birth =
      seat.birth_cert_id ||
      seat.pubkey ||
      seat.pubkey_hint ||
      proofBody?.birth_cert_id ||
      "";

    let seatHealth = health;
    let seatLabel = label;
    let bodyLive = false;

    if (proofBody?.health) {
      seatHealth = mapPlaceHealth(proofBody.health);
      seatLabel = String(proofBody.health);
      bodyLive = proofBody.health === "ok" || proofBody.health === "online";
    } else if (seat.unit_alive === false && seat.expected_online) {
      seatHealth = "stopped";
      seatLabel = "unit dead";
    } else if (seat.unit_alive === true) {
      seatHealth = "online";
      seatLabel = seat.unit_pid ? `unit live` : "unit live";
      bodyLive = true;
    } else if (!seat.expected_online) {
      seatHealth = "stopped";
      seatLabel = "not expected online";
    }

    const surfaceKind =
      proofBody?.surface_kind || seat.surface_kind || undefined;
    const surfaceId = proofBody?.surface_id || seat.surface_id || undefined;

    return {
      seatId: seat.seat_id,
      hostId: proofBody?.host_id || hostId,
      hostRole: proofBody?.host_role || hostRole,
      model: seat.model || proofBody?.model || "",
      runtimes: seat.runtimes || [],
      channels: seat.channels || [],
      expectedOnline: Boolean(seat.expected_online),
      health: seatHealth,
      healthLabel: seatLabel,
      relayOk: Boolean(status?.relay?.ok),
      ollamaOk: Boolean(status?.ollama?.ok),
      // Privacy: do not surface full path in card model for display
      surfaceRoot: undefined,
      surfaceId,
      surfaceKind,
      birthCertId: birth || undefined,
      birthCertShort: shortDna(birth),
      bodyId: proofBody?.body_id || seat.body_id || undefined,
      leaseEpoch: proofBody?.lease_epoch ?? seat.lease_epoch,
      projectIds: seat.project_ids || [],
      unitPid: null,
      bodyLive,
    };
  });
}
