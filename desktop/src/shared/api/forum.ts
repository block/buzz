import type {
  ForumPost,
  ForumPostsResponse,
  ForumThreadResponse,
  ThreadReply,
} from "@/shared/api/types";
import { KIND_FORUM_POST } from "@/shared/constants/kinds";
import { resolveEventAuthorPubkey } from "@/shared/lib/authors";

import { invokeTauri } from "./tauri";

type RawThreadSummary = {
  reply_count: number;
  descendant_count: number;
  last_reply_at: number | null;
  participants: string[];
};

type RawForumPost = {
  event_id: string;
  pubkey: string;
  content: string;
  kind: number;
  created_at: number;
  channel_id: string;
  tags: string[][];
  sig: string;
  thread_summary: RawThreadSummary | null;
  reactions: unknown;
};

type RawForumPostsResponse = {
  messages: RawForumPost[];
  next_cursor: number | null;
};

type RawThreadReply = {
  event_id: string;
  pubkey: string;
  content: string;
  kind: number;
  created_at: number;
  channel_id: string;
  tags: string[][];
  sig: string;
  parent_event_id: string | null;
  root_event_id: string | null;
  depth: number;
  broadcast: boolean;
  reactions: unknown;
};

type RawForumThreadResponse = {
  root: RawForumPost;
  replies: RawThreadReply[];
  total_replies: number;
  next_cursor: string | null;
};

function fromRawForumPost(
  post: RawForumPost,
  relaySelfPubkey?: string | null,
): ForumPost {
  return {
    eventId: post.event_id,
    pubkey: resolveEventAuthorPubkey({
      event: {
        id: post.event_id,
        pubkey: post.pubkey,
        created_at: post.created_at,
        kind: post.kind,
        tags: post.tags,
        content: post.content,
        sig: post.sig,
      },
      relaySelfPubkey,
    }),
    content: post.content,
    kind: post.kind,
    createdAt: post.created_at,
    channelId: post.channel_id,
    tags: post.tags,
    threadSummary: post.thread_summary
      ? {
          replyCount: post.thread_summary.reply_count,
          descendantCount: post.thread_summary.descendant_count,
          lastReplyAt: post.thread_summary.last_reply_at,
          participants: post.thread_summary.participants,
        }
      : null,
  };
}

function fromRawThreadReply(
  reply: RawThreadReply,
  relaySelfPubkey?: string | null,
): ThreadReply {
  return {
    eventId: reply.event_id,
    pubkey: resolveEventAuthorPubkey({
      event: {
        id: reply.event_id,
        pubkey: reply.pubkey,
        created_at: reply.created_at,
        kind: reply.kind,
        tags: reply.tags,
        content: reply.content,
        sig: reply.sig,
      },
      relaySelfPubkey,
    }),
    content: reply.content,
    kind: reply.kind,
    createdAt: reply.created_at,
    channelId: reply.channel_id,
    tags: reply.tags,
    parentEventId: reply.parent_event_id,
    rootEventId: reply.root_event_id,
    depth: reply.depth,
  };
}

export async function getForumPosts(
  channelId: string,
  limit?: number,
  before?: number,
  relaySelfPubkey?: string | null,
): Promise<ForumPostsResponse> {
  const response = await invokeTauri<RawForumPostsResponse>("get_forum_posts", {
    channelId,
    limit: limit ?? null,
    before: before ?? null,
  });

  return {
    posts: response.messages
      .filter((m) => m.kind === KIND_FORUM_POST)
      .map((post) => fromRawForumPost(post, relaySelfPubkey)),
    nextCursor: response.next_cursor,
  };
}

export async function getForumThread(
  channelId: string,
  eventId: string,
  limit?: number,
  cursor?: string,
  relaySelfPubkey?: string | null,
): Promise<ForumThreadResponse> {
  const response = await invokeTauri<RawForumThreadResponse>(
    "get_forum_thread",
    {
      channelId,
      eventId,
      limit: limit ?? null,
      cursor: cursor ?? null,
    },
  );

  return {
    post: fromRawForumPost(response.root, relaySelfPubkey),
    replies: response.replies.map((reply) =>
      fromRawThreadReply(reply, relaySelfPubkey),
    ),
    totalReplies: response.total_replies,
    nextCursor: response.next_cursor,
  };
}
