import {
  FIBRE_KINDS,
  isFibreKind,
  clampScore,
} from "./apply.mjs";

const URGENCY_PATTERN =
  /\b(asap|urgent|urgently|blocker|blocked|blocking|deadline|by (?:eod|tomorrow|monday)|ptal|please review|can you|could you|need(?:s)? your|waiting on you|sign ?off|approve|root cause)\b/i;

const COMMITMENT_PATTERN =
  /\b(i(?:'ll| will)|let me|i can get|i'll get|before standup|by tomorrow)\b/i;

const LOW_VALUE_PATTERN =
  /^(?:\+1|ty|thx|thanks|thank you|nice|cool|lol|haha|ok|okay|k|got it|sounds good|congrats|welcome|gm|good morning|morning|hi|hey|hello|yep|yes|no|done|same|this|ditto|👍|🎉|✅)[\s!.?]*$/i;

export { FIBRE_KINDS };

export function summarizeFibre(fibre) {
  return {
    id: fibre.id,
    kind: fibre.kind,
    title: fibre.title,
    summary: fibre.summary,
    score: fibre.score,
    people: (fibre.people ?? []).map((person) => person.label),
    eventIds: (fibre.artifacts ?? []).map((artifact) => artifact.eventId),
    threadRootId: fibre.artifacts?.[0]?.threadRootId ?? null,
    channelName: fibre.channelName,
  };
}

export function buildLessons(feedback) {
  const lessons = {
    events: new Map(),
    authors: new Map(),
    channels: new Map(),
    threads: new Map(),
    examples: [],
  };

  for (const row of feedback) {
    const delta =
      row.userAction === "dismissed"
        ? -1
        : row.userAction === "done" || row.userAction === "delegated"
          ? 1
          : 0;
    if (delta === 0) continue;

    for (const [key, map] of [
      [row.eventId, lessons.events],
      [row.authorPubkey, lessons.authors],
      [row.channelId, lessons.channels],
      [row.threadRootId, lessons.threads],
    ]) {
      if (!key) continue;
      map.set(key, (map.get(key) ?? 0) + delta);
    }

    if (lessons.examples.length < 12 && row.preview) {
      lessons.examples.push({
        preview: String(row.preview).slice(0, 160),
        userAction: row.userAction,
      });
    }
  }

  return lessons;
}

function matchingOpenFibre(message, openFibres) {
  if (message.threadRootId) {
    const byThread = openFibres.find((fibre) =>
      (fibre.artifacts ?? []).some(
        (artifact) =>
          artifact.threadRootId === message.threadRootId ||
          artifact.eventId === message.threadRootId,
      ),
    );
    if (byThread) return byThread;
  }

  return openFibres.find((fibre) =>
    (fibre.artifacts ?? []).some(
      (artifact) => artifact.eventId === message.eventId,
    ),
  );
}

function alreadyAttached(message, openFibres) {
  return openFibres.some((fibre) =>
    (fibre.artifacts ?? []).some(
      (artifact) => artifact.eventId === message.eventId,
    ),
  );
}

function pickKind(message, content) {
  if (/\b(root cause|incident|blocker|blocked|rollback)\b/i.test(content)) {
    return "blocker";
  }
  if (COMMITMENT_PATTERN.test(content) && message.isSelf) {
    return "commitment";
  }
  if (URGENCY_PATTERN.test(content) || (message.isMention && content.length > 20)) {
    return content.includes("?") ? "question" : "ask";
  }
  if (content.includes("?")) return "question";
  if (message.isSelf && content.length > 40) return "idea";
  return null;
}

function scoreFor(kind, message, lessons) {
  let score = 40;
  const signals = [];

  if (message.isMention) {
    score += 30;
    signals.push({ weight: "+30", label: "Direct @mention" });
  }
  if (message.isDm) {
    score += 20;
    signals.push({ weight: "+20", label: "Direct message" });
  }
  if (kind === "blocker") {
    score += 25;
    signals.push({ weight: "+25", label: "Incident or blocker language" });
  }
  if (kind === "commitment") {
    score += 15;
    signals.push({ weight: "+15", label: "Commitment made by you" });
  }
  if (kind === "ask") {
    score += 12;
    signals.push({ weight: "+12", label: "Actionable instruction" });
  }
  if (contentHasQuestion(message)) {
    score += 8;
    signals.push({ weight: "+8", label: "Asks a question" });
  }

  const authorBias = (lessons.authors.get(message.authorPubkey) ?? 0) * 8;
  const channelBias = (lessons.channels.get(message.channelId) ?? 0) * 4;
  const bias = authorBias + channelBias;
  if (bias !== 0) {
    score += bias;
    signals.push({
      weight: bias > 0 ? `+${bias}` : `${bias}`,
      label:
        bias > 0
          ? "Similar items you kept before"
          : "Similar items you dismissed before",
    });
  }

  return { score: clampScore(score), signals };
}

function contentHasQuestion(message) {
  return (message.content ?? "").includes("?");
}

function headline(content) {
  const line = content.trim().split("\n")[0]?.trim() ?? "";
  if (line.length <= 90) return line;
  return `${line.slice(0, 87).trimEnd()}…`;
}

/**
 * One fibre per qualifying message. Same-thread messages update an open fibre
 * instead of creating a second one. Never merges.
 */
export function heuristicActions(messages, openFibres, lessons) {
  const actions = [];

  for (const message of messages) {
    const content = (message.content ?? "").trim();
    if (!content || LOW_VALUE_PATTERN.test(content) || content.length < 8) {
      actions.push({ type: "skip", eventId: message.eventId });
      continue;
    }

    if (alreadyAttached(message, openFibres)) {
      actions.push({ type: "skip", eventId: message.eventId });
      continue;
    }

    const existing = matchingOpenFibre(message, openFibres);
    if (existing) {
      actions.push({
        type: "update",
        fibreId: existing.id,
        eventIds: [message.eventId],
      });
      continue;
    }

    if ((lessons.events.get(message.eventId) ?? 0) < 0) {
      actions.push({ type: "skip", eventId: message.eventId });
      continue;
    }

    const kind = pickKind(message, content);
    if (!kind) {
      actions.push({ type: "skip", eventId: message.eventId });
      continue;
    }

    const { score, signals } = scoreFor(kind, message, lessons);
    const where = message.channelName ? ` in #${message.channelName}` : "";
    const why =
      signals[0]?.label
        ? `${signals[0].label}${where}.`
        : `Looks like a ${kind}${where}.`;

    actions.push({
      type: "create",
      kind,
      title: headline(content) || `New ${kind}`,
      summary: content.slice(0, 280),
      why,
      whyShort: signals[0]?.label ?? why,
      score,
      signals,
      eventIds: [message.eventId],
    });
  }

  return actions;
}

export function parseLlmActions(payload, openFibres) {
  const openIds = new Set(openFibres.map((fibre) => fibre.id));
  const raw = Array.isArray(payload?.actions) ? payload.actions : [];
  const actions = [];

  for (const row of raw) {
    if (!row || typeof row !== "object") continue;
    if (row.type === "skip") {
      actions.push({ type: "skip", eventId: row.eventId });
      continue;
    }
    if (row.type === "create") {
      actions.push({
        type: "create",
        kind: isFibreKind(row.kind) ? row.kind : "fyi",
        title: row.title,
        summary: row.summary,
        why: row.why,
        whyShort: row.whyShort,
        score: clampScore(row.score),
        signals: Array.isArray(row.signals) ? row.signals : [],
        eventIds: Array.isArray(row.eventIds) ? row.eventIds : [],
      });
      continue;
    }
    if (row.type === "update" && openIds.has(row.fibreId)) {
      actions.push({
        type: "update",
        fibreId: row.fibreId,
        kind: isFibreKind(row.kind) ? row.kind : undefined,
        title: row.title,
        summary: row.summary,
        why: row.why,
        whyShort: row.whyShort,
        score: row.score,
        signals: row.signals,
        eventIds: Array.isArray(row.eventIds) ? row.eventIds : [],
      });
      continue;
    }
    if (row.type === "merge") {
      const fibreIds = Array.isArray(row.fibreIds)
        ? row.fibreIds.filter((id) => openIds.has(id))
        : [];
      const into = openIds.has(row.into) ? row.into : fibreIds[0];
      if (!into || fibreIds.length < 2) continue;
      actions.push({
        type: "merge",
        fibreIds,
        into,
        title: row.title,
        summary: row.summary,
        why: row.why,
        score: row.score,
        eventIds: Array.isArray(row.eventIds) ? row.eventIds : [],
      });
    }
  }

  return actions;
}

export function buildPrompt(messages, openFibres, lessons) {
  const corrections = lessons.examples.length
    ? `\nThe user previously corrected you on these; respect the pattern:\n${lessons.examples
        .map((example) => `- "${example.preview}" -> user ${example.userAction}`)
        .join("\n")}\n`
    : "";

  const fibreJson = openFibres.map(summarizeFibre);
  const messageJson = messages.map((message) => ({
    eventId: message.eventId,
    channel: message.channelName,
    author: message.authorLabel,
    isDm: message.isDm,
    isMention: message.isMention,
    isSelf: message.isSelf,
    threadRootId: message.threadRootId,
    content: (message.content ?? "").slice(0, 500),
  }));

  return `You extract fibres from a team chat. A fibre is an idea, ask, decision, commitment, question, blocker, or FYI — not a raw message. One fibre groups one or more messages (N >= 1). A message may belong to more than one fibre.

Kinds: ${FIBRE_KINDS.join(", ")}.

Open (incomplete) fibres — consolidate into these when the new message belongs to the same work:
${JSON.stringify(fibreJson)}
${corrections}
New messages:
${JSON.stringify(messageJson)}

Reply with JSON only:
{"actions":[
  {"type":"create","kind":"ask","title":"...","summary":"...","why":"...","whyShort":"...","score":84,"signals":[{"weight":"+12","label":"..."}],"eventIds":["..."]},
  {"type":"update","fibreId":"...","title":"...","summary":"...","why":"...","score":90,"eventIds":["..."]},
  {"type":"merge","fibreIds":["a","b"],"into":"a","title":"...","summary":"...","score":90,"eventIds":["..."]},
  {"type":"skip","eventId":"..."}
]}

Rules:
- skip acknowledgements, empty, and messages that are not a fibre.
- update an open fibre when the message continues it.
- merge when two open fibres are the same thread of work.
- a message may produce more than one action (create AND update).
- score is 0-100. why explains ranking in one or two sentences.`;
}

async function llmActions(messages, openFibres, lessons) {
  const apiKey = process.env.OPENAI_API_KEY;
  if (!apiKey) throw new Error("OPENAI_API_KEY is not set");

  const response = await fetch("https://api.openai.com/v1/chat/completions", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${apiKey}`,
    },
    body: JSON.stringify({
      model: process.env.TRIAGE_MODEL ?? "gpt-4o-mini",
      response_format: { type: "json_object" },
      messages: [
        { role: "user", content: buildPrompt(messages, openFibres, lessons) },
      ],
    }),
  });

  if (!response.ok) {
    throw new Error(`OpenAI request failed: ${response.status}`);
  }

  const payload = await response.json();
  const parsed = JSON.parse(payload.choices?.[0]?.message?.content ?? "{}");
  return parseLlmActions(parsed, openFibres);
}

/**
 * Classify a batch of new messages against the current open-fibre set.
 */
export async function classifyMessages(messages, openFibres, feedback) {
  const lessons = buildLessons(feedback);
  const fallback = heuristicActions(messages, openFibres, lessons);

  if (process.env.TRIAGE_LLM === "1") {
    try {
      const llm = await llmActions(messages, openFibres, lessons);
      if (llm.length > 0) return llm;
    } catch (error) {
      console.warn(`[triage] LLM classification failed: ${error.message}`);
    }
  }

  return fallback;
}
