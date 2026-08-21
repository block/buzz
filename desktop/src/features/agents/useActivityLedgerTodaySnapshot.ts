import * as React from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  OWNER_TODAY_SNAPSHOT_CAPABILITY,
  OWNER_TODAY_SNAPSHOT_SCHEMA,
  getJournalAuthorityArtifacts,
  queryJournalAuthorityArtifacts,
  readAllArchivedObserverEventsForRange,
  writeOwnerTodaySnapshot,
  type JournalAuthorityArtifact,
} from "@/shared/api/tauriArchive";
import {
  activityLedgerDayRange,
  applyAuthorityToTodayActivity,
  buildTodayActivityFromArchivedEvents,
} from "./activityLedgerToday";

const SNAPSHOT_REFRESH_MS = 60_000;
const SNAPSHOT_LIFETIME_SECONDS = 5 * 60;
const MAX_AUTHORITY_RANGE_RESULTS = 500;

function localDay(now: Date): string {
  const year = now.getFullYear();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const date = String(now.getDate()).padStart(2, "0");
  return `${year}-${month}-${date}`;
}

async function loadCompleteAuthorityWindow(input: {
  startCreatedAt: number;
  endCreatedAt: number;
  journalIds: readonly string[];
}): Promise<JournalAuthorityArtifact[]> {
  const artifacts = await queryJournalAuthorityArtifacts({
    startCreatedAt: input.startCreatedAt,
    endCreatedAt: input.endCreatedAt,
    limit: MAX_AUTHORITY_RANGE_RESULTS,
  });
  if (artifacts.length < MAX_AUTHORITY_RANGE_RESULTS) return artifacts;

  // A full page is ambiguous. Fall back to journal-scoped reads so a busy day
  // cannot silently omit an older owner edit or verification.
  const byJournal = await Promise.all(
    [...new Set(input.journalIds)].map((journalId) =>
      getJournalAuthorityArtifacts(journalId),
    ),
  );
  return byJournal.flat();
}

/**
 * Publish the canonical owner Today surface for local read-only agent tools.
 * The backend binds it to the active identity and atomically writes it 0600.
 */
export function useActivityLedgerTodaySnapshot(): void {
  const ownerPubkey = useIdentityQuery().data?.pubkey;
  const managedAgents = useManagedAgentsQuery().data ?? [];
  const managedIdentities = React.useMemo(
    () =>
      managedAgents.map((agent) => ({
        pubkey: agent.pubkey,
        name: agent.name,
      })),
    [managedAgents],
  );

  React.useEffect(() => {
    if (!ownerPubkey) return;
    let disposed = false;
    let inFlight: Promise<void> | null = null;

    const publish = () => {
      if (disposed || inFlight) return;
      inFlight = (async () => {
        const now = new Date();
        const day = localDay(now);
        const range = activityLedgerDayRange(day);
        const archived = await readAllArchivedObserverEventsForRange({
          ...range,
          pageSize: 500,
        });
        const observedSurface = await buildTodayActivityFromArchivedEvents({
          day,
          agents: managedIdentities,
          events: archived,
        });
        const authority = await loadCompleteAuthorityWindow({
          ...range,
          journalIds: observedSurface.journals.map((journal) => journal.id),
        });
        const surface = applyAuthorityToTodayActivity(
          observedSurface,
          authority,
        );
        const generatedAt = Math.floor(now.getTime() / 1_000);
        await writeOwnerTodaySnapshot({
          schema: OWNER_TODAY_SNAPSHOT_SCHEMA,
          ownerPubkey,
          generatedAt,
          expiresAt: generatedAt + SNAPSHOT_LIFETIME_SECONDS,
          capability: OWNER_TODAY_SNAPSHOT_CAPABILITY,
          surface,
          rawEvents: [],
        });
      })()
        .catch((error) => {
          console.warn(
            "[activity-ledger] failed to publish Today snapshot",
            error,
          );
        })
        .finally(() => {
          inFlight = null;
        });
    };

    publish();
    const interval = window.setInterval(publish, SNAPSHOT_REFRESH_MS);
    return () => {
      disposed = true;
      window.clearInterval(interval);
    };
  }, [managedIdentities, ownerPubkey]);
}
