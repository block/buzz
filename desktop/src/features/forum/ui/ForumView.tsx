import { MessageSquareText } from "lucide-react";
import * as React from "react";

import { useProfileQuery, useUsersBatchQuery } from "@/features/profile/hooks";
import { mergeCurrentProfileIntoLookup } from "@/features/profile/lib/identity";
import type { Channel } from "@/shared/api/types";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import { Skeleton } from "@/shared/ui/skeleton";
import { VirtualizedList } from "@/shared/ui/VirtualizedList";

import {
  useCreateForumPostMutation,
  useCreateForumReplyMutation,
  useDeleteForumPostMutation,
  useDeleteForumReplyMutation,
  useForumPostsQuery,
  useForumThreadQuery,
} from "../hooks";
import { ForumComposer } from "./ForumComposer";
import { ForumPostCard } from "./ForumPostCard";
import { ForumThreadPanel } from "./ForumThreadPanel";

type ForumViewProps = {
  channel: Channel;
  currentPubkey?: string;
  onClosePost: () => void;
  onSelectPost: (postId: string) => void;
  onTargetReached?: (messageId: string) => void;
  selectedPostId: string | null;
  targetReplyId: string | null;
};

function canDelete(postPubkey: string, currentPubkey?: string): boolean {
  if (!currentPubkey) return false;
  // Author can always delete their own posts. Admin check would need
  // channel member role data — for now, author-only is sufficient.
  return postPubkey.toLowerCase() === currentPubkey.toLowerCase();
}

export function ForumView({
  channel,
  currentPubkey,
  onClosePost,
  onSelectPost,
  onTargetReached,
  selectedPostId,
  targetReplyId,
}: ForumViewProps) {
  const [isComposerOpen, setIsComposerOpen] = React.useState(false);
  const postsScrollRef = React.useRef<HTMLDivElement>(null);

  const profileQuery = useProfileQuery();
  const postsQuery = useForumPostsQuery(channel);
  const threadQuery = useForumThreadQuery(
    selectedPostId ? channel.id : null,
    selectedPostId,
  );
  const createPostMutation = useCreateForumPostMutation(channel);
  const createReplyMutation = useCreateForumReplyMutation(channel);
  const deletePostMutation = useDeleteForumPostMutation(channel);
  const deleteReplyMutation = useDeleteForumReplyMutation(
    channel,
    selectedPostId,
  );

  const posts = postsQuery.data?.posts ?? [];

  // Collect all pubkeys from posts and thread for profile resolution
  const allPubkeys = React.useMemo(() => {
    const pubkeys = new Set<string>();
    for (const post of posts) {
      pubkeys.add(post.pubkey);
      if (post.threadSummary?.participants) {
        for (const pk of post.threadSummary.participants) {
          pubkeys.add(pk);
        }
      }
    }
    if (threadQuery.data) {
      pubkeys.add(threadQuery.data.post.pubkey);
      for (const reply of threadQuery.data.replies) {
        pubkeys.add(reply.pubkey);
      }
    }
    return [...pubkeys];
  }, [posts, threadQuery.data]);

  const profilesQuery = useUsersBatchQuery(allPubkeys, {
    enabled: allPubkeys.length > 0,
  });
  const effectiveCurrentPubkey = currentPubkey ?? profileQuery.data?.pubkey;
  const profiles = React.useMemo(
    () =>
      mergeCurrentProfileIntoLookup(
        profilesQuery.data?.profiles,
        profileQuery.data,
      ),
    [profileQuery.data, profilesQuery.data?.profiles],
  );

  const previousChannelIdRef = React.useRef(channel.id);
  React.useEffect(() => {
    if (previousChannelIdRef.current === channel.id) {
      return;
    }

    previousChannelIdRef.current = channel.id;
    setIsComposerOpen(false);
  }, [channel.id]);

  if (selectedPostId) {
    const threadPost = threadQuery.data?.post;
    const canDeleteExpandedPost = threadPost
      ? canDelete(threadPost.pubkey, effectiveCurrentPubkey)
      : false;

    return (
      <ForumThreadPanel
        canDeletePost={canDeleteExpandedPost}
        currentPubkey={effectiveCurrentPubkey}
        isDeletingPost={deletePostMutation.isPending}
        isLoading={threadQuery.isLoading}
        isSendingReply={createReplyMutation.isPending}
        onBack={onClosePost}
        onDeletePost={(eventId) => {
          deletePostMutation.mutate({ eventId }, { onSuccess: onClosePost });
        }}
        onDeleteReply={(eventId) => {
          deleteReplyMutation.mutate({ eventId });
        }}
        channelId={channel.id}
        onReply={(content, mentionPubkeys, mediaTags) =>
          createReplyMutation.mutateAsync({
            content,
            parentEventId: selectedPostId,
            mentionPubkeys,
            mediaTags,
          })
        }
        onTargetReached={onTargetReached}
        profiles={profiles}
        targetEventId={targetReplyId}
        thread={threadQuery.data}
      />
    );
  }

  return (
    <div className={cn("flex h-full flex-col", channelChrome.contentPadding)}>
      {/* New post area */}
      <div className="border-b border-border/60 p-4">
        {isComposerOpen ? (
          <ForumComposer
            autocompleteBelow
            channelId={channel.id}
            isSending={createPostMutation.isPending}
            onCancel={() => setIsComposerOpen(false)}
            onSubmit={async (content, mentionPubkeys, mediaTags) => {
              await createPostMutation.mutateAsync({
                content,
                mentionPubkeys,
                mediaTags,
              });
              setIsComposerOpen(false);
            }}
            placeholder="Write your post..."
            profiles={profiles}
          />
        ) : (
          <button
            className="w-full rounded-xl border border-dashed border-border/80 px-4 py-3 text-left text-sm text-muted-foreground transition-colors hover:border-border hover:bg-accent/30 hover:text-foreground"
            disabled={!channel.isMember || channel.archivedAt !== null}
            onClick={() => setIsComposerOpen(true)}
            type="button"
          >
            {channel.archivedAt
              ? "This forum is archived."
              : !channel.isMember
                ? "Join this forum to create posts."
                : "Start a new post..."}
          </button>
        )}
      </div>

      {/* Post list */}
      <div
        className="flex-1 overflow-y-auto"
        data-scroll-restoration-id={`forum-list:${channel.id}`}
        ref={postsScrollRef}
      >
        {postsQuery.isLoading ? (
          <div className="space-y-3 p-4">
            <Skeleton className="h-24 w-full rounded-xl" />
            <Skeleton className="h-24 w-full rounded-xl" />
            <Skeleton className="h-24 w-full rounded-xl" />
          </div>
        ) : posts.length === 0 ? (
          <div className="flex flex-col items-center justify-center gap-3 px-4 py-16 text-center">
            <MessageSquareText className="h-10 w-10 text-muted-foreground/40" />
            <div>
              <p className="text-sm font-medium text-foreground/70">
                No posts yet
              </p>
              <p className="mt-1 text-xs text-muted-foreground">
                Start a discussion by creating the first post.
              </p>
            </div>
          </div>
        ) : (
          <VirtualizedList
            estimateSize={120}
            getItemKey={(post) => post.eventId}
            innerClassName="p-4"
            items={posts}
            renderItem={(post) => (
              <div className="pb-3">
                <ForumPostCard
                  canDelete={canDelete(post.pubkey, effectiveCurrentPubkey)}
                  currentPubkey={effectiveCurrentPubkey}
                  isActive={selectedPostId === post.eventId}
                  isDeleting={
                    deletePostMutation.isPending &&
                    deletePostMutation.variables?.eventId === post.eventId
                  }
                  onClick={() => onSelectPost(post.eventId)}
                  onDelete={(eventId) => {
                    deletePostMutation.mutate({ eventId });
                  }}
                  post={post}
                  profiles={profiles}
                />
              </div>
            )}
            scrollRef={postsScrollRef}
          />
        )}
      </div>
    </div>
  );
}
