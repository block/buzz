import 'package:flutter/foundation.dart';

import '../../shared/relay/relay.dart';

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

  /// Build a [ForumPost] from a raw Nostr event (kind:45001).
  factory ForumPost.fromEvent(NostrEvent event) {
    return ForumPost(
      eventId: event.id,
      pubkey: event.pubkey,
      content: event.content,
      kind: event.kind,
      createdAt: event.createdAt,
      channelId: event.channelId ?? '',
      tags: event.tags,
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
  factory ThreadReply.fromEvent(NostrEvent event) {
    final ref = event.threadReference;
    return ThreadReply(
      eventId: event.id,
      pubkey: event.pubkey,
      content: event.content,
      kind: event.kind,
      createdAt: event.createdAt,
      channelId: event.channelId ?? '',
      tags: event.tags,
      parentEventId: ref.parentId,
      rootEventId: ref.rootId,
      depth: 0,
    );
  }

  /// Extract mention pubkeys from p-tags.
  List<String> get mentionPubkeys => [
    for (final tag in tags)
      if (tag.length >= 2 && tag[0] == 'p') tag[1],
  ];
}

/// Paginated response for forum posts.
@immutable
class ForumPostsResponse {
  final List<ForumPost> posts;
  final int? nextCursor;

  const ForumPostsResponse({required this.posts, this.nextCursor});

  factory ForumPostsResponse.fromJson(Map<String, dynamic> json) {
    final messages = json['messages'] as List<dynamic>? ?? const [];
    return ForumPostsResponse(
      posts: messages
          .cast<Map<String, dynamic>>()
          .map(ForumPost.fromJson)
          .toList(),
      nextCursor: json['next_cursor'] as int?,
    );
  }

  /// Build from a list of kind:45001 events. Posts are sorted newest-first.
  factory ForumPostsResponse.fromEvents(List<NostrEvent> events) {
    final posts = events.map(ForumPost.fromEvent).toList()
      ..sort((a, b) => b.createdAt.compareTo(a.createdAt));
    return ForumPostsResponse(posts: posts, nextCursor: null);
  }
}

/// Response for a single forum thread with replies.
@immutable
class ForumThreadResponse {
  final ForumPost post;
  final List<ThreadReply> replies;
  final int totalReplies;
  final String? nextCursor;

  const ForumThreadResponse({
    required this.post,
    required this.replies,
    required this.totalReplies,
    this.nextCursor,
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
      nextCursor: json['next_cursor'] as String?,
    );
  }

  /// Build from a root event and a list of reply events. Replies are sorted
  /// oldest-first so the UI can render them top-down.
  factory ForumThreadResponse.fromEvents({
    required NostrEvent root,
    required List<NostrEvent> replies,
  }) {
    final sortedReplies = replies.map(ThreadReply.fromEvent).toList()
      ..sort((a, b) => a.createdAt.compareTo(b.createdAt));
    return ForumThreadResponse(
      post: ForumPost.fromEvent(root),
      replies: sortedReplies,
      totalReplies: sortedReplies.length,
      nextCursor: null,
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
