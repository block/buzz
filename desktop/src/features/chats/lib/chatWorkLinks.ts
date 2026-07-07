import { extractSupportedLinkPreviews } from "@/shared/lib/linkPreview";

export type ChatPullRequestSource = {
  href: string;
  timestampMs: number | null;
};

/**
 * Return the latest chat message that points to exactly one GitHub PR.
 * Status updates often mention several PRs; choosing the first/last link from
 * that kind of mixed message makes one chat's work panel drift to another PR.
 */
export function latestUnambiguousPullRequestHref(
  messages: readonly { content: string; created_at?: number }[],
): string | null {
  return latestUnambiguousPullRequestSource(messages)?.href ?? null;
}

export function latestUnambiguousPullRequestSource(
  messages: readonly { content: string; created_at?: number }[],
): ChatPullRequestSource | null {
  for (let index = messages.length - 1; index >= 0; index--) {
    const source = pullRequestSourceFromMessage(messages[index]);
    if (source) {
      return source;
    }
  }
  return null;
}

export function collectUnambiguousPullRequestSources(
  messages: readonly { content: string; created_at?: number }[],
): ChatPullRequestSource[] {
  return messages.flatMap((message) => {
    const source = pullRequestSourceFromMessage(message);
    return source ? [source] : [];
  });
}

function pullRequestSourceFromMessage(message: {
  content: string;
  created_at?: number;
}): ChatPullRequestSource | null {
  const hrefs = [
    ...new Set(
      extractSupportedLinkPreviews(message.content)
        .filter((candidate) => candidate.kind === "github-pull-request")
        .map((preview) => preview.href),
    ),
  ];
  if (hrefs.length !== 1) {
    return null;
  }
  return {
    href: hrefs[0],
    timestampMs:
      typeof message.created_at === "number"
        ? message.created_at * 1_000
        : null,
  };
}
