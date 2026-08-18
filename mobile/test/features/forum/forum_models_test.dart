import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/forum/forum_models.dart';

Map<String, dynamic> _postJson({
  String eventId = 'evt1',
  String pubkey = 'alice',
  String content = 'Hello world',
  int kind = 45001,
  int createdAt = 1000,
  String channelId = 'ch1',
  List<List<String>>? tags,
  Map<String, dynamic>? threadSummary,
}) => {
  'event_id': eventId,
  'pubkey': pubkey,
  'content': content,
  'kind': kind,
  'created_at': createdAt,
  'channel_id': channelId,
  'tags':
      tags ??
      [
        ['h', channelId],
      ],
  ...?threadSummary != null ? {'thread_summary': threadSummary} : null,
};

Map<String, dynamic> _replyJson({
  String eventId = 'reply1',
  String pubkey = 'bob',
  String content = 'Nice post',
  int kind = 45003,
  int createdAt = 2000,
  String channelId = 'ch1',
  String? parentEventId = 'evt1',
  String? rootEventId = 'evt1',
  int depth = 1,
}) => {
  'event_id': eventId,
  'pubkey': pubkey,
  'content': content,
  'kind': kind,
  'created_at': createdAt,
  'channel_id': channelId,
  'tags': [
    ['h', channelId],
  ],
  'parent_event_id': parentEventId,
  'root_event_id': rootEventId,
  'depth': depth,
};

Map<String, dynamic> _summaryJson({
  int replyCount = 3,
  int descendantCount = 5,
  int? lastReplyAt = 3000,
  List<String> participants = const ['bob', 'carol'],
}) => {
  'reply_count': replyCount,
  'descendant_count': descendantCount,
  'last_reply_at': lastReplyAt,
  'participants': participants,
};

void main() {
  group('ForumPost.fromJson', () {
    test('parses complete post with thread summary', () {
      final json = _postJson(
        threadSummary: _summaryJson(),
        tags: [
          ['h', 'ch1'],
          ['p', 'bob'],
        ],
      );
      final post = ForumPost.fromJson(json);

      expect(post.eventId, 'evt1');
      expect(post.pubkey, 'alice');
      expect(post.content, 'Hello world');
      expect(post.kind, 45001);
      expect(post.createdAt, 1000);
      expect(post.channelId, 'ch1');
      expect(post.tags, hasLength(2));
      expect(post.threadSummary, isNotNull);
      expect(post.threadSummary!.replyCount, 3);
      expect(post.threadSummary!.descendantCount, 5);
      expect(post.threadSummary!.lastReplyAt, 3000);
      expect(post.threadSummary!.participants, ['bob', 'carol']);
    });

    test('parses post without thread summary', () {
      final post = ForumPost.fromJson(_postJson());
      expect(post.threadSummary, isNull);
    });

    test('extracts mention pubkeys from p-tags', () {
      final post = ForumPost.fromJson(
        _postJson(
          tags: [
            ['h', 'ch1'],
            ['p', 'bob'],
            ['p', 'carol'],
          ],
        ),
      );
      expect(post.mentionPubkeys, ['bob', 'carol']);
    });

    test('returns empty mentions when no p-tags', () {
      final post = ForumPost.fromJson(
        _postJson(
          tags: [
            ['h', 'ch1'],
          ],
        ),
      );
      expect(post.mentionPubkeys, isEmpty);
    });

    test('skips malformed p-tags with length < 2', () {
      final post = ForumPost.fromJson(
        _postJson(
          tags: [
            ['h', 'ch1'],
            ['p'],
            ['p', 'bob'],
          ],
        ),
      );
      expect(post.mentionPubkeys, ['bob']);
    });
  });

  group('ForumThreadSummary.fromJson', () {
    test('parses all fields', () {
      final summary = ForumThreadSummary.fromJson(_summaryJson());
      expect(summary.replyCount, 3);
      expect(summary.descendantCount, 5);
      expect(summary.lastReplyAt, 3000);
      expect(summary.participants, ['bob', 'carol']);
    });

    test('defaults missing fields', () {
      final summary = ForumThreadSummary.fromJson(const <String, dynamic>{});
      expect(summary.replyCount, 0);
      expect(summary.descendantCount, 0);
      expect(summary.lastReplyAt, isNull);
      expect(summary.participants, isEmpty);
    });
  });

  group('ThreadReply.fromJson', () {
    test('parses a full reply', () {
      final reply = ThreadReply.fromJson(_replyJson());
      expect(reply.eventId, 'reply1');
      expect(reply.pubkey, 'bob');
      expect(reply.content, 'Nice post');
      expect(reply.kind, 45003);
      expect(reply.parentEventId, 'evt1');
      expect(reply.rootEventId, 'evt1');
      expect(reply.depth, 1);
    });

    test('defaults depth to 0 when missing', () {
      final json = _replyJson();
      json.remove('depth');
      final reply = ThreadReply.fromJson(json);
      expect(reply.depth, 0);
    });

    test('handles null parent and root', () {
      final reply = ThreadReply.fromJson(
        _replyJson(parentEventId: null, rootEventId: null),
      );
      expect(reply.parentEventId, isNull);
      expect(reply.rootEventId, isNull);
    });
  });

  group('ForumPostsResponse.fromJson', () {
    test('parses paginated response', () {
      final response = ForumPostsResponse.fromJson({
        'messages': [_postJson(), _postJson(eventId: 'evt2')],
        'next_cursor': 900,
      });
      expect(response.posts, hasLength(2));
      expect(response.nextCursor, 900);
    });

    test('handles empty messages', () {
      final response = ForumPostsResponse.fromJson({
        'messages': <dynamic>[],
        'next_cursor': null,
      });
      expect(response.posts, isEmpty);
      expect(response.nextCursor, isNull);
    });

    test('handles missing messages key', () {
      final response = ForumPostsResponse.fromJson(const <String, dynamic>{});
      expect(response.posts, isEmpty);
    });
  });

  group('ForumThreadResponse.fromJson', () {
    test('parses thread with root and replies', () {
      final response = ForumThreadResponse.fromJson({
        'root': _postJson(),
        'replies': [_replyJson(), _replyJson(eventId: 'reply2')],
        'total_replies': 2,
        'next_cursor': 'abc',
      });
      expect(response.post.eventId, 'evt1');
      expect(response.replies, hasLength(2));
      expect(response.totalReplies, 2);
      expect(response.nextCursor, 'abc');
    });

    test('handles empty replies', () {
      final response = ForumThreadResponse.fromJson({
        'root': _postJson(),
        'replies': <dynamic>[],
        'total_replies': 0,
      });
      expect(response.replies, isEmpty);
      expect(response.totalReplies, 0);
      expect(response.nextCursor, isNull);
    });
  });

  group('formatRelativeTime', () {
    int now() => DateTime.now().millisecondsSinceEpoch ~/ 1000;

    test('returns just now for < 60s', () {
      expect(formatRelativeTime(now() - 30), 'just now');
    });

    test('returns minutes format', () {
      expect(formatRelativeTime(now() - 120), '2m ago');
    });

    test('returns hours format', () {
      expect(formatRelativeTime(now() - 7200), '2h ago');
    });

    test('returns days format', () {
      expect(formatRelativeTime(now() - 172800), '2d ago');
    });

    test('returns date format for > 7 days', () {
      final ts = now() - (8 * 86400);
      final result = formatRelativeTime(ts);
      // Should be M/D/YYYY format.
      expect(result, matches(RegExp(r'^\d{1,2}/\d{1,2}/\d{4}$')));
    });
  });
}
