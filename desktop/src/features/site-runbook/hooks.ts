import * as React from "react";

import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";

import { pinRunbookRef, sidRunbookRef } from "./lib/keys";
import {
  acceptProcedure,
  archiveProcedure,
  deleteProcedure,
  rejectProcedure,
  setAgentBrief,
  setProcedurePersisted,
  updateProcedure,
  upsertAgentProcedure,
} from "./lib/mutations";
import { emptyRunbook, shapeRunbookInject } from "./lib/serialize";
import {
  clearSiteRunbook,
  configureSiteRunbooksScope,
  getPendingProcedureCount,
  getSiteRunbookOrEmpty,
  setSiteRunbook,
  subscribeSiteRunbooks,
} from "./lib/store";
import { countPendingInRunbook } from "./lib/pendingCount";
import type { SiteRunbook, SiteRunbookRef } from "./lib/types";

function useSiteRunbooksConfigured(): void {
  const identity = useIdentityQuery();
  const { activeCommunity } = useCommunities();
  const pubkey = identity.data?.pubkey ?? "";
  // Identity has no relayUrl; scope matches pinned-sites (pubkey + active community relay).
  const relayUrl = activeCommunity?.relayUrl ?? "";
  React.useEffect(() => {
    if (!pubkey || !relayUrl) return;
    configureSiteRunbooksScope(pubkey, relayUrl);
  }, [pubkey, relayUrl]);
}

function useRunbookSnapshot(ref: SiteRunbookRef | null): SiteRunbook {
  useSiteRunbooksConfigured();
  const [epoch, bump] = React.useReducer((n: number) => n + 1, 0);
  React.useEffect(() => subscribeSiteRunbooks(bump), []);
  // biome-ignore lint/correctness/useExhaustiveDependencies: epoch forces re-read after store emits
  return React.useMemo(() => {
    if (!ref) return emptyRunbook();
    return getSiteRunbookOrEmpty(ref);
  }, [ref, epoch]);
}

export function useSiteRunbook(ref: SiteRunbookRef | null): {
  runbook: SiteRunbook;
  setBrief: (brief: string) => void;
  accept: (procedureId: string) => void;
  reject: (procedureId: string) => void;
  archive: (procedureId: string) => void;
  remove: (procedureId: string) => void;
  update: (
    procedureId: string,
    patch: { title?: string; steps?: string },
  ) => void;
  setPersisted: (procedureId: string, persisted: boolean) => void;
  addManual: (title: string, steps: string) => void;
  clear: () => void;
  inject: ReturnType<typeof shapeRunbookInject>;
} {
  const runbook = useRunbookSnapshot(ref);

  const mutate = React.useCallback(
    (next: SiteRunbook) => {
      if (!ref) return;
      setSiteRunbook(ref, next);
    },
    [ref],
  );

  return {
    runbook,
    setBrief: (brief) => {
      if (!ref) return;
      mutate(setAgentBrief(runbook, brief));
    },
    accept: (procedureId) => mutate(acceptProcedure(runbook, procedureId)),
    reject: (procedureId) => mutate(rejectProcedure(runbook, procedureId)),
    archive: (procedureId) => mutate(archiveProcedure(runbook, procedureId)),
    remove: (procedureId) => mutate(deleteProcedure(runbook, procedureId)),
    update: (procedureId, patch) =>
      mutate(updateProcedure(runbook, procedureId, patch)),
    setPersisted: (procedureId, persisted) =>
      mutate(setProcedurePersisted(runbook, procedureId, persisted)),
    addManual: (title, steps) => {
      // Human-authored: auto-active, not persisted until they check Persist.
      const { runbook: next } = upsertAgentProcedure(runbook, {
        title,
        steps,
      });
      mutate(next);
    },
    clear: () => {
      if (!ref) return;
      clearSiteRunbook(ref);
    },
    inject: shapeRunbookInject(runbook),
  };
}

export function usePinSiteRunbook(pinId: string | null | undefined) {
  const ref = React.useMemo(
    () => (pinId ? pinRunbookRef(pinId) : null),
    [pinId],
  );
  return useSiteRunbook(ref);
}

export function useSidSiteRunbook(sid: string | null | undefined) {
  const ref = React.useMemo(() => (sid ? sidRunbookRef(sid) : null), [sid]);
  return useSiteRunbook(ref);
}

/** Live count of pending procedure proposals across all scoped runbooks. */
export function usePendingProcedureCount(): number {
  useSiteRunbooksConfigured();
  const [epoch, bump] = React.useReducer((n: number) => n + 1, 0);
  React.useEffect(() => subscribeSiteRunbooks(bump), []);
  return React.useMemo(() => {
    void epoch;
    return getPendingProcedureCount();
  }, [epoch]);
}

/** Pending count for one runbook ref (chip badge). */
export function useRunbookPendingCount(ref: SiteRunbookRef | null): number {
  const runbook = useRunbookSnapshot(ref);
  return countPendingInRunbook(runbook);
}
