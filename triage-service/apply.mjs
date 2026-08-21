import { randomUUID } from "node:crypto";

export const FIBRE_KINDS = [
  "blocker",
  "decision",
  "ask",
  "commitment",
  "idea",
  "question",
  "fyi",
];

const KIND_SET = new Set(FIBRE_KINDS);

export const FIBRE_LANES = ["important", "hot", "other"];

/** Above this priority a fibre is important regardless of how busy it is. */
export const IMPORTANT_SCORE_THRESHOLD = 70;

/** Engagement at or above this is a discussion worth knowing about. */
export const HOT_ENGAGEMENT_THRESHOLD = 60;

export function isFibreKind(value) {
  return KIND_SET.has(value);
}

export function clampScore(value) {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return 0;
  return Math.max(0, Math.min(100, Math.round(numeric)));
}

/** Lanes are tried in order: important wins over hot, hot wins over other. */
export function deriveLane(score, engagement) {
  if (clampScore(score) > IMPORTANT_SCORE_THRESHOLD) return "important";
  if (clampScore(engagement) >= HOT_ENGAGEMENT_THRESHOLD) return "hot";
  return "other";
}

export function messageToArtifact(message) {
  return {
    eventId: message.eventId,
    channelId: message.channelId ?? null,
    channelName: message.channelName ?? null,
    threadRootId: message.threadRootId ?? null,
    authorPubkey: message.authorPubkey ?? null,
    authorLabel: message.authorLabel ?? null,
    content: (message.content ?? "").slice(0, 2000),
    createdAt: message.createdAt ?? null,
    isDm: Boolean(message.isDm),
  };
}

export function peopleFromArtifacts(artifacts) {
  const seen = new Map();
  for (const artifact of artifacts) {
    const pubkey = artifact.authorPubkey;
    if (!pubkey || seen.has(pubkey)) continue;
    seen.set(pubkey, {
      pubkey,
      label: artifact.authorLabel || pubkey.slice(0, 8),
    });
  }
  return [...seen.values()];
}

function uniqueIds(ids) {
  const seen = new Set();
  const result = [];
  for (const id of ids ?? []) {
    if (typeof id !== "string" || !id || seen.has(id)) continue;
    seen.add(id);
    result.push(id);
  }
  return result;
}

function attachArtifacts(fibre, incoming, now) {
  const fibreChannel = fibre.channelId ?? fibre.artifacts[0]?.channelId ?? null;
  const existing = new Set(fibre.artifacts.map((artifact) => artifact.eventId));
  let added = 0;
  for (const artifact of incoming) {
    if (existing.has(artifact.eventId)) continue;
    if (
      fibreChannel &&
      artifact.channelId &&
      artifact.channelId !== fibreChannel
    ) {
      continue;
    }
    fibre.artifacts.push(artifact);
    existing.add(artifact.eventId);
    added += 1;
  }
  if (added > 0) {
    fibre.people = peopleFromArtifacts(fibre.artifacts);
    const primary = fibre.artifacts[0];
    fibre.channelId = primary?.channelId ?? fibre.channelId ?? null;
    fibre.channelName = primary?.channelName ?? fibre.channelName ?? null;
    fibre.isDm = primary?.isDm ?? fibre.isDm ?? false;
  }
  fibre.updatedAt = now;
  return added;
}

function patchFibreFields(fibre, action, now) {
  if (action.kind && isFibreKind(action.kind)) fibre.kind = action.kind;
  if (typeof action.title === "string" && action.title.trim()) {
    fibre.title = action.title.trim();
  }
  if (typeof action.summary === "string" && action.summary.trim()) {
    fibre.summary = action.summary.trim();
  }
  if (typeof action.why === "string" && action.why.trim()) {
    fibre.why = action.why.trim();
  }
  if (typeof action.whyShort === "string" && action.whyShort.trim()) {
    fibre.whyShort = action.whyShort.trim();
  } else if (fibre.why && !fibre.whyShort) {
    fibre.whyShort = fibre.why.slice(0, 120);
  }
  if (action.score !== undefined) fibre.score = clampScore(action.score);
  if (action.engagement !== undefined) {
    fibre.engagement = clampScore(action.engagement);
  }
  if (Array.isArray(action.signals)) fibre.signals = action.signals;
  fibre.lane = deriveLane(fibre.score, fibre.engagement);
  fibre.updatedAt = now;
}

function artifactsForEventIds(eventIds, messagesById) {
  return uniqueIds(eventIds)
    .map((eventId) => messagesById.get(eventId))
    .filter(Boolean)
    .map(messageToArtifact);
}

/**
 * Applies classifier actions to an in-memory fibre list. Re-attaching an
 * event that a fibre already owns is a no-op.
 */
export function applyFibreActions({ fibres, messages, actions, now }) {
  const clock = now ?? Math.floor(Date.now() / 1000);
  const byId = new Map(fibres.map((fibre) => [fibre.id, structuredClone(fibre)]));
  const messagesById = new Map(
    messages.map((message) => [message.eventId, message]),
  );
  const changes = [];
  const ingestedEventIds = uniqueIds(messages.map((message) => message.eventId));

  for (const action of actions ?? []) {
    if (!action || typeof action !== "object") continue;

    if (action.type === "skip") {
      continue;
    }

    if (action.type === "create") {
      const artifacts = artifactsForEventIds(action.eventIds, messagesById);
      if (artifacts.length === 0) continue;
      const channelId = artifacts[0].channelId;
      const sameChannel = artifacts.filter(
        (artifact) =>
          !channelId || !artifact.channelId || artifact.channelId === channelId,
      );
      const kind = isFibreKind(action.kind) ? action.kind : "fyi";
      const title =
        (typeof action.title === "string" && action.title.trim()) ||
        sameChannel[0].content.trim().split("\n")[0] ||
        "Untitled fibre";
      const score = clampScore(action.score ?? 50);
      const engagement = clampScore(action.engagement ?? 0);
      const fibre = {
        id: randomUUID(),
        kind,
        status: "open",
        score,
        engagement,
        lane: deriveLane(score, engagement),
        title,
        summary:
          (typeof action.summary === "string" && action.summary.trim()) ||
          sameChannel[0].content.slice(0, 280),
        why: typeof action.why === "string" ? action.why.trim() : "",
        whyShort:
          (typeof action.whyShort === "string" && action.whyShort.trim()) ||
          (typeof action.why === "string" ? action.why.trim().slice(0, 120) : ""),
        signals: Array.isArray(action.signals) ? action.signals : [],
        channelId: sameChannel[0].channelId,
        channelName: sameChannel[0].channelName,
        isDm: sameChannel[0].isDm,
        people: peopleFromArtifacts(sameChannel),
        artifacts: sameChannel,
        createdAt: clock,
        updatedAt: clock,
      };
      byId.set(fibre.id, fibre);
      changes.push({ type: "create", fibreId: fibre.id });
      continue;
    }

    if (action.type === "update") {
      const fibre = byId.get(action.fibreId);
      if (!fibre || fibre.status !== "open") continue;
      const artifacts = artifactsForEventIds(action.eventIds, messagesById);
      attachArtifacts(fibre, artifacts, clock);
      patchFibreFields(fibre, action, clock);
      changes.push({ type: "update", fibreId: fibre.id });
      continue;
    }

    if (action.type === "merge") {
      const intoId = action.into ?? action.fibreIds?.[0];
      const target = byId.get(intoId);
      if (!target || target.status !== "open") continue;
      const sourceIds = uniqueIds(action.fibreIds).filter(
        (id) => id !== intoId && byId.has(id),
      );
      for (const sourceId of sourceIds) {
        const source = byId.get(sourceId);
        if (!source) continue;
        attachArtifacts(target, source.artifacts, clock);
        source.status = "dismissed";
        source.updatedAt = clock;
      }
      const extra = artifactsForEventIds(action.eventIds, messagesById);
      attachArtifacts(target, extra, clock);
      patchFibreFields(target, action, clock);
      changes.push({ type: "merge", fibreId: target.id, merged: sourceIds });
    }
  }

  return {
    fibres: [...byId.values()],
    changes,
    ingestedEventIds,
  };
}
