// BW-aware issue creation (P4D). Creates the 1621 root through the same
// generic signing/publish plumbing the legacy path uses, then — only when
// the target repository actually has a current BW role policy — signs a
// deliberate BW-enroll transition and, if the reporter supplied acceptance
// criteria or non-goals, one initial issue-update. A root created here never
// carries the legacy `p` recipient tag: NIP-BW's kind:1621 shape allows only
// `a`/`subject`(/`auth`), so any extra tag makes the root permanently
// unenrollable (see crates/buzz-core/src/bw/shape.rs). Repositories without a
// BW policy get a plain root exactly like before; it simply is not yet a BW
// issue, and never silently becomes one.
import { useQuery, useQueryClient, useMutation } from "@tanstack/react-query";

import type { Repository } from "./hooks";
import { fetchBwSnapshot } from "./bwProjection";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent, invokeTauri } from "@/shared/api/tauri";
import { KIND_GIT_ISSUE } from "@/shared/constants/kinds";

export function buildBwIssueRootTags(
  repoAddress: string,
  title: string,
): string[][] {
  if (!repoAddress.startsWith("30617:")) {
    throw new Error("Issue repo address must reference a kind:30617 repo.");
  }
  const subject = title.trim();
  if (!subject) {
    throw new Error("Issue title is required.");
  }
  if (subject.length > 256) {
    throw new Error("Issue title must be 256 characters or fewer.");
  }
  return [
    ["a", repoAddress],
    ["subject", subject],
  ];
}

export function normalizeTemplateLines(lines: string[]): string[] {
  return lines.map((line) => line.trim()).filter((line) => line.length > 0);
}

export type CreateBwIssueInput = {
  repoAddress: string;
  title: string;
  content: string;
  acceptanceCriteria: string[];
  nonGoals: string[];
};

export type CreateBwIssueResult = {
  issueId: string;
  bwEnrolled: boolean;
  bwError: string | null;
};

/** Whether the target repository currently has an owner-signed BW policy in
 * force. Repositories without one never get an enroll attempt: their roots
 * stay ordinary legacy issues, exactly as before. */
export async function isProjectBwActive(repoAddress: string): Promise<boolean> {
  const snapshot = await fetchBwSnapshot(repoAddress);
  return snapshot.activation !== null;
}

export function useProjectBwActivation(repoAddress: string | null | undefined) {
  return useQuery({
    queryKey: ["project-bw-activation", repoAddress ?? "none"],
    queryFn: () => isProjectBwActive(repoAddress as string),
    enabled: Boolean(repoAddress),
    staleTime: 30_000,
  });
}

/** Create a BW-shaped root, then attempt enrollment (and an initial text
 * patch, if any criteria/non-goals were supplied). A failed enrollment
 * attempt never rolls back or hides the already-published root — it is
 * reported so the caller can show "not yet a BW issue" with the reason,
 * never a silent partial success. */
export async function createProjectBwIssue(
  input: CreateBwIssueInput,
): Promise<CreateBwIssueResult> {
  const event = await signRelayEvent({
    kind: KIND_GIT_ISSUE,
    content: input.content.trim(),
    tags: buildBwIssueRootTags(input.repoAddress, input.title),
  });
  await relayClient.publishEvent(
    event,
    "Timed out creating issue.",
    "Failed to create issue.",
  );
  const issueId = event.id;

  try {
    await invokeTauri("submit_project_bw_record", {
      input: {
        repo: input.repoAddress,
        record: "issue-state",
        tags: [["issue", issueId]],
        content: { state: "triage" },
        delegate: false,
      },
    });

    const acceptanceCriteria = normalizeTemplateLines(input.acceptanceCriteria);
    const nonGoals = normalizeTemplateLines(input.nonGoals);
    if (acceptanceCriteria.length > 0 || nonGoals.length > 0) {
      const patch: Record<string, string[]> = {};
      if (acceptanceCriteria.length > 0) {
        patch.acceptance_criteria = acceptanceCriteria;
      }
      if (nonGoals.length > 0) {
        patch.non_goals = nonGoals;
      }
      await invokeTauri("submit_project_bw_record", {
        input: {
          repo: input.repoAddress,
          record: "issue-update",
          tags: [["issue", issueId]],
          content: { patch },
          delegate: false,
        },
      });
    }
    return { issueId, bwEnrolled: true, bwError: null };
  } catch (error) {
    return {
      issueId,
      bwEnrolled: false,
      bwError: error instanceof Error ? error.message : String(error),
    };
  }
}

type CreateProjectBwIssueMutationInput = {
  title: string;
  body: string;
  acceptanceCriteria?: string[];
  nonGoals?: string[];
};

export function useCreateProjectBwIssueMutation(
  project: Repository | null | undefined,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (input: CreateProjectBwIssueMutationInput) => {
      if (!project) throw new Error("No project selected.");
      const result = await createProjectBwIssue({
        repoAddress: project.repoAddress,
        title: input.title,
        content: input.body,
        acceptanceCriteria: input.acceptanceCriteria ?? [],
        nonGoals: input.nonGoals ?? [],
      });
      if (!result.bwEnrolled) {
        throw new BwIssueCreatedButNotEnrolledError(result);
      }
      return result.issueId;
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: ["project", project?.id ?? "none", "issues"],
        }),
        queryClient.invalidateQueries({ queryKey: ["projects", "work-items"] }),
        queryClient.invalidateQueries({
          queryKey: ["projects", "activity-summaries"],
        }),
      ]);
    },
  });
}

/** The root was created (and is real, visible history) but the enroll
 * attempt failed. Distinct from a hard failure so callers can still
 * navigate to the created issue and show why it is not yet a BW issue. */
export class BwIssueCreatedButNotEnrolledError extends Error {
  issueId: string;
  constructor(result: CreateBwIssueResult) {
    super(result.bwError ?? "Issue created, but BW enrollment failed.");
    this.issueId = result.issueId;
  }
}
