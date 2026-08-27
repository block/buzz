import * as React from "react";
import {
  ExternalLink,
  FileOutput,
  FileText,
  Film,
  Image as ImageIcon,
  LoaderCircle,
  RefreshCcw,
} from "lucide-react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { getThreadReference } from "@/features/messages/lib/threading";
import type { OutboxArtifact } from "@/features/outbox/lib/artifacts";
import { useOutboxArtifacts } from "@/features/outbox/useOutboxArtifacts";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { openArtifactFile } from "@/shared/api/tauriMedia";
import type { SearchHit } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";
import { UserAvatar } from "@/shared/ui/UserAvatar";

function formatArtifactSize(bytes: number | undefined) {
  if (bytes === undefined || bytes < 0 || !Number.isFinite(bytes)) return null;
  if (bytes < 1_024) return `${bytes} B`;
  if (bytes < 1_048_576) return `${(bytes / 1_024).toFixed(1)} KB`;
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

function formatArtifactTime(createdAt: number) {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(createdAt * 1_000));
}

function ArtifactIcon({ artifact }: { artifact: OutboxArtifact }) {
  const iconClassName = "h-5 w-5";
  if (artifact.kind === "image") {
    return <ImageIcon className={iconClassName} />;
  }
  if (artifact.kind === "video") {
    return <Film className={iconClassName} />;
  }
  return <FileText className={iconClassName} />;
}

function OutboxLoadingState() {
  return (
    <div className="mx-auto w-full max-w-4xl space-y-3 px-5 py-6">
      {[0, 1, 2].map((index) => (
        <div
          className="flex items-center gap-4 rounded-2xl border border-border/60 p-4"
          key={index}
        >
          <Skeleton className="h-11 w-11 rounded-xl" />
          <div className="min-w-0 flex-1 space-y-2">
            <Skeleton className="h-4 w-56 max-w-full" />
            <Skeleton className="h-3 w-80 max-w-full" />
          </div>
          <Skeleton className="h-8 w-20 rounded-lg" />
        </div>
      ))}
    </div>
  );
}

export function OutboxScreen() {
  const artifactsQuery = useOutboxArtifacts();
  const artifacts = artifactsQuery.data ?? [];
  const channels = useChannelsQuery().data ?? [];
  const channelById = React.useMemo(
    () => new Map(channels.map((channel) => [channel.id, channel])),
    [channels],
  );
  const authorPubkeys = React.useMemo(
    () => [...new Set(artifacts.map((artifact) => artifact.authorPubkey))],
    [artifacts],
  );
  const profiles = useUsersBatchQuery(authorPubkeys, {
    enabled: authorPubkeys.length > 0,
  }).data?.profiles;
  const { openSearchHit } = useAppNavigation();
  const [openingArtifactId, setOpeningArtifactId] = React.useState<
    string | null
  >(null);

  const handleOpenArtifact = React.useCallback(
    async (artifact: OutboxArtifact) => {
      setOpeningArtifactId(artifact.id);
      try {
        await openArtifactFile(artifact.url, artifact.filename);
      } catch (error) {
        toast.error("Could not open artifact", {
          description:
            error instanceof Error ? error.message : "Artifact open failed.",
        });
      } finally {
        setOpeningArtifactId((current) =>
          current === artifact.id ? null : current,
        );
      }
    },
    [],
  );

  const handleOpenSource = React.useCallback(
    (artifact: OutboxArtifact) => {
      if (!artifact.channelId) return;
      const channel = channelById.get(artifact.channelId);
      const threadRootId = getThreadReference(artifact.sourceTags).rootId;
      const hit: SearchHit = {
        eventId: artifact.eventId,
        content: artifact.sourceContent,
        kind: artifact.eventKind,
        pubkey: artifact.authorPubkey,
        channelId: artifact.channelId,
        channelName: channel?.name ?? null,
        createdAt: artifact.createdAt,
        score: 0,
        threadRootId,
      };
      void openSearchHit(hit);
    },
    [channelById, openSearchHit],
  );

  return (
    <section
      aria-labelledby="outbox-title"
      className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
      data-testid="outbox-screen"
    >
      <header className="shrink-0 border-b border-border/45 bg-background/85 px-5 py-4 backdrop-blur-md">
        <div className="mx-auto flex w-full max-w-4xl items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="mb-1 flex items-center gap-2 text-primary">
              <FileOutput className="h-4 w-4" />
              <span className="text-xs font-semibold uppercase tracking-[0.14em]">
                Delivered work
              </span>
            </div>
            <h1
              className="text-xl font-semibold tracking-tight text-foreground"
              id="outbox-title"
            >
              Outbox
            </h1>
            <p className="mt-1 text-sm text-muted-foreground">
              Files your agents finished and attached, newest first.
            </p>
          </div>
          <Button
            aria-label="Refresh Outbox"
            data-testid="refresh-outbox"
            disabled={artifactsQuery.isFetching}
            onClick={() => void artifactsQuery.refetch()}
            size="icon"
            type="button"
            variant="ghost"
          >
            <RefreshCcw
              className={cn(
                "h-4 w-4",
                artifactsQuery.isFetching && "animate-spin",
              )}
            />
          </Button>
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {artifactsQuery.isLoading ? <OutboxLoadingState /> : null}

        {!artifactsQuery.isLoading && artifactsQuery.isError ? (
          <div className="mx-auto flex h-full w-full max-w-xl flex-col items-center justify-center px-6 text-center">
            <div className="mb-4 flex h-12 w-12 items-center justify-center rounded-2xl bg-destructive/10 text-destructive">
              <FileOutput className="h-5 w-5" />
            </div>
            <h2 className="text-base font-semibold">Outbox could not load</h2>
            <p className="mt-1 text-sm text-muted-foreground">
              Check the community connection, then try again.
            </p>
            <Button
              className="mt-4"
              onClick={() => void artifactsQuery.refetch()}
              size="sm"
              type="button"
              variant="outline"
            >
              Try again
            </Button>
          </div>
        ) : null}

        {!artifactsQuery.isLoading &&
        !artifactsQuery.isError &&
        artifacts.length === 0 ? (
          <div className="mx-auto flex h-full w-full max-w-xl flex-col items-center justify-center px-6 text-center">
            <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-2xl border border-primary/20 bg-primary/10 text-primary shadow-xs">
              <FileOutput className="h-6 w-6" />
            </div>
            <h2 className="text-base font-semibold">Your Outbox is ready</h2>
            <p className="mt-1 max-w-md text-sm leading-6 text-muted-foreground">
              When an agent attaches a completed file to its result message, it
              will appear here automatically. No separate deliverables channel
              or folder hunt.
            </p>
          </div>
        ) : null}

        {!artifactsQuery.isLoading &&
        !artifactsQuery.isError &&
        artifacts.length > 0 ? (
          <div
            className="mx-auto w-full max-w-4xl space-y-3 px-5 py-6"
            data-testid="outbox-artifact-list"
          >
            {artifacts.map((artifact) => {
              const profile = profiles?.[artifact.authorPubkey];
              const agentLabel = resolveUserLabel({
                pubkey: artifact.authorPubkey,
                profiles,
              });
              const channel = artifact.channelId
                ? channelById.get(artifact.channelId)
                : undefined;
              const sizeLabel = formatArtifactSize(artifact.size);
              const opening = openingArtifactId === artifact.id;

              return (
                <article
                  className="group flex flex-col gap-4 rounded-2xl border border-border/60 bg-card/70 p-4 shadow-xs transition-colors hover:border-border hover:bg-card sm:flex-row sm:items-center"
                  data-testid="outbox-artifact"
                  key={artifact.id}
                >
                  <button
                    aria-label={`Open ${artifact.filename}`}
                    className="flex min-w-0 flex-1 items-center gap-4 rounded-xl text-left focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
                    disabled={opening}
                    onClick={() => void handleOpenArtifact(artifact)}
                    type="button"
                  >
                    <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-primary/15 bg-primary/10 text-primary transition-transform group-hover:-translate-y-0.5">
                      {opening ? (
                        <LoaderCircle className="h-5 w-5 animate-spin" />
                      ) : (
                        <ArtifactIcon artifact={artifact} />
                      )}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-semibold text-foreground">
                        {artifact.filename}
                      </span>
                      {artifact.sourceSummary ? (
                        <span className="mt-1 block line-clamp-1 text-sm text-muted-foreground">
                          {artifact.sourceSummary}
                        </span>
                      ) : null}
                      <span className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
                        <span className="inline-flex items-center gap-1.5">
                          <UserAvatar
                            avatarUrl={profile?.avatarUrl ?? null}
                            displayName={agentLabel}
                            fallbackDelayMs={0}
                            size="xs"
                          />
                          {agentLabel}
                        </span>
                        {channel ? <span>#{channel.name}</span> : null}
                        {sizeLabel ? <span>{sizeLabel}</span> : null}
                        <time
                          dateTime={new Date(
                            artifact.createdAt * 1_000,
                          ).toISOString()}
                        >
                          {formatArtifactTime(artifact.createdAt)}
                        </time>
                      </span>
                    </span>
                  </button>

                  <div className="flex shrink-0 items-center gap-2 pl-[3.75rem] sm:pl-0">
                    <Button
                      data-testid="open-outbox-artifact"
                      disabled={opening}
                      onClick={() => void handleOpenArtifact(artifact)}
                      size="sm"
                      type="button"
                    >
                      {opening ? "Opening" : "Open"}
                    </Button>
                    {artifact.channelId ? (
                      <Button
                        aria-label={`Open source conversation for ${artifact.filename}`}
                        data-testid="open-outbox-source"
                        onClick={() => handleOpenSource(artifact)}
                        size="icon"
                        title="Open source conversation"
                        type="button"
                        variant="ghost"
                      >
                        <ExternalLink className="h-4 w-4" />
                      </Button>
                    ) : null}
                  </div>
                </article>
              );
            })}
          </div>
        ) : null}
      </div>
    </section>
  );
}
