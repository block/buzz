import { ArrowLeft, GitPullRequest, MessageSquare, Send } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  type Project,
  type ProjectPullRequest,
  useCreateProjectPullRequestCommentMutation,
} from "@/features/projects/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import { ProfileIdentityButton } from "./ProjectProfileIdentity";

function compactDate(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

function pluralize(count: number, singular: string, plural = `${singular}s`) {
  return `${count} ${count === 1 ? singular : plural}`;
}

function profileForPubkey(pubkey: string, profiles?: UserProfileLookup) {
  return profiles?.[normalizePubkey(pubkey)] ?? null;
}

function labelForPubkey(pubkey: string, profiles?: UserProfileLookup) {
  const profile = profileForPubkey(pubkey, profiles);
  return (
    profile?.displayName?.trim() ||
    profile?.nip05Handle?.trim() ||
    `${pubkey.slice(0, 8)}…${pubkey.slice(-4)}`
  );
}

function AuthorIdentity({
  profiles,
  pubkey,
  role,
}: {
  profiles?: UserProfileLookup;
  pubkey: string;
  role?: React.ReactNode;
}) {
  const profile = profileForPubkey(pubkey, profiles);
  return (
    <ProfileIdentityButton
      align="center"
      avatarSize="xs"
      avatarUrl={profile?.avatarUrl ?? null}
      isAgent={profile?.isAgent === true}
      label={labelForPubkey(pubkey, profiles)}
      pubkey={pubkey}
      role={role}
    />
  );
}

function PullRequestRow({
  onOpen,
  pullRequest,
}: {
  onOpen: () => void;
  pullRequest: ProjectPullRequest;
}) {
  return (
    <button
      className="flex w-full min-w-0 items-start gap-3 p-3 text-left transition-colors hover:bg-muted/30 focus-visible:bg-muted/30 focus-visible:outline-hidden"
      onClick={onOpen}
      type="button"
    >
      <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <GitPullRequest className="h-4 w-4" />
      </div>
      <div className="min-w-0 flex-1 space-y-1">
        <p className="truncate text-sm font-medium text-foreground">
          {pullRequest.title}
        </p>
        <p className="truncate text-xs text-muted-foreground">
          {pullRequest.branchName ? `${pullRequest.branchName} · ` : ""}
          {pullRequest.updateCount > 0
            ? `${pluralize(pullRequest.updateCount, "update")} · `
            : ""}
          {pullRequest.comments.length > 0
            ? `${pluralize(pullRequest.comments.length, "comment")} · `
            : ""}
          {compactDate(pullRequest.updatedAt)}
        </p>
        {pullRequest.content ? (
          <p className="line-clamp-2 text-sm text-muted-foreground">
            {pullRequest.content}
          </p>
        ) : null}
      </div>
      {pullRequest.commit ? (
        <code className="shrink-0 rounded-md bg-background/55 px-2 py-1 text-xs text-muted-foreground">
          {pullRequest.commit.slice(0, 7)}
        </code>
      ) : null}
    </button>
  );
}

function PullRequestDetail({
  onBack,
  profiles,
  project,
  pullRequest,
}: {
  onBack: () => void;
  profiles?: UserProfileLookup;
  project: Project;
  pullRequest: ProjectPullRequest;
}) {
  const [comment, setComment] = React.useState("");
  const commentMutation = useCreateProjectPullRequestCommentMutation(project);
  const canSubmit = comment.trim().length > 0 && !commentMutation.isPending;

  const handleSubmit = React.useCallback(
    (event: React.FormEvent) => {
      event.preventDefault();
      if (!canSubmit) return;

      commentMutation.mutate(
        { content: comment, pullRequest },
        {
          onSuccess: () => {
            setComment("");
            toast.success("Comment posted.");
          },
          onError: (error) => {
            toast.error(
              error instanceof Error
                ? error.message
                : "Failed to post comment.",
            );
          },
        },
      );
    },
    [canSubmit, comment, commentMutation, pullRequest],
  );

  return (
    <div className="divide-y divide-border/50">
      <header className="space-y-3 p-4">
        <div className="flex min-w-0 items-start gap-3">
          <Button
            aria-label="Back to pull requests"
            className="mt-0.5 h-8 w-8 shrink-0"
            onClick={onBack}
            size="icon"
            variant="ghost"
          >
            <ArrowLeft className="h-4 w-4" />
          </Button>
          <div className="min-w-0 flex-1 space-y-2">
            <div className="flex min-w-0 items-start justify-between gap-3">
              <div className="min-w-0">
                <p className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
                  <GitPullRequest className="h-3.5 w-3.5" />
                  Pull request
                </p>
                <h3 className="mt-1 line-clamp-2 text-base font-semibold text-foreground">
                  {pullRequest.title}
                </h3>
              </div>
              {pullRequest.commit ? (
                <code className="shrink-0 rounded-md bg-background/55 px-2 py-1 text-xs text-muted-foreground">
                  {pullRequest.commit.slice(0, 7)}
                </code>
              ) : null}
            </div>
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
              <AuthorIdentity
                profiles={profiles}
                pubkey={pullRequest.author}
                role={`opened ${compactDate(pullRequest.createdAt)}`}
              />
              {pullRequest.branchName ? (
                <span>Branch: {pullRequest.branchName}</span>
              ) : null}
              <span>Updated {compactDate(pullRequest.updatedAt)}</span>
            </div>
          </div>
        </div>
        {pullRequest.content ? (
          <div className="rounded-lg border border-border/50 bg-background/45 p-3">
            <Markdown
              className="text-sm"
              content={pullRequest.content}
              interactive={false}
            />
          </div>
        ) : null}
      </header>

      {pullRequest.updates.length > 0 ? (
        <section className="space-y-3 p-4">
          <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Updates
          </h4>
          {pullRequest.updates.map((update) => (
            <article className="space-y-1" key={update.id}>
              <div className="flex min-w-0 items-center justify-between gap-3">
                <AuthorIdentity
                  profiles={profiles}
                  pubkey={update.author}
                  role={compactDate(update.createdAt)}
                />
                {update.commit ? (
                  <code className="shrink-0 rounded-md bg-background/55 px-2 py-1 text-xs text-muted-foreground">
                    {update.commit.slice(0, 7)}
                  </code>
                ) : null}
              </div>
              {update.content ? (
                <p className="text-sm text-muted-foreground">
                  {update.content}
                </p>
              ) : null}
            </article>
          ))}
        </section>
      ) : null}

      <section className="space-y-3 p-4">
        <h4 className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          <MessageSquare className="h-3.5 w-3.5" />
          Discussion
        </h4>
        {pullRequest.comments.length > 0 ? (
          <div className="space-y-3">
            {pullRequest.comments.map((item) => (
              <article
                className="rounded-lg border border-border/50 bg-background/45 p-3"
                key={item.id}
              >
                <div className="mb-2">
                  <AuthorIdentity
                    profiles={profiles}
                    pubkey={item.author}
                    role={compactDate(item.createdAt)}
                  />
                </div>
                <Markdown
                  className="text-sm"
                  content={item.content}
                  interactive={false}
                />
              </article>
            ))}
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">No comments yet.</p>
        )}
        <form className="space-y-2" onSubmit={handleSubmit}>
          <textarea
            className="min-h-24 w-full resize-y rounded-lg border border-border/50 bg-background/55 px-3 py-2 text-sm text-foreground outline-hidden placeholder:text-muted-foreground focus:border-ring"
            onChange={(event) => setComment(event.target.value)}
            placeholder="Add a comment…"
            value={comment}
          />
          <div className="flex justify-end">
            <Button
              className="gap-1.5"
              disabled={!canSubmit}
              size="sm"
              type="submit"
            >
              <Send className="h-3.5 w-3.5" />
              Comment
            </Button>
          </div>
        </form>
      </section>
    </div>
  );
}

export function PullRequestsPanel({
  error,
  isLoading,
  profiles,
  project,
  pullRequests,
}: {
  error: unknown;
  isLoading: boolean;
  profiles?: UserProfileLookup;
  project: Project;
  pullRequests: ProjectPullRequest[];
}) {
  const [selectedPullRequestId, setSelectedPullRequestId] = React.useState<
    string | null
  >(null);
  const selectedPullRequest =
    pullRequests.find((item) => item.id === selectedPullRequestId) ?? null;

  React.useEffect(() => {
    if (
      selectedPullRequestId &&
      !pullRequests.some((item) => item.id === selectedPullRequestId)
    ) {
      setSelectedPullRequestId(null);
    }
  }, [pullRequests, selectedPullRequestId]);

  if (isLoading) {
    return (
      <p className="p-4 text-sm text-muted-foreground">
        Loading pull requests…
      </p>
    );
  }

  if (pullRequests.length === 0) {
    return (
      <p className="p-4 text-sm text-muted-foreground">
        {error
          ? "Could not load pull requests for this repository."
          : "No pull requests yet."}
      </p>
    );
  }

  if (selectedPullRequest) {
    return (
      <PullRequestDetail
        onBack={() => setSelectedPullRequestId(null)}
        profiles={profiles}
        project={project}
        pullRequest={selectedPullRequest}
      />
    );
  }

  return (
    <div className="divide-y divide-border/50">
      {pullRequests.map((pullRequest) => (
        <PullRequestRow
          key={pullRequest.id}
          onOpen={() => setSelectedPullRequestId(pullRequest.id)}
          pullRequest={pullRequest}
        />
      ))}
    </div>
  );
}
