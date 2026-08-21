import {
  emptyLaneCounts,
  isFibreLane,
  type Fibre,
  type FibreLane,
  type FibreLaneCounts,
  type FibreStatus,
  type FibresResponse,
} from "@/features/triage/api";

export type FibresPayload = Partial<FibresResponse> & { fibre?: Fibre };

/** Fibres from an engine that predates lanes still need a column. */
function laneOf(fibre: Fibre): FibreLane {
  return isFibreLane(fibre.lane) ? fibre.lane : "other";
}

export function countLanes(fibres: readonly Fibre[]): FibreLaneCounts {
  const counts = emptyLaneCounts();
  for (const fibre of fibres) counts[laneOf(fibre)] += 1;
  return counts;
}

export function fibresInLane(
  fibres: readonly Fibre[],
  lane: FibreLane,
): Fibre[] {
  return fibres.filter((fibre) => laneOf(fibre) === lane);
}

export function normalizeFibresResponse(
  payload: FibresPayload | null | undefined,
  previous?: Pick<FibresResponse, "done"> | null,
): FibresResponse {
  const fibres = payload?.fibres ?? [];
  const openIds = new Set(fibres.map((fibre) => fibre.id));
  let done: Fibre[];
  if (Array.isArray(payload?.done)) {
    done = payload.done;
  } else {
    // Stale engines omit `done`. Keep the client cache instead of wiping it.
    done = (previous?.done ?? []).filter((fibre) => !openIds.has(fibre.id));
    const patched = payload?.fibre;
    if (
      patched?.status === "done" &&
      !openIds.has(patched.id) &&
      !done.some((fibre) => fibre.id === patched.id)
    ) {
      done = [patched, ...done];
    }
  }
  return {
    fibres,
    done,
    openCount: payload?.openCount ?? fibres.length,
    doneCount: payload?.doneCount ?? done.length,
    clearedCount: payload?.clearedCount ?? 0,
    laneCounts: payload?.laneCounts ?? countLanes(fibres),
    changes: payload?.changes,
    ingested: payload?.ingested,
  };
}

export function moveFibreStatus(
  current: FibresResponse,
  id: string,
  status: FibreStatus,
): FibresResponse {
  const open = current.fibres ?? [];
  const done = current.done ?? [];
  const fromOpen = open.find((fibre) => fibre.id === id);
  const fromDone = done.find((fibre) => fibre.id === id);
  const fibre = fromOpen ?? fromDone;
  if (!fibre) return current;

  const nextFibre = { ...fibre, status };
  const nextOpen = open.filter((item) => item.id !== id);
  const nextDone = done.filter((item) => item.id !== id);
  if (status === "open") nextOpen.unshift(nextFibre);
  if (status === "done") nextDone.unshift(nextFibre);

  const wasCleared = !fromOpen;
  const isCleared = status !== "open";
  let clearedDelta = 0;
  if (wasCleared && !isCleared) clearedDelta = -1;
  if (!wasCleared && isCleared) clearedDelta = 1;

  return {
    ...current,
    fibres: nextOpen,
    done: nextDone,
    openCount: nextOpen.length,
    doneCount: nextDone.length,
    laneCounts: countLanes(nextOpen),
    clearedCount: Math.max(0, current.clearedCount + clearedDelta),
  };
}
