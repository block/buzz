import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../channels/channel_management_provider.dart';
import '../custom_emoji/custom_emoji.dart';
import '../custom_emoji/custom_emoji_provider.dart';
import 'forum_models.dart';

/// Fetches forum posts (kind:45001) for a channel from the relay.
///
/// Posts are top-level events tagged `#h:<channelId>`. Invalidate to refresh
/// (e.g. after creating a new post).
final forumPostsProvider = FutureProvider.family<ForumPostsResponse, String>((
  ref,
  channelId,
) async {
  final session = ref.watch(relaySessionProvider.notifier);
  final events = await session.fetchHistory(
    NostrFilters.forumPosts(channelId, limit: 50),
  );
  return ForumPostsResponse.fromEvents(events);
});

/// Fetches a forum thread (root post + replies) from the relay.
final forumThreadProvider =
    FutureProvider.family<
      ForumThreadResponse,
      ({String channelId, String eventId})
    >((ref, args) async {
      final session = ref.watch(relaySessionProvider.notifier);

      final results = await Future.wait([
        // Root event lookup by id.
        session.fetchHistory(
          NostrFilter(
            kinds: const [9, 40002, 45001, 45003],
            ids: [args.eventId],
            limit: 1,
          ),
        ),
        // Replies pointing at this root.
        session.fetchHistory(
          NostrFilters.forumThread(args.eventId, args.channelId),
        ),
      ]);

      final rootEvents = results[0];
      final replyEvents = results[1];
      if (rootEvents.isEmpty) {
        throw Exception('Forum thread not found: ${args.eventId}');
      }
      return ForumThreadResponse.fromEvents(
        root: rootEvents.first,
        replies: replyEvents,
      );
    });

/// Creates a new forum post (kind:45001).
Future<void> createForumPost(
  WidgetRef ref, {
  required String channelId,
  required String content,
  List<String> mentionPubkeys = const [],
  List<List<String>> mediaTags = const [],
}) async {
  final config = ref.read(relayConfigProvider);
  final session = ref.read(relaySessionProvider.notifier);
  final relay = SignedEventRelay(session: session, nsec: config.nsec);

  final selfPubkey = relay.pubkey?.toLowerCase();
  final seen = <String>{?selfPubkey};
  final normalizedMentions = [
    for (final pk in mentionPubkeys)
      if (seen.add(pk.toLowerCase())) pk,
  ];

  await relay.submit(
    kind: EventKind.forumPost,
    content: content,
    tags: [
      ['h', channelId],
      for (final pk in normalizedMentions) ['p', pk],
      ...mediaTags,
      ...buildCustomEmojiTags(content, ref.read(customEmojiListProvider)),
    ],
  );
  ref.invalidate(forumPostsProvider(channelId));
}

/// Creates a reply to a forum post (kind:45003).
Future<void> createForumReply(
  WidgetRef ref, {
  required String channelId,
  required String parentEventId,
  required String content,
  List<String> mentionPubkeys = const [],
  List<List<String>> mediaTags = const [],
}) async {
  final config = ref.read(relayConfigProvider);
  final session = ref.read(relaySessionProvider.notifier);
  final relay = SignedEventRelay(session: session, nsec: config.nsec);

  final selfPubkey = relay.pubkey?.toLowerCase();
  final seen = <String>{?selfPubkey};
  final normalizedMentions = [
    for (final pk in mentionPubkeys)
      if (seen.add(pk.toLowerCase())) pk,
  ];

  await relay.submit(
    kind: EventKind.forumComment,
    content: content,
    tags: [
      ['h', channelId],
      ['e', parentEventId, '', 'reply'],
      for (final pk in normalizedMentions) ['p', pk],
      ...mediaTags,
      ...buildCustomEmojiTags(content, ref.read(customEmojiListProvider)),
    ],
  );
  ref.invalidate(forumPostsProvider(channelId));
  ref.invalidate(
    forumThreadProvider((channelId: channelId, eventId: parentEventId)),
  );
}

/// Deletes a forum post or reply and invalidates relevant caches.
Future<void> deleteForumEvent(
  WidgetRef ref, {
  required String channelId,
  required String eventId,
  String? rootEventId,
}) async {
  final actions = ref.read(channelActionsProvider);
  await actions.deleteMessage(channelId: channelId, eventId: eventId);
  ref.invalidate(forumPostsProvider(channelId));
  if (rootEventId != null) {
    ref.invalidate(
      forumThreadProvider((channelId: channelId, eventId: rootEventId)),
    );
  }
}
