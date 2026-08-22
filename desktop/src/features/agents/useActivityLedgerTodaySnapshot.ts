import * as React from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { getRelayWsUrl } from "@/shared/api/tauri";
import {
  OWNER_TODAY_SNAPSHOT_CAPABILITY,
  OWNER_TODAY_SNAPSHOT_SCHEMA,
  getJournalAuthorityArtifacts,
  iterateArchivedObserverEventPagesForRange,
  writeOwnerTodaySnapshot,
  type JournalAuthorityArtifact,
} from "@/shared/api/tauriArchive";
import {
  activityLedgerArchiveQueryRange,
  applyAuthorityToTodayActivity,
  buildBoundedTodayActivitySurface,
  buildTodayActivityFromArchivedPages,
} from "./activityLedgerToday";

const SNAPSHOT_REFRESH_MS = 60_000;
const SNAPSHOT_LIFETIME_SECONDS = 5 * 60;
const AUTHORITY_READ_CONCURRENCY = 8;

function localDay(now: Date): string {
  const year = now.getFullYear();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const date = String(now.getDate()).padStart(2, "0");
  return `${year}-${month}-${date}`;
}

export function canPublishActivityLedgerTodaySnapshot(
  ownerPubkey: string | undefined,
  managedAgentsLoaded: boolean,
): ownerPubkey is string {
  return Boolean(ownerPubkey && managedAgentsLoaded);
}

/**
 * Load authority by retained journal identity, not artifact creation day. A
 * journal can span midnight while its latest owner edit belongs to yesterday.
 */
export async function loadActivityLedgerTodayAuthority(
  journalIds: readonly string[],
  load: (
    journalId: string,
  ) => Promise<JournalAuthorityArtifact[]> = getJournalAuthorityArtifacts,
): Promise<JournalAuthorityArtifact[]> {
  const uniqueJournalIds = [...new Set(journalIds.filter(Boolean))];
  const byJournal: JournalAuthorityArtifact[][] = Array.from(
    { length: uniqueJournalIds.length },
    () => [],
  );
  let nextIndex = 0;
  const worker = async () => {
    for (;;) {
      const index = nextIndex;
      nextIndex += 1;
      const journalId = uniqueJournalIds[index];
      if (journalId === undefined) return;
      byJournal[index] = await load(journalId);
    }
  };
  await Promise.all(
    Array.from(
      { length: Math.min(AUTHORITY_READ_CONCURRENCY, uniqueJournalIds.length) },
      worker,
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
  const managedAgentsQuery = useManagedAgentsQuery();
  const managedAgents = managedAgentsQuery.data ?? [];
  const managedIdentities = React.useMemo(
    () =>
      managedAgents.map((agent) => ({
        pubkey: agent.pubkey,
        name: agent.name,
      })),
    [managedAgents],
  );

  React.useEffect(() => {
    if (
      !canPublishActivityLedgerTodaySnapshot(
        ownerPubkey,
        managedAgentsQuery.isSuccess,
      )
    ) {
      return;
    }
    let disposed = false;
    let inFlight: Promise<void> | null = null;

    const publish = () => {
      if (disposed || inFlight) return;
      inFlight = (async () => {
        const reconstructionStartedAt = new Date();
        const relayUrl = await getRelayWsUrl();
        const day = localDay(reconstructionStartedAt);
        const range = activityLedgerArchiveQueryRange(day);
        const archivedPages = iterateArchivedObserverEventPagesForRange({
          ...range,
          pageSize: 500,
        });
        const observedSurface = await buildTodayActivityFromArchivedPages({
          day,
          agents: managedIdentities,
          pages: archivedPages,
        });
        const authority = await loadActivityLedgerTodayAuthority(
          observedSurface.journals.map((journal) => journal.id),
        );
        const surface = buildBoundedTodayActivitySurface(
          applyAuthorityToTodayActivity(observedSurface, authority),
        );
        // Timestamp the artifact at publication, not before archive paging,
        // decryption, authority loading, and projection. A slow reconstruction
        // must not write a snapshot that is already near or past expiry.
        const generatedAt = Math.floor(Date.now() / 1_000);
        await writeOwnerTodaySnapshot({
          schema: OWNER_TODAY_SNAPSHOT_SCHEMA,
          ownerPubkey,
          relayUrl,
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
  }, [managedAgentsQuery.isSuccess, managedIdentities, ownerPubkey]);
}
