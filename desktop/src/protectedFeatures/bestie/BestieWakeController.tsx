import * as React from "react";

import { useChannelsQuery } from "@/features/channels/hooks";
import {
  useChannelMessagesQuery,
  useChannelSubscription,
  useSendMessageMutation,
} from "@/features/messages/hooks";
import { useThreadRepliesForRoots } from "@/features/messages/useThreadReplies";
import type { RelayEvent } from "@/shared/api/types";
import { getAgentWorkingState } from "@/features/agents/agentWorkingSignal";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  clearBestieUnreadMessage,
  ingestBestieAgentMessageCreatedAt,
  markBestieAgentMessagesSeen,
  noteBestieAgentMessageCreatedAt,
} from "./bestieAttentionStore";
import { isBestiePopoverSystemNoise } from "./bestiePopoverNewMessage";
import {
  BESTIE_COFFEE_BREW_EVENT,
  BESTIE_COFFEE_RUN_MARKER,
  formatBestieCoffeeRunPrompt,
  isBestieAgentOnlineForCoffee,
  logBestieCoffeeFire,
} from "./bestieCoffeeSchedule";
import {
  isBestieCoffeeCaptureMessageKind,
  isBestieCoffeePendingStale,
  messageLooksLikeBestieCoffeeTrigger,
  messageLooksLikeBestieCompetingSystemTrigger,
} from "./bestieCoffeeLive";
import { startBestieCoffeeRunner } from "./bestieCoffeeRunner";
import {
  abandonBestieCoffeePendingForScope,
  applyBestieCoffeeAgentReply,
  beginBestieCoffeeRunForScope,
  claimMissedBestieCoffeeScheduleForScope,
  clearBestieCoffeePendingRunForScope,
  getBestieCoffeeState,
  isBestieCoffeeStubEntry,
  setBestieCoffeePendingTriggerForScope,
  useBestieCoffee,
} from "./bestieCoffeeStore";
import {
  applyBestieJobActionsFromAgentMessage,
  applyBestieJobIntentFromUserMessage,
  getBestieJobState,
  markBestieJobFiredForScope,
  useBestieJobs,
} from "./bestieJobStore";
import {
  applyBestieScratchActionsFromAgentMessage,
  applyBestieScratchIntentFromUserMessage,
} from "./bestieScratchStore";
import {
  BESTIE_JOB_RUN_MARKER,
  formatBestieJobRunPrompt,
} from "./bestieJobSchedule";
import { startBestieJobRunner } from "./bestieJobRunner";
import {
  applyBestieListActionsFromAgentMessage,
  applyBestieListIntentFromUserMessage,
  getBestieListState,
  useBestieList,
} from "./bestieListStore";
import {
  clearBestieNudge,
  getBestieNudge,
  setBestieNudge,
  type BestieNudge,
} from "./bestieNudgeStore";
import { findBestieDmChannel } from "./filterBestieDmChannels";
import {
  BESTIE_WAKE_INTERVAL_MS,
  startBestieWakeScheduler,
} from "./bestieWakeScheduler";
import {
  BESTIE_THREAD_SUMMARIZE_EVENT,
  BESTIE_THREAD_SUMMARIZE_MARKER,
  formatBestieThreadSummarizePrompt,
  parseBestieThreadIdFromSummarizePrompt,
} from "./bestieThreadProtocol";
import {
  abandonBestieThreadSummarizeForScope,
  applyBestieThreadSummarizeReply,
  clearBestieThreadSummarizeForScope,
  getBestieThreadState,
  setBestieThreadSummarizeTriggerForScope,
  useBestieThreads,
} from "./bestieThreadStore";
import {

  isBestieThreadSummarizePendingStale,
  messageLooksLikeBestieSummarizeCompetingTrigger,
  BESTIE_THREAD_SUMMARIZE_ABANDONED_OUTPUT,
} from "./bestieThreadSummarizeLive";
import { useBestie } from "./useBestie";

/** Top-level DM marker for due-reminder notifications (never threaded). */
export const BESTIE_REMINDER_NOTIFY_MARKER = "[Bestie reminder]";

function formatBestieReminderNotifyPrompt(nudge: BestieNudge): string {
  return `${BESTIE_REMINDER_NOTIFY_MARKER}\n\n${nudge.title}\n${nudge.body}`;
}

/**
 * Mounts wake loop, Jobs runner, Coffee scheduler, and watches Assistant DM
 * messages for list/job fences + NL intents + coffee reply capture.
 *
 * Jobs, due-reminder notifies, coffee, and summarize always post as **new
 * top-level Assistant DM messages** (`parentEventId: null`) — never a second
 * channel and never thread replies. Mid-turn notifies rely on ACP
 * `BUZZ_ACP_MULTIPLE_EVENT_HANDLING=queue` so they wait for the current
 * thinking turn, then run as their own top-level turn (not Drop/Steer).
 */
/** Process-wide lock — Strict Mode / remount must not double-send coffee. */
let bestieCoffeeSendLock = false;

export function BestieWakeController() {

  const bestie = useBestie();
  const identityQuery = useIdentityQuery();
  const channelsQuery = useChannelsQuery();
  const ownerPubkey = identityQuery.data?.pubkey ?? "";
  const agentPubkey = bestie.assignedAgent?.pubkey ?? "";
  const relayUrl = bestie.relayUrl ?? "";
  const presenceStatus = bestie.presenceStatus;

  const bestieChannel = React.useMemo(
    () =>
      findBestieDmChannel(
        channelsQuery.data ?? [],
        ownerPubkey,
        agentPubkey || null,
      ),
    [agentPubkey, channelsQuery.data, ownerPubkey],
  );

  const messagesQuery = useChannelMessagesQuery(bestieChannel);
  // Keep Bestie DM live even when the user is elsewhere so Brew /
  // Summarize in-thread replies hit the thread-replies cache.
  useChannelSubscription(bestieChannel);
  const sendMutation = useSendMessageMutation(
    bestieChannel,
    identityQuery.data,
  );

  const listScope = React.useMemo(() => {
    if (!relayUrl || !ownerPubkey || !agentPubkey) return null;
    return {
      agentPubkey: normalizePubkey(agentPubkey),
      ownerPubkey: normalizePubkey(ownerPubkey),
      relayUrl,
    };
  }, [agentPubkey, ownerPubkey, relayUrl]);

  const listState = useBestieList(listScope);
  const jobState = useBestieJobs(listScope);
  const coffeeState = useBestieCoffee(listScope);
  const threadCaptureState = useBestieThreads(listScope);

  // Agent Brew / Summarize replies parent to the top-level trigger, so they
  // live in the thread-replies cache — NOT the roots-only channel window.
  // Load those subtrees or capture never sees the outcome.
  const captureReplyRootIds = React.useMemo(() => {
    const ids: string[] = [];
    const seen = new Set<string>();
    const push = (id: string | null | undefined) => {
      if (!id || seen.has(id)) return;
      seen.add(id);
      ids.push(id);
    };
    push(coffeeState.pendingRun?.triggerMessageId);
    push(threadCaptureState.pendingSummarize?.triggerMessageId);
    // Open Coffee stubs (empty / abandoned / unbound) must load thread replies
    // for Path C upgrade — including rows that somehow have a reply id but no body.
    for (const entry of coffeeState.entries) {
      if (!isBestieCoffeeStubEntry(entry)) continue;
      push(entry.triggerMessageId);
    }
    const ownerNorm = ownerPubkey ? normalizePubkey(ownerPubkey) : null;
    if (ownerNorm) {
      for (const event of messagesQuery.data ?? []) {
        if (typeof event.content !== "string") continue;
        if (normalizePubkey(event.pubkey) !== ownerNorm) continue;
        if (messageLooksLikeBestieCoffeeTrigger(event.content)) {
          push(event.id);
          continue;
        }
        if (event.content.includes(BESTIE_THREAD_SUMMARIZE_MARKER)) {
          push(event.id);
        }
      }
    }
    return ids;
  }, [
    coffeeState.entries,
    coffeeState.pendingRun?.triggerMessageId,
    messagesQuery.data,
    ownerPubkey,
    threadCaptureState.pendingSummarize?.triggerMessageId,
  ]);
  const captureThreadReplies = useThreadRepliesForRoots(
    bestieChannel,
    captureReplyRootIds,
  );
  const ensureAgentRunningRef = React.useRef(bestie.ensureAgentRunning);
  ensureAgentRunningRef.current = bestie.ensureAgentRunning;
  const sendMutateRef = React.useRef(sendMutation.mutateAsync);
  sendMutateRef.current = sendMutation.mutateAsync;
  const channelRef = React.useRef(bestieChannel);
  channelRef.current = bestieChannel;
  const presenceRef = React.useRef(presenceStatus);
  presenceRef.current = presenceStatus;
  const seededUnreadRef = React.useRef(false);
  const seededUnreadChannelRef = React.useRef<string | null>(null);
  /** Agent message ids already absorbed for footer attention (hydrate + live). */
  const seenAgentMessageIdsRef = React.useRef(new Set<string>());
  const firingJobIdsRef = React.useRef(new Set<string>());
  const coffeeSendingRef = React.useRef(false);
  const postedReminderNudgeIdsRef = React.useRef(new Set<string>());

  const sendTopLevel = React.useCallback(async (content: string) => {
    const channel = channelRef.current;
    if (!channel) return;
    await sendMutateRef.current({
      content,
      // Product rule: Jobs / Reminders / Coffee never parent into a thread.
      parentEventId: null,
      targetChannel: channel,
    });
  }, []);

  const runCoffeeTurn = React.useCallback(
    async (
      source: "scheduled" | "brew",
      options?: { alreadyBegun?: boolean; path?: "schedule" | "brew-click" },
    ) => {
      if (!listScope || coffeeSendingRef.current || bestieCoffeeSendLock) return;
      // Allowed fires only: explicit Brew click, or scheduled begin already
      // claimed inside the 08:00 window. Reject any other caller.
      const pending = getBestieCoffeeState(listScope).pendingRun;
      if (source === "brew") {
        if (!pending || pending.source !== "brew") {
          console.warn("[bestie-coffee] reject brew fire without brew lock", {
            pending,
          });
          return;
        }
      } else if (source === "scheduled") {
        if (!options?.alreadyBegun) {
          console.warn("[bestie-coffee] reject scheduled fire without begin", {
            pending,
          });
          return;
        }
        if (!pending || pending.source !== "scheduled") {
          console.warn(
            "[bestie-coffee] reject scheduled fire without scheduled lock",
            { pending },
          );
          return;
        }
      }
      // Trigger already posted for this run — never send another [Bestie coffee].
      if (pending.triggerMessageId) {
        console.info("[bestie-coffee] skip re-send; trigger already posted", {
          source,
          triggerMessageId: pending.triggerMessageId,
        });
        return;
      }
      coffeeSendingRef.current = true;
      bestieCoffeeSendLock = true;
      const path = options?.path ?? (source === "brew" ? "brew-click" : "schedule");
      try {
        logBestieCoffeeFire({
          path,
          source,
          detail: {
            startedAt: pending.startedAt,
            dayKeyClaimed:
              source === "scheduled"
                ? getBestieCoffeeState(listScope).lastScheduledDayKey
                : undefined,
          },
        });
        await ensureAgentRunningRef.current();
        // Re-check after await — remount/race may have posted already.
        const pendingAfter = getBestieCoffeeState(listScope).pendingRun;
        if (
          !pendingAfter ||
          pendingAfter.source !== source ||
          pendingAfter.triggerMessageId
        ) {
          console.info("[bestie-coffee] skip send after await", {
            source,
            pendingAfter,
          });
          return;
        }
        await sendTopLevel(formatBestieCoffeeRunPrompt(source));
      } catch {
        // Keep lastScheduledDayKey — day was claimed; only drop the lock.
        clearBestieCoffeePendingRunForScope(listScope);
      } finally {
        coffeeSendingRef.current = false;
        bestieCoffeeSendLock = false;
      }
    },
    [listScope, sendTopLevel],
  );

  // Agent fence + user NL for lists/jobs; coffee reply capture; footer unread.
  React.useEffect(() => {
    if (!listScope || !bestieChannel || !agentPubkey || !ownerPubkey) return;
    if (seededUnreadChannelRef.current !== bestieChannel.id) {
      seededUnreadChannelRef.current = bestieChannel.id;
      seededUnreadRef.current = false;
      seenAgentMessageIdsRef.current = new Set();
      // Drop any leftover glow from a prior channel / hot reload.
      clearBestieUnreadMessage();
    }
    const agentNorm = normalizePubkey(agentPubkey);
    const ownerNorm = normalizePubkey(ownerPubkey);
    const events = messagesQuery.data ?? [];

    // Pass 1: bind coffee + summarize pending → trigger ids; collect unmatched
    // triggers so failure / NCP / error replies still land on Coffee / Threads.
    const unmatchedCoffeeTriggers = new Map();
    const unmatchedSummarizeTriggers = new Map();
    {
      const coffeeState = getBestieCoffeeState(listScope);
      // Only real captures (with a reply id) count as matched — abandoned
      // timeout rows keep replyMessageId null so a late reply can upgrade.
      const matchedCoffeeTriggerIds = new Set(
        coffeeState.entries
          .filter((entry) => entry.replyMessageId)
          .map((entry) => entry.triggerMessageId)
          .filter((id): id is string => typeof id === "string" && id.length > 0),
      );
      const forgottenCoffeeTriggerIds = new Set(
        coffeeState.forgottenTriggerIds,
      );
      const threadState = getBestieThreadState(listScope);
      // Prefer the newest owner coffee trigger at/after pending start so an
      // older leftover /hula-coffee root cannot steal the bind (or capture).
      {
        const livePending = getBestieCoffeeState(listScope).pendingRun;
        if (livePending && !livePending.triggerMessageId) {
          let bestId: string | null = null;
          let bestAt = -1;
          for (const event of events) {
            if (typeof event.content !== "string" || event.content.length === 0) {
              continue;
            }
            if (normalizePubkey(event.pubkey) !== ownerNorm) continue;
            if (!messageLooksLikeBestieCoffeeTrigger(event.content)) continue;
            const createdAt =
              typeof event.created_at === "number"
                ? event.created_at
                : livePending.startedAt;
            if (createdAt + 5 < livePending.startedAt) continue;
            if (createdAt >= bestAt) {
              bestAt = createdAt;
              bestId = event.id;
            }
          }
          if (bestId) {
            setBestieCoffeePendingTriggerForScope(listScope, bestId);
          }
        }
      }
      // Seed from open stubs first — trigger may have aged out of the
      // roots-only channel window while the in-thread agent reply is still
      // loadable via thread-replies.
      for (const entry of coffeeState.entries) {
        if (!entry.triggerMessageId || !isBestieCoffeeStubEntry(entry)) continue;
        if (forgottenCoffeeTriggerIds.has(entry.triggerMessageId)) continue;
        if (matchedCoffeeTriggerIds.has(entry.triggerMessageId)) continue;
        unmatchedCoffeeTriggers.set(entry.triggerMessageId, entry.source);
      }

      for (const event of events) {
        if (typeof event.content !== "string" || event.content.length === 0) {
          continue;
        }
        if (normalizePubkey(event.pubkey) !== ownerNorm) continue;

        if (messageLooksLikeBestieCoffeeTrigger(event.content)) {
          if (
            !matchedCoffeeTriggerIds.has(event.id) &&
            !forgottenCoffeeTriggerIds.has(event.id)
          ) {
            const pendingNow = getBestieCoffeeState(listScope).pendingRun;
            const entrySource = coffeeState.entries.find(
              (entry) => entry.triggerMessageId === event.id,
            )?.source;
            const source =
              entrySource ??
              pendingNow?.source ??
              (coffeeState.lastScheduledDayKey != null ? "scheduled" : "brew");
            unmatchedCoffeeTriggers.set(event.id, source);
          }
          continue;
        }

        if (event.content.includes(BESTIE_THREAD_SUMMARIZE_MARKER)) {
          const livePending = getBestieThreadState(listScope).pendingSummarize;
          if (livePending && !livePending.triggerMessageId) {
            setBestieThreadSummarizeTriggerForScope(listScope, event.id);
          }
          const trackingId =
            parseBestieThreadIdFromSummarizePrompt(event.content) ??
            getBestieThreadState(listScope).pendingSummarize?.threadId ??
            null;
          if (!trackingId) continue;
          // Skip if this thread already has a *real* summary after this prompt.
          // Abandoned timeout text must stay unmatched so a late in-thread
          // reply can still upgrade the row.
          const tracked = threadState.threads.find(
            (thread) => thread.id === trackingId,
          );
          if (
            tracked?.lastSummaryAt != null &&
            typeof event.created_at === "number" &&
            event.created_at + 5 < tracked.lastSummaryAt &&
            tracked.lastSummary !== BESTIE_THREAD_SUMMARIZE_ABANDONED_OUTPUT
          ) {
            continue;
          }
          unmatchedSummarizeTriggers.set(event.id, trackingId);
        }
      }
    }

    // Merge in-thread replies under coffee/summarize triggers (roots-only
    // channel window never includes them).
    const replyEvents = captureThreadReplies.events ?? [];
    const eventsForAgentCapture: RelayEvent[] = [];
    const seenEventIds = new Set<string>();
    for (const event of [...events, ...replyEvents]) {
      if (seenEventIds.has(event.id)) continue;
      seenEventIds.add(event.id);
      eventsForAgentCapture.push(event);
    }

    const agentCreatedAts = [];
    const newAgentAttentionEvents: RelayEvent[] = [];
    for (const event of eventsForAgentCapture) {
      if (typeof event.content !== "string" || event.content.length === 0) {
        continue;
      }
      const author = normalizePubkey(event.pubkey);
      if (author === agentNorm) {
        // Capture + attention only for chat message rows — channel windows also
        // carry kind:7 ACP 👀/💬 reactions on the coffee trigger. Writing the
        // capture from this same observation loop must skip those or Coffee
        // finalizes as 👀 and never upgrades when the real reply lands.
        const isCaptureMessage = isBestieCoffeeCaptureMessageKind(event.kind);
        if (typeof event.created_at === "number" && isCaptureMessage) {
          agentCreatedAts.push(event.created_at);
        }
        if (isCaptureMessage && !seenAgentMessageIdsRef.current.has(event.id)) {
          seenAgentMessageIdsRef.current.add(event.id);
          newAgentAttentionEvents.push(event);
        }
        if (!isCaptureMessage) {
          continue;
        }
        const createdAt =
          typeof event.created_at === "number"
            ? event.created_at
            : Math.floor(Date.now() / 1000);
        applyBestieCoffeeAgentReply(
          listScope,
          event.id,
          event.content,
          createdAt,
          event.tags,
          unmatchedCoffeeTriggers,
        );
        applyBestieThreadSummarizeReply(
          listScope,
          event.id,
          event.content,
          createdAt,
          event.tags,
          unmatchedSummarizeTriggers,
        );
        applyBestieListActionsFromAgentMessage(
          listScope,
          event.id,
          event.content,
        );
        applyBestieJobActionsFromAgentMessage(
          listScope,
          event.id,
          event.content,
        );
        applyBestieScratchActionsFromAgentMessage(
          listScope,
          event.id,
          event.content,
        );
        continue;
      }
      if (author === ownerNorm) {
        // System-injected turns — skip NL list/job parsers.
        if (
          event.content.includes(BESTIE_JOB_RUN_MARKER) ||
          event.content.includes(BESTIE_COFFEE_RUN_MARKER) ||
          event.content.includes(BESTIE_REMINDER_NOTIFY_MARKER) ||
          event.content.includes(BESTIE_THREAD_SUMMARIZE_MARKER)
        ) {
          continue;
        }
        applyBestieListIntentFromUserMessage(
          listScope,
          event.id,
          event.content,
        );
        applyBestieJobIntentFromUserMessage(listScope, event.id, event.content);
        applyBestieScratchIntentFromUserMessage(
          listScope,
          event.id,
          event.content,
        );
      }
    }

    if (agentCreatedAts.length === 0) {
      // Still finish hydrate when the DM has no agent posts yet but reply
      // capture for coffee/summarize roots has settled.
      if (
        !seededUnreadRef.current &&
        !(captureReplyRootIds.length > 0 && captureThreadReplies.isPending)
      ) {
        clearBestieUnreadMessage();
        seededUnreadRef.current = true;
      }
      return;
    }
    const maxCreated = Math.max(...agentCreatedAts);
    if (!seededUnreadRef.current) {
      // History (roots + late thread-reply hydrate) is never "new".
      markBestieAgentMessagesSeen(maxCreated);
      clearBestieUnreadMessage();
      const repliesPending =
        captureReplyRootIds.length > 0 && captureThreadReplies.isPending;
      if (!repliesPending) {
        seededUnreadRef.current = true;
      }
      return;
    }
    // Live path: only brand-new agent ids, and never system-noise turns
    // (coffee / jobs / reminder / summarize / teach) — those must not pulse
    // when the popover has no New-message banner.
    for (const event of newAgentAttentionEvents) {
      if (typeof event.created_at !== "number") continue;
      if (isBestiePopoverSystemNoise(event.content)) {
        noteBestieAgentMessageCreatedAt(event.created_at);
        continue;
      }
      ingestBestieAgentMessageCreatedAt(event.created_at);
    }
  }, [
    agentPubkey,
    bestieChannel,
    captureReplyRootIds.length,
    captureThreadReplies.events,
    captureThreadReplies.isPending,
    listScope,
    messagesQuery.data,
    ownerPubkey,
  ]);

  const wakeHandlesRef = React.useRef<ReturnType<
    typeof startBestieWakeScheduler
  > | null>(null);
  const jobHandlesRef = React.useRef<ReturnType<
    typeof startBestieJobRunner
  > | null>(null);
  const coffeeHandlesRef = React.useRef<ReturnType<
    typeof startBestieCoffeeRunner
  > | null>(null);

  // Autonomous wake + proactive nudge; due-reminder also posts top-level DM.
  React.useEffect(() => {
    if (!listScope) return;
    const handles = startBestieWakeScheduler({
      getListState: () => getBestieListState(listScope),
      getPreviousNudgeId: () => getBestieNudge()?.id ?? null,
      intervalMs: BESTIE_WAKE_INTERVAL_MS,
      onNudge: (nudge) => {
        setBestieNudge(nudge);
        if (nudge.reason !== "due-reminder") return;
        if (postedReminderNudgeIdsRef.current.has(nudge.id)) return;
        postedReminderNudgeIdsRef.current.add(nudge.id);
        void (async () => {
          try {
            await ensureAgentRunningRef.current();
            await sendTopLevel(formatBestieReminderNotifyPrompt(nudge));
          } catch {
            // Best-effort notify.
          }
        })();
      },
      onWakeAgent: () => {
        void ensureAgentRunningRef.current().catch(() => {
          // Best-effort wake.
        });
      },
    });
    wakeHandlesRef.current = handles;
    return () => {
      handles.stop();
      wakeHandlesRef.current = null;
    };
  }, [listScope, sendTopLevel]);

  // Jobs: when due, mark fired once and send the job prompt as a top-level turn.
  React.useEffect(() => {
    if (!listScope) return;
    const handles = startBestieJobRunner({
      getJobState: () => getBestieJobState(listScope),
      intervalMs: BESTIE_WAKE_INTERVAL_MS,
      onDueJobs: (jobs) => {
        for (const job of jobs) {
          if (job.nextDueAt == null) continue;
          if (firingJobIdsRef.current.has(job.id)) continue;
          const dueAt = job.nextDueAt;
          const marked = markBestieJobFiredForScope(listScope, job.id, dueAt);
          if (!marked) continue;
          firingJobIdsRef.current.add(job.id);
          const content = formatBestieJobRunPrompt(job.title, job.prompt);
          void (async () => {
            try {
              await ensureAgentRunningRef.current();
              await sendTopLevel(content);
            } catch {
              // Best-effort; slot already marked so we do not double-send.
            } finally {
              firingJobIdsRef.current.delete(job.id);
            }
          })();
        }
      },
    });
    jobHandlesRef.current = handles;
    return () => {
      handles.stop();
      jobHandlesRef.current = null;
    };
  }, [listScope, sendTopLevel]);

  // Coffee: ONLY (1) Brew click event (2) once inside local 08:00 window.
  // Wake / Jobs / NL / remount / presence must not start a turn outside that.
  React.useEffect(() => {
    if (!listScope) return;
    const handles = startBestieCoffeeRunner({
      getCoffeeState: () => getBestieCoffeeState(listScope),
      getIsAgentOnline: () =>
        isBestieAgentOnlineForCoffee(presenceRef.current),
      intervalMs: BESTIE_WAKE_INTERVAL_MS,
      onDueCoffee: () => {
        // Claim day + pending *before* any await so wake/remount re-ticks
        // cannot pass the schedule gate a second time.
        const begun = beginBestieCoffeeRunForScope(listScope, "scheduled");
        if (!begun) return;
        void runCoffeeTurn("scheduled", {
          alreadyBegun: true,
          path: "schedule",
        });
      },
      onMissedWindow: (nowSeconds) => {
        claimMissedBestieCoffeeScheduleForScope(listScope, nowSeconds);
        console.info("[bestie-coffee] missed-window claim", {
          at: new Date(nowSeconds * 1000).toISOString(),
          path: "schedule-miss",
        });
      },
    });
    coffeeHandlesRef.current = handles;

    const onBrew = () => {
      void runCoffeeTurn("brew", { path: "brew-click" });
    };
    window.addEventListener(BESTIE_COFFEE_BREW_EVENT, onBrew);
    return () => {
      handles.stop();
      coffeeHandlesRef.current = null;
      window.removeEventListener(BESTIE_COFFEE_BREW_EVENT, onBrew);
    };
  }, [listScope, runCoffeeTurn]);

  // When open reminders / todos change, re-tick wake one-shots.
  const openListSignature = React.useMemo(
    () =>
      listState.items
        .filter((item) => item.status === "open")
        .map((item) => `${item.id}:${item.kind}:${item.dueAt ?? ""}`)
        .sort()
        .join("|"),
    [listState],
  );
  React.useEffect(() => {
    if (!listScope || !openListSignature) return;
    wakeHandlesRef.current?.tick();
  }, [listScope, openListSignature]);

  const openJobSignature = React.useMemo(
    () =>
      jobState.jobs
        .filter((job) => job.enabled)
        .map((job) => `${job.id}:${job.nextDueAt ?? ""}`)
        .sort()
        .join("|"),
    [jobState],
  );
  React.useEffect(() => {
    if (!listScope || !openJobSignature) return;
    jobHandlesRef.current?.tick();
  }, [listScope, openJobSignature]);

  React.useEffect(() => {
    if (!listScope) return;
    coffeeHandlesRef.current?.tick();
  }, [coffeeState.pendingRun, coffeeState.lastScheduledDayKey, listScope, presenceStatus]);

  // Drop / finalize abandoned coffee + summarize pending (idle, hard timeout,
  // or superseded by a newer system turn) so Brew/Summarize cannot stick on 👀.
  React.useEffect(() => {
    if (!listScope || !bestieChannel) return;
    const check = () => {
      const nowSeconds = Math.floor(Date.now() / 1000);
      const working = getAgentWorkingState(
        listScope.agentPubkey,
        bestieChannel.id,
      ).working;
      const events = messagesQuery.data ?? [];
      const ownerNorm = ownerPubkey ? normalizePubkey(ownerPubkey) : null;
      let latestCoffeeCompeting: number | null = null;
      let latestSummarizeCompeting: number | null = null;
      if (ownerNorm) {
        for (const event of events) {
          if (typeof event.content !== "string") continue;
          if (normalizePubkey(event.pubkey) !== ownerNorm) continue;
          const createdAt =
            typeof event.created_at === "number"
              ? event.created_at
              : nowSeconds;
          if (messageLooksLikeBestieCompetingSystemTrigger(event.content)) {
            if (
              latestCoffeeCompeting == null ||
              createdAt > latestCoffeeCompeting
            ) {
              latestCoffeeCompeting = createdAt;
            }
          }
          if (messageLooksLikeBestieSummarizeCompetingTrigger(event.content)) {
            if (
              latestSummarizeCompeting == null ||
              createdAt > latestSummarizeCompeting
            ) {
              latestSummarizeCompeting = createdAt;
            }
          }
        }
      }
      const coffeePending = getBestieCoffeeState(listScope).pendingRun;
      if (
        coffeePending &&
        isBestieCoffeePendingStale({
          agentWorkingOnBestieDm: working,
          nowSeconds,
          pendingRun: coffeePending,
          latestCompetingTriggerAt: latestCoffeeCompeting,
        })
      ) {
        abandonBestieCoffeePendingForScope(listScope);
      }
      const summarizePending = getBestieThreadState(listScope).pendingSummarize;
      if (
        summarizePending &&
        isBestieThreadSummarizePendingStale({
          agentWorkingOnBestieDm: working,
          nowSeconds,
          pendingSummarize: summarizePending,
          latestCompetingTriggerAt: latestSummarizeCompeting,
        })
      ) {
        abandonBestieThreadSummarizeForScope(listScope);
      }
    };
    check();
    const timer = window.setInterval(check, 15_000);
    return () => window.clearInterval(timer);
  }, [bestieChannel, coffeeState.pendingRun, listScope, messagesQuery.data, ownerPubkey]);

  // Clear stale nudge when the outstanding set is emptied.
  React.useEffect(() => {
    const nudge = getBestieNudge();
    if (!nudge) return;
    const stillOpen = nudge.itemIds.some((id) =>
      listState.items.some((item) => item.id === id && item.status === "open"),
    );
    if (!stillOpen) clearBestieNudge();
  }, [listState]);

  // Threads: Summarize button → top-level Bestie DM turn with summarize prompt.
  React.useEffect(() => {
    if (!listScope) return;
    const onSummarize = (event: Event) => {
      const detail = (event as CustomEvent<{ threadId?: string }>).detail;
      const threadId = detail?.threadId;
      if (!threadId) return;
      const state = getBestieThreadState(listScope);
      const thread = state.threads.find((entry) => entry.id === threadId);
      if (!thread) {
        clearBestieThreadSummarizeForScope(listScope);
        return;
      }
      void (async () => {
        try {
          await ensureAgentRunningRef.current();
          await sendTopLevel(formatBestieThreadSummarizePrompt(thread));
        } catch {
          clearBestieThreadSummarizeForScope(listScope);
        }
      })();
    };
    window.addEventListener(BESTIE_THREAD_SUMMARIZE_EVENT, onSummarize);
    return () =>
      window.removeEventListener(BESTIE_THREAD_SUMMARIZE_EVENT, onSummarize);
  }, [listScope, sendTopLevel]);

  return null;
}
