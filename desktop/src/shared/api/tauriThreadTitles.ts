import { invokeTauri } from "@/shared/api/tauri";

/** A shared thread title (NIP-AR artifact, type `buzz.thread-title`). */
export type ThreadTitleRecord = {
  title: string;
  channelId: string;
  rootId: string;
  revision: string;
  author: string;
  updatedAt: number;
};

type RawThreadTitleRecord = {
  title: string;
  channel: string;
  root: string;
  revision: string;
  author: string;
  updated_at: number;
};

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
