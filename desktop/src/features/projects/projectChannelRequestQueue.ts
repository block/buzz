import type { ProjectChannelRequest } from "@/features/projects/projectChannelRequest";

export const MAX_PENDING_PROJECT_CHANNEL_REQUESTS = 100;
const MAX_SEEN_PROJECT_CHANNEL_REQUEST_IDS =
  MAX_PENDING_PROJECT_CHANNEL_REQUESTS * 2 + 1;

export type AcceptedProjectChannelRequest = {
  agentPubkey: string;
  request: ProjectChannelRequest;
};

export type ProjectChannelRequestQueue = {
  activeRequestId: string | null;
  pending: AcceptedProjectChannelRequest[];
  seenRequestIds: Set<string>;
};

export type EnqueueProjectChannelRequestResult =
  | { status: "show"; candidate: AcceptedProjectChannelRequest }
  | { status: "queued" | "duplicate" | "overflow" };

export function createProjectChannelRequestQueue(): ProjectChannelRequestQueue {
  return {
    activeRequestId: null,
    pending: [],
    seenRequestIds: new Set(),
  };
}

export function enqueueProjectChannelRequest(
  queue: ProjectChannelRequestQueue,
  candidate: AcceptedProjectChannelRequest,
): EnqueueProjectChannelRequestResult {
  const requestId = candidate.request.requestId;
  if (queue.seenRequestIds.has(requestId)) return { status: "duplicate" };

  if (
    queue.activeRequestId !== null &&
    queue.pending.length >= MAX_PENDING_PROJECT_CHANNEL_REQUESTS
  ) {
    // Keep the requests already visible to the owner and drop the newest one.
    // Do not mark it seen: a later retry may be accepted after space opens.
    return { status: "overflow" };
  }

  queue.seenRequestIds.add(requestId);
  pruneSeenRequestIds(queue);

  if (queue.activeRequestId === null) {
    queue.activeRequestId = requestId;
    return { status: "show", candidate };
  }

  queue.pending.push(candidate);
  return { status: "queued" };
}

export function advanceProjectChannelRequestQueue(
  queue: ProjectChannelRequestQueue,
): AcceptedProjectChannelRequest | null {
  const next = queue.pending.shift() ?? null;
  queue.activeRequestId = next?.request.requestId ?? null;
  pruneSeenRequestIds(queue);
  return next;
}

function pruneSeenRequestIds(queue: ProjectChannelRequestQueue) {
  if (queue.seenRequestIds.size <= MAX_SEEN_PROJECT_CHANNEL_REQUEST_IDS) return;

  const pendingIds = new Set(
    queue.pending.map((candidate) => candidate.request.requestId),
  );
  for (const requestId of queue.seenRequestIds) {
    if (requestId !== queue.activeRequestId && !pendingIds.has(requestId)) {
      queue.seenRequestIds.delete(requestId);
      if (queue.seenRequestIds.size <= MAX_SEEN_PROJECT_CHANNEL_REQUEST_IDS) {
        return;
      }
    }
  }
}
