import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  myRelayMembershipLookupQueryKey,
  relayMembersQueryKey,
} from "@/features/community-members/hooks";
import { useMyRelayMembershipLookupQuery } from "@/features/community-members/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import {
  joinAlertBody,
  joinAlertSummaryBody,
  joinAlertTitle,
  normalizeJoinPubkey,
  readJoinAlertLedger,
  reconcileJoinAlertLedger,
  writeJoinAlertLedger,
  type JoinAlertLedger,
  JOIN_ALERT_MAX_INDIVIDUAL,
} from "@/features/community-members/lib/joinAlerts";
import { sendDesktopNotification } from "@/features/notifications/lib/desktop";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { relayClient } from "@/shared/api/relayClient";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  canManageCommunityMembers,
  relayMembersFromEvent,
} from "@/shared/api/relayMembers";
import { getUsersBatch } from "@/shared/api/tauriProfiles";
import type { RelayEvent, RelayMember } from "@/shared/api/types";

const KIND_NIP43_MEMBERSHIP_LIST = 13534;
const KIND_NIP43_MEMBER_ADDED = 8000;

/**
 * Trailing window for coalescing kind:8000-triggered snapshot refetches.
 *
 * Long enough that a bulk add collapses to a single REQ, short enough that a
 * lone join still feels immediate — the accelerator exists only to beat the
 * live snapshot's own arrival, so sub-second is the whole budget.
 */
const MEMBER_REFRESH_DEBOUNCE_MS = 500;

/**
 * Everything one mounted effect run is allowed to act on.
 *
 * The subscription callbacks that deliver snapshots belong to the effect run
 * that registered them, but `handleSnapshot` is a `useEffectEvent` and so reads
 * whatever is *currently* rendered. Between the re-render that switches
 * community and that effect's cleanup, those two disagree — and a snapshot from
 * the old community would be folded into the new community's ledger under the
 * new community's storage key.
 *
 * Binding the identity, the ledger, and the ordering state into one object
 * created by the effect run turns those scattered ambient reads into a single
 * value with an identity that can be compared. `handleSnapshot` still reads
 * `sessionRef.current`, so it is the surrounding ordering that makes the bug
 * unreachable: cleanup retires the session before the next run installs its
 * own, each retired callback is stopped by its run's `disposed` flag, and every
 * send boundary re-checks that the session it captured is still the live one.
 */
type JoinAlertSession = {
  communityId: string;
  viewerPubkey: string;
  ledger: JoinAlertLedger;
  /**
   * `created_at` of the newest snapshot already folded in.
   *
   * A snapshot older than this is a stale view of the roster — an in-flight
   * refetch that resolves after a newer live frame — and must not be treated as
   * current. Without this, an older frame can re-alert a departed key or, worse,
   * re-assert an authorization a newer frame just revoked.
   */
  newestSnapshotAt: number;
  /**
   * Latched once a snapshot shows the viewer is no longer owner/admin.
   *
   * Fail-closed, and deliberately stronger than the `newestSnapshotAt` fence:
   * the relay can publish two snapshots within the same second, so an
   * equal-`created_at` stale frame passes a strictly-older fence. Dropping
   * equal timestamps instead would discard legitimate same-second joins. The
   * latch removes the timestamp from the safety argument entirely — once
   * revocation is observed, this session is done disclosing, whatever order the
   * remaining frames arrive in.
   *
   * Re-promotion is unaffected: nothing invalidates the membership lookup on
   * promotion, so regaining the panel already requires a reload today.
   */
  revoked: boolean;
};

/**
 * Trailing quiet window for coalescing join alerts ACROSS snapshots.
 *
 * The per-snapshot cap bounds "one snapshot, many keys". It does nothing for
 * "one burst, many snapshots": the relay republishes the whole 13534 as each
 * concurrent add commits, so a 50-join storm arrives as a handful of growing
 * rosters and each one independently emitted its own capped batch. Max measured
 * 10 banners from 50 real joins at `fdeda44f0` for exactly this reason.
 *
 * Sized above the observed intermediate-snapshot cadence so a burst lands in
 * one batch, and above MEMBER_REFRESH_DEBOUNCE_MS so an 8000-triggered refetch
 * folds into the same window rather than flushing behind it.
 */
const JOIN_ALERT_NOTIFY_WINDOW_MS = 1_500;

/**
 * Ceiling on how long a batch may be deferred by the trailing window.
 *
 * `JOIN_ALERT_NOTIFY_WINDOW_MS` is a pure trailing debounce: every snapshot
 * re-arms it, so a join cadence faster than the window defers delivery for as
 * long as the joins keep coming. Measured before this clamp existed: 13 joins at
 * ~700ms intervals produced zero notifications across 9.1 continuous seconds.
 *
 * That is the wrong shape for an alerting feature, and it is worse than mere
 * lateness — the ledger is persisted per snapshot while delivery waits, so a
 * quit or community switch mid-drip drops a batch the ledger already recorded as
 * alerted, and it is never re-announced. Clamping bounds both the silence and
 * that loss window.
 *
 * Sized against both ends rather than picked round: it must exceed the span a
 * bulk add's intermediate snapshots occupy, or the clamp would split the burst
 * this window exists to collapse, and it must sit BELOW the measured drip above,
 * or it would leave the case that motivated it unchanged. A burst's snapshots
 * arrive within a second or two of each other; the drip ran 9.1s. Five seconds
 * clears the first by a wide margin and cuts the second roughly in half.
 */
const JOIN_ALERT_MAX_DEFERRAL_MS = 5_000;

/**
 * Notify community owners/admins the first time a key appears in their roster.
 *
 * Delivery rests on a live kind:13534 subscription because that snapshot is the
 * only membership signal covering every join path with cross-pod propagation;
 * see `lib/joinAlerts.ts` for the full rationale. Desktop's other 13534 read
 * (`relayMembers.ts`) is a one-shot fetch, so without this subscription no
 * snapshot ever arrives passively and nothing could fire.
 *
 * The kind:8000 delta is subscribed purely to shorten latency on the paths that
 * emit one. It refreshes the authoritative snapshot rather than alerting from
 * the delta's own payload, so one ledger governs both signals and the pair
 * cannot double-alert.
 *
 * Viewer, community, and role are read from context rather than passed in:
 * `AppShell` is at the file-size ratchet ceiling, so the mount has to stay a
 * single call.
 */
export function useCommunityJoinAlerts({ enabled }: { enabled: boolean }) {
  const queryClient = useQueryClient();
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const membershipQuery = useMyRelayMembershipLookupQuery();

  const communityId = activeCommunity?.id ?? null;
  const communityName = activeCommunity?.name ?? null;
  const normalizedViewer = normalizeJoinPubkey(
    identityQuery.data?.pubkey ?? "",
  );
  const active =
    enabled &&
    canManageCommunityMembers(membershipQuery.data) &&
    communityId !== null &&
    normalizedViewer.length > 0;

  // Session for the current effect run. Callbacks read it through this ref so
  // they stay stable — re-subscribing on every roster change would drop deltas
  // in the gap between REQ and CLOSE — but every read is validated against the
  // session's own bound community, never against ambient render state.
  const sessionRef = React.useRef<JoinAlertSession | null>(null);

  // Community name is read fresh rather than captured, because a rename does not
  // re-key the effect and a captured name would go stale. Guarded by id at use
  // time so it can only ever label its own community.
  const communityNameRef = React.useRef<{ id: string; name: string } | null>(
    null,
  );
  communityNameRef.current =
    communityId === null
      ? null
      : { id: communityId, name: communityName ?? "" };

  const resolveTitle = React.useCallback((session: JoinAlertSession) => {
    const named = communityNameRef.current;
    // Fall back to the generic title rather than a name belonging to a
    // different community.
    return joinAlertTitle(
      named?.id === session.communityId ? named.name : null,
    );
  }, []);

  // Pending cross-snapshot batch. A burst arrives as several growing rosters,
  // so alerts accumulate here and flush once the roster stops moving.
  //
  // `pendingEventRef` holds the LATEST snapshot only, as the notification's
  // click target. Every key in the batch is present in that roster (the ledger
  // is monotonic within a burst), so the newest snapshot is the accurate
  // referent for the whole batch.
  const pendingRef = React.useRef<string[]>([]);
  const pendingEventRef = React.useRef<RelayEvent | null>(null);
  const notifyTimerRef = React.useRef<number | null>(null);
  // When the batch currently pending first enqueued, for the deferral clamp.
  const pendingSinceRef = React.useRef<number | null>(null);

  // Cancellation token for flushes already past the refs.
  //
  // Clearing the refs cannot stop a flush that has already consumed them and
  // is parked on an await, and every send in `flushPending` sits behind one:
  // the profile lookup, and each notification itself. A demotion, removal,
  // unmount, or community switch landing in that window would otherwise still
  // deliver — Max and Wren both found this at 5d0d2b4c.
  //
  // Bumped ONLY by `clearPending`, never by an ordinary enqueue, so an
  // authorized batch queued while an earlier flush's lookup is in flight
  // neither cancels it nor is cancelled by it: both deliver. Cancellation is
  // the only thing that invalidates a claim.
  const flushGenerationRef = React.useRef(0);

  /** Drop anything queued but not yet delivered, in flight or not. */
  const clearPending = React.useCallback(() => {
    pendingRef.current = [];
    pendingEventRef.current = null;
    pendingSinceRef.current = null;
    flushGenerationRef.current += 1;
    if (notifyTimerRef.current !== null) {
      window.clearTimeout(notifyTimerRef.current);
      notifyTimerRef.current = null;
    }
  }, []);

  const flushPending = React.useEffectEvent(async () => {
    const session = sessionRef.current;
    const alerts = pendingRef.current;
    const event = pendingEventRef.current;
    pendingRef.current = [];
    pendingEventRef.current = null;
    pendingSinceRef.current = null;
    if (alerts.length === 0 || !event || !session) return;
    // A session that observed revocation never delivers, even if a batch was
    // queued before the latch closed.
    if (session.revoked) return;

    // Claim this batch. Checked again at every side-effect boundary below —
    // not merely after the awaits that exist today, so that adding an await
    // later cannot silently reopen the disclosure.
    const generation = flushGenerationRef.current;
    const cancelled = () =>
      flushGenerationRef.current !== generation ||
      sessionRef.current !== session ||
      session.revoked;

    // Bind the title to the community these keys were queued under, not to
    // whatever is active when the send resolves.
    const title = resolveTitle(session);

    // Resolve display names so the alert reads "Alice joined" rather than a
    // truncated key; a lookup failure degrades to the key, it does not skip.
    //
    // Above the cap the batch collapses into one summary, so skip the profile
    // fetch entirely — it would be a 250-key request whose result is unused.
    if (alerts.length > JOIN_ALERT_MAX_INDIVIDUAL) {
      if (cancelled()) return;
      await sendDesktopNotification({
        body: joinAlertSummaryBody(alerts.length),
        target: {
          channelId: null,
          eventId: event.id,
          kind: event.kind,
          pubkey: undefined,
        },
        title,
      });
      return;
    }

    let profiles: UserProfileLookup | undefined;
    try {
      profiles = (await getUsersBatch(alerts)).profiles;
    } catch {
      profiles = undefined;
    }

    for (const pubkey of alerts) {
      // Per-send, not once after the lookup: a demotion landing between two
      // named sends must suppress the rest of the batch, not just the batch
      // that had not started.
      if (cancelled()) return;
      await sendDesktopNotification({
        body: joinAlertBody(
          resolveUserLabel({ preferResolvedSelfLabel: true, profiles, pubkey }),
        ),
        target: {
          channelId: null,
          eventId: event.id,
          kind: event.kind,
          pubkey,
        },
        title,
      });
    }
  });

  const handleSnapshot = React.useEffectEvent(async (event: RelayEvent) => {
    const session = sessionRef.current;
    if (!session) return;
    // Already revoked: this session neither alerts nor learns anything further.
    if (session.revoked) return;

    const roster = relayMembersFromEvent(event);
    const rosterPubkeys = roster.map((member) => member.pubkey);
    if (rosterPubkeys.length === 0) return;

    // Drop a stale view of the roster before it can be treated as current.
    //
    // An in-flight refetch (kind:8000 accelerator or reconnect) can resolve
    // AFTER a newer live frame. Processing it would fold a superseded roster in
    // as authoritative — re-alerting a departed key, and re-asserting an
    // authorization the newer frame revoked. Strictly older only: two snapshots
    // can share a second, and dropping equal timestamps would discard real
    // joins. The revocation latch, not this fence, is what makes the privacy
    // arm safe at equal timestamps.
    const snapshotAt = event.created_at;
    if (snapshotAt < session.newestSnapshotAt) return;

    // The roster can change shape without anything being new to us (a removal
    // or a role change), so refresh the panel regardless of alert eligibility.
    //
    // Written directly rather than invalidated. `invalidateQueries` refetches
    // every ACTIVE observer, and `listRelayMembers` is a REQ frame
    // (`fetchFirstEvent({ kinds: [13534], limit: 1 })`), so with the members
    // panel open this path emitted one REQ per accepted snapshot — measured
    // 1:1 across 20 snapshots, live and in unit, against a documented budget
    // of limit x window = 50 REQ per 5s (`default_human_ws()` = 10/s,
    // `WS_BURST_WINDOW_SECS` = 5; REQ is billed as `WsEvents`). A join burst
    // large enough to matter would rate-limit the owner out of their own app,
    // and unlike the kind:8000 accelerator this path is not behind
    // `MEMBER_REFRESH_DEBOUNCE_MS`.
    //
    // The refetch was never load-bearing: `roster` above is the output of the
    // same `relayMembersFromEvent` parser `listRelayMembers` feeds the query
    // with (`relayMembers.ts:125-127`), from a snapshot this session has
    // already accepted as current — so the write is the identical shape and
    // strictly fresher than a refetch, which would race the stream that
    // triggered it. The stale fence above guarantees no superseded roster
    // reaches here, and the query client is per-community
    // (`CommunityQueryProvider key={communityKey}`, `App.tsx:556`), so this
    // non-community-scoped key cannot be written across a switch.
    queryClient.setQueryData<RelayMember[]>(relayMembersQueryKey, roster);

    // Authorize against the snapshot in hand, not the cached role that mounted
    // this effect. `useMyRelayMembershipLookupQuery` is only invalidated by this
    // client's own membership mutations, and `staleTime` marks data stale
    // without scheduling a refetch, so a viewer demoted by another admin keeps
    // a cached owner/admin role for as long as the app stays open — and would
    // otherwise keep learning every later joiner's identity from a role they no
    // longer hold. The snapshot carries the viewer's own role
    // (`["member", pubkey, role]`, relay-signed in `publish_nip43_membership_locked`),
    // so the event that revokes authorization is the same event that would
    // disclose the join. Checking it here closes that race in one read rather
    // than racing an async invalidation.
    //
    // Fail closed: a snapshot that does not list the viewer at all means they
    // were removed outright.
    const viewerEntry = roster.find(
      (member) => member.pubkey === session.viewerPubkey,
    );
    if (viewerEntry?.role !== "owner" && viewerEntry?.role !== "admin") {
      // Latch, so no later frame — including an older authorized snapshot still
      // in flight — can re-open disclosure for this session.
      session.revoked = true;
      // Revocation must also drop anything queued but not yet delivered.
      // Batching across snapshots would otherwise reopen the disclosure Wren
      // found as a *delayed* one: joins accumulated while authorized would
      // still fire from a timer after the snapshot that revoked the role.
      clearPending();
      // Refresh the mount gate so the subscriptions themselves tear down.
      void queryClient.invalidateQueries({
        queryKey: myRelayMembershipLookupQueryKey,
      });
      return;
    }

    // Fence advances only here: past the roster and authorization checks, on a
    // frame this session actually accepts as its current view. Advancing it at
    // the comparison instead would let a frame rejected for some *other* reason
    // push the fence past a legitimate frame still in flight, dropping a real
    // snapshot as though it were stale.
    session.newestSnapshotAt = snapshotAt;

    const { alerts, changed, ledger } = reconcileJoinAlertLedger({
      ledger: session.ledger,
      rosterPubkeys,
      viewerPubkey: session.viewerPubkey,
    });
    if (!changed) return;

    // Persisted before notifying, never after: a crash between the two must
    // lose the notification rather than repeat it on every later snapshot.
    //
    // A write that cannot land (quota still exceeded after cache eviction)
    // leaves the session's ledger alone deliberately. Advancing it would mark
    // these keys seen in memory while nothing reached storage, so the alert
    // would be lost until a reload; leaving it means the next snapshot retries
    // the write and the alert survives to whichever attempt lands. The notify is
    // skipped either way — a false return means nothing was persisted, so
    // notifying here is exactly the "repeat on every later snapshot" this
    // ordering exists to prevent.
    if (
      !writeJoinAlertLedger(session.communityId, session.viewerPubkey, ledger)
    ) {
      return;
    }
    session.ledger = ledger;
    if (alerts.length === 0) return;

    // Queue rather than notify. Persistence and the ledger advance stay
    // synchronous per snapshot (above), so cross-snapshot dedupe still holds
    // and a crash before the flush loses the alert rather than repeating it —
    // the ordering invariant this feature already committed to. Only the
    // delivery is deferred, onto a trailing quiet window, so one burst
    // produces one alert instead of one per intermediate snapshot.
    pendingRef.current.push(...alerts);
    pendingEventRef.current = event;
    if (notifyTimerRef.current !== null) {
      window.clearTimeout(notifyTimerRef.current);
    }
    const now = Date.now();
    if (pendingSinceRef.current === null) pendingSinceRef.current = now;
    // Clamp the trailing window so a sustained drip cannot defer delivery (and
    // the ledger-already-written loss window) without bound.
    const deadline = pendingSinceRef.current + JOIN_ALERT_MAX_DEFERRAL_MS;
    const delay = Math.max(
      0,
      Math.min(JOIN_ALERT_NOTIFY_WINDOW_MS, deadline - now),
    );
    notifyTimerRef.current = window.setTimeout(() => {
      notifyTimerRef.current = null;
      void flushPending();
    }, delay);
  });

  React.useEffect(() => {
    if (!active || communityId === null) return;

    // One session per effect run. Every callback below reaches this community's
    // ledger and this viewer's role through it and cannot reach any other, so a
    // switch mid-flight is a cancelled session rather than a mislabeled alert.
    const session: JoinAlertSession = {
      communityId,
      ledger: readJoinAlertLedger(communityId, normalizedViewer),
      newestSnapshotAt: 0,
      revoked: false,
      viewerPubkey: normalizedViewer,
    };
    sessionRef.current = session;

    let disposed = false;
    const disposers: Array<() => Promise<void>> = [];
    let refreshTimeout: number | null = null;

    const track = (unsubscribe: () => Promise<void>) => {
      if (disposed) {
        void unsubscribe();
        return;
      }
      disposers.push(unsubscribe);
    };

    const fetchSnapshot = () => {
      void relayClient
        .fetchFirstEvent({ kinds: [KIND_NIP43_MEMBERSHIP_LIST], limit: 1 })
        .then((snapshot) => {
          if (!disposed && snapshot) void handleSnapshot(snapshot);
        })
        .catch(() => {
          // Best effort: the live 13534 subscription still delivers.
        });
    };

    /**
     * Coalesce refetches on a trailing window.
     *
     * Each refetch is a REQ frame, and REQ is billed against the same per-
     * principal `WsEvents` budget as the user's own sends (default 10/s over a
     * 5s window). A bulk add emits one kind:8000 per member, so an uncoalesced
     * 1:1 refetch would spend the budget the owner needs for messages and
     * channel opens — rate-limiting them out of their own app. One snapshot is
     * authoritative for the whole burst, so the trailing edge loses nothing.
     */
    const refreshSnapshot = () => {
      if (disposed || refreshTimeout !== null) return;
      refreshTimeout = window.setTimeout(() => {
        refreshTimeout = null;
        if (!disposed) fetchSnapshot();
      }, MEMBER_REFRESH_DEBOUNCE_MS);
    };

    void relayClient
      .subscribeLive({ kinds: [KIND_NIP43_MEMBERSHIP_LIST], limit: 1 }, (e) => {
        if (!disposed) void handleSnapshot(e);
      })
      .then(track)
      .catch((error) => {
        console.error("Couldn’t subscribe to community membership", error);
      });

    // Accelerator only: refetch the authoritative snapshot instead of trusting
    // the delta, so the ledger only ever sees one consistent roster view.
    void relayClient
      .subscribeLive({ kinds: [KIND_NIP43_MEMBER_ADDED], limit: 0 }, () => {
        if (!disposed) refreshSnapshot();
      })
      .then(track)
      .catch((error) => {
        console.error("Couldn’t subscribe to community joins", error);
      });

    // A reconnect can span joins that landed while the socket was down, and
    // `limit: 1` backfill is not guaranteed to redeliver them.
    const unsubscribeReconnect =
      relayClient.subscribeToReconnects(refreshSnapshot);

    return () => {
      disposed = true;
      if (refreshTimeout !== null) window.clearTimeout(refreshTimeout);
      // Retire the session before dropping the batch, so any flush already past
      // the refs sees `sessionRef.current !== session` and stops. Guarded in
      // case a later run has already installed its own.
      if (sessionRef.current === session) sessionRef.current = null;
      // Drop the queued batch too, not just its timer: on a community switch
      // this effect re-keys, and keys accumulated for the old community must
      // not flush against the new one.
      clearPending();
      unsubscribeReconnect();
      for (const dispose of disposers) void dispose();
    };
  }, [active, communityId, normalizedViewer, clearPending]);
}
