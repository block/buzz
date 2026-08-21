import { applyFibreActions, clampScore, deriveLane } from "./apply.mjs";
import { decisionsToActions, extractDecisions } from "./extract.mjs";
import { EMPTY_LESSONS } from "./lessons.mjs";
import { prioritizeFibres } from "./prioritize.mjs";

export { buildLessons } from "./lessons.mjs";
export { describeMode } from "./llm.mjs";

function touchedFibreIds(changes) {
  const ids = new Set();
  for (const change of changes) {
    if (change.fibreId) ids.add(change.fibreId);
  }
  return ids;
}

function ignoredEntries(decisions, messages) {
  const messagesById = new Map(
    messages.map((message) => [message.eventId, message]),
  );
  return decisions
    .filter((decision) => decision.type === "ignore")
    .map((decision) => {
      const message = messagesById.get(decision.eventId);
      return {
        eventId: decision.eventId,
        channelId: message?.channelId ?? null,
        channelName: message?.channelName ?? null,
        authorLabel: message?.authorLabel ?? null,
        preview: (message?.content ?? "").slice(0, 160),
        wasAddressed: Boolean(message?.isMention || message?.isDm),
        reason: decision.reason || "No reason given",
        createdAt: message?.createdAt ?? null,
      };
    });
}

/**
 * Runs both triage stages over one batch.
 *
 * Stage 1 decides fibre membership for every message; stage 2 rescores every
 * fibre that changed as a result. Persistence is the caller's job — this only
 * reads the transcript for context.
 */
export async function triageBatch({
  pubkey,
  messages,
  fibres,
  lessons = EMPTY_LESSONS,
  now,
}) {
  const openFibres = fibres.filter((fibre) => fibre.status === "open");

  const decisions = await extractDecisions({
    pubkey,
    messages,
    openFibres,
    lessons,
  });

  const applied = applyFibreActions({
    fibres,
    messages,
    actions: decisionsToActions(decisions, messages),
    now,
  });

  const touched = touchedFibreIds(applied.changes);
  const priorities = await prioritizeFibres({
    pubkey,
    fibres: applied.fibres.filter(
      (fibre) => touched.has(fibre.id) && fibre.status === "open",
    ),
    lessons,
    now,
  });

  const nextFibres = applied.fibres.map((fibre) => {
    const priority = priorities.get(fibre.id);
    if (!priority) return fibre;
    return {
      ...fibre,
      score: clampScore(priority.score),
      engagement: clampScore(priority.engagement),
      lane: priority.lane ?? deriveLane(priority.score, priority.engagement),
      why: priority.why || fibre.why,
      whyShort: (priority.why || fibre.whyShort || "").slice(0, 120),
      signals: priority.signals.length > 0 ? priority.signals : fibre.signals,
    };
  });

  return {
    fibres: nextFibres,
    changes: applied.changes,
    ingestedEventIds: applied.ingestedEventIds,
    decisions,
    ignored: ignoredEntries(decisions, messages),
  };
}
