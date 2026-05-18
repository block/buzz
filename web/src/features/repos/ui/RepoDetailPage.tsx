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
    <div className="flex items-center gap-2 rounded-md border border-input bg-muted/50 px-3 py-2">
      <code className="min-w-0 flex-1 truncate text-sm">{url}</code>
      <button
        type="button"
        onClick={handleCopy}
        className="shrink-0 text-muted-foreground hover:text-foreground"
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
    <div className="flex w-full gap-8 px-4 py-8">
      <div className="min-w-0 flex-1">
        <div className="h-5 w-24 animate-pulse rounded bg-muted" />
        <div className="mt-6 h-8 w-64 animate-pulse rounded bg-muted" />
        <div className="mt-3 h-5 w-96 animate-pulse rounded bg-muted" />
        <div className="mt-8 space-y-3">
          <div className="h-4 w-32 animate-pulse rounded bg-muted" />
          <div className="h-10 w-full animate-pulse rounded bg-muted" />
        </div>
      </div>
      <aside className="hidden w-72 shrink-0 lg:block" />
    </div>
  );
}

type Tab = "code" | "commits";

function RepoTabs({
  treeEntries,
  treeLoading,
  commits,
  commitsLoading,
  readme,
  readmeLoading,
}: {
  treeEntries: TreeEntry[] | undefined;
  treeLoading: boolean;
  commits: CommitInfo[] | undefined;
  commitsLoading: boolean;
  readme: ReadmeResult | null | undefined;
  readmeLoading: boolean;
}) {
  const [tab, setTab] = useState<Tab>("code");

  return (
    <div className="mt-6">
      {/* Tab bar */}
      <div className="flex gap-1 border-b border-border">
        <button
          type="button"
          onClick={() => setTab("code")}
          className={`px-4 py-2 text-sm font-medium transition-colors ${
            tab === "code"
              ? "border-b-2 border-foreground text-foreground"
              : "text-muted-foreground hover:text-foreground"
          }`}
        >
          Code
        </button>
        <button
          type="button"
          onClick={() => setTab("commits")}
          className={`px-4 py-2 text-sm font-medium transition-colors ${
            tab === "commits"
              ? "border-b-2 border-foreground text-foreground"
              : "text-muted-foreground hover:text-foreground"
          }`}
        >
          Commits
        </button>
      </div>

      {/* Tab content */}
      {tab === "code" && (
        <>
          <RepoTreeSection entries={treeEntries} isLoading={treeLoading} />
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
  const { data: repo, isLoading, error } = useRepo(repoId);
  const { data: refs, isLoading: refsLoading } = useRepoRefs(repoId);

  const defaultRef = refs?.head?.ref ?? "main";
  const owner = repo?.owner ?? "";
  const repoName = repo?.id ?? "";

  const {
    data: treeEntries,
    isLoading: treeLoading,
    error: treeError,
  } = useGitTree(owner, repoName, defaultRef);
  const {
    data: commits,
    isLoading: commitsLoading,
    error: commitsError,
  } = useGitLog(owner, repoName, defaultRef);
  const { data: readme, isLoading: readmeLoading } = useGitReadme(
    owner,
    repoName,
    defaultRef,
  );

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
      <div className="flex w-full gap-8 px-4 py-8">
        <div className="min-w-0 flex-1">
          <Link
            to="/"
            className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
          >
            <ArrowLeft className="h-4 w-4" />
            Back to repositories
          </Link>
          <div className="mt-12 text-center">
            <BookMarked className="mx-auto h-10 w-10 text-muted-foreground" />
            <h1 className="mt-4 text-xl font-semibold">Repository not found</h1>
            <p className="mt-1 text-sm text-muted-foreground">
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
    <div className="flex w-full gap-8 px-4 py-8">
      {/* Main content */}
      <div className="min-w-0 flex-1">
        {/* Back link */}
        <Link
          to="/"
          className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="h-4 w-4" />
          Back to repositories
        </Link>

        {/* Mobile-only connect button */}
        <div className="mt-4 lg:hidden">
          <ConnectButton className="w-full" />
        </div>

        {/* Header */}
        <div className="mt-6">
          <div className="flex items-center gap-3">
            <BookMarked className="h-6 w-6 shrink-0 text-muted-foreground" />
            <h1 className="text-2xl font-semibold tracking-tight">
              {repo.name}
            </h1>
            <Badge variant="outline">Public</Badge>
          </div>
          {repo.description && (
            <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
              {repo.description}
            </p>
          )}
          <p className="mt-2 text-xs text-muted-foreground">
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
          treeEntries={treeEntries}
          treeLoading={treeLoading}
          commits={commits}
          commitsLoading={commitsLoading}
          readme={readme}
          readmeLoading={readmeLoading}
        />

        {/* Clone URLs */}
        {repo.cloneUrls.length > 0 && (
          <div className="mt-8">
            <h2 className="mb-3 text-sm font-semibold">Clone</h2>
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
              <Button variant="outline" asChild>
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
            <Button variant="outline" asChild>
              <a href={`/channels/${repo.channelId}`}>
                <MessageSquare className="h-4 w-4" />
                View channel
              </a>
            </Button>
          </div>
        )}
      </div>

      {/* Sidebar */}
      <aside className="hidden w-72 shrink-0 border-l border-border pl-8 lg:block">
        <div className="space-y-6">
          {/* Open in Sprout */}
          <ConnectButton className="w-full" />

          {/* People */}
          <div>
            <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold">
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
