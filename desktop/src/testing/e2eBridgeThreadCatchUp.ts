/**
 * E2E double for the thread catch-up commands.
 *
 * Generation is deterministic over the mock channel's own thread events, so
 * every cited id is a real message the spec can navigate to. Settings and
 * shared recaps persist in localStorage so a reload sees what was shared.
 * Nothing here touches read state or posts chat messages, matching native.
 * Like native, generation records a SHA-256 per source and share refuses with
 * `source_changed` when a cited source's current content no longer matches.
 *
 * Per-test controls live on `window.__BUZZ_E2E_THREAD_CATCH_UP__`.
 */
import type { RelayEvent } from "../shared/api/types.ts";

const STORAGE_KEY = "buzz-e2e-thread-catch-up.v1";
const CONFLICT_ERROR =
  "conflict: this recap changed since it was loaded; your correction was not saved";
const HEX64_RE = /^[a-f0-9]{64}$/;

type Scope = {
  relayUrl: string;
  signerPubkey: string;
  channelId: string;
  rootEventId: string;
};
type Settings = { baseUrl: string; model: string; confirmedLocal: boolean };
type Point = {
  id: string;
  category: "decision" | "question";
  text: string;
  sourceIds: string[];
};
type Recap = {
  version: 1;
  channelId: string;
  rootEventId: string;
  since: number;
  until: number;
  points: Point[];
  sourceIds: string[];
  coverage: {
    messageCount: number;
    inputMessageCount: number;
    truncated: boolean;
    warnings: string[];
  };
  sourceRevisions?: Record<string, string>;
};
type Correction = {
  eventId: string;
  pointId: string;
  text: string;
  authorPubkey: string;
  createdAt: number;
};
type Shared = {
  artifactId: string;
  eventId: string;
  authorPubkey: string;
  createdAt: number;
  recap: Recap;
  corrections: Correction[];
  historyComplete: boolean;
};
type Stored = { settings: Settings | null; shared: Shared[] };

export type ThreadCatchUpMockControls = {
  /** Reject the next generation with this message. */
  failNextGeneration?: string;
  /** Hold each generation this long so a spec can cancel it. */
  generationDelayMs?: number;
  /** Land a revision from `conflictAuthor` just before the next correction. */
  conflictNextCorrection?: boolean;
  conflictAuthor?: string;
  /** Feed only the newest N retrieved messages to the "model", like native's input budget. */
  inputLimit?: number;
  /** Report shared recaps whose earliest revisions are no longer retrievable. */
  historyComplete?: boolean;
  /** Refuse the next share as if a cited source changed since generation. */
  shareSourceChanged?: boolean;
  /** Extra unreadable entries and truncation reported by the list. */
  unreadableCount?: number;
  complete?: boolean;
  /** Generation request ids that were cancelled, in order. */
  cancelled?: string[];
  generated?: Array<{ since: number; until: number }>;
};

declare global {
  interface Window {
    __BUZZ_E2E_THREAD_CATCH_UP__?: ThreadCatchUpMockControls;
  }
}

function controls(): ThreadCatchUpMockControls {
  window.__BUZZ_E2E_THREAD_CATCH_UP__ ??= {};
  return window.__BUZZ_E2E_THREAD_CATCH_UP__;
}

function load(): Stored {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw) return JSON.parse(raw) as Stored;
  } catch {}
  return { settings: null, shared: [] };
}

function save(stored: Stored) {
  window.localStorage.setItem(STORAGE_KEY, JSON.stringify(stored));
}

function randomHex64() {
  const bytes = crypto.getRandomValues(new Uint8Array(32));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
    "",
  );
}

function nowSeconds() {
  return Math.floor(Date.now() / 1_000);
}

function inputOf<T>(payload: unknown): T {
  const input = (payload as { input?: T } | null)?.input;
  if (!input) throw new Error("missing input");
  return input;
}

function isNumericLoopback(baseUrl: string) {
  try {
    const url = new URL(baseUrl);
    return (
      url.protocol === "http:" &&
      (url.hostname === "[::1]" || /^127\.\d+\.\d+\.\d+$/.test(url.hostname))
    );
  } catch {
    return false;
  }
}

function threadEvents(events: RelayEvent[], rootEventId: string) {
  const members = new Set([rootEventId]);
  const sorted = [...events].sort((a, b) => a.created_at - b.created_at);
  for (const event of sorted) {
    if (
      event.tags.some((tag) => tag[0] === "e" && tag[1] && members.has(tag[1]))
    ) {
      members.add(event.id);
    }
  }
  return sorted.filter((event) => members.has(event.id));
}

async function contentRevision(content: string) {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(content),
  );
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

/** Deterministic recap: "decided" lines are decisions, "?" lines questions. */
async function buildRecap(
  scope: Scope,
  events: RelayEvent[],
  since: number,
  until: number,
  inputLimit?: number,
): Promise<Recap> {
  const retrieved = threadEvents(events, scope.rootEventId).filter(
    (event) =>
      HEX64_RE.test(event.id) &&
      event.created_at >= since &&
      event.created_at <= until,
  );
  const inWindow =
    inputLimit === undefined ? retrieved : retrieved.slice(-inputLimit);
  const truncated = inWindow.length < retrieved.length;
  const points: Point[] = [];
  for (const event of inWindow) {
    if (points.length >= 12) break;
    const text = event.content.trim();
    const category = /\bdecided\b/i.test(text)
      ? "decision"
      : text.endsWith("?")
        ? "question"
        : null;
    if (!category) continue;
    points.push({
      id: `p${points.length + 1}`,
      category,
      text:
        category === "decision"
          ? `Decision: ${text}`
          : `Open question: ${text}`,
      sourceIds: [event.id],
    });
  }
  return {
    version: 1,
    channelId: scope.channelId,
    rootEventId: scope.rootEventId,
    since,
    until,
    points,
    sourceIds: inWindow.map((event) => event.id),
    coverage: {
      messageCount: retrieved.length,
      inputMessageCount: inWindow.length,
      truncated,
      warnings: truncated
        ? [
            `Only the newest ${inWindow.length} retrieved messages fit in the model input.`,
          ]
        : [],
    },
    sourceRevisions: Object.fromEntries(
      await Promise.all(
        inWindow
          .filter((event) =>
            points.some((point) => point.sourceIds.includes(event.id)),
          )
          .map(async (event) => [
            event.id,
            await contentRevision(event.content),
          ]),
      ),
    ),
  };
}

async function citedSourceChanged(recap: Recap, events: RelayEvent[]) {
  if (!recap.sourceRevisions) return false;
  const byId = new Map(events.map((event) => [event.id, event]));
  for (const point of recap.points) {
    for (const id of point.sourceIds) {
      const event = byId.get(id);
      if (
        !event ||
        (await contentRevision(event.content)) !== recap.sourceRevisions[id]
      ) {
        return true;
      }
    }
  }
  return false;
}

const pending = new Map<string, (error: Error) => void>();

/**
 * Returns `{ handled: false }` for commands outside this feature so the main
 * bridge can keep its "unsupported command" failure for everything else.
 */
export async function handleThreadCatchUpMockCommand(
  command: string,
  payload: unknown,
  getChannelEvents: (channelId: string) => RelayEvent[],
): Promise<{ handled: false } | { handled: true; value: unknown }> {
  const stored = load();
  switch (command) {
    case "get_thread_catch_up_settings":
      return {
        handled: true,
        value: stored.settings ?? {
          baseUrl: "",
          model: "",
          confirmedLocal: false,
        },
      };
    case "set_thread_catch_up_settings": {
      const input = inputOf<Settings>(payload);
      if (!isNumericLoopback(input.baseUrl) || !input.confirmedLocal) {
        throw new Error("local-only mode requires a numeric loopback endpoint");
      }
      stored.settings = input;
      save(stored);
      return { handled: true, value: input };
    }
    case "generate_thread_catch_up": {
      const input = inputOf<{
        requestId: string;
        scope: Scope;
        since: number;
        until: number;
      }>(payload);
      const state = controls();
      state.generated = [
        ...(state.generated ?? []),
        { since: input.since, until: input.until },
      ];
      if (!stored.settings) throw new Error("catch-up is not set up");
      const delayMs = state.generationDelayMs ?? 0;
      if (delayMs > 0) {
        await new Promise<void>((resolve, reject) => {
          const timer = window.setTimeout(() => {
            pending.delete(input.requestId);
            resolve();
          }, delayMs);
          pending.set(input.requestId, (error) => {
            window.clearTimeout(timer);
            reject(error);
          });
        });
      }
      const failure = state.failNextGeneration;
      if (failure) {
        state.failNextGeneration = undefined;
        throw new Error(failure);
      }
      return {
        handled: true,
        value: await buildRecap(
          input.scope,
          getChannelEvents(input.scope.channelId),
          input.since,
          input.until,
          state.inputLimit,
        ),
      };
    }
    case "cancel_thread_catch_up": {
      const { requestId } = inputOf<{ requestId: string }>(payload);
      const state = controls();
      state.cancelled = [...(state.cancelled ?? []), requestId];
      pending.get(requestId)?.(new Error("catch-up was cancelled"));
      pending.delete(requestId);
      return { handled: true, value: null };
    }
    case "list_thread_catch_up_artifacts": {
      const { scope } = inputOf<{ scope: Scope }>(payload);
      const state = controls();
      const recaps = stored.shared
        .filter(
          (shared) =>
            shared.recap.channelId === scope.channelId &&
            shared.recap.rootEventId === scope.rootEventId,
        )
        .sort((a, b) => updatedAt(b) - updatedAt(a))
        .map((shared) =>
          state.historyComplete === false
            ? { ...shared, historyComplete: false }
            : shared,
        );
      return {
        handled: true,
        value: {
          recaps,
          unreadableCount: state.unreadableCount ?? 0,
          complete: state.complete ?? true,
        },
      };
    }
    case "share_thread_catch_up": {
      const { scope, recap } = inputOf<{ scope: Scope; recap: Recap }>(payload);
      const state = controls();
      if (
        state.shareSourceChanged ||
        (await citedSourceChanged(recap, getChannelEvents(scope.channelId)))
      ) {
        state.shareSourceChanged = false;
        throw new Error(
          "source_changed: a cited message changed after this recap was generated",
        );
      }
      const shared: Shared = {
        artifactId: crypto.randomUUID(),
        eventId: randomHex64(),
        authorPubkey: scope.signerPubkey,
        createdAt: nowSeconds(),
        recap,
        corrections: [],
        historyComplete: true,
      };
      stored.shared.push(shared);
      save(stored);
      return { handled: true, value: shared };
    }
    case "correct_thread_catch_up": {
      const input = inputOf<{
        scope: Scope;
        artifactId: string;
        prevEventId: string;
        pointId: string;
        text: string;
      }>(payload);
      const shared = stored.shared.find(
        (entry) => entry.artifactId === input.artifactId,
      );
      if (!shared) throw new Error("this shared recap is no longer available");
      const state = controls();
      if (state.conflictNextCorrection) {
        state.conflictNextCorrection = false;
        appendCorrection(shared, {
          pointId: input.pointId,
          text: "Another member corrected this first.",
          authorPubkey: state.conflictAuthor ?? "a".repeat(64),
        });
        save(stored);
        throw new Error(CONFLICT_ERROR);
      }
      if (shared.eventId !== input.prevEventId) throw new Error(CONFLICT_ERROR);
      appendCorrection(shared, {
        pointId: input.pointId,
        text: input.text,
        authorPubkey: input.scope.signerPubkey,
      });
      save(stored);
      return { handled: true, value: shared };
    }
    default:
      return { handled: false };
  }
}

function appendCorrection(
  shared: Shared,
  correction: Pick<Correction, "pointId" | "text" | "authorPubkey">,
) {
  const eventId = randomHex64();
  shared.corrections.push({ ...correction, eventId, createdAt: nowSeconds() });
  shared.eventId = eventId;
}

function updatedAt(shared: Shared) {
  return shared.corrections.reduce(
    (latest, correction) => Math.max(latest, correction.createdAt),
    shared.createdAt,
  );
}
