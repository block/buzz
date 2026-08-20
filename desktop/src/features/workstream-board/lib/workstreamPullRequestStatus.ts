import * as React from "react";

import { eventToProjectPullRequest } from "@/features/projects/projectPullRequests.mjs";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_GIT_PULL_REQUEST,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_OPEN,
} from "@/shared/constants/kinds";
import { buildPullRequestLink, parseEntityLink } from "@/shared/lib/entityLink";

export type WorkstreamPullRequestReference =
  | {
      kind: "buzz";
      rawUrl: string;
      href: string;
      identity: string;
      repoAddress: string;
      eventId: string;
      repository: string;
    }
  | {
      kind: "github";
      rawUrl: string;
      href: string;
      identity: string;
      owner: string;
      repository: string;
      number: number;
    }
  | {
      kind: "unsupported";
      rawUrl: string;
      href: string;
      identity: string;
      reason: "malformed" | "unsupported-provider" | "invalid-field";
    };

export type WorkstreamReviewState =
  | "approved"
  | "changes-requested"
  | "review-requested"
  | "no-reviews";

export type WorkstreamPullRequestStatus = {
  status: "live";
  provider: "buzz" | "github";
  href: string;
  repository: string;
  number: number | null;
  title: string;
  lifecycle: "draft" | "open" | "merged" | "closed";
  reviewState: WorkstreamReviewState;
};

export type WorkstreamPullRequestUnknown = {
  status: "unknown";
  href: string;
  reason: "not-found";
};

export type WorkstreamPullRequestDisplayState =
  | { status: "loading" }
  | { status: "error"; href: string }
  | {
      status: "unsupported";
      href: string;
      reason: "malformed" | "unsupported-provider" | "invalid-field";
    }
  | WorkstreamPullRequestStatus
  | WorkstreamPullRequestUnknown;

const GITHUB_HOSTS = new Set(["github.com", "www.github.com"]);
const GITHUB_URL_RE = /^https:\/\/github\.com\/[^/]+\/[^/]+\/pull\/\d+\/?$/i;
const MAX_GITHUB_POLLS = 10;
const DEFAULT_GITHUB_POLL_INTERVAL_MS = 30_000;
const DEFAULT_GITHUB_POLL_LIFETIME_MS = 5 * 60_000;
const NIP34_STATUS_KINDS = [
  KIND_GIT_STATUS_OPEN,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
];

function normalizeRawUrl(value: unknown): string {
  if (typeof value === "string") return value.trim();
  if (typeof value === "object" && value !== null && !Array.isArray(value)) {
    const url = (value as { url?: unknown }).url;
    if (typeof url === "string") return url.trim();
  }
  return "[invalid reference]";
}

function unsupportedReference(
  rawUrl: string,
  reason: Extract<
    WorkstreamPullRequestReference,
    { kind: "unsupported" }
  >["reason"],
): WorkstreamPullRequestReference {
  return {
    kind: "unsupported",
    rawUrl,
    href: rawUrl,
    identity: `unsupported:${reason}:${rawUrl}`,
    reason,
  };
}

/** Parse a single v1 canvas PR reference without throwing. */
export function parseWorkstreamPullRequestReference(
  value: unknown,
): WorkstreamPullRequestReference {
  const rawUrl = normalizeRawUrl(value);
  if (!rawUrl || rawUrl === "[invalid reference]") {
    return unsupportedReference(rawUrl, "invalid-field");
  }

  if (rawUrl.startsWith("buzz://")) {
    const parsed = parseEntityLink(rawUrl);
    if (!parsed.ok || parsed.value.type !== "pr") {
      return unsupportedReference(rawUrl, "malformed");
    }

    const { id, owner, dtag } = parsed.value;
    return {
      kind: "buzz",
      rawUrl,
      href: buildPullRequestLink({ dtag, id, owner }),
      identity: `buzz:${owner.toLowerCase()}:${dtag}:${id.toLowerCase()}`,
      repoAddress: `30617:${owner.toLowerCase()}:${dtag}`,
      eventId: id.toLowerCase(),
      repository: dtag,
    };
  }

  let parsed: URL;
  try {
    parsed = new URL(rawUrl);
  } catch {
    return unsupportedReference(rawUrl, "malformed");
  }

  if (!GITHUB_HOSTS.has(parsed.hostname.toLowerCase())) {
    return unsupportedReference(rawUrl, "unsupported-provider");
  }
  if (
    parsed.username ||
    parsed.password ||
    parsed.port ||
    parsed.protocol !== "https:"
  ) {
    return unsupportedReference(rawUrl, "malformed");
  }

  const segments = parsed.pathname.split("/").filter(Boolean);
  if (
    segments.length !== 4 ||
    segments[2].toLowerCase() !== "pull" ||
    !/^\d+$/.test(segments[3])
  ) {
    return unsupportedReference(rawUrl, "malformed");
  }

  const number = Number(segments[3]);
  if (
    !Number.isSafeInteger(number) ||
    number < 1 ||
    !GITHUB_URL_RE.test(rawUrl)
  ) {
    return unsupportedReference(rawUrl, "malformed");
  }

  const owner = segments[0].toLowerCase();
  const repository = segments[1].toLowerCase();
  const href = `https://github.com/${owner}/${repository}/pull/${number}`;
  return {
    kind: "github",
    rawUrl,
    href,
    identity: `github:${owner}/${repository}#${number}`,
    owner,
    repository,
    number,
  };
}

export function parseWorkstreamPullRequestReferences(
  values: readonly unknown[],
): WorkstreamPullRequestReference[] {
  return values.map(parseWorkstreamPullRequestReference);
}

type GithubResponse = {
  state?: unknown;
  draft?: unknown;
  merged_at?: unknown;
  number?: unknown;
  title?: unknown;
  requested_reviewers?: unknown;
};

type GithubReview = {
  user?: { login?: unknown } | null;
  state?: unknown;
  submitted_at?: unknown;
  updated_at?: unknown;
};

export type GithubFetch = (
  input: RequestInfo | URL,
  init?: RequestInit,
) => Promise<Response>;

type EventFetcher = (
  filter: Parameters<typeof relayClient.fetchEvents>[0],
) => Promise<RelayEvent[]>;

function isGithubReview(value: unknown): value is GithubReview {
  return typeof value === "object" && value !== null;
}

function reviewState(
  reviews: unknown[],
  requestedReviewers: unknown,
): WorkstreamReviewState {
  const latestByReviewer = new Map<string, GithubReview>();
  for (const review of reviews.filter(isGithubReview)) {
    const login =
      typeof review.user?.login === "string" ? review.user.login : null;
    if (!login) continue;
    const current = latestByReviewer.get(login.toLowerCase());
    const currentAt = String(
      current?.submitted_at ?? current?.updated_at ?? "",
    );
    const nextAt = String(review.submitted_at ?? review.updated_at ?? "");
    if (!current || nextAt >= currentAt) {
      latestByReviewer.set(login.toLowerCase(), review);
    }
  }

  const latestStates = [...latestByReviewer.values()].map((review) =>
    typeof review.state === "string" ? review.state.toUpperCase() : "",
  );
  if (latestStates.includes("CHANGES_REQUESTED")) return "changes-requested";
  if (latestStates.includes("APPROVED")) return "approved";
  if (Array.isArray(requestedReviewers) && requestedReviewers.length > 0) {
    return "review-requested";
  }
  return "no-reviews";
}

function lifecycleFromGithubResponse(
  response: GithubResponse,
): WorkstreamPullRequestStatus["lifecycle"] {
  if (typeof response.merged_at === "string" && response.merged_at) {
    return "merged";
  }
  if (response.draft === true) return "draft";
  return response.state === "closed" ? "closed" : "open";
}

async function fetchJson(
  fetcher: GithubFetch,
  url: string,
  signal?: AbortSignal,
): Promise<unknown> {
  const response = await fetcher(url, {
    headers: { Accept: "application/vnd.github+json" },
    signal,
  });
  if (response.status === 404) return null;
  if (!response.ok) {
    throw new Error(`GitHub metadata request failed (${response.status}).`);
  }
  return response.json();
}

/** Fetch bounded, unauthenticated GitHub PR metadata for one canonical PR. */
export async function fetchGithubPullRequestStatus(
  reference: Extract<WorkstreamPullRequestReference, { kind: "github" }>,
  options: { fetcher?: GithubFetch; signal?: AbortSignal } = {},
): Promise<WorkstreamPullRequestStatus | WorkstreamPullRequestUnknown> {
  const fetcher = options.fetcher ?? fetch;
  const base = `https://api.github.com/repos/${reference.owner}/${reference.repository}/pulls/${reference.number}`;
  const [rawPullRequest, rawReviews] = await Promise.all([
    fetchJson(fetcher, base, options.signal),
    fetchJson(fetcher, `${base}/reviews?per_page=100`, options.signal),
  ]);

  if (rawPullRequest === null) {
    return { status: "unknown", href: reference.href, reason: "not-found" };
  }
  if (
    typeof rawPullRequest !== "object" ||
    rawPullRequest === null ||
    !Array.isArray(rawReviews)
  ) {
    throw new Error("GitHub metadata response was unavailable.");
  }

  const response = rawPullRequest as GithubResponse;
  if (
    typeof response.title !== "string" ||
    response.number !== reference.number
  ) {
    throw new Error("GitHub metadata response was invalid.");
  }

  return {
    status: "live",
    provider: "github",
    href: reference.href,
    repository: `${reference.owner}/${reference.repository}`,
    number: reference.number,
    title: response.title,
    lifecycle: lifecycleFromGithubResponse(response),
    reviewState: reviewState(rawReviews, response.requested_reviewers),
  };
}

/** Consume the existing NIP-34 root/status event streams for one Buzz PR. */
export async function fetchBuzzPullRequestStatus(
  reference: Extract<WorkstreamPullRequestReference, { kind: "buzz" }>,
  fetchEvents: EventFetcher = (filter) => relayClient.fetchEvents(filter),
): Promise<WorkstreamPullRequestStatus | WorkstreamPullRequestUnknown> {
  const roots = await fetchEvents({
    kinds: [KIND_GIT_PULL_REQUEST],
    ids: [reference.eventId],
    "#a": [reference.repoAddress],
    limit: 1,
  });
  const root = roots.find(
    (event) => event.id.toLowerCase() === reference.eventId,
  );
  if (!root) {
    return { status: "unknown", href: reference.href, reason: "not-found" };
  }

  const [updates, comments, statuses] = await Promise.all([
    fetchEvents({ kinds: [1634], "#a": [reference.repoAddress], limit: 500 }),
    fetchEvents({ kinds: [1], "#a": [reference.repoAddress], limit: 500 }),
    fetchEvents({
      kinds: NIP34_STATUS_KINDS,
      "#a": [reference.repoAddress],
      limit: 500,
    }),
  ]);
  const pullRequest = eventToProjectPullRequest(
    root,
    updates,
    comments,
    statuses,
  );
  return {
    status: "live",
    provider: "buzz",
    href: reference.href,
    repository: reference.repository,
    number: null,
    title: pullRequest.title,
    lifecycle:
      pullRequest.status.toLowerCase() as WorkstreamPullRequestStatus["lifecycle"],
    reviewState:
      pullRequest.changeRequests.length > 0
        ? "changes-requested"
        : pullRequest.approvals.length > 0
          ? "approved"
          : pullRequest.reviewers.length > 0
            ? "review-requested"
            : "no-reviews",
  };
}

type PollerListener = (state: WorkstreamPullRequestDisplayState) => void;
type PollerReference = Exclude<
  WorkstreamPullRequestReference,
  { kind: "unsupported" }
>;
type PollerEntry = {
  reference: PollerReference;
  listeners: Set<PollerListener>;
  state: WorkstreamPullRequestDisplayState;
  controller: AbortController | null;
  timer: ReturnType<typeof setTimeout> | null;
  startedAt: number;
  pollCount: number;
};

export type WorkstreamPollingRegistryOptions = {
  fetcher?: GithubFetch;
  fetchEvents?: EventFetcher;
  intervalMs?: number;
  maxPolls?: number;
  maxLifetimeMs?: number;
  now?: () => number;
  schedule?: (
    callback: () => void,
    delayMs: number,
  ) => ReturnType<typeof setTimeout>;
  cancel?: (timer: ReturnType<typeof setTimeout>) => void;
};

/** One bounded, route-scoped metadata stream per canonical PR identity. */
export function createWorkstreamPollingRegistry(
  options: WorkstreamPollingRegistryOptions = {},
) {
  const githubFetcher = options.fetcher ?? fetch;
  const fetchEvents =
    options.fetchEvents ?? ((filter) => relayClient.fetchEvents(filter));
  const intervalMs = options.intervalMs ?? DEFAULT_GITHUB_POLL_INTERVAL_MS;
  const maxPolls = options.maxPolls ?? MAX_GITHUB_POLLS;
  const maxLifetimeMs =
    options.maxLifetimeMs ?? DEFAULT_GITHUB_POLL_LIFETIME_MS;
  const now = options.now ?? (() => Date.now());
  const schedule =
    options.schedule ?? ((callback, delay) => setTimeout(callback, delay));
  const cancel = options.cancel ?? ((timer) => clearTimeout(timer));
  const entries = new Map<string, PollerEntry>();

  const stop = (entry: PollerEntry) => {
    if (entry.timer !== null) cancel(entry.timer);
    entry.timer = null;
    entry.controller?.abort();
    entry.controller = null;
    entries.delete(entry.reference.identity);
  };

  const notify = (entry: PollerEntry) => {
    for (const listener of entry.listeners) listener(entry.state);
  };

  const fetchStatus = (entry: PollerEntry, signal: AbortSignal) =>
    entry.reference.kind === "github"
      ? fetchGithubPullRequestStatus(entry.reference, {
          fetcher: githubFetcher,
          signal,
        })
      : fetchBuzzPullRequestStatus(entry.reference, fetchEvents);

  const poll = async (entry: PollerEntry) => {
    if (!entries.has(entry.reference.identity) || entry.listeners.size === 0)
      return;
    const controller = new AbortController();
    entry.controller = controller;
    try {
      entry.state = await fetchStatus(entry, controller.signal);
      notify(entry);
    } catch {
      if (controller.signal.aborted && entry.listeners.size === 0) return;
      entry.state = { status: "error", href: entry.reference.href };
      notify(entry);
    } finally {
      entry.controller = null;
      entry.pollCount += 1;
      const bounded =
        entry.pollCount >= maxPolls || now() - entry.startedAt >= maxLifetimeMs;
      if (entry.listeners.size > 0 && !bounded) {
        entry.timer = schedule(() => {
          entry.timer = null;
          void poll(entry);
        }, intervalMs);
      }
    }
  };

  const subscribe = (reference: PollerReference, listener: PollerListener) => {
    let entry = entries.get(reference.identity);
    if (!entry) {
      entry = {
        reference,
        listeners: new Set(),
        state: { status: "loading" },
        controller: null,
        timer: null,
        startedAt: now(),
        pollCount: 0,
      };
      entries.set(reference.identity, entry);
    }
    entry.listeners.add(listener);
    if (
      entry.pollCount === 0 &&
      entry.controller === null &&
      entry.timer === null
    ) {
      void poll(entry);
    }
    listener(entry.state);
    return () => {
      if (!entry?.listeners.delete(listener)) return;
      if (entry.listeners.size === 0) stop(entry);
    };
  };

  return {
    subscribe,
    entryCount: () => entries.size,
    reset: () => {
      for (const entry of [...entries.values()]) stop(entry);
    },
  };
}

const defaultWorkstreamPollingRegistry = createWorkstreamPollingRegistry();

type WorkstreamPollingRegistry = Pick<
  ReturnType<typeof createWorkstreamPollingRegistry>,
  "subscribe"
>;

export function useWorkstreamPullRequestStatuses(
  references: readonly WorkstreamPullRequestReference[],
  pollingRegistry: WorkstreamPollingRegistry = defaultWorkstreamPollingRegistry,
): ReadonlyMap<string, WorkstreamPullRequestDisplayState> {
  const [states, setStates] = React.useState<
    ReadonlyMap<string, WorkstreamPullRequestDisplayState>
  >(() => new Map());
  const referencesByIdentity = React.useMemo(() => {
    const next = new Map<string, WorkstreamPullRequestReference>();
    for (const reference of references) next.set(reference.identity, reference);
    return next;
  }, [references]);

  React.useEffect(() => {
    const initial = new Map<string, WorkstreamPullRequestDisplayState>();
    for (const reference of referencesByIdentity.values()) {
      initial.set(
        reference.identity,
        reference.kind === "unsupported"
          ? {
              status: "unsupported",
              href: reference.href,
              reason: reference.reason,
            }
          : { status: "loading" },
      );
    }
    setStates(initial);

    const unsubscribers: Array<() => void> = [];
    for (const reference of referencesByIdentity.values()) {
      if (reference.kind === "unsupported") continue;
      const unsubscribe = pollingRegistry.subscribe(reference, (state) => {
        setStates((current) => {
          const updated = new Map(current);
          updated.set(reference.identity, state);
          return updated;
        });
      });
      unsubscribers.push(unsubscribe);
    }
    return () => {
      for (const unsubscribe of unsubscribers) unsubscribe();
    };
  }, [pollingRegistry, referencesByIdentity]);

  return states;
}

export const workstreamPollingLimits = {
  intervalMs: DEFAULT_GITHUB_POLL_INTERVAL_MS,
  maxLifetimeMs: DEFAULT_GITHUB_POLL_LIFETIME_MS,
  maxPolls: MAX_GITHUB_POLLS,
};
