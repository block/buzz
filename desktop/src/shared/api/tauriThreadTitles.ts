import { invokeTauri } from "@/shared/api/tauri";

/** A shared thread title (NIP-AR artifact, type `buzz.thread-title`). */
export type ThreadTitle = {
  title: string;
  channelId: string;
  rootId: string;
  revision: string;
  author: string;
  updatedAt: number;
  /** Newest of the last reply and the title revision, in unix seconds. */
  lastActivityAt: number;
};

type RawThreadTitle = {
  title: string;
  channel: string;
  root: string;
  revision: string;
  author: string;
  updated_at: number;
  last_activity_at: number;
};

/** Every live thread title the user can read, most recently active first. */
export async function listThreadTitles(): Promise<ThreadTitle[]> {
  const raw = await invokeTauri<RawThreadTitle[]>("list_thread_titles");
  return raw.map((entry) => ({
    title: entry.title,
    channelId: entry.channel,
    rootId: entry.root,
    revision: entry.revision,
    author: entry.author,
    updatedAt: entry.updated_at,
    lastActivityAt: entry.last_activity_at,
  }));
}

/** A thread's live title, looked up by its root (no activity ordering). */
export type ThreadTitleRecord = Omit<ThreadTitle, "lastActivityAt">;

type RawThreadTitleRecord = Omit<RawThreadTitle, "last_activity_at">;

/** Live titles for specific thread roots; untitled roots are omitted. */
export async function getThreadTitles(
  rootIds: readonly string[],
): Promise<ThreadTitleRecord[]> {
  const raw = await invokeTauri<RawThreadTitleRecord[]>("get_thread_titles", {
    rootIds,
  });
  return raw.map((entry) => ({
    title: entry.title,
    channelId: entry.channel,
    rootId: entry.root,
    revision: entry.revision,
    author: entry.author,
    updatedAt: entry.updated_at,
  }));
}

export type SetThreadTitleInput = {
  channelId: string;
  rootId: string;
  /** New title, or `null` to clear it. */
  title: string | null;
  /** Revision the edit started from; `null` when the thread had no title. */
  expectedRevision: string | null;
};

export type SetThreadTitleResult = {
  title: string | null;
  revision: string | null;
};

/**
 * Set or clear a thread's title. Rejects with a `conflict:` error instead of
 * overwriting when the title is no longer at `expectedRevision`.
 */
export async function setThreadTitle(
  input: SetThreadTitleInput,
): Promise<SetThreadTitleResult> {
  return invokeTauri<SetThreadTitleResult>("set_thread_title", {
    channelId: input.channelId,
    rootId: input.rootId,
    title: input.title,
    expectedRevision: input.expectedRevision,
  });
}
