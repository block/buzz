const ATTENTION_THRESHOLD = 0.5;

const URGENCY_PATTERN =
  /\b(asap|urgent|urgently|blocker|blocked|blocking|deadline|by (?:eod|tomorrow|monday)|ptal|please review|can you|could you|need(?:s)? your|waiting on you|sign ?off|approve)\b/i;

const LOW_VALUE_PATTERN =
  /^(?:\+1|ty|thx|thanks|thank you|nice|cool|lol|haha|ok|okay|k|got it|sounds good|congrats|welcome|gm|good morning|morning|hi|hey|hello|yep|yes|no|done|same|this|ditto|👍|🎉|✅)[\s!.?]*$/i;

/**
 * Aggregate prior user corrections into per-dimension weights.
 *
 * `promoted` means the user rescued something the agent called noise, so that
 * dimension should score higher next time; `dismissed` means the opposite.
 */
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
      row.userAction === "promoted" || row.userAction === "adopted"
        ? 1
        : row.userAction === "dismissed"
          ? -1
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
        preview: row.preview.slice(0, 160),
        userAction: row.userAction,
        suggestedVerdict: row.suggestedVerdict,
      });
    }
  }

  return lessons;
}

/**
 * A correction on this exact message or thread overrides the heuristic outright
 * rather than nudging it. Anything less lets a strongly-scored base verdict
 * ignore an explicit instruction, which is the opposite of learning.
 */
function lessonOverride(candidate, lessons) {
  const event = lessons.events.get(candidate.eventId) ?? 0;
  if (event !== 0) {
    return { weight: event, scope: "this message" };
  }

  const thread = candidate.threadRootId
    ? (lessons.threads.get(candidate.threadRootId) ?? 0)
    : 0;
  if (thread !== 0) {
    return { weight: thread, scope: "this thread" };
  }

  return null;
}

/** Author and channel history are weaker, so they only bias the score. */
function lessonBias(candidate, lessons) {
  const author = lessons.authors.get(candidate.authorPubkey) ?? 0;
  const channel = lessons.channels.get(candidate.channelId) ?? 0;
  const raw = author * 0.25 + channel * 0.1;
  return Math.max(-0.6, Math.min(0.6, raw));
}

function scoreCandidate(candidate, lessons) {
  const content = (candidate.content ?? "").trim();
  const signals = [];
  let score = 0;

  if (candidate.isDm) {
    score += 0.45;
    signals.push("sent to you directly");
  }
  if (candidate.isMention) {
    score += 0.4;
    signals.push("mentions you");
  }
  if (URGENCY_PATTERN.test(content)) {
    score += 0.2;
    signals.push("asks for action");
  }
  if (content.includes("?")) {
    score += 0.15;
    signals.push("asks a question");
  }
  if (candidate.source === "channel" && !candidate.isMention) {
    score -= 0.25;
    signals.push("channel chatter you were not addressed in");
  }
  if (LOW_VALUE_PATTERN.test(content) || content.length < 12) {
    score -= 0.3;
    signals.push("short acknowledgement");
  }

  const bias = lessonBias(candidate, lessons);
  if (bias !== 0) {
    score += bias;
    signals.push(
      bias > 0
        ? "similar items you kept before"
        : "similar items you dismissed before",
    );
  }

  const override = lessonOverride(candidate, lessons);
  if (override) {
    const verdict = override.weight > 0 ? "attention" : "noise";
    return {
      verdict,
      confidence: 1,
      signals: [
        verdict === "attention"
          ? `you told me ${override.scope} matters`
          : `you dismissed ${override.scope} before`,
        ...signals,
      ],
      score,
      learned: true,
    };
  }

  const verdict = score >= ATTENTION_THRESHOLD ? "attention" : "noise";
  const confidence = Math.min(
    1,
    Math.abs(score - ATTENTION_THRESHOLD) * 1.6 + 0.2,
  );

  return { verdict, confidence, signals, score, learned: false };
}

function composeReason(candidate, { verdict, signals, learned }) {
  const where = candidate.channelName ? ` in #${candidate.channelName}` : "";

  // A learned verdict leads with the correction so the shift is visible.
  const lead = learned
    ? signals[0]
    : signals.slice(0, 2).join(" and ");

  if (!lead) {
    return verdict === "attention"
      ? `Unread message${where} that looks like it needs a response.`
      : `Routine unread message${where}.`;
  }

  const subject = lead.charAt(0).toUpperCase() + lead.slice(1);
  if (learned) {
    return `${subject}${where}.`;
  }

  return verdict === "attention"
    ? `${subject}${where}.`
    : `${subject}${where} — safe to skip.`;
}

function heuristicSuggestions(candidates, lessons) {
  return candidates.map((candidate) => {
    const scored = scoreCandidate(candidate, lessons);
    return {
      eventId: candidate.eventId,
      channelId: candidate.channelId ?? null,
      threadRootId: candidate.threadRootId ?? null,
      verdict: scored.verdict,
      reason: composeReason(candidate, scored),
      confidence: Number(scored.confidence.toFixed(2)),
      learned: scored.learned,
      source: "heuristic",
    };
  });
}

function buildPrompt(candidates, lessons) {
  const corrections = lessons.examples.length
    ? `\nThe user previously corrected you on these; respect the pattern:\n${lessons.examples
        .map(
          (example) =>
            `- "${example.preview}" -> you said ${example.suggestedVerdict}, user ${example.userAction}`,
        )
        .join("\n")}\n`
    : "";

  const items = candidates
    .map((candidate) =>
      JSON.stringify({
        eventId: candidate.eventId,
        channel: candidate.channelName,
        author: candidate.authorLabel,
        isDm: candidate.isDm,
        isMention: candidate.isMention,
        content: (candidate.content ?? "").slice(0, 500),
      }),
    )
    .join("\n");

  return `You triage a chat inbox. For each message decide "attention" (the user should act or reply) or "noise" (safe to skip). Give a reason under 15 words explaining the decision and the thread's context.
${corrections}
Messages:
${items}

Reply with JSON only: {"suggestions":[{"eventId":"...","verdict":"attention|noise","reason":"...","confidence":0.0}]}`;
}

async function llmSuggestions(candidates, lessons) {
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
      messages: [{ role: "user", content: buildPrompt(candidates, lessons) }],
    }),
  });

  if (!response.ok) {
    throw new Error(`OpenAI request failed: ${response.status}`);
  }

  const payload = await response.json();
  const parsed = JSON.parse(payload.choices?.[0]?.message?.content ?? "{}");
  const byEventId = new Map(
    (parsed.suggestions ?? []).map((suggestion) => [
      suggestion.eventId,
      suggestion,
    ]),
  );

  // Fall back per-item so a partial LLM response still yields a full result set.
  const fallback = heuristicSuggestions(candidates, lessons);
  return fallback.map((base) => {
    // An explicit user correction outranks the model.
    if (base.learned) return base;

    const llm = byEventId.get(base.eventId);
    if (!llm?.verdict) return base;
    return {
      ...base,
      verdict: llm.verdict === "attention" ? "attention" : "noise",
      reason: llm.reason ?? base.reason,
      confidence: Number(llm.confidence ?? base.confidence),
      source: "llm",
    };
  });
}

export async function classify(candidates, feedback) {
  const lessons = buildLessons(feedback);

  if (process.env.TRIAGE_LLM === "1") {
    try {
      return await llmSuggestions(candidates, lessons);
    } catch (error) {
      console.warn(`[triage] LLM classification failed: ${error.message}`);
    }
  }

  return heuristicSuggestions(candidates, lessons);
}
