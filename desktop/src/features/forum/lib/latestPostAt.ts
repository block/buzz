import type { ForumPostsResponse } from "@/shared/api/types";

/**
 * Newest top-level post time across the loaded pages, in unix seconds.
 *
 * The channel read marker needs the forum equivalent of "newest top-level
 * message". Replies are deliberately excluded: opening the post list shows
 * every new post, not the replies inside each thread, so a thread keeps its
 * own unread state until it is opened.
 */
export function latestForumPostAt(
  pages: ForumPostsResponse[] | undefined,
): number | null {
  let latest: number | null = null;
  for (const page of pages ?? []) {
    for (const post of page.posts) {
      if (latest === null || post.createdAt > latest) {
        latest = post.createdAt;
      }
    }
  }
  return latest;
}
