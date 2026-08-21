import { clampScore, deriveLane } from "./apply.mjs";
import { EMPTY_LESSONS, renderCorrections } from "./lessons.mjs";
import { callLlmJson, llmEnabled } from "./llm.mjs";
import { fibreSignals, scoreAdjustments } from "./signals.mjs";
import { limitSummary } from "./summary.mjs";

const KIND_WEIGHT = {
  blocker: 25,
  decision: 18,
  ask: 12,
  commitment: 15,
  question: 8,
  idea: 4,
  fyi: 0,
};

/**
 * Base priority without a model: kind, how directly it is aimed at the
 * viewer, and what they have kept or dismissed before. Weighted so a plain
 * mention lands in `other` and only a real blocker or a chased ask clears the
 * important threshold — the model, when available, judges this far better.
 */
export function heuristicScore(fibre, signals, lessons = EMPTY_LESSONS) {
  let score = 30;

  if (signals.mentionCount > 0) score += 20;
  if (signals.isDm) score += 15;
  score += KIND_WEIGHT[fibre.kind] ?? 0;

  const authors = new Set(
    (fibre.people ?? []).map((person) => person.pubkey).filter(Boolean),
  );
  for (const author of authors) {
    score += (lessons.authors.get(author) ?? 0) * 8;
  }
  score += (lessons.channels.get(fibre.channelId) ?? 0) * 4;

  return clampScore(score);
}

function promptFibre(fibre, signals) {
  return {
    fibreId: fibre.id,
    kind: fibre.kind,
    title: fibre.title,
    summary: fibre.summary,
    channel: fibre.channelName,
    isDm: Boolean(fibre.isDm),
    people: (fibre.people ?? []).map((person) => person.label),
    messages: (fibre.artifacts ?? []).map((artifact) => ({
      author: artifact.authorLabel,
      content: (artifact.content ?? "").slice(0, 400),
    })),
    facts: {
      mentionsOfYou: signals.mentionCount,
      unansweredPings: signals.unansweredPings,
      youHaveReplied: signals.hasReplyFromViewer,
      messagesInLast30Min: signals.velocity.messages,
      peopleTalking: signals.velocity.participants,
      youWorkedOnThisBefore: signals.pastInvolvement.any,
      incidentChannel: signals.incidentChannel,
      incidentOngoing: signals.incidentOngoing,
      followUpOnly: signals.followUpOnly,
    },
  };
}

export function buildPrioritizePrompt({ fibres, signalsById, lessons }) {
  const fibreJson = fibres.map((fibre) =>
    promptFibre(fibre, signalsById.get(fibre.id)),
  );

  return `You are stage 2 of a two-stage triage system for a team chat. Stage 1 already decided what belongs to a fibre. Your only job is to score how much each fibre deserves the viewer's attention, from 0 to 100.

Fibres, each with the messages it holds and computed facts about its conversation:
${JSON.stringify(fibreJson)}
${renderCorrections(lessons)}
Reply with JSON only, one entry per fibre:
{"scores":[{"fibreId":"...","score":84,"why":"One or two sentences on why it ranks here."}]}

How to judge importance:
- Something that needs an action from the viewer outranks something that only informs them.
- Blast radius decides the top of the scale. Blocking many users or an important client is critical (85-100). Blocking a colleague from doing their job is important (70-85).
- An FYI depends on context: whether the viewer works on that project, and whether it is a change that gets expensive to undo later, such as an architectural decision. A consequential FYI on the viewer's own area can be important; a passing update elsewhere is not.
- Social logistics with no deadline — lunch orders, scheduling chatter — sit near the bottom (0-25) even when the viewer is tagged.
- A deadline only raises the score when the work behind it matters. Judge the work, not the word.
- Being tagged means the fibre was worth considering. It does not by itself make it important. Weigh what is actually being asked.
- In an incident channel, an ongoing incident is critical. A follow-up with no time pressure is worth knowing about but is not urgent — score it in the middle rather than at either extreme.
- A pure follow-up ("bump", "any update") adds no new information and should not raise the score on its own.
- unansweredPings above 1 means people are waiting on the viewer specifically. Score it high.
- A thread the viewer has worked on before is worth knowing about even when nothing is being asked of them.
- A busy thread is not automatically important; a separate deterministic measure already handles "engaging". Score importance only.

Return a score for every fibreId listed. why is one or two sentences addressed to the viewer.`;
}

export function parsePriorityScores(payload) {
  const rows = Array.isArray(payload?.scores) ? payload.scores : [];
  const byId = new Map();
  for (const row of rows) {
    if (!row || typeof row.fibreId !== "string") continue;
    if (!Number.isFinite(Number(row.score))) continue;
    byId.set(row.fibreId, {
      score: clampScore(row.score),
      why: typeof row.why === "string" ? limitSummary(row.why) : "",
    });
  }
  return byId;
}

function resolveWhy(base, adjustments, fibre) {
  if (base) return base;
  const strongest = adjustments.entries[0]?.label;
  if (strongest) return `${strongest}.`;
  return fibre.channelName ? `Ongoing ${fibre.kind} in #${fibre.channelName}.` : "";
}

/**
 * Stage 2. Scores every fibre stage 1 touched, applies the deterministic
 * adjustments on top, and derives the lane. Existing fibres are rescored, not
 * left on a stale priority — that is how repeated pings climb.
 */
export async function prioritizeFibres({
  pubkey,
  fibres,
  lessons = EMPTY_LESSONS,
  now = Math.floor(Date.now() / 1000),
}) {
  if (fibres.length === 0) return new Map();

  const signalsById = new Map(
    fibres.map((fibre) => [fibre.id, fibreSignals(pubkey, fibre, now)]),
  );

  let judged = new Map();
  if (llmEnabled()) {
    try {
      const payload = await callLlmJson(
        buildPrioritizePrompt({ fibres, signalsById, lessons }),
      );
      judged = parsePriorityScores(payload);
    } catch (error) {
      console.warn(`[triage] stage 2 fell back to the heuristic: ${error.message}`);
    }
  }

  const results = new Map();
  for (const fibre of fibres) {
    const signals = signalsById.get(fibre.id);
    const adjustments = scoreAdjustments(signals);
    const judgement = judged.get(fibre.id);
    const base = judgement?.score ?? heuristicScore(fibre, signals, lessons);
    const score = clampScore(base + adjustments.delta);
    const engagement = signals.engagement;

    results.set(fibre.id, {
      score,
      engagement,
      lane: deriveLane(score, engagement),
      why: resolveWhy(judgement?.why, adjustments, fibre),
      signals: adjustments.entries,
    });
  }

  return results;
}
