import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import {
  reconcileReviewFeedbackEvent,
  sendArtifactFeedbackMessage,
} from "@/shared/api/tauriArtifacts";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";
import { assertReviewHeadCurrent } from "./hooks";
import type {
  FeedbackViewport,
  ReviewBlock,
  ReviewRevision,
} from "./lib/reviewContract";
import {
  type ReviewFeedbackDeps,
  ReviewFeedbackSubmission,
} from "./lib/reviewFeedbackSubmission";
import {
  createReviewOutbox,
  type FeedbackPublishState,
} from "./lib/reviewOutbox";
import {
  findSubmission,
  forgetSubmission,
  listUnfinished,
  rememberSubmission,
} from "./lib/reviewSubmissionStore";
import type { ReviewCommentPhase } from "./ui/ReviewCommentForm";

/** Where keyboard focus returns when the comment form closes. */
export type ReturnFocus =
  | { kind: "frame"; blockId: string }
  | { kind: "element"; element: HTMLElement };

export type ReviewScope = { relayUrl?: string; signerPubkey?: string };

/**
 * The mounted workbench's live tenant scope. A submission outlives the
 * workbench that created it (see `reviewSubmissionStore`), so it must not
 * close over one workbench's props: it reads whichever workbench is mounted
 * now, and reads nothing while none is, which fails closed.
 */
let mountedScope: (() => ReviewScope) | null = null;

/**
 * The durable outbox of unfinished signed comments, in the webview's local
 * storage (see `reviewOutbox`): no key material, and bounded.
 */
const liveOutbox = createReviewOutbox({
  getItem: (key) => window.localStorage.getItem(key),
  setItem: (key, value) => {
    if (!setLocalStorageItemWithRecovery(key, value)) {
      throw new Error("Local storage is not writable.");
    }
  },
  removeItem: (key) => window.localStorage.removeItem(key),
});

/**
 * Live dependencies. Publication carries an `isCurrent` guard, evaluated
 * against the live community and account immediately before every send and
 * retry, so a community switch during signing or a retry can never route a
 * signed comment to another tenant. Exported so that scope guard can be
 * exercised without the network.
 */
export const liveFeedbackDeps: ReviewFeedbackDeps = {
  newFeedbackId: () => crypto.randomUUID(),
  nowSeconds: () => Math.floor(Date.now() / 1000),
  signEvent: (template) =>
    signRelayEvent({
      kind: template.kind,
      content: template.content,
      tags: template.tags,
    }),
  publishEvent: (event, isCurrent) =>
    relayClient.publishEvent(
      event,
      "Timed out publishing feedback.",
      "Failed to publish feedback.",
      isCurrent,
    ),
  sendNotification: sendArtifactFeedbackMessage,
  isScopeCurrent: (context) => {
    const scope = mountedScope?.();
    return (
      scope !== undefined &&
      scope.relayUrl === context.expectedRelayUrl &&
      scope.signerPubkey?.toLowerCase() ===
        context.expectedSignerPubkey.toLowerCase()
    );
  },
  assertHeadCurrent: assertReviewHeadCurrent,
  reconcileFeedback: reconcileReviewFeedbackEvent,
  outbox: liveOutbox,
};

type UseReviewComposerOptions = {
  revision: ReviewRevision;
  blocks: ReviewBlock[];
  /** Null when the notification named no agent: feedback cannot be sent. */
  executiveAgentPubkey: string | null;
  parentEventId: string;
  rootEventId: string;
  /**
   * Why creating and signing a new comment is paused right now (checking for a
   * newer revision, the head check failed, a newer revision exists), or null
   * when it may proceed. A frozen draft is never held back by it: retrying
   * only re-sends its identical signed event or wake, which the relay
   * reconciles whatever the head is.
   */
  blockedReason: string | null;
  measureViewport: () => FeedbackViewport;
  focusFrameBlock: (id: string) => boolean;
  /** Live tenant scope; both parts are required to submit. */
  getScope: () => ReviewScope;
  onFeedbackSent: () => void;
  deps?: ReviewFeedbackDeps;
};

/** A signed comment that has not finished: feedback published and agent woken. */
export type PendingDelivery = {
  blockId: string;
  blockTitle: string;
  /** What is known about the relay holding the signed feedback. */
  publishState: FeedbackPublishState;
  /** The relay accepted the feedback; only the agent notification remains. */
  feedbackPublished: boolean;
  /**
   * The signed feedback revision, whenever the relay may hold it. It is what
   * ties a comment the relay lists to the wake still owed for it.
   */
  feedbackEventId: string | null;
  /** The relay provably holds nothing, so the comment may be abandoned. */
  canDiscard: boolean;
};

/**
 * A signed, unfinished comment that belongs to a different revision of the
 * reviewed artifact than the one open. Its form, Retry, and Discard belong to
 * the exact revision it was signed against, so it is listed here only to stay
 * discoverable and to name that revision.
 */
export type OtherRevisionDelivery = PendingDelivery & {
  /** The exact reviewed revision the comment was signed against. */
  revisionEventId: string;
  revisionNumber: number;
};

function pendingDeliveryOf(
  blockId: string,
  blockTitle: string,
  draft: ReviewFeedbackSubmission,
): PendingDelivery {
  return {
    blockId,
    blockTitle,
    publishState: draft.publishState,
    feedbackPublished: draft.feedbackPublished,
    feedbackEventId: draft.feedbackEventId,
    canDiscard: draft.canDiscard,
  };
}

function describeError(error: unknown): string {
  return error instanceof Error && error.message
    ? error.message
    : "Something went wrong sending your feedback.";
}

/**
 * Reviewer interaction state for the Review Canvas: which block's comment form
 * is open, per-block drafts (a signed draft is frozen and only ever retried),
 * the duplicate-submit guard, focus restoration, and screen-reader
 * announcements. Selection from the frame and from the chrome block list both
 * land in `open`, so every input modality reaches the same form.
 *
 * A signed draft is saved to the durable outbox (`reviewOutbox`) before
 * anything is sent and remembered live (`reviewSubmissionStore`) under the
 * exact relay, signer, channel, artifact, reviewed revision, and block, so
 * neither leaving the workbench nor quitting Buzz loses a comment the relay
 * may already hold: returning to that revision restores its locked form and
 * Retry. It is forgotten only once feedback and wake both completed, or when
 * the reviewer discards a comment the relay provably holds nothing for (never
 * sent, or refused and confirmed absent). An ambiguous or accepted comment is
 * never discardable or replaceable.
 */
export function useReviewComposer(options: UseReviewComposerOptions) {
  const optionsRef = React.useRef(options);
  optionsRef.current = options;
  // This workbench's view of its drafts. The store is the memory across
  // workbenches; this map also keeps a draft visible, and failing closed, if
  // the live scope changes while the workbench stays open.
  const draftsRef = React.useRef(new Map<string, ReviewFeedbackSubmission>());
  const submittingRef = React.useRef(false);
  const mountedRef = React.useRef(true);
  const returnFocusRef = React.useRef<ReturnFocus | null>(null);
  const [activeBlockId, setActiveBlockId] = React.useState<string | null>(null);
  const [texts, setTexts] = React.useState<Record<string, string>>({});
  const [phase, setPhase] = React.useState<ReviewCommentPhase>({
    kind: "idle",
  });
  const [announcement, setAnnouncement] = React.useState("");
  // Re-render after mutating the (ref-held) draft map.
  const [draftVersion, setDraftVersion] = React.useState(0);

  React.useEffect(() => {
    mountedRef.current = true;
    const readScope = () => optionsRef.current.getScope();
    mountedScope = readScope;
    return () => {
      mountedRef.current = false;
      if (mountedScope === readScope) mountedScope = null;
    };
  }, []);

  /**
   * The comment in progress for a block, if one holds work worth keeping: a
   * signed draft, or one mid-run. A draft that never reached the signer holds
   * nothing and is not returned. Falls back to the store so a draft signed in
   * an earlier workbench (same relay, signer, and revision) is found.
   */
  const draftFor = React.useCallback((blockId: string) => {
    const local = draftsRef.current.get(blockId);
    if (local) {
      if (!local.completed && (local.frozen || local.running)) return local;
      draftsRef.current.delete(blockId);
    }
    const { revision, getScope, deps } = optionsRef.current;
    const scope = getScope();
    if (!scope.relayUrl || !scope.signerPubkey) return undefined;
    const stored = findSubmission(
      {
        relayUrl: scope.relayUrl,
        signerPubkey: scope.signerPubkey,
        channelId: revision.channelId,
        artifactId: revision.artifactId,
        revisionEventId: revision.eventId,
        blockId,
      },
      deps ?? liveFeedbackDeps,
    );
    if (!stored || stored.completed || !(stored.frozen || stored.running)) {
      return undefined;
    }
    draftsRef.current.set(blockId, stored);
    return stored;
  }, []);

  const restoreFocus = React.useCallback(() => {
    const target = returnFocusRef.current;
    returnFocusRef.current = null;
    if (!target) return;
    if (target.kind === "frame") {
      // The frame reports whether it could take focus; when it cannot (a
      // degraded or reloading frame), land on the block's chrome entry instead
      // of dropping focus to the page.
      if (!optionsRef.current.focusFrameBlock(target.blockId)) {
        document
          .querySelector<HTMLElement>(
            `[data-testid="review-block-${CSS.escape(target.blockId)}"]`,
          )
          ?.focus();
      }
    } else if (target.element.isConnected) {
      target.element.focus();
    }
  }, []);

  const open = React.useCallback((blockId: string, from: ReturnFocus) => {
    if (submittingRef.current) return;
    returnFocusRef.current = from;
    setActiveBlockId(blockId);
    setPhase({ kind: "idle" });
    setAnnouncement(
      `Comment form opened for ${optionsRef.current.blocks.find((block) => block.id === blockId)?.title ?? "block"}.`,
    );
  }, []);

  const cancel = React.useCallback(() => {
    setActiveBlockId(null);
    setPhase({ kind: "idle" });
    setAnnouncement("Comment cancelled.");
    restoreFocus();
  }, [restoreFocus]);

  const setText = React.useCallback(
    (value: string) => {
      if (activeBlockId === null) return;
      setTexts((current) => ({ ...current, [activeBlockId]: value }));
    },
    [activeBlockId],
  );

  const discard = React.useCallback(async () => {
    if (activeBlockId === null || submittingRef.current) return;
    const blockId = activeBlockId;
    const draft = draftFor(blockId);
    // Only a comment the relay provably holds nothing for may be dropped; an
    // ambiguous or accepted one is never discardable (its Discard is not
    // offered either).
    if (!draft?.canDiscard) return;
    submittingRef.current = true;
    setPhase({ kind: "discarding" });
    setAnnouncement("Checking with the relay before discarding…");
    try {
      const outcome = await draft.discard();
      if (outcome === "held") {
        const message =
          "The relay already holds this feedback, so it was kept. Retry to notify the agent.";
        setPhase({ kind: "error", message });
        setAnnouncement(message);
        return;
      }
      draftsRef.current.delete(blockId);
      forgetSubmission(draft);
      // Hand the text back for editing, including after a remount that lost it.
      setTexts((current) => ({
        ...current,
        [blockId]: current[blockId] ?? draft.request,
      }));
      setPhase({ kind: "idle" });
      setAnnouncement("Comment discarded. You can edit it and send again.");
    } catch (error) {
      const message = describeError(error);
      setPhase({ kind: "error", message });
      setAnnouncement(`The comment was kept. ${message}`);
    } finally {
      submittingRef.current = false;
      setDraftVersion((version) => version + 1);
    }
  }, [activeBlockId, draftFor]);

  const submit = React.useCallback(async () => {
    const current = optionsRef.current;
    const block = current.blocks.find((item) => item.id === activeBlockId);
    if (submittingRef.current || !block) return;
    const existing = draftFor(block.id);
    const agent = current.executiveAgentPubkey;
    // A signed draft finishes with the agent and thread it captured, so only a
    // new comment needs the notification's agent.
    if (!existing && !agent) return;
    // Only a new comment waits on the head; a signed draft retries regardless.
    if (current.blockedReason && !existing) {
      setPhase({ kind: "error", message: current.blockedReason });
      setAnnouncement(`Feedback was not sent. ${current.blockedReason}`);
      return;
    }
    submittingRef.current = true;
    setPhase({ kind: "sending" });
    setAnnouncement("Sending feedback…");
    let draft = existing;
    try {
      if (!draft) {
        const scope = current.getScope();
        if (!scope.relayUrl || !scope.signerPubkey) {
          throw new Error(
            "The active community and account are not ready yet. Try again in a moment.",
          );
        }
        if (!agent) {
          throw new Error("No executive agent is recorded for this review.");
        }
        draft = new ReviewFeedbackSubmission(
          {
            revision: current.revision,
            block,
            request: texts[block.id] ?? "",
            viewport: current.measureViewport(),
            executiveAgentPubkey: agent,
            parentEventId: current.parentEventId,
            rootEventId: current.rootEventId,
            expectedRelayUrl: scope.relayUrl,
            expectedSignerPubkey: scope.signerPubkey,
          },
          current.deps ?? liveFeedbackDeps,
        );
        draftsRef.current.set(block.id, draft);
        // Remembered before the run starts: a workbench that unmounts while
        // the comment is being signed or sent leaves the run to finish or
        // fail, and either outcome stays recoverable.
        rememberSubmission(draft);
      }
      await draft.submit();
      draftsRef.current.delete(block.id);
      forgetSubmission(draft);
      setTexts((existingTexts) => {
        const { [block.id]: _sent, ...rest } = existingTexts;
        return rest;
      });
      setActiveBlockId(null);
      setPhase({ kind: "idle" });
      setAnnouncement(
        `Feedback sent on ${block.title}. The executive agent was notified.`,
      );
      // A workbench that unmounted mid-run must not pull focus into whichever
      // one is showing now.
      if (mountedRef.current) restoreFocus();
      current.onFeedbackSent();
    } catch (error) {
      // A comment that never reached the signer holds nothing to resume.
      if (draft && !draft.frozen && !draft.running) {
        draftsRef.current.delete(block.id);
        forgetSubmission(draft);
      }
      const message = describeError(error);
      setPhase({ kind: "error", message });
      setAnnouncement(`Feedback was not sent. ${message}`);
    } finally {
      submittingRef.current = false;
      setDraftVersion((version) => version + 1);
    }
  }, [activeBlockId, draftFor, restoreFocus, texts]);

  const scope = options.getScope();
  const lookupDeps = options.deps ?? liveFeedbackDeps;
  // Every unfinished signed comment on this reviewed artifact (all of its
  // revisions), from the store and, behind it, the durable outbox. Re-read
  // after every change this hook makes to drafts held outside React.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `draftVersion` is the re-read signal for drafts held outside React.
  const unfinished = React.useMemo(
    () =>
      scope.relayUrl && scope.signerPubkey
        ? listUnfinished(
            {
              relayUrl: scope.relayUrl,
              signerPubkey: scope.signerPubkey,
              channelId: options.revision.channelId,
              artifactId: options.revision.artifactId,
            },
            lookupDeps,
          )
        : [],
    [
      draftVersion,
      scope.relayUrl,
      scope.signerPubkey,
      options.revision.channelId,
      options.revision.artifactId,
      lookupDeps,
    ],
  );

  // This workbench adopts every stored comment on its revision, so a draft
  // stays visible, and fails closed, if the live scope later changes under it.
  // A comment whose publish ended without the relay's answer is also asked
  // about once per live handle: if the relay holds it, it is accepted and only
  // its wake is owed, which also settles whether it may ever be discarded.
  const reconciledRef = React.useRef(new WeakSet<ReviewFeedbackSubmission>());
  React.useEffect(() => {
    for (const draft of unfinished) {
      if (
        draft.context.revision.eventId === optionsRef.current.revision.eventId
      ) {
        draftsRef.current.set(draft.context.block.id, draft);
      }
      const undecided =
        draft.publishState === "ambiguous" || draft.publishState === "rejected";
      if (!undecided || draft.running || reconciledRef.current.has(draft)) {
        continue;
      }
      reconciledRef.current.add(draft);
      void draft.reconcile().then(() => {
        if (mountedRef.current) setDraftVersion((version) => version + 1);
      });
    }
  }, [unfinished]);

  const activeDraft =
    activeBlockId === null ? undefined : draftFor(activeBlockId);
  // This workbench's own drafts win over the stored ones, so a draft stays
  // visible (and fails closed) if the live scope changes under it.
  const heldByBlock = new Map<string, ReviewFeedbackSubmission>();
  for (const draft of unfinished) {
    if (draft.context.revision.eventId === options.revision.eventId) {
      heldByBlock.set(draft.context.block.id, draft);
    }
  }
  for (const [blockId, draft] of draftsRef.current) {
    if (!draft.completed && (draft.frozen || draft.running)) {
      heldByBlock.set(blockId, draft);
    }
  }
  const pendingDeliveries: PendingDelivery[] = [];
  for (const block of options.blocks) {
    const draft = heldByBlock.get(block.id);
    if (draft?.frozen) {
      pendingDeliveries.push(pendingDeliveryOf(block.id, block.title, draft));
    }
  }
  // The durable outbox is authoritative: a comment signed against another
  // revision of this artifact is still owed its finish. It is listed under its
  // own revision and block, never as this revision's.
  const otherRevisionDeliveries: OtherRevisionDelivery[] = [];
  for (const draft of unfinished) {
    const { revision, block } = draft.context;
    if (revision.eventId === options.revision.eventId) continue;
    otherRevisionDeliveries.push({
      ...pendingDeliveryOf(block.id, block.title, draft),
      revisionEventId: revision.eventId,
      revisionNumber: revision.revision,
    });
  }
  otherRevisionDeliveries.sort(
    (a, b) =>
      b.revisionNumber - a.revisionNumber ||
      a.blockTitle.localeCompare(b.blockTitle),
  );
  // Whatever the relay lists for these, their agent wake has not been
  // confirmed, so the listing must not present them as delivered.
  const unfinishedFeedbackEventIds = new Set<string>();
  for (const draft of [...unfinished, ...heldByBlock.values()]) {
    const eventId = draft.feedbackEventId;
    if (eventId && !draft.completed) unfinishedFeedbackEventIds.add(eventId);
  }

  return {
    activeBlockId,
    // A held draft shows the exact text it signed, even after a remount that
    // lost the typed text.
    text:
      activeBlockId === null
        ? ""
        : (activeDraft?.request ?? texts[activeBlockId] ?? ""),
    phase,
    locked: activeDraft !== undefined,
    /** What still pauses the form's send; null once its draft is signed. */
    blockedReason: activeDraft?.frozen ? null : options.blockedReason,
    /** Comments signed but not finished: reachable for retry whatever the head. */
    pendingDeliveries,
    /** Signed feedback whose agent wake is not confirmed, on this artifact. */
    unfinishedFeedbackEventIds,
    /** What is known about the relay holding the open comment, if signed. */
    publishState: activeDraft?.publishState ?? null,
    /** The relay provably holds nothing for the open comment. */
    canDiscard: activeDraft?.canDiscard ?? false,
    feedbackPublished: activeDraft?.feedbackPublished ?? false,
    /**
     * Unfinished comments signed against another revision of this artifact,
     * from the durable outbox: discoverable here, finished on their own
     * revision.
     */
    otherRevisionDeliveries,
    announcement,
    open,
    cancel,
    setText,
    submit,
    discard,
  };
}
