import 'package:flutter/foundation.dart';

import '../../shared/relay/relay.dart';
import '../channels/channel_event_order.dart';
import '../channels/channel_window.dart';
import '../channels/thread_window.dart';
import '../channels/timeline_message.dart';
import 'forum_window.dart';

/// A top-level forum post with an optional thread summary.
@immutable
class ForumPost {
  final String eventId;
  final String pubkey;
  final String content;
  final int kind;
  final int createdAt;
  final String channelId;
  final List<List<String>> tags;
  final ForumThreadSummary? threadSummary;

  const ForumPost({
    required this.eventId,
    required this.pubkey,
    required this.content,
    required this.kind,
    required this.createdAt,
    required this.channelId,
    required this.tags,
    this.threadSummary,
  });

  factory ForumPost.fromJson(Map<String, dynamic> json) {
    final rawSummary = json['thread_summary'] as Map<String, dynamic>?;
    return ForumPost(
      eventId: json['event_id'] as String,
      pubkey: json['pubkey'] as String,
      content: json['content'] as String,
      kind: json['kind'] as int,
      createdAt: json['created_at'] as int,
      channelId: json['channel_id'] as String,
      tags: (json['tags'] as List<dynamic>)
          .map((t) => (t as List<dynamic>).map((e) => e as String).toList())
          .toList(),
      threadSummary: rawSummary != null
          ? ForumThreadSummary.fromJson(rawSummary)
          : null,
    );
  }

  /// Builds a post from its immutable event and optional display overlays.
  factory ForumPost.fromEvent(
    NostrEvent event, {
    TimelineMessage? display,
    ChannelWindowThreadSummary? summary,
  }) {
    return ForumPost(
      eventId: event.id,
      pubkey: event.pubkey,
      content: display?.content ?? event.content,
      kind: event.kind,
      createdAt: event.createdAt,
      channelId: event.channelId ?? '',
      tags: display?.tags ?? event.tags,
      threadSummary: summary == null
          ? null
          : ForumThreadSummary(
              replyCount: summary.replyCount,
              descendantCount: summary.descendantCount,
              lastReplyAt: summary.lastReplyAt,
              participants: summary.participantPubkeys,
            ),
    );
  }

  /// Extract mention pubkeys from p-tags.
  List<String> get mentionPubkeys => [
    for (final tag in tags)
      if (tag.length >= 2 && tag[0] == 'p') tag[1],
  ];
}

/// Summary of replies on a forum post.
@immutable
class ForumThreadSummary {
  final int replyCount;
  final int descendantCount;
  final int? lastReplyAt;
  final List<String> participants;

  const ForumThreadSummary({
    required this.replyCount,
    required this.descendantCount,
    this.lastReplyAt,
    required this.participants,
  });

  factory ForumThreadSummary.fromJson(Map<String, dynamic> json) {
    return ForumThreadSummary(
      replyCount: json['reply_count'] as int? ?? 0,
      descendantCount: json['descendant_count'] as int? ?? 0,
      lastReplyAt: json['last_reply_at'] as int?,
      participants:
          (json['participants'] as List<dynamic>?)
              ?.map((e) => e as String)
              .toList() ??
          const [],
    );
  }
}

/// A reply within a forum thread.
@immutable
class ThreadReply {
  final String eventId;
  final String pubkey;
  final String content;
  final int kind;
  final int createdAt;
  final String channelId;
  final List<List<String>> tags;
  final String? parentEventId;
  final String? rootEventId;
  final int depth;

  const ThreadReply({
    required this.eventId,
    required this.pubkey,
    required this.content,
    required this.kind,
    required this.createdAt,
    required this.channelId,
    required this.tags,
    this.parentEventId,
    this.rootEventId,
    required this.depth,
  });

  factory ThreadReply.fromJson(Map<String, dynamic> json) {
    return ThreadReply(
      eventId: json['event_id'] as String,
      pubkey: json['pubkey'] as String,
      content: json['content'] as String,
      kind: json['kind'] as int,
      createdAt: json['created_at'] as int,
      channelId: json['channel_id'] as String,
      tags: (json['tags'] as List<dynamic>)
          .map((t) => (t as List<dynamic>).map((e) => e as String).toList())
          .toList(),
      parentEventId: json['parent_event_id'] as String?,
      rootEventId: json['root_event_id'] as String?,
      depth: json['depth'] as int? ?? 0,
    );
  }

  /// Build a [ThreadReply] from a raw Nostr event.
  factory ThreadReply.fromEvent(
    NostrEvent event, {
    TimelineMessage? display,
    String? resolvedRootId,
  }) {
    final ref = event.threadReference;
    return ThreadReply(
      eventId: event.id,
      pubkey: event.pubkey,
      content: display?.content ?? event.content,
      kind: event.kind,
      createdAt: event.createdAt,
      channelId: event.channelId ?? '',
      tags: display?.tags ?? event.tags,
      parentEventId: ref.parentId,
      rootEventId: resolvedRootId ?? ref.rootId,
      depth: 0,
    );
  }

  /// Extract mention pubkeys from p-tags.
  List<String> get mentionPubkeys => [
    for (final tag in tags)
      if (tag.length >= 2 && tag[0] == 'p') tag[1],
  ];
}

/// Shared loading and completeness signals for forum readers.
abstract interface class ForumReadResponse {
  bool get hasMore;
  Object? get refreshError;
}

/// A deletion that must remove the retained root content immediately.
class ForumPostDeleted implements Exception {
  const ForumPostDeleted();
}

/// Paginated response for forum posts.
@immutable
class ForumPostsResponse implements ForumReadResponse {
  final List<ForumPost> posts;
  final ChannelPageCursor? nextCursor;
  final bool isLoadingMore;
  final Object? loadMoreError;
  @override
  final Object? refreshError;

  /// A loaded page has unconfirmed fallback history or auxiliary authority.
  /// This is independent of a failed refresh and has no retry affordance.
  final bool hasUnconfirmedHistory;

  /// More rows remain, or a bounded legacy scan has not established exhaustion.
  @override
  bool get hasMore => nextCursor != null;

  const ForumPostsResponse({
    required this.posts,
    this.nextCursor,
    this.isLoadingMore = false,
    this.loadMoreError,
    this.refreshError,
    this.hasUnconfirmedHistory = false,
  });

  factory ForumPostsResponse.fromJson(Map<String, dynamic> json) {
    final messages = json['messages'] as List<dynamic>? ?? const [];
    return ForumPostsResponse(
      posts: messages
          .cast<Map<String, dynamic>>()
          .map(ForumPost.fromJson)
          .toList(),
      nextCursor: parseWindowCursor(json['next_cursor']),
    );
  }

  /// Applies chat's edit/deletion semantics to mixed-kind root pages.
  factory ForumPostsResponse.fromWindow(
    ChannelWindowStore store, {
    bool isLoadingMore = false,
    Object? loadMoreError,
    Object? refreshError,
  }) {
    final events = _forumDisplayEvents(store);
    final display = {
      for (final message in formatTimeline(
        events,
        contentKinds: NostrFilters.forumRootKinds,
      ))
        message.id: message,
    };
    final summaries = channelWindowThreadSummaries(store);
    final roots =
        [
          for (final page in store.pages)
            for (final row in page.rows)
              if (row.event.threadReference.parentId == null &&
                  display.containsKey(row.event.id))
                row.event,
        ]..sort((a, b) {
          final time = b.createdAt.compareTo(a.createdAt);
          return time == 0 ? a.id.compareTo(b.id) : time;
        });
    return ForumPostsResponse(
      posts: [
        for (final event in roots)
          ForumPost.fromEvent(
            event,
            display: display[event.id],
            summary: summaries[event.id],
          ),
      ],
      nextCursor: channelWindowNextCursor(store),
      isLoadingMore: isLoadingMore,
      loadMoreError: loadMoreError,
      refreshError: refreshError,
      hasUnconfirmedHistory: store.pages.whereType<ForumWindowPage>().any(
        (page) => page.incompletePostIds.isNotEmpty,
      ),
    );
  }
}

List<NostrEvent> _forumDisplayEvents(ChannelWindowStore store) {
  final auxiliaryTargets = {
    for (final page in store.pages)
      if (page is ForumWindowPage) ...page.auxiliaryTargets,
  };
  return [
    for (final event in flattenChannelWindowEvents(store))
      if (auxiliaryTargets[event.id] case final targets?)
        // Display-only projection: ignored references must not affect other
        // retained or subsequently loaded posts. The signed event in the store
        // stays untouched; this projection is never verified or sent.
        NostrEvent(
          id: event.id,
          pubkey: event.pubkey,
          createdAt: event.createdAt,
          kind: event.kind,
          content: event.content,
          sig: event.sig,
          tags: event.tags
              .where(
                (tag) =>
                    tag.length < 2 || tag[0] != 'e' || targets.contains(tag[1]),
              )
              .toList(),
        )
      else
        event,
  ];
}

/// Response for a single forum thread with replies.
@immutable
class ForumThreadResponse implements ForumReadResponse {
  final ForumPost post;
  final List<ThreadReply> replies;
  final int totalReplies;
  final int unresolvedReplies;

  /// Missing ancestry, resource caps, and continuation are never exhaustion.
  bool get isIncomplete =>
      hasMore ||
      unresolvedReplies > 0 ||
      loadMoreError != null ||
      refreshError != null;
  final ChannelPageCursor? nextCursor;
  final bool isLoadingMore;
  final Object? loadMoreError;
  @override
  final Object? refreshError;

  /// True when older replies remain outside the loaded thread window.
  @override
  bool get hasMore => nextCursor != null;

  const ForumThreadResponse({
    required this.post,
    required this.replies,
    required this.totalReplies,
    this.unresolvedReplies = 0,
    this.nextCursor,
    this.isLoadingMore = false,
    this.loadMoreError,
    this.refreshError,
  });

  factory ForumThreadResponse.fromJson(Map<String, dynamic> json) {
    final repliesJson = json['replies'] as List<dynamic>? ?? const [];
    return ForumThreadResponse(
      post: ForumPost.fromJson(json['root'] as Map<String, dynamic>),
      replies: repliesJson
          .cast<Map<String, dynamic>>()
          .map(ThreadReply.fromJson)
          .toList(),
      totalReplies: json['total_replies'] as int? ?? 0,
      nextCursor: parseWindowCursor(json['next_cursor']),
    );
  }

  /// Applies auxiliary events while retaining the original reply ancestry.
  factory ForumThreadResponse.fromWindow({
    required NostrEvent root,
    required ChannelWindowStore store,
    required Set<String> verifiedReplyIds,
    bool isLoadingMore = false,
    Object? loadMoreError,
    Object? refreshError,
  }) {
    final events = [root, ...flattenChannelWindowEvents(store)]
      ..sort(compareChannelTimelineEventsChronologically);
    final display = {
      for (final message in formatTimeline(
        events,
        contentKinds: EventKind.channelMessageEventKinds,
      ))
        message.id: message,
    };
    final rootDisplay = display[root.id];
    if (rootDisplay == null) throw const ForumPostDeleted();
    final rows = [
      for (final page in store.pages)
        for (final row in page.rows)
          if (display.containsKey(row.event.id) &&
              verifiedReplyIds.contains(row.event.id))
            row.event,
    ]..sort(compareThreadRepliesChronologically);
    final replies = [
      for (final event in rows)
        ThreadReply.fromEvent(
          event,
          display: display[event.id],
          resolvedRootId: root.id,
        ),
    ];
    return ForumThreadResponse(
      post: ForumPost.fromEvent(root, display: rootDisplay),
      replies: replies,
      totalReplies: replies.length,
      unresolvedReplies: store.pages
          .expand((page) => page.rows)
          .where(
            (row) =>
                display.containsKey(row.event.id) &&
                !verifiedReplyIds.contains(row.event.id),
          )
          .length,
      nextCursor: channelWindowNextCursor(store),
      isLoadingMore: isLoadingMore,
      loadMoreError: loadMoreError,
      refreshError: refreshError,
    );
  }
}

/// Format a unix timestamp as a relative time string (e.g. "2h ago").
String formatRelativeTime(int timestamp) {
  final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
  final diff = now - timestamp;

  if (diff < 60) return 'just now';
  if (diff < 3600) return '${diff ~/ 60}m ago';
  if (diff < 86400) return '${diff ~/ 3600}h ago';
  if (diff < 604800) return '${diff ~/ 86400}d ago';

  final dt = DateTime.fromMillisecondsSinceEpoch(
    timestamp * 1000,
    isUtc: true,
  ).toLocal();
  return '${dt.month}/${dt.day}/${dt.year}';
}
