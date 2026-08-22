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

export function activityLedgerTodaySnapshotDayGate(
  reconstructedDay: string,
  publicationTime: Date,
  rebuildAttempted: boolean,
): { action: "publish" | "rebuild" | "discard"; day: string } {
  const day = localDay(publicationTime);
  if (day === reconstructedDay) return { action: "publish", day };
  return { action: rebuildAttempted ? "discard" : "rebuild", day };
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
  relayUrl: string,
  journalScopes: readonly { agentPubkey: string; journalId: string }[],
  load: (
    relayUrl: string,
    agentPubkey: string,
    journalId: string,
  ) => Promise<JournalAuthorityArtifact[]> = getJournalAuthorityArtifacts,
): Promise<JournalAuthorityArtifact[]> {
  const uniqueJournalScopes = [
    ...new Map(
      journalScopes
        .filter((scope) => scope.agentPubkey && scope.journalId)
        .map((scope) => [
          `${scope.agentPubkey}\u0000${scope.journalId}`,
          scope,
        ]),
    ).values(),
  ];
  const byJournal: JournalAuthorityArtifact[][] = Array.from(
    { length: uniqueJournalScopes.length },
    () => [],
  );
  let nextIndex = 0;
  const worker = async () => {
    for (;;) {
      const index = nextIndex;
      nextIndex += 1;
      const scope = uniqueJournalScopes[index];
      if (scope === undefined) return;
      byJournal[index] = await load(
        relayUrl,
        scope.agentPubkey,
        scope.journalId,
      );
    }
  };
  await Promise.all(
    Array.from(
      {
        length: Math.min(
          AUTHORITY_READ_CONCURRENCY,
          uniqueJournalScopes.length,
        ),
      },
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
        const relayUrl = await getRelayWsUrl();
        let day = localDay(new Date());
        let rebuildAttempted = false;
        for (;;) {
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
            relayUrl,
            observedSurface.journals.map((journal) => ({
              agentPubkey: journal.agentPubkey,
              journalId: journal.id,
            })),
          );
          const surface = buildBoundedTodayActivitySurface(
            applyAuthorityToTodayActivity(observedSurface, authority, relayUrl),
          );
          const publicationTime = new Date();
          const dayGate = activityLedgerTodaySnapshotDayGate(
            day,
            publicationTime,
            rebuildAttempted,
          );
          if (dayGate.action === "discard") return;
          if (dayGate.action === "rebuild") {
            day = dayGate.day;
            rebuildAttempted = true;
            continue;
          }
          // Timestamp at publication, after paging, decryption, authority, and
          // the day gate, so a slow reconstruction never receives fresh
          // validity after its local day has ended.
          const generatedAt = Math.floor(publicationTime.getTime() / 1_000);
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
          return;
        }
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
