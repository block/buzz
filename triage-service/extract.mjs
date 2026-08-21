import {
  FIBRE_KINDS,
  HOT_ENGAGEMENT_THRESHOLD,
  isFibreKind,
} from "./apply.mjs";
import { EMPTY_LESSONS, renderCorrections } from "./lessons.mjs";
import { callLlmJson, llmEnabled } from "./llm.mjs";
import { isFollowUp, isLowValue, messageContext } from "./signals.mjs";
import { headline, limitSummary, narrativeSummary } from "./summary.mjs";

/** Above this many open fibres, only the batch's own channels get full detail. */
export const FULL_FIBRE_CONTEXT_LIMIT = 150;

/**
 * How long after one message another from the same author still reads as the
 * same thought. People routinely split one point across several lines.
 */
export const BURST_WINDOW_SECONDS = 300;

/**
 * Openers that refer back to what was just said. Topic-neutral evidence of
 * continuation, which is what lets the burst rule group without judging the
 * subject matter — the model handles that, and can veto with `newTopic`.
 */
const CONTINUATION_PATTERN =
  /^\s*(?:so|and|but|also|plus|or|though|although|because|since|which|besides|anyway|basically|actually|i mean|as in|that said|even so|otherwise|then|it|its|it's|this|that|these|those|they|there)\b/i;

export function startsAsContinuation(content) {
  return CONTINUATION_PATTERN.test(String(content ?? "").trim());
}

const URGENCY_PATTERN =
  /\b(asap|urgent|urgently|blocker|blocked|blocking|deadline|by (?:eod|tomorrow|monday)|ptal|please review|can you|could you|need(?:s)? your|waiting on you|sign ?off|approve|root cause)\b/i;

const COMMITMENT_PATTERN =
  /\b(i(?:'ll| will)|let me|i can get|i'll get|before standup|by tomorrow)\b/i;

const DECISION_PATTERN =
  /\b(decision|decided|proposal|rfc|we should (?:switch|move|adopt)|architecture)\b/i;

const BLOCKER_PATTERN =
  /\b(root cause|incident|blocker|blocked|rollback|outage|down)\b/i;

export function fibreChannelId(fibre) {
  return fibre?.channelId ?? fibre?.artifacts?.[0]?.channelId ?? null;
}

function sameChannelId(left, right) {
  return Boolean(left) && left === right;
}

function fibreThreadIds(fibre) {
  const ids = new Set();
  for (const artifact of fibre?.artifacts ?? []) {
    if (artifact.threadRootId) ids.add(artifact.threadRootId);
    if (artifact.eventId) ids.add(artifact.eventId);
  }
  return [...ids];
}

export function summarizeFibre(fibre) {
  return {
    id: fibre.id,
    kind: fibre.kind,
    title: fibre.title,
    summary: fibre.summary,
    channelId: fibreChannelId(fibre),
    channel: fibre.channelName,
    isDm: Boolean(fibre.isDm),
    threadRootIds: fibreThreadIds(fibre),
    people: (fibre.people ?? []).map((person) => person.label),
    artifactCount: (fibre.artifacts ?? []).length,
    lastActivityAt: fibre.updatedAt ?? null,
  };
}

function compactFibre(fibre) {
  return {
    id: fibre.id,
    kind: fibre.kind,
    title: fibre.title,
    channelId: fibreChannelId(fibre),
  };
}

/**
 * Every incomplete fibre goes to the model — consolidation is the point of
 * this stage. Only once the open set is large do out-of-scope channels drop
 * to a one-line form.
 */
export function summarizeOpenFibres(openFibres, channelIds = []) {
  if (openFibres.length <= FULL_FIBRE_CONTEXT_LIMIT) {
    return openFibres.map(summarizeFibre);
  }
  const scope = new Set(channelIds.filter(Boolean));
  return openFibres.map((fibre) =>
    scope.has(fibreChannelId(fibre)) ? summarizeFibre(fibre) : compactFibre(fibre),
  );
}

export function alreadyAttached(message, openFibres) {
  return openFibres.some((fibre) =>
    (fibre.artifacts ?? []).some(
      (artifact) => artifact.eventId === message.eventId,
    ),
  );
}

/** The open fibre a message most plausibly continues, same channel only. */
export function matchingOpenFibre(message, openFibres) {
  const sameChannel = openFibres.filter((fibre) =>
    sameChannelId(fibreChannelId(fibre), message.channelId),
  );

  if (message.threadRootId) {
    const byThread = sameChannel.find((fibre) =>
      fibreThreadIds(fibre).includes(message.threadRootId),
    );
    if (byThread) return byThread;
  }

  if (message.isDm) {
    const byDm = sameChannel.find((fibre) => fibre.isDm);
    if (byDm) return byDm;
  }

  return null;
}

export function pickKind(message, content) {
  if (BLOCKER_PATTERN.test(content)) return "blocker";
  if (DECISION_PATTERN.test(content)) return "decision";
  if (COMMITMENT_PATTERN.test(content)) return "commitment";
  if (URGENCY_PATTERN.test(content) || (message.isMention && content.length > 20)) {
    return content.includes("?") ? "question" : "ask";
  }
  if (content.includes("?")) return "question";
  if (message.isMention) return "ask";
  if (message.isDm) return "fyi";
  return null;
}

function ignoreDecision(eventId, reason) {
  return { eventId, type: "ignore", reason };
}

function createDecision(message, kind) {
  const content = (message.content ?? "").trim();
  const resolved = kind ?? pickKind(message, content) ?? "fyi";
  return {
    eventId: message.eventId,
    type: "create",
    kind: resolved,
    title: headline(content) || `New ${resolved}`,
    summary: narrativeSummary(resolved, message, content),
  };
}

function toCreate(decision, message) {
  const fallback = createDecision(message, isFibreKind(decision.kind) ? decision.kind : null);
  return {
    ...fallback,
    title: decision.title?.trim() || fallback.title,
    summary: limitSummary(decision.summary) || fallback.summary,
    // The model's grouping intent has to survive the rewrite, or batch
    // grouping silently reverts to one fibre per message.
    groupKey: decision.groupKey,
    newTopic: decision.newTopic,
  };
}

/**
 * The decision the service makes without a model: mechanical, conservative,
 * and the same code path used for any message the model leaves out.
 */
export function heuristicDecision(
  message,
  openFibres,
  lessons = EMPTY_LESSONS,
  context,
) {
  const eventId = message.eventId;
  const content = (message.content ?? "").trim();

  if (!content) return ignoreDecision(eventId, "Empty message");
  if (message.isSelf) return ignoreDecision(eventId, "You wrote this");
  if (alreadyAttached(message, openFibres)) {
    return ignoreDecision(eventId, "Already part of a fibre");
  }

  const busyThread = (context?.engagement ?? 0) >= HOT_ENGAGEMENT_THRESHOLD;
  const existing = matchingOpenFibre(message, openFibres);
  if (
    existing &&
    (message.isMention || message.isDm || isFollowUp(content) || busyThread)
  ) {
    return { eventId, type: "attach", fibreId: existing.id };
  }

  // A bare acknowledgement carries nothing even when it is aimed at you.
  if (isLowValue(content)) {
    return ignoreDecision(eventId, "Acknowledgement or chatter");
  }

  const addressed = message.isMention || message.isDm;
  if (!addressed) {
    if (content.length < 8) {
      return ignoreDecision(eventId, "Too short to carry anything");
    }
    if ((lessons.events.get(eventId) ?? 0) < 0) {
      return ignoreDecision(eventId, "You dismissed this before");
    }
  }

  const kind = pickKind(message, content);
  if (kind) return createDecision(message, kind);

  // Nothing is being asked, but a thread this busy is a discussion worth
  // knowing about. One fibre for the thread, not one per message.
  if (busyThread) {
    return { ...createDecision(message, "fyi"), groupKey: message.threadRootId };
  }

  return ignoreDecision(eventId, "Carries no ask, question, or news");
}

/**
 * Hard rules the model cannot bypass. Guarantees exactly one decision per
 * input message and biases toward keeping anything addressed to the viewer.
 */
/**
 * The fibre a message continues cannot always be found in `openFibres`: when
 * the message it follows arrived in this same batch, that fibre does not exist
 * yet. The trail records what each earlier message in the batch landed on so a
 * continuation can join it.
 *
 * Two shapes count as continuing:
 *  - a follow-up ("bump", "any update") in a thread the batch already touched
 *  - the same author carrying on within {@link BURST_WINDOW_SECONDS}, where the
 *    message opens by referring back ("so …", "and …", "it …")
 *
 * A topic change opens neither way, and the model can always veto by marking
 * the message `newTopic`.
 */
function continuationTarget(decision, message, trail) {
  if (decision.groupKey || decision.newTopic) return null;

  const content = message.content ?? "";
  // A bare acknowledgement continues the words but not the thought.
  if (isLowValue(content)) return null;
  const followUp = isFollowUp(content);
  if (!followUp && !startsAsContinuation(content)) return null;

  const sameChannel = trail.filter(
    (entry) => entry.channelId === message.channelId,
  );

  if (followUp && message.threadRootId) {
    const byThread = sameChannel.find((entry) =>
      entry.threadIds.has(message.threadRootId),
    );
    if (byThread) return byThread;
  }

  const within = (entry) =>
    Number.isFinite(message.createdAt) &&
    Number.isFinite(entry.lastAt) &&
    message.createdAt - entry.lastAt <= BURST_WINDOW_SECONDS;

  // Latest first: a burst continues whatever the author said most recently.
  return (
    [...sameChannel]
      .reverse()
      .find(
        (entry) =>
          entry.authorPubkey &&
          entry.authorPubkey === message.authorPubkey &&
          within(entry),
      ) ?? null
  );
}

function joinTrailEntry(entry, message) {
  if (message.threadRootId) entry.threadIds.add(message.threadRootId);
  entry.threadIds.add(message.eventId);
  entry.lastAt = Math.max(entry.lastAt ?? 0, message.createdAt ?? 0);
}

function recordTrail(decision, message, trail) {
  const key =
    decision.type === "attach"
      ? `fibre:${decision.fibreId}`
      : `group:${decision.groupKey ?? decision.eventId}`;
  const existing = trail.find((entry) => entry.key === key);
  if (existing) {
    joinTrailEntry(existing, message);
    return;
  }
  const entry = {
    key,
    groupKey: decision.type === "attach" ? null : (decision.groupKey ?? decision.eventId),
    fibreId: decision.type === "attach" ? decision.fibreId : null,
    channelId: message.channelId,
    authorPubkey: message.authorPubkey ?? null,
    threadIds: new Set(),
    lastAt: 0,
  };
  joinTrailEntry(entry, message);
  trail.push(entry);
}

export function constrainDecisions({
  decisions,
  messages,
  openFibres,
  lessons = EMPTY_LESSONS,
  contexts,
}) {
  const openById = new Map(openFibres.map((fibre) => [fibre.id, fibre]));
  const proposed = new Map();
  for (const decision of decisions ?? []) {
    if (!decision?.eventId || proposed.has(decision.eventId)) continue;
    proposed.set(decision.eventId, decision);
  }

  const trail = [];
  return messages.map((message) => {
    const context = contexts?.get(message.eventId);
    const raw = proposed.get(message.eventId);
    let decision = raw
      ? enforce(raw, message, openFibres, openById, lessons, context)
      : heuristicDecision(message, openFibres, lessons, context);

    // `ignore` is included: a line that only makes sense as the tail of the
    // message before it reads as nothing on its own, so it gets dropped on its
    // own merits. It can only ever join a fibre here, never start one.
    if (decision.type === "create" || decision.type === "ignore") {
      const target = continuationTarget(decision, message, trail);
      if (target?.fibreId) {
        decision = {
          eventId: message.eventId,
          type: "attach",
          fibreId: target.fibreId,
        };
      } else if (target) {
        const base =
          decision.type === "create"
            ? decision
            : createDecision(message, null);
        decision = { ...base, type: "create", groupKey: target.groupKey };
      }
    }

    if (decision.type === "create" || decision.type === "attach") {
      recordTrail(decision, message, trail);
    }
    return decision;
  });
}

function enforce(decision, message, openFibres, openById, lessons, context) {
  const eventId = message.eventId;
  const content = (message.content ?? "").trim();

  if (!content) return ignoreDecision(eventId, "Empty message");
  if (message.isSelf) return ignoreDecision(eventId, "You wrote this");
  if (alreadyAttached(message, openFibres)) {
    return ignoreDecision(eventId, "Already part of a fibre");
  }

  if (decision.type === "attach") {
    const fibre = openById.get(decision.fibreId);
    if (fibre && sameChannelId(fibreChannelId(fibre), message.channelId)) {
      return { ...decision, eventId };
    }
    return toCreate(decision, message);
  }

  if (decision.type === "merge") {
    const fibres = (decision.fibreIds ?? [])
      .map((id) => openById.get(id))
      .filter(Boolean);
    const channelId = fibres.length > 0 ? fibreChannelId(fibres[0]) : null;
    const coherent =
      fibres.length >= 2 &&
      fibres.every((fibre) => sameChannelId(fibreChannelId(fibre), channelId)) &&
      sameChannelId(channelId, message.channelId);
    if (coherent) {
      const into = openById.has(decision.into) ? decision.into : fibres[0].id;
      return { ...decision, eventId, into, fibreIds: fibres.map((f) => f.id) };
    }
    const survivor = fibres.find((fibre) =>
      sameChannelId(fibreChannelId(fibre), message.channelId),
    );
    if (survivor) return { eventId, type: "attach", fibreId: survivor.id };
    return toCreate(decision, message);
  }

  if (decision.type === "ignore") {
    // Being addressed does not make a message important, but it does make
    // dropping it silently the expensive mistake. Only genuine noise and
    // content-free follow-ups are allowed through.
    if (!message.isMention && !message.isDm) return decision;
    if (isLowValue(content)) return decision;
    const existing = matchingOpenFibre(message, openFibres);
    if (existing) return { eventId, type: "attach", fibreId: existing.id };
    if (isFollowUp(content)) return decision;
    return heuristicDecision(message, openFibres, lessons, context);
  }

  return toCreate(decision, message);
}

/**
 * Folds per-message decisions into the action shape `applyFibreActions`
 * consumes. Creates sharing a `groupKey` collapse into one fibre, which is
 * how a single new discussion becomes one fibre instead of several.
 */
export function decisionsToActions(decisions, messages) {
  const messagesById = new Map(
    messages.map((message) => [message.eventId, message]),
  );
  const actions = [];
  const groups = new Map();

  for (const decision of decisions ?? []) {
    const message = messagesById.get(decision.eventId);
    if (!message || decision.type === "ignore") continue;

    if (decision.type === "attach") {
      actions.push({
        type: "update",
        fibreId: decision.fibreId,
        kind: decision.kind,
        title: decision.title,
        summary: decision.summary,
        eventIds: [decision.eventId],
      });
      continue;
    }

    if (decision.type === "merge") {
      actions.push({
        type: "merge",
        fibreIds: decision.fibreIds,
        into: decision.into,
        title: decision.title,
        summary: decision.summary,
        eventIds: [decision.eventId],
      });
      continue;
    }

    // Channel is part of the key because a fibre never spans channels.
    const key = `${decision.groupKey ?? decision.eventId}::${message.channelId ?? ""}`;
    const existing = groups.get(key);
    if (existing) {
      existing.eventIds.push(decision.eventId);
      continue;
    }
    const action = {
      type: "create",
      kind: decision.kind,
      title: decision.title,
      summary: decision.summary,
      eventIds: [decision.eventId],
    };
    groups.set(key, action);
    actions.push(action);
  }

  return actions;
}

export function parseExtractDecisions(payload) {
  const rows = Array.isArray(payload?.decisions) ? payload.decisions : [];
  const decisions = [];

  for (const row of rows) {
    if (!row || typeof row !== "object" || typeof row.eventId !== "string") {
      continue;
    }
    const base = { eventId: row.eventId };

    if (row.type === "ignore") {
      decisions.push({
        ...base,
        type: "ignore",
        reason: typeof row.reason === "string" ? row.reason.trim() : "",
      });
      continue;
    }
    if (row.type === "attach" && typeof row.fibreId === "string") {
      decisions.push({
        ...base,
        type: "attach",
        fibreId: row.fibreId,
        kind: isFibreKind(row.kind) ? row.kind : undefined,
        title: typeof row.title === "string" ? row.title.trim() : undefined,
        summary: limitSummary(row.summary) || undefined,
      });
      continue;
    }
    if (row.type === "merge" && Array.isArray(row.fibreIds)) {
      decisions.push({
        ...base,
        type: "merge",
        fibreIds: row.fibreIds.filter((id) => typeof id === "string"),
        into: typeof row.into === "string" ? row.into : undefined,
        title: typeof row.title === "string" ? row.title.trim() : undefined,
        summary: limitSummary(row.summary) || undefined,
      });
      continue;
    }
    if (row.type === "create") {
      decisions.push({
        ...base,
        type: "create",
        kind: isFibreKind(row.kind) ? row.kind : "fyi",
        title: typeof row.title === "string" ? row.title.trim() : "",
        summary: limitSummary(row.summary),
        groupKey: typeof row.groupKey === "string" ? row.groupKey : undefined,
        newTopic: row.newTopic === true,
      });
    }
  }

  return decisions;
}

function promptMessage(message, context, previous) {
  const adjacent =
    previous && previous.channelId === message.channelId
      ? {
          secondsAfterPrevious: Math.max(
            0,
            (message.createdAt ?? 0) - (previous.createdAt ?? 0),
          ),
          previousFromSameAuthor: previous.authorPubkey === message.authorPubkey,
          previousEventId: previous.eventId,
        }
      : {};

  return {
    eventId: message.eventId,
    channelId: message.channelId,
    channel: message.channelName,
    author: message.authorLabel,
    isDm: Boolean(message.isDm),
    isMention: Boolean(message.isMention),
    threadRootId: message.threadRootId,
    content: (message.content ?? "").slice(0, 800),
    contextScope: context?.scope ?? "none",
    threadEngagement: context?.engagement ?? 0,
    ...adjacent,
    context: (context?.messages ?? []).map((row) => ({
      author: row.isSelf ? "you" : row.authorLabel,
      content: row.content.slice(0, 240),
    })),
  };
}

export function buildExtractPrompt({ messages, contexts, openFibres, lessons }) {
  const channelIds = messages.map((message) => message.channelId);
  const fibreJson = summarizeOpenFibres(openFibres, channelIds);
  const messageJson = messages.map((message, index) =>
    promptMessage(message, contexts?.get(message.eventId), messages[index - 1]),
  );

  return `You are stage 1 of a two-stage triage system for a team chat. Your only job is to decide, for each new message, whether it belongs to a fibre and which one. You do not rank anything — a later stage decides importance. Never ignore a message merely because it looks unimportant; that is not your call here.

A fibre is one unit of work or knowledge: an idea, ask, decision, commitment, question, blocker, or FYI. One fibre holds one or more messages. Kinds: ${FIBRE_KINDS.join(", ")}.

Open fibres (incomplete). Attach when the message continues work one of these already tracks:
${JSON.stringify(fibreJson)}
${renderCorrections(lessons)}
New messages in chronological order, each with the conversation around it. "contextScope" is "thread" when the viewer was tagged inside a thread and the whole thread is given, otherwise the recent channel window. "threadEngagement" is 0-100 for how busy the thread is right now. "secondsAfterPrevious" and "previousFromSameAuthor" describe the message directly above in this list. Context entries authored by the viewer show as "you":
${JSON.stringify(messageJson)}

Reply with JSON only, exactly one decision per message:
{"decisions":[
  {"eventId":"...","type":"ignore","reason":"lunch logistics, nothing to act on"},
  {"eventId":"...","type":"attach","fibreId":"...","title":"...","summary":"..."},
  {"eventId":"...","type":"create","kind":"ask","title":"...","summary":"...","groupKey":"deploy-rollback","newTopic":false},
  {"eventId":"...","type":"merge","fibreIds":["a","b"],"into":"a"}
]}

Rules:
- Emit a decision for every eventId listed above. Omitting one is a failure.
- ignore: greetings, acknowledgements, banter, social logistics, and anything carrying no work and no knowledge. Always give a short reason.
- attach: the message continues an open fibre's work. Same channel, usually the same threadRootId. A follow-up ("bump", "ping", "any update", "still blocked") is never a new fibre — attach it to the fibre it is chasing.
- create: genuinely new work or a new topic. When several messages in this batch are one new discussion, give them the same groupKey so they become one fibre. A follow-up chasing another message in this same batch gets that message's groupKey — the open fibre list above does not include fibres this batch is about to create.
- People split one thought across several lines. When previousFromSameAuthor is true and secondsAfterPrevious is small, the default is that both messages are the same fibre: give this message the previous one's groupKey, or attach it to the same open fibre the previous one joined. Judge the content, not just the timing — if the author has moved to a different subject, set "newTopic": true on that message and give it its own fibre. Differing kinds are not a reason to split: an ask and the consequence the author spells out a line later are one fibre.
- merge: two open fibres turned out to be the same work. Rare, and never across channels.
- A thread with high threadEngagement is a live discussion. Even when nothing is asked of the viewer, it deserves one fibre — give every message in that thread the same groupKey so it stays one fibre rather than several.
- Never attach or merge across channelId boundaries.
- Refine title and summary when a message changes what the fibre is about.
- title is a short headline of a few words. summary names the people and states what happened, e.g. "Vlad asked Jacob to run the two triage scripts in #hack-project-mesh before the next build." One to three sentences, never a nameless one-liner.`;
}

/**
 * Stage 1. Returns exactly one decision per message, LLM-led with the
 * heuristic filling in whatever the model does not answer for.
 */
export async function extractDecisions({
  pubkey,
  messages,
  openFibres,
  lessons = EMPTY_LESSONS,
}) {
  const contexts = new Map(
    messages.map((message) => [message.eventId, messageContext(pubkey, message)]),
  );

  let decisions = [];
  if (llmEnabled() && messages.length > 0) {
    try {
      const payload = await callLlmJson(
        buildExtractPrompt({ messages, contexts, openFibres, lessons }),
      );
      decisions = parseExtractDecisions(payload);
    } catch (error) {
      console.warn(`[triage] stage 1 fell back to the heuristic: ${error.message}`);
    }
  }

  return constrainDecisions({
    decisions,
    messages,
    openFibres,
    lessons,
    contexts,
  });
}
