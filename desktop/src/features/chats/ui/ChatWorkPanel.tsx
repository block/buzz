import * as React from "react";
import { toast } from "sonner";
import {
  ChevronDown,
  CircleCheck,
  CircleDashed,
  CircleX,
  GitBranch,
  LoaderCircle,
  MessageSquareText,
  Pin,
} from "lucide-react";

import {
  CHAT_PR_UNPINNED,
  clearChatPinnedPr,
  type ChatPinnedPr,
  readChatPinnedPr,
  updateChatWorkAutomation,
  useChatWorkAutomation,
  writeChatPinnedPr,
} from "@/features/chats/lib/chatWorkAutomation";
import {
  clearChatWorkBindingPr,
  updateChatWorkBinding,
} from "@/features/chats/lib/chatWorkBinding";
import {
  type GithubCheckSummary,
  parseGithubPullRequestRef,
  useGithubCheckSummaryQuery,
  useGithubCommentStateQuery,
  useGithubPrForBranchQuery,
  useGithubPullRequestQuery,
} from "@/shared/lib/githubPullRequest";
import { parseSupportedLinkPreview } from "@/shared/lib/linkPreview";
import { cn } from "@/shared/lib/cn";
import { AnimatedTitleText } from "@/shared/ui/animated-title-text";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Checkbox } from "@/shared/ui/checkbox";
import { GithubPullRequestCard } from "@/shared/ui/link-preview-attachment";

const CHIP_CLASS =
  "flex items-center gap-1.5 rounded-2xl border border-border/70 bg-muted/30 px-3 py-2.5 text-xs";

// Last branch/PR the panel showed per chat. Async sources (the PR query)
// resolve after mount, and without this the chip replays its swap animation
// and the card replays its pop-in on every visit to the chat — the
// animations should mark NEW information, not navigation.
const lastShownBranchByChat = new Map<string, string>();
const lastShownPrByChat = new Map<string, string>();

// A consumed watermark must not strand a persisting condition: if CI is
// still red (or comments still open) and NO turn is running, armed
// automation re-nudges after this cooldown — the earlier nudge may have
// landed while the agent was stopped or the message failed to take.
const RENUDGE_COOLDOWN_MS = 15 * 60_000;

type PinnedPrState = {
  chatId: string;
  pinned: ChatPinnedPr | null;
};

/**
 * Right-hand work drawer for a chat: branch, live PR card, and a CI monitor
 * once the agent has produced a pull request; an empty state before that.
 * The monitor expands to the individual check runs and the automation
 * toggles — when armed, CI failures and newly-open review threads prompt the
 * chat's agent automatically (deduped per head sha / open-thread watermark).
 */
export function ChatWorkPanel({
  agentName = "Fizz",
  branch = null,
  chatId,
  isTurnActive = false,
  onAutomationPrompt,
  open = true,
  prHref,
  projectPath = null,
}: {
  /** Default agent's display name — named in the automation feedback. */
  agentName?: string;
  /** Live branch from the agent's worktree/checkout activity, if any. */
  branch?: string | null;
  chatId: string;
  /** Whether an agent turn is currently running in this chat. */
  isTurnActive?: boolean;
  onAutomationPrompt?: (content: string, kind: "ci" | "comments") => void;
  open?: boolean;
  prHref?: string | null;
  /** Project directory — enables PR discovery by branch via the git remote. */
  projectPath?: string | null;
}) {
  // No PR link in the chat yet? Discover it from the branch via the
  // project's git remote (agents don't always paste the URL). All GitHub
  // polling pauses while the drawer is closed — a hidden panel burning the
  // API rate limit is how the monitor went stale-empty.
  const automation = useChatWorkAutomation(chatId);
  // Armed automation keeps watching with the drawer closed; otherwise a
  // hidden panel stops polling entirely.
  const monitorActive =
    open || automation.autoFixCi || automation.addressComments;
  // Pin resolution order: a MANUAL pin outranks everything (the user said
  // "this is the PR"), then a link posted in THIS chat, then the remembered
  // auto pin, then branch discovery — discovery alone is ambiguous when
  // agents reuse a worktree across chats in one project.
  const [pinnedState, setPinnedState] = React.useState<PinnedPrState>(() => ({
    chatId,
    pinned: readChatPinnedPr(chatId),
  }));
  const pinned =
    pinnedState.chatId === chatId
      ? pinnedState.pinned
      : readChatPinnedPr(chatId);
  React.useEffect(() => {
    setPinnedState({ chatId, pinned: readChatPinnedPr(chatId) });
  }, [chatId]);
  // The empty-string sentinel means "user unlinked — no PR for this chat":
  // discovery stays off, posted links still win.
  const isUnpinned = pinned?.href === CHAT_PR_UNPINNED && pinned.manual;
  const manualHref = pinned?.manual && pinned.href ? pinned.href : null;
  const autoPinnedHref = !pinned?.manual && pinned?.href ? pinned.href : null;
  const discoveredPrQuery = useGithubPrForBranchQuery(
    monitorActive && !prHref && !manualHref && !autoPinnedHref && !isUnpinned
      ? projectPath
      : null,
    branch,
  );
  const effectiveHref =
    manualHref ??
    prHref ??
    autoPinnedHref ??
    (isUnpinned ? null : (discoveredPrQuery.data ?? null));
  React.useEffect(() => {
    // Remember PR links posted in this chat, but never persist branch
    // discovery: discovery is useful as a live hint and too ambiguous to
    // become durable chat ownership.
    const current = readChatPinnedPr(chatId);
    if (current?.manual) {
      return;
    }
    if (prHref && prHref !== current?.href) {
      writeChatPinnedPr(chatId, prHref);
      updateChatWorkBinding(
        chatId,
        { prHref, prDetached: false },
        { replacePr: false },
      );
      setPinnedState({ chatId, pinned: { href: prHref, manual: false } });
    }
  }, [chatId, prHref]);
  const handleUnlinkPr = React.useCallback(() => {
    writeChatPinnedPr(chatId, CHAT_PR_UNPINNED, true);
    updateChatWorkBinding(
      chatId,
      { prHref: null, prDetached: true },
      { replacePr: true },
    );
    setPinnedState({
      chatId,
      pinned: { href: CHAT_PR_UNPINNED, manual: true },
    });
  }, [chatId]);
  const [isPinEditorOpen, setIsPinEditorOpen] = React.useState(false);
  const [pinInput, setPinInput] = React.useState("");
  const handlePinSubmit = React.useCallback(() => {
    const trimmed = pinInput.trim();
    if (!parseGithubPullRequestRef(trimmed)) {
      toast.error("Enter a full GitHub pull request URL");
      return;
    }
    writeChatPinnedPr(chatId, trimmed, true);
    updateChatWorkBinding(
      chatId,
      { prHref: trimmed, prDetached: false },
      { replacePr: true },
    );
    setPinnedState({ chatId, pinned: { href: trimmed, manual: true } });
    setIsPinEditorOpen(false);
    setPinInput("");
  }, [chatId, pinInput]);
  const preview = effectiveHref
    ? parseSupportedLinkPreview(effectiveHref)
    : null;
  const parsedRef = effectiveHref
    ? parseGithubPullRequestRef(effectiveHref)
    : null;
  const ref = monitorActive ? parsedRef : null;
  const prQuery = useGithubPullRequestQuery(ref);
  const pr = prQuery.data ?? null;
  React.useEffect(() => {
    const pinnedHref = !pinned?.manual && pinned?.href ? pinned.href : null;
    const normalizedBranch = branch?.trim();
    const prHeadRef = pr?.headRef?.trim();
    if (
      !pinnedHref ||
      effectiveHref !== pinnedHref ||
      !normalizedBranch ||
      !prHeadRef ||
      prHeadRef === normalizedBranch
    ) {
      return;
    }

    clearChatPinnedPr(chatId);
    clearChatWorkBindingPr(chatId, pinnedHref);
    setPinnedState({ chatId, pinned: null });
  }, [branch, chatId, effectiveHref, pinned, pr?.headRef]);
  const checksQuery = useGithubCheckSummaryQuery(ref, pr?.headSha);
  const checks = checksQuery.data ?? null;
  const commentStateQuery = useGithubCommentStateQuery(ref);
  const openThreads = commentStateQuery.data?.openThreads ?? 0;
  // Live activity wins over the PR's head ref: the agent may have moved to a
  // new worktree since opening the PR, and activity updates immediately.
  // While async sources are still resolving, fall back to what this chat
  // last showed so a revisit renders the branch statically from the first
  // frame instead of animating in from the placeholder.
  const resolvedBranch = branch?.trim() || pr?.headRef?.trim() || null;
  const currentBranch =
    resolvedBranch ?? lastShownBranchByChat.get(chatId) ?? null;
  React.useEffect(() => {
    if (resolvedBranch) {
      lastShownBranchByChat.set(chatId, resolvedBranch);
    }
  }, [chatId, resolvedBranch]);
  // Latched per href for this mount: the effect below records the href as
  // seen immediately, and un-latching would strip the class mid-animation.
  const prEntranceDecisions = React.useRef(new Map<string, boolean>());
  let isNewPrForChat = false;
  if (preview) {
    const latched = prEntranceDecisions.current.get(preview.href);
    if (latched === undefined) {
      isNewPrForChat = lastShownPrByChat.get(chatId) !== preview.href;
      prEntranceDecisions.current.set(preview.href, isNewPrForChat);
    } else {
      isNewPrForChat = latched;
    }
  }
  React.useEffect(() => {
    if (preview?.href) {
      lastShownPrByChat.set(chatId, preview.href);
    }
  }, [chatId, preview?.href]);

  const ciConditionActive = Boolean(
    checks && checks.failed > 0 && checks.pending === 0,
  );
  const sendCiNudge = React.useCallback(() => {
    if (!onAutomationPrompt || !effectiveHref || !pr || !checks) {
      return;
    }
    updateChatWorkAutomation(chatId, {
      lastCiNudgeSha: pr.headSha,
      lastCiNudgeAt: Date.now(),
    });
    onAutomationPrompt(
      `CI is failing on ${effectiveHref} (${checks.failed} of ${checks.total} checks). Investigate the failures and push fixes until the checks pass.`,
      "ci",
    );
    // The prompt itself is invisible in the timeline — acknowledge it.
    toast.success(`Asked ${agentName} to fix the CI failures`);
  }, [agentName, chatId, checks, effectiveHref, onAutomationPrompt, pr]);
  const sendCommentNudge = React.useCallback(() => {
    if (!onAutomationPrompt || !effectiveHref) {
      return;
    }
    updateChatWorkAutomation(chatId, {
      lastCommentNudgeCount: Math.max(openThreads, 1),
      lastCommentNudgeAt: Date.now(),
    });
    onAutomationPrompt(
      `There are unanswered review comments on ${effectiveHref}. Address each comment and its replies, push any needed changes, reply to the threads, and resolve every conversation that has been addressed.`,
      "comments",
    );
    toast.success(`Asked ${agentName} to address the review comments`);
  }, [agentName, chatId, effectiveHref, onAutomationPrompt, openThreads]);

  // Automation: prompt the agent on CI failure / newly-open review threads.
  // Watermarks keep this to one nudge per failing sha and per rise in open
  // threads (the thread watermark re-arms at zero); the cooldown path above
  // covers conditions that persist with no agent working.
  React.useEffect(() => {
    if (!onAutomationPrompt || !effectiveHref || !pr) {
      return;
    }
    if (automation.autoFixCi && ciConditionActive && checks) {
      const isNewFailure = automation.lastCiNudgeSha !== pr.headSha;
      const cooledDown =
        !isTurnActive &&
        Date.now() - (automation.lastCiNudgeAt ?? 0) > RENUDGE_COOLDOWN_MS;
      if (isNewFailure || cooledDown) {
        sendCiNudge();
      }
    }
    const threadWatermark = automation.lastCommentNudgeCount ?? 0;
    if (openThreads === 0 && threadWatermark !== 0) {
      updateChatWorkAutomation(chatId, { lastCommentNudgeCount: 0 });
    } else if (automation.addressComments && openThreads > 0) {
      const isNewComment = openThreads > threadWatermark;
      const cooledDown =
        !isTurnActive &&
        Date.now() - (automation.lastCommentNudgeAt ?? 0) > RENUDGE_COOLDOWN_MS;
      if (isNewComment || cooledDown) {
        sendCommentNudge();
      }
    }
  }, [
    automation,
    chatId,
    checks,
    ciConditionActive,
    effectiveHref,
    isTurnActive,
    onAutomationPrompt,
    openThreads,
    pr,
    sendCiNudge,
    sendCommentNudge,
  ]);

  return (
    <aside
      aria-hidden={!open}
      className={cn(
        "flex shrink-0 flex-col overflow-hidden transition-[width,opacity] duration-300 ease-out",
        open ? "w-96 opacity-100" : "pointer-events-none w-0 opacity-0",
      )}
      data-testid="chat-work-panel"
    >
      {/* Fixed-width inner wrapper so content never reflows mid-slide. */}
      <div className="w-96 overflow-y-auto py-4 pl-1 pr-4">
        <div className="flex flex-col gap-2">
          <div className={CHIP_CLASS}>
            <GitBranch className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            {/* Swaps with the title animation when the agent moves to a new
                worktree/branch (or the placeholder resolves to a branch). */}
            <AnimatedTitleText
              className={cn(
                "min-w-0",
                currentBranch ? "font-mono" : "text-muted-foreground",
              )}
              // Keyed by chat: switching chats remounts the text statically
              // (first render never animates) — only an in-place branch
              // change for THIS chat plays the swap.
              key={chatId}
              text={currentBranch ?? "No current branch"}
            />
          </div>
          {preview ? (
            // Keyed by href so a NEW pull request re-runs the pop-in, not
            // just the first one.
            <div
              className={cn(
                "flex flex-col gap-2",
                isNewPrForChat && "buzz-work-card-in",
              )}
              key={preview.href}
            >
              <GithubPullRequestCard className="w-full" preview={preview} />
              {manualHref || !prHref ? (
                <div className="flex items-center justify-end gap-2 text-2xs text-muted-foreground/70">
                  <button
                    className="hover:text-foreground hover:underline"
                    data-testid="chat-work-change-pr"
                    onClick={() => setIsPinEditorOpen(true)}
                    type="button"
                  >
                    Not this chat's PR? Change
                  </button>
                  <span aria-hidden="true">·</span>
                  <button
                    className="hover:text-foreground hover:underline"
                    data-testid="chat-work-unlink-pr"
                    onClick={handleUnlinkPr}
                    type="button"
                  >
                    Unlink
                  </button>
                </div>
              ) : null}
              <details
                className={cn(CHIP_CLASS, "group/ci block p-0")}
                data-testid="chat-ci-monitor"
              >
                <summary className="flex cursor-pointer select-none list-none items-center gap-1.5 px-3 py-2.5 [&::-webkit-details-marker]:hidden">
                  <CiStatus checks={checks} />
                  <span
                    aria-hidden="true"
                    className="mx-0.5 text-muted-foreground/50"
                  >
                    ·
                  </span>
                  <MessageSquareText className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  <span className="min-w-0 truncate text-muted-foreground">
                    {openThreads} open comment{openThreads === 1 ? "" : "s"}
                  </span>
                  <ChevronDown className="ml-auto h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform group-open/ci:rotate-180" />
                </summary>
                <div className="flex flex-col gap-2.5 border-t border-border/40 px-3 py-2.5">
                  {checks && checks.runs.length > 0 ? (
                    <ul className="flex flex-col gap-1.5">
                      {dedupeRunKeys(checks.runs).map(({ key, run }) => (
                        <li className="flex items-center gap-1.5" key={key}>
                          <CheckRunIcon state={run.state} />
                          <span className="min-w-0 truncate text-muted-foreground">
                            {run.name}
                          </span>
                        </li>
                      ))}
                    </ul>
                  ) : (
                    <span className="text-muted-foreground">
                      No checks reported yet.
                    </span>
                  )}
                  <div aria-hidden="true" className="h-px bg-border/40" />
                  <div className="flex items-center gap-2">
                    <label
                      className="flex min-w-0 flex-1 cursor-pointer items-center gap-2"
                      htmlFor="automation-auto-fix-ci"
                    >
                      <Checkbox
                        checked={automation.autoFixCi}
                        data-testid="automation-auto-fix-ci"
                        id="automation-auto-fix-ci"
                        onCheckedChange={(checked) =>
                          updateChatWorkAutomation(chatId, {
                            autoFixCi: checked === true,
                          })
                        }
                      />
                      <span>Auto-fix CI failures</span>
                    </label>
                    {ciConditionActive && !isTurnActive ? (
                      <button
                        className="shrink-0 font-medium text-primary hover:underline"
                        data-testid="automation-run-ci-now"
                        onClick={sendCiNudge}
                        type="button"
                      >
                        Run now
                      </button>
                    ) : null}
                  </div>
                  <div className="flex items-center gap-2">
                    <label
                      className="flex min-w-0 flex-1 cursor-pointer items-center gap-2"
                      htmlFor="automation-address-comments"
                    >
                      <Checkbox
                        checked={automation.addressComments}
                        data-testid="automation-address-comments"
                        id="automation-address-comments"
                        onCheckedChange={(checked) =>
                          updateChatWorkAutomation(chatId, {
                            addressComments: checked === true,
                          })
                        }
                      />
                      <span>Address comments & resolve</span>
                    </label>
                    {openThreads > 0 && !isTurnActive ? (
                      <button
                        className="shrink-0 font-medium text-primary hover:underline"
                        data-testid="automation-run-comments-now"
                        onClick={sendCommentNudge}
                        type="button"
                      >
                        Run now
                      </button>
                    ) : null}
                  </div>
                </div>
              </details>
            </div>
          ) : null}
          {isPinEditorOpen ? (
            <form
              className="flex items-center gap-1.5"
              onSubmit={(event) => {
                event.preventDefault();
                handlePinSubmit();
              }}
            >
              <Input
                autoFocus
                className="h-8 flex-1 text-xs"
                data-testid="chat-work-pin-input"
                onChange={(event) => setPinInput(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    setIsPinEditorOpen(false);
                    setPinInput("");
                  }
                }}
                placeholder="https://github.com/owner/repo/pull/123"
                value={pinInput}
              />
              <Button size="sm" type="submit" variant="secondary">
                Pin
              </Button>
            </form>
          ) : !preview ? (
            <button
              className={cn(
                CHIP_CLASS,
                "cursor-pointer text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground",
              )}
              data-testid="chat-work-pin-pr"
              onClick={() => setIsPinEditorOpen(true)}
              type="button"
            >
              <Pin className="h-3.5 w-3.5 shrink-0" />
              Pin a pull request…
            </button>
          ) : null}
        </div>
      </div>
    </aside>
  );
}

/** Matrix jobs can repeat a check name; suffix repeats for stable keys. */
function dedupeRunKeys(runs: GithubCheckSummary["runs"]) {
  const seen = new Map<string, number>();
  return runs.map((run) => {
    const count = seen.get(run.name) ?? 0;
    seen.set(run.name, count + 1);
    return { key: count === 0 ? run.name : `${run.name} (${count})`, run };
  });
}

function CheckRunIcon({ state }: { state: "pending" | "success" | "failure" }) {
  if (state === "pending") {
    return (
      <LoaderCircle className="sprout-arc-spinner h-3.5 w-3.5 shrink-0 text-muted-foreground" />
    );
  }
  if (state === "failure") {
    return (
      <CircleX className="h-3.5 w-3.5 shrink-0 text-[color:var(--status-deleted)]" />
    );
  }
  return (
    <CircleCheck className="h-3.5 w-3.5 shrink-0 text-[color:var(--status-added)]" />
  );
}

function CiStatus({ checks }: { checks: GithubCheckSummary | null }) {
  if (!checks || checks.total === 0) {
    return (
      <>
        <CircleDashed className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="text-muted-foreground">No checks</span>
      </>
    );
  }
  if (checks.pending > 0) {
    return (
      <>
        <LoaderCircle className="sprout-arc-spinner h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        <span className="text-muted-foreground">
          CI running ({checks.total - checks.pending}/{checks.total})
        </span>
      </>
    );
  }
  if (checks.failed > 0) {
    return (
      <>
        <CircleX className="h-3.5 w-3.5 shrink-0 text-[color:var(--status-deleted)]" />
        <span className="font-medium text-[color:var(--status-deleted)]">
          CI failing ({checks.failed})
        </span>
      </>
    );
  }
  return (
    <>
      <CircleCheck className="h-3.5 w-3.5 shrink-0 text-[color:var(--status-added)]" />
      <span className="font-medium text-[color:var(--status-added)]">
        CI passing
      </span>
    </>
  );
}
