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

const EMPTY_LESSONS = {
  events: new Map(),
  authors: new Map(),
  channels: new Map(),
  threads: new Map(),
  examples: [],
};

export { FIBRE_KINDS };

function fibreChannelId(fibre) {
  return fibre?.channelId ?? fibre?.artifacts?.[0]?.channelId ?? null;
}

function fibreThreadIds(fibre) {
  const ids = new Set();
  for (const artifact of fibre?.artifacts ?? []) {
    if (artifact.threadRootId) ids.add(artifact.threadRootId);
    if (artifact.eventId) ids.add(artifact.eventId);
  }
  return ids;
}

function sameChannelId(left, right) {
  return Boolean(left) && left === right;
}

function mustCover(message) {
  return Boolean(message?.isMention || message?.isDm);
}

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
    channelId: fibreChannelId(fibre),
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
  const sameChannel = openFibres.filter((fibre) =>
    sameChannelId(fibreChannelId(fibre), message.channelId),
  );

  if (message.threadRootId) {
    const byThread = sameChannel.find((fibre) =>
      (fibre.artifacts ?? []).some(
        (artifact) =>
          artifact.threadRootId === message.threadRootId ||
          artifact.eventId === message.threadRootId,
      ),
    );
    if (byThread) return byThread;
  }

  if (message.isDm) {
    const byDm = sameChannel.find((fibre) => fibre.isDm);
    if (byDm) return byDm;
  }

  return (
    sameChannel.find((fibre) =>
      (fibre.artifacts ?? []).some(
        (artifact) => artifact.eventId === message.eventId,
      ),
    ) ?? null
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
  if (message.isMention) return content.includes("?") ? "question" : "ask";
  if (message.isDm) return "fyi";
  return null;
}

function shouldAttachToFibre(message, fibre) {
  if (!fibre) return false;
  if (!sameChannelId(fibreChannelId(fibre), message.channelId)) return false;
  const sameThread = Boolean(
    message.threadRootId &&
      (fibre.artifacts ?? []).some(
        (artifact) =>
          artifact.threadRootId === message.threadRootId ||
          artifact.eventId === message.threadRootId,
      ),
  );
  const sameDm = Boolean(
    message.isDm && fibre.isDm && sameChannelId(fibreChannelId(fibre), message.channelId),
  );
  if (!sameThread && !sameDm) return false;
  return mustCover(message);
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

const SUMMARY_MAX_CHARS = 600;
const SUMMARY_MAX_SENTENCES = 3;

const KIND_LEAD = {
  ask: "asked",
  question: "asked",
  blocker: "flagged a blocker",
  decision: "raised a decision",
  commitment: "committed",
  idea: "shared an idea",
  fyi: "shared",
};

/**
 * Keep a summary as long as it needs to be, but never more than a few
 * sentences. Collapses whitespace so LLM output stays readable in the pane.
 */
export function limitSummary(text) {
  const trimmed = String(text ?? "")
    .replace(/\s+/g, " ")
    .trim();
  if (!trimmed) return "";
  const sentences =
    trimmed.match(/[^.!?]+[.!?]+(?:\s+|$)|[^.!?]+$/g) ?? [trimmed];
  let out = "";
  let count = 0;
  for (const raw of sentences) {
    const sentence = raw.trim();
    if (!sentence) continue;
    const next = out ? `${out} ${sentence}` : sentence;
    if (
      count > 0 &&
      (count >= SUMMARY_MAX_SENTENCES || next.length > SUMMARY_MAX_CHARS)
    ) {
      break;
    }
    out =
      next.length > SUMMARY_MAX_CHARS
        ? `${next.slice(0, SUMMARY_MAX_CHARS - 1).trimEnd()}…`
        : next;
    count += 1;
    if (out.endsWith("…")) break;
  }
  return out;
}

export function narrativeSummary(kind, message, content) {
  const who = (message.authorLabel || "Someone").trim();
  const where = message.isDm
    ? " in a DM"
    : message.channelName
      ? ` in #${message.channelName}`
      : "";
  const excerpt = limitSummary(content);
  const lead = KIND_LEAD[kind] ?? "wrote";
  if (!excerpt) return limitSummary(`${who} ${lead}${where}.`);
  return limitSummary(`${who} ${lead}${where}: ${excerpt}`);
}

function createAction(message, lessons) {
  const content = (message.content ?? "").trim();
  const kind = pickKind(message, content) ?? "fyi";
  const { score, signals } = scoreFor(kind, message, lessons);
  const where = message.channelName ? ` in #${message.channelName}` : "";
  const why =
    signals[0]?.label
      ? `${signals[0].label}${where}.`
      : `Looks like a ${kind}${where}.`;

  return {
    type: "create",
    kind,
    title: headline(content) || `New ${kind}`,
    summary: narrativeSummary(kind, message, content),
    why,
    whyShort: signals[0]?.label ?? why,
    score,
    signals,
    eventIds: [message.eventId],
  };
}

function coveredEventIds(actions) {
  const ids = new Set();
  for (const action of actions) {
    if (!action || action.type === "skip") continue;
    for (const eventId of action.eventIds ?? []) ids.add(eventId);
  }
  return ids;
}

function groupEventIdsByChannel(eventIds, messagesById) {
  const groups = new Map();
  for (const eventId of eventIds ?? []) {
    const message = messagesById.get(eventId);
    if (!message) continue;
    const key = message.channelId ?? "";
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(eventId);
  }
  return [...groups.values()];
}

function mergeSharesThread(left, right) {
  const rightIds = fibreThreadIds(right);
  if (rightIds.size === 0) return false;
  for (const id of fibreThreadIds(left)) {
    if (rightIds.has(id)) return true;
  }
  return false;
}

/**
 * Hard clustering rules the LLM cannot bypass: mentions/DMs must become a
 * fibre, and fibres stay on one channel.
 */
export function constrainActions(actions, messages, openFibres, lessons = EMPTY_LESSONS) {
  const messagesById = new Map(
    messages.map((message) => [message.eventId, message]),
  );
  const openById = new Map(openFibres.map((fibre) => [fibre.id, fibre]));
  const constrained = [];

  for (const action of actions ?? []) {
    if (!action || typeof action !== "object") continue;

    if (action.type === "skip") {
      const message = messagesById.get(action.eventId);
      if (
        message &&
        mustCover(message) &&
        !alreadyAttached(message, openFibres)
      ) {
        continue;
      }
      constrained.push(action);
      continue;
    }

    if (action.type === "create") {
      const groups = groupEventIdsByChannel(action.eventIds, messagesById);
      for (const eventIds of groups) {
        constrained.push({ ...action, eventIds });
      }
      continue;
    }

    if (action.type === "update") {
      const fibre = openById.get(action.fibreId);
      if (!fibre) continue;
      const channelId = fibreChannelId(fibre);
      const eventIds = (action.eventIds ?? []).filter((eventId) => {
        const message = messagesById.get(eventId);
        return message && sameChannelId(message.channelId, channelId);
      });
      if (eventIds.length === 0) continue;
      constrained.push({ ...action, eventIds });
      continue;
    }

    if (action.type === "merge") {
      const fibreIds = Array.isArray(action.fibreIds) ? action.fibreIds : [];
      const fibres = fibreIds
        .map((id) => openById.get(id))
        .filter(Boolean);
      if (fibres.length < 2) continue;
      const channelId = fibreChannelId(fibres[0]);
      if (!fibres.every((fibre) => sameChannelId(fibreChannelId(fibre), channelId))) {
        continue;
      }
      const [first, ...rest] = fibres;
      if (!rest.every((fibre) => mergeSharesThread(first, fibre))) continue;
      constrained.push(action);
      continue;
    }

    constrained.push(action);
  }

  const covered = coveredEventIds(constrained);
  for (const message of messages) {
    if (!mustCover(message)) continue;
    if (alreadyAttached(message, openFibres)) continue;
    if (covered.has(message.eventId)) continue;
    constrained.push(createAction(message, lessons));
    covered.add(message.eventId);
  }

  return constrained;
}

/**
 * Mentions of you and DMs always become (or join) a fibre. Same-thread
 * chatter is skipped. A fibre-worthy reply in a thread that already has a
 * fibre becomes its own fibre.
 */
export function heuristicActions(messages, openFibres, lessons) {
  const actions = [];

  for (const message of messages) {
    const content = (message.content ?? "").trim();
    if (!content) {
      actions.push({ type: "skip", eventId: message.eventId });
      continue;
    }

    if (alreadyAttached(message, openFibres)) {
      actions.push({ type: "skip", eventId: message.eventId });
      continue;
    }

    const cover = mustCover(message);
    if (
      !cover &&
      (LOW_VALUE_PATTERN.test(content) || content.length < 8)
    ) {
      actions.push({ type: "skip", eventId: message.eventId });
      continue;
    }

    const existing = matchingOpenFibre(message, openFibres);
    if (shouldAttachToFibre(message, existing)) {
      actions.push({
        type: "update",
        fibreId: existing.id,
        eventIds: [message.eventId],
      });
      continue;
    }

    if (!cover && (lessons.events.get(message.eventId) ?? 0) < 0) {
      actions.push({ type: "skip", eventId: message.eventId });
      continue;
    }

    const kind = pickKind(message, content);
    if (!kind) {
      actions.push({ type: "skip", eventId: message.eventId });
      continue;
    }

    actions.push(createAction(message, lessons));
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
        summary: limitSummary(row.summary),
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
        summary: limitSummary(row.summary),
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
        summary: limitSummary(row.summary),
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
    channelId: message.channelId,
    channel: message.channelName,
    author: message.authorLabel,
    isDm: message.isDm,
    isMention: message.isMention,
    isSelf: message.isSelf,
    threadRootId: message.threadRootId,
    content: (message.content ?? "").slice(0, 800),
  }));

  return `You extract fibres from a team chat. A fibre is an idea, ask, decision, commitment, question, blocker, or FYI — not a raw message. It may be a single message, a mention, a DM, or a relevant thread (N >= 1). It is not "everyone who said hello this week."

Kinds: ${FIBRE_KINDS.join(", ")}.

Open fibres (incomplete). Attach a new message only when it is the same work in the same channel, usually the same threadRootId:
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
- Default action is skip. Most workspace messages are not fibres (greetings, +1s, offhand replies).
- Every isMention: true and every isDm: true MUST create or update. 1:1 fibres are expected and correct.
- update only when the message is in the same channelId as the fibre and is actually the same work (usually the same threadRootId).
- merge almost never, and never across channels. Prefer two fibres over one bloated fibre.
- A fibre-worthy reply in a thread that already has a fibre should usually create a new fibre, not be swallowed.
- Never attach a message from a different channelId to an open fibre.
- score is 0-100. why explains ranking in one or two sentences.
- title is a short headline (a few words), not a restatement of the summary.
- summary names the people and states what happened, e.g. "Vlad asked jacob to run the two triage scripts in #hack-project-mesh before the next build." Write one to three sentences — as long as a reader needs to understand the fibre, never a fourth. Do not write a nameless one-liner like "Request for a status update."`;
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
  let actions = heuristicActions(messages, openFibres, lessons);

  if (process.env.TRIAGE_LLM === "1") {
    try {
      const llm = await llmActions(messages, openFibres, lessons);
      if (llm.length > 0) actions = llm;
    } catch (error) {
      console.warn(`[triage] LLM classification failed: ${error.message}`);
    }
  }

  return constrainActions(actions, messages, openFibres, lessons);
}
