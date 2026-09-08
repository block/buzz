import { parse } from "yaml";

export type ChannelBackedTask = {
  task: { title: string; description: string };
  parentChannel: string;
  originatingThread: string;
  branch: { repository: string; name: string } | null;
};

function string(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

/** Read the experimental channel-backed task contract from canvas frontmatter. */
export function parseChannelBackedTask(
  content: string | null | undefined,
): ChannelBackedTask | null {
  const frontmatter = content?.match(
    /^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/,
  )?.[1];
  if (!frontmatter) return null;

  let value: unknown;
  try {
    value = parse(frontmatter);
  } catch {
    return null;
  }
  if (!value || typeof value !== "object") return null;

  const record = value as Record<string, unknown>;
  if (record.buzz_schema !== "channel-backed-task/v1") return null;
  if (!record.task || typeof record.task !== "object") return null;
  const task = record.task as Record<string, unknown>;
  const title = string(task.title);
  const description = string(task.description);
  const parentChannel = string(record.parent_channel);
  const originatingThread = string(record.originating_thread);
  if (!title || !description || !parentChannel || !originatingThread) {
    return null;
  }

  if (record.branch == null) {
    return {
      task: { title, description },
      parentChannel,
      originatingThread,
      branch: null,
    };
  }
  if (typeof record.branch !== "object") return null;
  const branch = record.branch as Record<string, unknown>;
  const repository = string(branch.repository);
  const name = string(branch.name);
  if (!repository || !name) return null;

  return {
    task: { title, description },
    parentChannel,
    originatingThread,
    branch: { repository, name },
  };
}

export function githubRepositoryUrl(repository: string): string | null {
  try {
    const url = new URL(repository);
    if (url.protocol !== "https:" || url.hostname !== "github.com") return null;
    const path = url.pathname.replace(/\.git$/, "").replace(/\/$/, "");
    return path.split("/").filter(Boolean).length === 2
      ? `https://github.com${path}`
      : null;
  } catch {
    return null;
  }
}
