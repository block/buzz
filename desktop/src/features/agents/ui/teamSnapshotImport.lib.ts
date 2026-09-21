import { i18n } from "@/i18n";
import type {
  TeamSnapshotImportMemberResult,
  TeamSnapshotImportResult,
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
        i18n.t("agents.team-import.memory-errors", {
          count: totalMemoryErrors,
        }),
      );
    }
    if (profileSyncFailureCount > 0) {
      parts.push(
        i18n.t("agents.team-import.profile-sync-errors", {
          count: profileSyncFailureCount,
        }),
      );
    }
    return {
      type: "error",
      message: i18n.t("agents.team-import.partial", {
        name: result.team.name,
        parts: parts.join(i18n.t("agents.team-import.and-separator")),
      }),
    };
  }
  return {
    type: "notice",
    message: i18n.t("agents.team-import.success", {
      count: memberCount,
      name: result.team.name,
    }),
  };
}
