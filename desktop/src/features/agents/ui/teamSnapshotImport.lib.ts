import type {
  TeamSnapshotImportMemberResult,
  TeamSnapshotImportResult,
  SnapshotMemoryLevel,
  SnapshotFormat,
} from "@/shared/api/tauriTeams";

// ── Import phase derivation ──────────────────────────────────────────────────

export type ImportPhase = "preview" | "confirming" | "result";

export function deriveImportPhase(
  result: TeamSnapshotImportResult | null,
  isConfirming: boolean,
): ImportPhase {
  return result !== null ? "result" : isConfirming ? "confirming" : "preview";
}

// ── Profile-sync failure aggregation ─────────────────────────────────────────

export function getProfileSyncFailures(
  members: TeamSnapshotImportMemberResult[],
): TeamSnapshotImportMemberResult[] {
  return members.filter((m) => m.profileSyncError !== null);
}

// ── Toast message derivation ─────────────────────────────────────────────────

export type ToastOutcome = {
  type: "notice" | "error";
  message: string;
};

export function deriveImportToast(
  result: TeamSnapshotImportResult,
): ToastOutcome {
  const memberCount = result.members.length;
  const totalMemoryErrors = result.members.reduce(
    (sum, m) => sum + m.memoryErrors.length,
    0,
  );
  const profileSyncFailureCount = getProfileSyncFailures(result.members).length;

  if (totalMemoryErrors > 0 || profileSyncFailureCount > 0) {
    const parts: string[] = [];
    if (totalMemoryErrors > 0) {
      parts.push(
        `${totalMemoryErrors} memory entr${totalMemoryErrors === 1 ? "y" : "ies"} failed to restore`,
      );
    }
    if (profileSyncFailureCount > 0) {
      parts.push(
        `${profileSyncFailureCount} member${profileSyncFailureCount === 1 ? "" : "s"} failed to sync profile${profileSyncFailureCount === 1 ? "" : "s"}`,
      );
    }
    return {
      type: "error",
      message: `${result.team.name} imported, but ${parts.join(" and ")}.`,
    };
  }
  return {
    type: "notice",
    message: `Imported ${result.team.name} with ${memberCount} member${memberCount === 1 ? "" : "s"}.`,
  };
}

// ── Send-encode argument builder ─────────────────────────────────────────────

export type TeamSnapshotSendArgs = {
  id: string;
  memoryLevel: SnapshotMemoryLevel;
  format: SnapshotFormat;
};

/** Build the args for `encodeTeamSnapshotForSend`. In-Buzz sends always use PNG. */
export function buildTeamSnapshotSendArgs(
  teamId: string,
  memoryLevel: SnapshotMemoryLevel,
): TeamSnapshotSendArgs {
  return { id: teamId, memoryLevel, format: "png" };
}
