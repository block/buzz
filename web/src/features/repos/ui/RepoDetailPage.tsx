import {
  ArrowLeft,
  BookMarked,
  Check,
  Copy,
  ExternalLink,
  MessageSquare,
  Users,
} from "lucide-react";
import { useEffect, useState } from "react";
import { Link, useParams } from "@tanstack/react-router";
import { toast } from "sonner";

import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { relativeTime } from "@/shared/lib/relative-time";
import { useRepoRefs } from "../use-repo-refs";
import { useRepo } from "../use-repos";
import {
  getMockRepo,
  mockRepoCommits,
  mockRepoReadme,
  mockRepoTree,
} from "../mock-repos";
import type { CommitInfo, ReadmeResult, TreeEntry } from "../git-client";
import { useGitTree, useGitLog, useGitReadme } from "../use-git-browse";
import { ConnectButton } from "./ConnectButton";
import { PubkeyAvatar } from "./PubkeyAvatar";
import { RepoRefsSection } from "./RepoRefsSection";
import { RepoTreeSection } from "./RepoTreeSection";
import { RepoCommitsSection } from "./RepoCommitsSection";
import { RepoReadmeSection } from "./RepoReadmeSection";

function CopyableUrl({ url }: { url: string }) {
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      toast.success("Copied to clipboard");
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error("Failed to copy to clipboard");
    }
  }

  return (
    <div className="flex items-center gap-2 rounded-md border border-black/10 bg-white px-3 py-2 dark:border-white/10 dark:bg-white/5">
      <code className="min-w-0 flex-1 truncate text-sm text-black dark:text-white">
        {url}
      </code>
      <button
        type="button"
        onClick={handleCopy}
        className="shrink-0 text-black/50 hover:text-black dark:text-white/50 dark:hover:text-white"
        aria-label="Copy clone URL"
      >
        {copied ? (
          <Check className="h-4 w-4 text-green-500" />
        ) : (
          <Copy className="h-4 w-4" />
        )}
      </button>
    </div>
  );
}

function DetailSkeleton() {
  return (
    <div className="flex w-full flex-1 gap-8 bg-[#F3F3F3] px-4 py-8 dark:bg-[#171717]">
      <div className="min-w-0 flex-1">
        <div className="h-5 w-24 animate-pulse rounded bg-black/10 dark:bg-white/10" />
        <div className="mt-6 h-8 w-64 animate-pulse rounded bg-black/10 dark:bg-white/10" />
        <div className="mt-3 h-5 w-96 animate-pulse rounded bg-black/10 dark:bg-white/10" />
        <div className="mt-8 space-y-3">
          <div className="h-4 w-32 animate-pulse rounded bg-black/10 dark:bg-white/10" />
          <div className="h-10 w-full animate-pulse rounded bg-black/10 dark:bg-white/10" />
        </div>
      </div>
      <aside className="hidden w-72 shrink-0 lg:block" />
    </div>
  );
}

function BackToRepositories({
  mockPreview = false,
}: {
  mockPreview?: boolean;
}) {
  const className =
    "inline-flex items-center gap-1 text-sm text-black/60 hover:text-black dark:text-white/60 dark:hover:text-white";
  const content = (
    <>
      <ArrowLeft className="h-4 w-4" />
      Back to repositories
    </>
  );

  return mockPreview ? (
    <a href="/?preview=repositories" className={className}>
      {content}
    </a>
  ) : (
    <Link to="/" className={className}>
      {content}
    </Link>
  );
}

type Tab = "code" | "commits";

function RepoTabs({
  repoId,
  treeEntries,
  treeLoading,
  commits,
  commitsLoading,
  readme,
  readmeLoading,
  preview,
}: {
  repoId: string;
  treeEntries: TreeEntry[] | undefined;
  treeLoading: boolean;
  commits: CommitInfo[] | undefined;
  commitsLoading: boolean;
  readme: ReadmeResult | null | undefined;
  readmeLoading: boolean;
  preview: boolean;
}) {
  const [tab, setTab] = useState<Tab>("code");

  return (
    <div className="mt-6">
      {/* Tab bar */}
      <div className="flex gap-1 border-b border-black/10 dark:border-white/10">
        <button
          type="button"
          onClick={() => setTab("code")}
          className={`px-4 py-2 text-sm font-medium transition-colors ${
            tab === "code"
              ? "border-b-2 border-black text-black dark:border-white dark:text-white"
              : "text-black/50 hover:text-black dark:text-white/50 dark:hover:text-white"
          }`}
        >
          Code
        </button>
        <button
          type="button"
          onClick={() => setTab("commits")}
          className={`px-4 py-2 text-sm font-medium transition-colors ${
            tab === "commits"
              ? "border-b-2 border-black text-black dark:border-white dark:text-white"
              : "text-black/50 hover:text-black dark:text-white/50 dark:hover:text-white"
          }`}
        >
          Commits
        </button>
      </div>

      {/* Tab content */}
      {tab === "code" && (
        <>
          <RepoTreeSection
            entries={treeEntries}
            isLoading={treeLoading}
            repoId={repoId}
            preview={preview}
          />
          <RepoReadmeSection readme={readme} isLoading={readmeLoading} />
        </>
      )}
      {tab === "commits" && (
        <RepoCommitsSection commits={commits} isLoading={commitsLoading} />
      )}
    </div>
  );
}

export function RepoDetailPage() {
  const { repoId } = useParams({ from: "/repos/$repoId" });
  const preview =
    import.meta.env.DEV &&
    new URLSearchParams(window.location.search).get("preview") ===
      "repositories";
  const mockRepo = preview ? getMockRepo(repoId) : undefined;
  const showMockRepo = Boolean(mockRepo);
  const {
    data: repo,
    isLoading,
    error,
  } = useRepo(repoId, {
    preview: showMockRepo,
  });
  const { data: refs, isLoading: refsLoading } = useRepoRefs(repoId, {
    preview: showMockRepo,
  });

  const defaultRef = refs?.head?.ref ?? "main";
  const owner = repo?.owner ?? "";
  const repoName = repo?.id ?? "";
  const browseOwner = showMockRepo ? "" : owner;

  const {
    data: fetchedTreeEntries,
    isLoading: isTreeLoading,
    error: treeError,
  } = useGitTree(browseOwner, repoName, defaultRef);
  const {
    data: fetchedCommits,
    isLoading: areCommitsLoading,
    error: commitsError,
  } = useGitLog(browseOwner, repoName, defaultRef);
  const { data: fetchedReadme, isLoading: isReadmeLoading } = useGitReadme(
    browseOwner,
    repoName,
    defaultRef,
  );
  const treeEntries = showMockRepo ? mockRepoTree : fetchedTreeEntries;
  const commits = showMockRepo ? mockRepoCommits : fetchedCommits;
  const readme = showMockRepo ? mockRepoReadme : fetchedReadme;
  const treeLoading = showMockRepo ? false : isTreeLoading;
  const commitsLoading = showMockRepo ? false : areCommitsLoading;
  const readmeLoading = showMockRepo ? false : isReadmeLoading;

  // Surface clone/browse errors — these are otherwise silent
  const browseError = treeError || commitsError;
  useEffect(() => {
    if (browseError) {
      console.error("[git-browse]", browseError);
    }
  }, [browseError]);

  useEffect(() => {
    if (error) {
      toast.error("Failed to load repository", {
        description: error.message,
      });
    }
  }, [error]);

  if (isLoading) return <DetailSkeleton />;

  if (!repo) {
    return (
      <div className="flex w-full flex-1 gap-8 bg-[#F3F3F3] px-4 py-8 text-black dark:bg-[#171717] dark:text-white">
        <div className="min-w-0 flex-1">
          <BackToRepositories />
          <div className="mt-12 text-center">
            <BookMarked className="mx-auto h-10 w-10 text-black/50 dark:text-white/50" />
            <h1 className="mt-4 text-xl font-semibold text-black dark:text-white">
              Repository not found
            </h1>
            <p className="mt-1 text-sm text-black/60 dark:text-white/60">
              This repository may have been removed or doesn't exist on this
              relay.
            </p>
          </div>
        </div>
        <aside className="hidden w-72 shrink-0 lg:block" />
      </div>
    );
  }

  return (
    <div className="flex w-full flex-1 gap-8 bg-[#F3F3F3] px-4 py-8 text-black dark:bg-[#171717] dark:text-white">
      {/* Main content */}
      <div className="min-w-0 flex-1">
        {/* Back link */}
        <BackToRepositories mockPreview={preview} />

        {/* Mobile-only connect button */}
        <div className="mt-4 lg:hidden">
          <ConnectButton className="w-full" />
        </div>

        {/* Header */}
        <div className="mt-6">
          <div className="flex items-center gap-3">
            <BookMarked className="h-6 w-6 shrink-0 text-black/50 dark:text-white/50" />
            <h1 className="text-2xl font-semibold tracking-tight text-black dark:text-white">
              {repo.name}
            </h1>
            <Badge
              variant="outline"
              className="border-black/15 text-black/60 dark:border-white/15 dark:text-white/60"
            >
              Public
            </Badge>
          </div>
          {repo.description && (
            <p className="mt-2 text-sm leading-relaxed text-black/60 dark:text-white/60">
              {repo.description}
            </p>
          )}
          <p className="mt-2 text-xs text-black/50 dark:text-white/50">
            Updated {relativeTime(repo.createdAt)}
          </p>
        </div>

        {/* Refs & HEAD */}
        <RepoRefsSection refs={refs} isLoading={refsLoading} />

        {/* Clone/browse error banner */}
        {browseError && (
          <div className="mt-6 rounded-md border border-destructive/50 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            Failed to load repository contents:{" "}
            {browseError instanceof Error
              ? browseError.message
              : String(browseError)}
          </div>
        )}

        {/* Tabs */}
        <RepoTabs
          repoId={repoId}
          treeEntries={treeEntries}
          treeLoading={treeLoading}
          commits={commits}
          commitsLoading={commitsLoading}
          readme={readme}
          readmeLoading={readmeLoading}
          preview={showMockRepo}
        />

        {/* Clone URLs */}
        {repo.cloneUrls.length > 0 && (
          <div className="mt-8">
            <h2 className="mb-3 text-sm font-semibold text-black dark:text-white">
              Clone
            </h2>
            <div className="space-y-2">
              {repo.cloneUrls.map((url) => (
                <CopyableUrl key={url} url={url} />
              ))}
            </div>
          </div>
        )}

        {/* External link — validate scheme to prevent javascript: XSS */}
        {(() => {
          if (!repo.webUrl) return null;
          let safe: string | null = null;
          try {
            safe = /^https?:/.test(new URL(repo.webUrl).protocol)
              ? repo.webUrl
              : null;
          } catch {
            safe = null;
          }
          if (!safe) return null;
          return (
            <div className="mt-6">
              <Button
                variant="outline"
                className="border-black/10 bg-white text-black hover:bg-black/5 dark:border-white/10 dark:bg-white/5 dark:text-white dark:hover:bg-white/10"
                asChild
              >
                <a href={safe} target="_blank" rel="noopener noreferrer">
                  <ExternalLink className="h-4 w-4" />
                  View on web
                </a>
              </Button>
            </div>
          );
        })()}

        {/* Channel link */}
        {repo.channelId && (
          <div className="mt-8">
            <Button
              variant="outline"
              className="border-black/10 bg-white text-black hover:bg-black/5 dark:border-white/10 dark:bg-white/5 dark:text-white dark:hover:bg-white/10"
              asChild
            >
              <a href={`/channels/${repo.channelId}`}>
                <MessageSquare className="h-4 w-4" />
                View channel
              </a>
            </Button>
          </div>
        )}
      </div>

      {/* Sidebar */}
      <aside className="hidden w-72 shrink-0 border-l border-black/10 pl-8 dark:border-white/10 lg:block">
        <div className="space-y-6">
          {/* Open in Buzz */}
          <ConnectButton className="w-full" />

          {/* People */}
          <div>
            <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold text-black dark:text-white">
              <Users className="h-4 w-4" />
              People
            </h3>
            <div className="flex flex-wrap gap-2">
              <PubkeyAvatar pubkey={repo.owner} />
              {repo.contributors
                .filter((c) => c !== repo.owner)
                .map((c) => (
                  <PubkeyAvatar key={c} pubkey={c} />
                ))}
            </div>
          </div>
        </div>
      </aside>
    </div>
  );
}
