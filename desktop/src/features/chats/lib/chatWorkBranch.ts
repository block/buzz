import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";
import { getToolString } from "@/features/agents/ui/agentSessionUtils";

/**
 * Derive the branch the chat's agent is currently working on from its shell
 * activity: the latest `git worktree add` / `git checkout` / `git switch`
 * that names a branch wins. This is what lets the work panel show a branch
 * as soon as the agent sets up a worktree — before any PR exists to report
 * `head.ref`.
 */
export function deriveChatWorkBranch(
  transcript: readonly TranscriptItem[],
): string | null {
  return deriveChatWorkBranchSource(transcript)?.branch ?? null;
}

export type ChatWorkBranchSource = {
  branch: string;
  timestampMs: number | null;
};

export function collectChatWorkBranchSources(
  transcript: readonly TranscriptItem[],
): ChatWorkBranchSource[] {
  const sources: ChatWorkBranchSource[] = [];
  for (const item of transcript) {
    if (item.type !== "tool") {
      continue;
    }
    const command = getToolString(item.args, ["command"]);
    if (!command) {
      continue;
    }
    const parsed = parseBranchFromCommand(command);
    if (parsed) {
      sources.push({
        branch: parsed,
        timestampMs: timestampToMs(item.timestamp),
      });
    }
  }
  return sources;
}

export function deriveChatWorkBranchSource(
  transcript: readonly TranscriptItem[],
): ChatWorkBranchSource | null {
  const sources = collectChatWorkBranchSources(transcript);
  return sources[sources.length - 1] ?? null;
}

/**
 * Fallback when tool activity predates the observer subscription (frames are
 * ephemeral): the agent's persisted chat messages usually announce the
 * branch — "Created a new worktree at … on branch kennylopez-dictation".
 * Parses "on branch <name>" / "branch `<name>`" phrasing plus any quoted git
 * commands in the text; the last mention across the messages wins.
 */
export function deriveBranchFromAgentMessages(
  messages: readonly { pubkey: string; content: string; created_at?: number }[],
  agentPubkey: string | null | undefined,
): string | null {
  return (
    deriveBranchSourceFromAgentMessages(messages, agentPubkey)?.branch ?? null
  );
}

export function collectBranchSourcesFromAgentMessages(
  messages: readonly { pubkey: string; content: string; created_at?: number }[],
  agentPubkey: string | null | undefined,
): ChatWorkBranchSource[] {
  if (!agentPubkey) {
    return [];
  }
  const sources: ChatWorkBranchSource[] = [];
  for (const message of messages) {
    if (message.pubkey !== agentPubkey) {
      continue;
    }
    const parsed =
      parseBranchFromCommand(message.content) ??
      parseSingleBranchFromProse(message.content);
    if (parsed) {
      sources.push({
        branch: parsed,
        timestampMs:
          typeof message.created_at === "number"
            ? message.created_at * 1_000
            : null,
      });
    }
  }
  return sources;
}

export function deriveBranchSourceFromAgentMessages(
  messages: readonly { pubkey: string; content: string; created_at?: number }[],
  agentPubkey: string | null | undefined,
): ChatWorkBranchSource | null {
  const sources = collectBranchSourcesFromAgentMessages(messages, agentPubkey);
  return sources[sources.length - 1] ?? null;
}

const PROSE_BRANCH_PATTERNS = [
  /\bon (?:the )?branch\s+[`'"]?([A-Za-z0-9._/-]+?)[`'".,)]*(?:\s|$)/i,
  /\bbranch\s+[`'"]([A-Za-z0-9._/-]+)[`'"]/i,
];

function parseSingleBranchFromProse(text: string): string | null {
  const branches: string[] = [];
  for (const pattern of PROSE_BRANCH_PATTERNS) {
    const flags = pattern.flags.includes("g")
      ? pattern.flags
      : `${pattern.flags}g`;
    const global = new RegExp(pattern.source, flags);
    for (const match of text.matchAll(global)) {
      const candidate = match[1];
      if (candidate && !looksLikeSha(candidate)) {
        branches.push(candidate);
      }
    }
  }

  const uniqueBranches = [...new Set(branches)];
  return uniqueBranches.length === 1 ? uniqueBranches[0] : null;
}

/** Latest branch named by any segment of a (possibly compound) command. */
export function parseBranchFromCommand(command: string): string | null {
  let branch: string | null = null;
  for (const segment of command.split(/&&|\|\||;|\n/)) {
    const parsed = parseBranchFromSegment(segment.trim());
    if (parsed) {
      branch = parsed;
    }
  }
  return branch;
}

function parseBranchFromSegment(segment: string): string | null {
  const tokens = tokenize(segment);
  const gitIndex = tokens.indexOf("git");
  if (gitIndex === -1) {
    return null;
  }
  // Skip `git -C <path> …` style global flags between `git` and the verb.
  let verbIndex = gitIndex + 1;
  while (verbIndex < tokens.length && tokens[verbIndex].startsWith("-")) {
    verbIndex += tokens[verbIndex] === "-C" ? 2 : 1;
  }
  const verb = tokens[verbIndex];
  const rest = tokens.slice(verbIndex + 1);

  if (verb === "worktree" && rest[0] === "add") {
    return parseWorktreeAdd(rest.slice(1));
  }
  if (verb === "checkout" || verb === "switch") {
    return parseCheckoutOrSwitch(rest);
  }
  return null;
}

function parseWorktreeAdd(args: string[]): string | null {
  const flagged = valueOfFlag(args, ["-b", "-B"]);
  if (flagged) {
    return flagged;
  }
  const positional = args.filter((token) => !token.startsWith("-"));
  if (positional.length >= 2) {
    // `git worktree add <path> <branch>` — a commit-ish second arg (sha)
    // isn't a branch name worth showing.
    return isPlausibleBranchName(positional[1]) ? positional[1] : null;
  }
  if (positional.length === 1) {
    // `git worktree add <path>` creates a branch named after the basename.
    const parts = positional[0].split("/").filter(Boolean);
    const basename = parts[parts.length - 1] ?? "";
    return isPlausibleBranchName(basename) ? basename : null;
  }
  return null;
}

function parseCheckoutOrSwitch(args: string[]): string | null {
  const flagged = valueOfFlag(args, ["-b", "-B", "-c", "-C"]);
  if (flagged) {
    return flagged;
  }
  // Plain `git checkout <branch>` / `git switch <branch>`: only a single
  // non-flag argument reads as a branch move — more args means file paths,
  // and `--` restores files, and a bare sha is a detached head.
  if (args.includes("--")) {
    return null;
  }
  const positional = args.filter((token) => !token.startsWith("-"));
  if (positional.length !== 1 || !isPlausibleBranchName(positional[0])) {
    return null;
  }
  return positional[0];
}

function valueOfFlag(args: string[], flags: string[]): string | null {
  for (let index = 0; index < args.length; index += 1) {
    if (flags.includes(args[index])) {
      const value = args[index + 1];
      if (value && isPlausibleBranchName(value)) {
        return value;
      }
    }
  }
  return null;
}

/**
 * Real git ref charset, no placeholders: template text like
 * `git checkout -b <branch>` in an explanatory message must never surface
 * "<branch>" as the chip's branch.
 */
function isPlausibleBranchName(token: string): boolean {
  return /^[A-Za-z0-9][A-Za-z0-9._/-]*$/.test(token) && !looksLikeSha(token);
}

function looksLikeSha(token: string): boolean {
  return /^[0-9a-f]{7,40}$/i.test(token);
}

function timestampToMs(timestamp: string | undefined): number | null {
  if (!timestamp) {
    return null;
  }
  const ms = Date.parse(timestamp);
  return Number.isFinite(ms) ? ms : null;
}

function tokenize(segment: string): string[] {
  return segment
    .split(/\s+/)
    .map((token) => token.replace(/^[`'"]+|[`'".,]+$/g, ""))
    .filter((token) => token.length > 0);
}
