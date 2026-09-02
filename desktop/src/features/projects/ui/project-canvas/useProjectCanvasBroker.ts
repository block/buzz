import * as React from "react";

import { useOpenDmMutation } from "@/features/channels/hooks";
import { normalizeRelayUrl } from "@/features/communities/communityStorage";
import {
  useAssignProjectIssueMutation,
  useUnassignProjectIssueMutation,
} from "@/features/projects/issueAssignments";
import { useUpdateProjectIssueStatusMutation } from "@/features/projects/issueMutations";
import { sendChannelMessage } from "@/shared/api/tauriMessages";
import { getUsersBatch, searchUsers } from "@/shared/api/tauriProfiles";
import { getAvatarSnapshotUrl } from "@/shared/lib/animatedAvatar";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  fetchCanvasAvatarDataUrl,
  selectAvatarsWithinBudget,
  toCanvasAvatarUploads,
} from "./canvasAvatars";
import {
  publishProjectCanvasAvatars,
  type ProjectCanvasPackageRequest,
} from "./projectCanvasCommands";
import type { Repository } from "@/features/projects/hooks";
import type { ProjectIssue } from "@/features/projects/projectIssues.mjs";
import type { ProjectPullRequest } from "@/features/projects/projectPullRequests.mjs";
import {
  ProjectCanvasBroker,
  type ProjectCanvasOpenTarget,
  type ProjectCanvasPersonRow,
  type ProjectCanvasReviewListRow,
  type ProjectCanvasTaskRow,
} from "./projectCanvasBroker";
import type { ProjectCanvasSnapshots } from "./projectCanvasProtocol";

const MAX_BROKER_ROWS = 50;
const MAX_BROKER_TEXT = 256;
const MAX_AVATAR_CACHE_ENTRIES = 32;

function boundedText(value: string, maxLength = MAX_BROKER_TEXT): string {
  return value.slice(0, maxLength);
}

/**
 * Bounded avatar inliner: canvas frames run with `connect-src 'none'`, so
 * people rows carry re-encoded data-URL avatars instead of relay URLs. Each
 * image is downscaled to a canvas-sized square before it is cached, so the
 * cache holds at most 32 entries of at most
 * {@link CANVAS_AVATAR_MAX_DATA_URL_LENGTH} each; failures are cached as
 * misses so a broken avatar cannot retrigger fetch loops.
 */
async function inlineAvatarDataUrl(
  cache: Map<string, string>,
  avatarUrl: string | null,
): Promise<string | null> {
  const snapshotUrl = getAvatarSnapshotUrl(avatarUrl);
  if (!snapshotUrl) return null;
  const cached = cache.get(snapshotUrl);
  if (cached !== undefined) {
    cache.delete(snapshotUrl);
    cache.set(snapshotUrl, cached);
    return cached || null;
  }
  let dataUrl: string | null = null;
  try {
    dataUrl = await fetchCanvasAvatarDataUrl(snapshotUrl);
  } catch {
    dataUrl = null;
  }
  const accepted = dataUrl?.startsWith("data:image/") ? dataUrl : "";
  cache.set(snapshotUrl, accepted);
  while (cache.size > MAX_AVATAR_CACHE_ENTRIES) {
    const oldest = cache.keys().next().value;
    if (oldest === undefined) break;
    cache.delete(oldest);
  }
  return accepted || null;
}

function toTaskRow(issue: ProjectIssue): ProjectCanvasTaskRow {
  return {
    assignees: issue.assignees.slice(0, 8),
    category: issue.category,
    commentCount: issue.comments.length,
    displayId: issue.id.slice(0, 8),
    id: issue.id,
    status: issue.status,
    title: boundedText(issue.title),
    updatedAt: issue.updatedAt,
  };
}

function toReviewListRow(
  pullRequest: ProjectPullRequest,
): ProjectCanvasReviewListRow {
  return {
    author: pullRequest.author,
    branch: pullRequest.branchName ? boundedText(pullRequest.branchName) : null,
    displayId: pullRequest.id.slice(0, 8),
    id: pullRequest.id,
    status: pullRequest.status,
    title: boundedText(pullRequest.title),
    updatedAt: pullRequest.updatedAt,
  };
}

type WorkQueryState<T> = {
  data: T[] | undefined;
  isError: boolean;
  isPending: boolean;
};

export function useProjectCanvasBroker({
  canvasRequest,
  identityPubkey,
  issues,
  onOpenTarget,
  primaryRepository,
  relayUrl,
  reviews,
  snapshots,
}: {
  canvasRequest: ProjectCanvasPackageRequest | null;
  identityPubkey: string | undefined;
  issues: WorkQueryState<ProjectIssue>;
  onOpenTarget: (target: ProjectCanvasOpenTarget) => void;
  primaryRepository: Repository | null;
  relayUrl: string | undefined;
  reviews: WorkQueryState<ProjectPullRequest>;
  snapshots: ProjectCanvasSnapshots;
}): ProjectCanvasBroker {
  const setStatusMutation =
    useUpdateProjectIssueStatusMutation(primaryRepository);
  const assignMutation = useAssignProjectIssueMutation(primaryRepository);
  const unassignMutation = useUnassignProjectIssueMutation(primaryRepository);
  const openDmMutation = useOpenDmMutation();

  const avatarCacheRef = React.useRef(new Map<string, string>());
  const canvasRequestRef = React.useRef(canvasRequest);
  canvasRequestRef.current = canvasRequest;
  const issuesRef = React.useRef<ProjectIssue[]>([]);
  issuesRef.current = issues.data ?? [];
  const identityRef = React.useRef(identityPubkey);
  identityRef.current = identityPubkey;
  const relayUrlRef = React.useRef(relayUrl);
  relayUrlRef.current = relayUrl;
  const onOpenTargetRef = React.useRef(onOpenTarget);
  onOpenTargetRef.current = onOpenTarget;
  const setStatusRef = React.useRef(setStatusMutation.mutateAsync);
  setStatusRef.current = setStatusMutation.mutateAsync;
  const assignRef = React.useRef(assignMutation.mutateAsync);
  assignRef.current = assignMutation.mutateAsync;
  const unassignRef = React.useRef(unassignMutation.mutateAsync);
  unassignRef.current = unassignMutation.mutateAsync;
  const openDmRef = React.useRef(openDmMutation.mutateAsync);
  openDmRef.current = openDmMutation.mutateAsync;

  const brokerRef = React.useRef<ProjectCanvasBroker | null>(null);
  if (brokerRef.current === null) {
    const lookupProfiles = async (
      pubkeys: string[],
    ): Promise<ProjectCanvasPersonRow[]> => {
      const batch = await getUsersBatch(pubkeys);
      const rows = await Promise.all(
        pubkeys.map(async (pubkey) => {
          const profile = batch.profiles[pubkey];
          return {
            avatarDataUrl: await inlineAvatarDataUrl(
              avatarCacheRef.current,
              profile?.avatarUrl ?? null,
            ),
            displayName: profile
              ? boundedText(profile.displayName ?? profile.name ?? "", 128) ||
                null
              : null,
            isAgent: profile?.isAgent ?? false,
            pubkey,
          };
        }),
      );
      // Publish every avatar before returning, so the frame can load the ones
      // the budget below drops from `./__buzz/avatar/<pubkey>` instead. A
      // failure here costs only those pictures — the inlined ones still
      // arrive, and the next lookup republishes from the same cache.
      const request = canvasRequestRef.current;
      if (request) {
        try {
          await publishProjectCanvasAvatars(
            request,
            toCanvasAvatarUploads(
              rows.map((row) => ({
                dataUrl: row.avatarDataUrl,
                pubkey: row.pubkey,
              })),
            ),
          );
        } catch {
          // Falls back to the inlined avatars and initials.
        }
      }
      // Inlined rows all travel in one RPC message. Without a combined ceiling
      // a handful of avatars overruns it and the frame gets a `too-large`
      // error instead of the lookup — losing the names too, not just the
      // pictures.
      const budgeted = selectAvatarsWithinBudget(
        rows.map((row) => row.avatarDataUrl),
      );
      return rows.map((row, index) => ({
        ...row,
        avatarDataUrl: budgeted[index] ?? null,
      }));
    };
    brokerRef.current = new ProjectCanvasBroker({
      lookupPeople: lookupProfiles,
      openTarget: async (target) => {
        onOpenTargetRef.current(target);
      },
      runTaskCommand: async (command) => {
        const issue = issuesRef.current.find(
          (candidate) =>
            candidate.id.toLowerCase() === command.task.id.toLowerCase(),
        );
        if (!issue) {
          throw new Error("Task is no longer available in this project.");
        }
        if (command.name === "tasks.setStatus") {
          await setStatusRef.current({ issue, status: command.status });
          return;
        }
        const signerPubkey = identityRef.current;
        if (!signerPubkey) {
          throw new Error("No signed-in identity is available.");
        }
        const assignee = command.assignee ?? signerPubkey.toLowerCase();
        const [profileRow] = await lookupProfiles([assignee]);
        const assigneeLabel =
          profileRow?.displayName ?? `${assignee.slice(0, 8)}…`;
        const mutate =
          command.name === "tasks.assign"
            ? assignRef.current
            : unassignRef.current;
        await mutate({
          assigneeLabel,
          assignees: [assignee],
          issue,
          signAsManagedOwner: false,
          signerPubkey,
        });
      },
      searchPeople: async (query, limit) => {
        const page = await searchUsers(query, limit);
        // Search rows stay avatar-free (initials fallback in the SDK); only
        // explicit lookups pay the data-URL inlining budget.
        return page.users.map((user) => ({
          avatarDataUrl: null,
          displayName: user.displayName
            ? boundedText(user.displayName, 128)
            : null,
          isAgent: user.isAgent,
          pubkey: user.pubkey.toLowerCase(),
        }));
      },
      sendDirectMessage: async (recipient, message) => {
        const signerPubkey = identityRef.current;
        if (!signerPubkey) {
          throw new Error("No signed-in identity is available.");
        }
        // Tenant scope captured before the first await: the backend fails
        // closed if the active community or identity changes mid-send, so a
        // suspended completion can never deliver into the wrong community.
        const expectedRelayUrl = relayUrlRef.current
          ? normalizeRelayUrl(relayUrlRef.current)
          : undefined;
        const expectedSignerPubkey = normalizePubkey(signerPubkey);
        const dm = await openDmRef.current({
          expectedRelayUrl,
          expectedSignerPubkey,
          pubkeys: [recipient],
        });
        // A freshly opened DM has no live subscription yet, so the first
        // message must take the HTTP path rather than the socket.
        await sendChannelMessage(
          dm.id,
          message,
          null,
          undefined,
          undefined,
          undefined,
          undefined,
          undefined,
          undefined,
          undefined,
          expectedRelayUrl,
          expectedSignerPubkey,
        );
      },
    });
  }
  const broker = brokerRef.current;

  const tasksState = React.useMemo(() => {
    if (!primaryRepository) {
      return { data: [], status: "ready" as const };
    }
    if (issues.isPending) return { data: null, status: "loading" as const };
    if (issues.isError) return { data: null, status: "error" as const };
    return {
      data: (issues.data ?? []).slice(0, MAX_BROKER_ROWS).map(toTaskRow),
      status: "ready" as const,
    };
  }, [issues.data, issues.isError, issues.isPending, primaryRepository]);

  const reviewsState = React.useMemo(() => {
    if (!primaryRepository) {
      return { data: [], status: "ready" as const };
    }
    if (reviews.isPending) return { data: null, status: "loading" as const };
    if (reviews.isError) return { data: null, status: "error" as const };
    return {
      data: (reviews.data ?? []).slice(0, MAX_BROKER_ROWS).map(toReviewListRow),
      status: "ready" as const,
    };
  }, [primaryRepository, reviews.data, reviews.isError, reviews.isPending]);

  React.useEffect(() => {
    broker.setSources({
      channels: snapshots.channels,
      project: snapshots.project,
      reviews: reviewsState,
      tasks: tasksState,
    });
  }, [broker, reviewsState, snapshots.channels, snapshots.project, tasksState]);

  return broker;
}
