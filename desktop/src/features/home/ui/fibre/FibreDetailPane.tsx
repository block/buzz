import * as React from "react";
import { Info } from "lucide-react";
import { toast } from "sonner";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  fibreArtifactCountLabel,
  fibreSourceLabel,
  formatFibreAge,
  latestArtifact,
  primaryThreadTarget,
  resolveFibrePersonLabel,
} from "@/features/home/ui/fibre/fibreFormat";
import { fibreKindMeta } from "@/features/home/ui/fibre/fibreKinds";
import type { FibreListTab } from "@/features/home/ui/fibre/fibreSort";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { Fibre } from "@/features/triage/api";
import { sendChannelMessage } from "@/shared/api/tauri";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const SHORTCUT_KBD =
  "rounded bg-current/20 px-1 py-px text-2xs font-medium text-current opacity-80";

type FibreDetailPaneProps = {
  currentPubkey?: string;
  fibre: Fibre | null;
  listTab: FibreListTab;
  nowMs: number;
  profiles?: UserProfileLookup;
  onDone: (fibre: Fibre) => void;
  onDismiss: (fibre: Fibre) => void;
  onOpenContext: (
    channelId: string,
    messageId: string,
    threadRootId?: string | null,
  ) => void;
  onReopen: (fibre: Fibre) => void;
};

function ScoreMeter({ color, score }: { color: string; score: number }) {
  return (
    <span className="flex items-end gap-0.5" aria-hidden>
      {[0, 1, 2, 3, 4].map((step) => (
        <span
          className="w-1 rounded-sm"
          key={step}
          style={{
            height: `${0.3125 + step * 0.125}rem`,
            background:
              score >= (step + 1) * 20 ? color : "rgba(255,255,255,0.14)",
          }}
        />
      ))}
    </span>
  );
}

function WhyThisRanksCard({ fibre }: { fibre: Fibre }) {
  return (
    <div className="text-left">
      <div className="flex items-center gap-2 text-xs text-primary">
        <Info className="h-3.5 w-3.5" />
        Why this ranks here
      </div>
      {fibre.why ? (
        <p className="mt-2 text-sm leading-relaxed text-foreground/90">
          {fibre.why}
        </p>
      ) : (
        <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
          One message, no interpretation needed — it reads for itself below.
        </p>
      )}
      {fibre.signals.length > 0 ? (
        <div className="mt-3 flex flex-wrap gap-1.5">
          {fibre.signals.map((signal) => (
            <span
              className="flex items-center gap-1.5 rounded-md bg-background/40 px-2 py-0.5 text-2xs text-muted-foreground"
              key={`${signal.weight}-${signal.label}`}
            >
              <span className="tabular-nums text-primary">{signal.weight}</span>
              {signal.label}
            </span>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function PriorityWhyHint({ fibre }: { fibre: Fibre }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          aria-label="Why this ranks here"
          className="rounded-full p-0.5 text-muted-foreground hover:text-foreground"
          data-testid="fibre-why-trigger"
          type="button"
        >
          <Info className="h-3.5 w-3.5" />
        </button>
      </TooltipTrigger>
      <TooltipContent
        align="end"
        className="max-w-sm overflow-visible rounded-lg bg-popover px-4 py-3.5 text-popover-foreground shadow-md ring-1 ring-border/60"
        data-testid="fibre-why-card"
        side="bottom"
        sideOffset={8}
      >
        <WhyThisRanksCard fibre={fibre} />
      </TooltipContent>
    </Tooltip>
  );
}

export function FibreDetailPane({
  currentPubkey,
  fibre,
  listTab,
  nowMs,
  onDone,
  onDismiss,
  onOpenContext,
  onReopen,
  profiles,
}: FibreDetailPaneProps) {
  const [delegateOpen, setDelegateOpen] = React.useState(false);
  const agentsQuery = useManagedAgentsQuery({ enabled: delegateOpen });
  const agents = agentsQuery.data ?? [];

  // Reset local chrome when the selected fibre changes.
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset on fibre identity, not object identity
  React.useEffect(() => {
    setDelegateOpen(false);
  }, [fibre?.id]);

  React.useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      if (!(event.target instanceof HTMLElement)) return;
      const tag = event.target.tagName;
      if (
        tag === "INPUT" ||
        tag === "TEXTAREA" ||
        event.target.isContentEditable
      ) {
        return;
      }
      const key = event.key.toLowerCase();
      if (key === "d") {
        if (fibre?.status === "done") return;
        event.preventDefault();
        setDelegateOpen((open) => !open);
      } else if (key === "escape") {
        setDelegateOpen(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [fibre?.status]);

  const jump = React.useCallback(
    (targetFibre: Fibre) => {
      const target = primaryThreadTarget(targetFibre);
      if (!target) {
        toast.error("This fibre has no source thread to open");
        return;
      }
      onOpenContext(target.channelId, target.messageId, target.threadRootId);
    },
    [onOpenContext],
  );

  const handleDelegate = React.useCallback(
    async (agent: { pubkey: string; name: string }) => {
      if (!fibre) return;
      const target = primaryThreadTarget(fibre);
      if (!target) {
        toast.error("This fibre has no channel to reply in");
        return;
      }
      const others = fibre.people.filter(
        (person) =>
          person.pubkey !== currentPubkey && person.pubkey !== agent.pubkey,
      );
      const greeting =
        others.length > 0
          ? `Hey ${others
              .map((person) =>
                resolveFibrePersonLabel(person, { currentPubkey, profiles }),
              )
              .join(", ")}`
          : "Hey";
      const content = `${greeting}, I'm going to ask @${agent.name} to take a crack at this.`;
      const mentionPubkeys = [
        ...others.map((person) => person.pubkey),
        agent.pubkey,
      ];
      try {
        await sendChannelMessage(
          target.channelId,
          content,
          target.messageId,
          undefined,
          mentionPubkeys,
        );
        toast.success(`Delegated to ${agent.name}`);
        onDone(fibre);
        setDelegateOpen(false);
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Could not delegate",
        );
      }
    },
    [currentPubkey, fibre, onDone, profiles],
  );

  if (listTab === "done" && !fibre) {
    return (
      <div
        className="flex flex-1 items-center justify-center p-10"
        data-testid="fibre-done-empty"
      >
        <div className="inbox-zero-copy max-w-sm text-center">
          <div className="text-xl font-medium tracking-tight">
            Nothing completed yet
          </div>
          <p className="mt-2.5 text-sm leading-relaxed text-muted-foreground">
            Fibres you mark done will collect here so you can reopen them later.
          </p>
        </div>
      </div>
    );
  }

  if (!fibre) {
    return <div className="min-h-0 min-w-0 flex-1" data-testid="fibre-zero" />;
  }

  const kind = fibreKindMeta(fibre.kind);
  const age = formatFibreAge(
    latestArtifact(fibre.artifacts)?.createdAt ?? fibre.updatedAt,
    nowMs,
  );
  const source = fibreSourceLabel(fibre);

  return (
    <div
      className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
      data-testid="fibre-detail"
    >
      <div className="flex h-[3.25rem] shrink-0 items-center gap-3 border-b border-border/60 px-5">
        <div className="text-sm text-muted-foreground">
          Fibre in <span className="text-foreground">{source}</span>
        </div>
        <div className="ml-auto flex items-center gap-3 text-muted-foreground">
          <TooltipProvider delayDuration={200}>
            <span className="flex items-center gap-1.5 text-xs">
              Priority
              <PriorityWhyHint fibre={fibre} />
            </span>
          </TooltipProvider>
          <span
            className="text-sm font-medium tabular-nums"
            style={{ color: kind.color }}
          >
            {fibre.score}
          </span>
          <ScoreMeter color={kind.color} score={fibre.score} />
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5">
        <div className="max-w-xl">
          <div className="flex items-center gap-2">
            <span
              className="rounded-full px-2.5 py-0.5 text-xs font-medium"
              style={{ color: kind.color, background: kind.tint }}
            >
              {kind.label}
            </span>
            <span className="text-xs text-muted-foreground">
              {age} old · {fibreArtifactCountLabel(fibre.artifacts.length)}
            </span>
          </div>
          <h1 className="mt-3.5 text-2xl font-medium leading-snug tracking-tight">
            {fibre.title}
          </h1>
          {fibre.summary ? (
            <p className="mt-3 text-base leading-relaxed text-muted-foreground">
              {fibre.summary}
            </p>
          ) : null}

          <section className="mt-8" data-testid="fibre-artifacts">
            <header className="mb-3">
              <h2 className="text-sm font-medium text-foreground">
                Source artifacts
              </h2>
              <p className="mt-0.5 text-xs text-muted-foreground">
                {fibreArtifactCountLabel(fibre.artifacts.length)} · {source}
              </p>
            </header>
            {fibre.artifacts.map((artifact) => {
              const authorLabel = resolveFibrePersonLabel(
                {
                  pubkey: artifact.authorPubkey,
                  label: artifact.authorLabel,
                },
                { currentPubkey, profiles },
              );
              const authorProfile = artifact.authorPubkey
                ? profiles?.[normalizePubkey(artifact.authorPubkey)]
                : undefined;
              return (
                <div
                  className="grid grid-cols-[2.125rem_minmax(0,1fr)] gap-x-3 rounded-2xl px-2 py-2 transition-colors hover:bg-muted/50 focus-within:bg-muted/50"
                  key={artifact.eventId}
                >
                  <UserAvatar
                    avatarUrl={authorProfile?.avatarUrl ?? null}
                    className="h-[2.125rem] w-[2.125rem]"
                    displayName={authorLabel}
                    size="md"
                  />
                  <div className="min-w-0">
                    <div className="mb-0.5 flex items-center gap-2">
                      <span className="text-sm font-medium">{authorLabel}</span>
                      <span className="text-2xs text-muted-foreground">
                        {artifact.createdAt
                          ? new Date(
                              artifact.createdAt * 1000,
                            ).toLocaleTimeString("en-US", {
                              hour: "numeric",
                              minute: "2-digit",
                            })
                          : ""}
                      </span>
                      <span className="rounded-md bg-muted px-1.5 py-px text-2xs text-muted-foreground">
                        {artifact.isDm
                          ? "DM"
                          : artifact.channelName
                            ? `#${artifact.channelName}`
                            : source}
                      </span>
                      {artifact.channelId ? (
                        <button
                          className="ml-auto text-2xs text-primary hover:underline"
                          onClick={() =>
                            onOpenContext(
                              artifact.channelId as string,
                              artifact.eventId,
                              artifact.threadRootId,
                            )
                          }
                          type="button"
                        >
                          Jump to thread
                        </button>
                      ) : null}
                    </div>
                    <div className="text-sm leading-relaxed text-muted-foreground">
                      <Markdown
                        className="[&>*+*]:mt-1.5"
                        content={artifact.content}
                        interactive={false}
                        linkPreviewsSuppressed
                      />
                    </div>
                  </div>
                </div>
              );
            })}
          </section>
        </div>
      </div>

      <div className="relative flex shrink-0 items-center gap-2 border-t border-border/60 px-5 py-3">
        {fibre.status === "done" ? (
          <>
            <Button
              data-testid="fibre-reopen"
              onClick={() => onReopen(fibre)}
              size="sm"
              type="button"
            >
              Reopen
            </Button>
            <Button
              data-testid="fibre-reply"
              onClick={() => jump(fibre)}
              size="sm"
              type="button"
              variant="secondary"
            >
              Reply
              <kbd className={SHORTCUT_KBD}>R</kbd>
            </Button>
          </>
        ) : (
          <>
            <Button
              data-testid="fibre-done"
              onClick={() => onDone(fibre)}
              size="sm"
              type="button"
            >
              Done
              <kbd className={SHORTCUT_KBD} data-testid="fibre-done-kbd">
                E
              </kbd>
            </Button>
            <Button
              onClick={() => toast.message("Snooze isn't wired yet")}
              size="sm"
              type="button"
              variant="secondary"
            >
              Snooze
              <kbd className={SHORTCUT_KBD}>H</kbd>
            </Button>
            <Button
              data-testid="fibre-reply"
              onClick={() => jump(fibre)}
              size="sm"
              type="button"
              variant="secondary"
            >
              Reply
              <kbd className={SHORTCUT_KBD}>R</kbd>
            </Button>
            <Button
              data-testid="fibre-delegate"
              onClick={() => setDelegateOpen((open) => !open)}
              size="sm"
              type="button"
              variant="secondary"
            >
              Delegate to agent
              <kbd className={SHORTCUT_KBD}>D</kbd>
            </Button>
            <Button
              className="ml-auto"
              data-testid="fibre-dismiss"
              onClick={() => onDismiss(fibre)}
              size="sm"
              title="Dismiss (X). Shift+X to dismiss and say why."
              type="button"
              variant="ghost"
            >
              Not a fibre
              <kbd className={SHORTCUT_KBD}>X</kbd>
            </Button>
          </>
        )}

        {delegateOpen ? (
          <div
            className="absolute bottom-14 right-5 w-72 rounded-lg bg-popover p-2 shadow-lg ring-1 ring-border"
            data-testid="fibre-delegate-menu"
          >
            <div className="px-2.5 pb-2 pt-1.5 text-xs text-muted-foreground">
              Hand this fibre to
            </div>
            {agents.length === 0 ? (
              <div className="px-2.5 py-3 text-sm text-muted-foreground">
                No agents available.
              </div>
            ) : (
              agents.map((agent) => (
                <button
                  className="grid w-full grid-cols-[1.875rem_minmax(0,1fr)] items-center gap-x-2.5 rounded-md px-2.5 py-2 text-left hover:bg-muted"
                  key={agent.pubkey}
                  onClick={() => {
                    void handleDelegate(agent);
                  }}
                  type="button"
                >
                  <UserAvatar
                    avatarUrl={null}
                    className="h-[1.875rem] w-[1.875rem] rounded-md"
                    displayName={agent.name}
                    size="sm"
                  />
                  <span className="min-w-0">
                    <span className="block text-sm text-foreground">
                      {agent.name}
                    </span>
                    <span className="block text-2xs text-muted-foreground">
                      {agent.runtime ?? "Agent"}
                    </span>
                  </span>
                </button>
              ))
            )}
          </div>
        ) : null}
      </div>
    </div>
  );
}
