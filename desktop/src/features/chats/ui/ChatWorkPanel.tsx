import * as React from "react";
import {
  ChevronDown,
  CircleCheck,
  CircleDashed,
  CircleX,
  GitBranch,
  LoaderCircle,
  MessageSquareText,
} from "lucide-react";

import {
  updateChatWorkAutomation,
  useChatWorkAutomation,
} from "@/features/chats/lib/chatWorkAutomation";
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
import { Checkbox } from "@/shared/ui/checkbox";
import { GithubPullRequestCard } from "@/shared/ui/link-preview-attachment";

const CHIP_CLASS =
  "flex items-center gap-1.5 rounded-2xl border border-border/70 bg-muted/30 px-3 py-2.5 text-xs";

/**
 * Right-hand work drawer for a chat: branch, live PR card, and a CI monitor
 * once the agent has produced a pull request; an empty state before that.
 * The monitor expands to the individual check runs and the automation
 * toggles — when armed, CI failures and newly-open review threads prompt the
 * chat's agent automatically (deduped per head sha / open-thread watermark).
 */
export function ChatWorkPanel({
  branch = null,
  chatId,
  onAutomationPrompt,
  open = true,
  prHref,
  projectPath = null,
}: {
  /** Live branch from the agent's worktree/checkout activity, if any. */
  branch?: string | null;
  chatId: string;
  onAutomationPrompt?: (content: string) => void;
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
  const discoveredPrQuery = useGithubPrForBranchQuery(
    monitorActive && !prHref ? projectPath : null,
    branch,
  );
  const effectiveHref = prHref ?? discoveredPrQuery.data ?? null;
  const preview = effectiveHref
    ? parseSupportedLinkPreview(effectiveHref)
    : null;
  const parsedRef = effectiveHref
    ? parseGithubPullRequestRef(effectiveHref)
    : null;
  const ref = monitorActive ? parsedRef : null;
  const prQuery = useGithubPullRequestQuery(ref);
  const pr = prQuery.data ?? null;
  const checksQuery = useGithubCheckSummaryQuery(ref, pr?.headSha);
  const checks = checksQuery.data ?? null;
  const commentStateQuery = useGithubCommentStateQuery(ref);
  const openThreads = commentStateQuery.data?.openThreads ?? 0;
  // Live activity wins over the PR's head ref: the agent may have moved to a
  // new worktree since opening the PR, and activity updates immediately.
  const currentBranch = branch?.trim() || pr?.headRef?.trim() || null;

  // Automation: prompt the agent on CI failure / newly-open review threads.
  // Watermarks in storage keep this to one nudge per failing sha and per
  // rise in open threads; the thread watermark re-arms once everything has
  // been replied to (count back at zero).
  React.useEffect(() => {
    if (!onAutomationPrompt || !effectiveHref || !pr) {
      return;
    }
    if (
      automation.autoFixCi &&
      checks &&
      checks.failed > 0 &&
      checks.pending === 0 &&
      automation.lastCiNudgeSha !== pr.headSha
    ) {
      updateChatWorkAutomation(chatId, { lastCiNudgeSha: pr.headSha });
      onAutomationPrompt(
        `CI is failing on ${effectiveHref} (${checks.failed} of ${checks.total} checks). Investigate the failures and push fixes until the checks pass.`,
      );
    }
    const threadWatermark = automation.lastCommentNudgeCount ?? 0;
    if (openThreads === 0 && threadWatermark !== 0) {
      updateChatWorkAutomation(chatId, { lastCommentNudgeCount: 0 });
    } else if (automation.addressComments && openThreads > threadWatermark) {
      updateChatWorkAutomation(chatId, { lastCommentNudgeCount: openThreads });
      onAutomationPrompt(
        `There are unanswered review comments on ${effectiveHref}. Address each comment and its replies, push any needed changes, reply to the threads, and resolve every conversation that has been addressed.`,
      );
    }
  }, [
    automation,
    chatId,
    checks,
    effectiveHref,
    onAutomationPrompt,
    openThreads,
    pr,
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
              text={currentBranch ?? "No current branch"}
            />
          </div>
          {preview ? (
            // Keyed by href so a NEW pull request re-runs the pop-in, not
            // just the first one.
            <div
              className="buzz-work-card-in flex flex-col gap-2"
              key={preview.href}
            >
              <GithubPullRequestCard className="w-full" preview={preview} />
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
                  <label
                    className="flex cursor-pointer items-center gap-2"
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
                  <label
                    className="flex cursor-pointer items-center gap-2"
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
                </div>
              </details>
            </div>
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
