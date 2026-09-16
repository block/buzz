import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../channels/channel_management_provider.dart';
import '../../shared/custom_emoji/custom_emoji.dart';
import '../../shared/custom_emoji/custom_emoji_provider.dart';
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

/// A forum event delivery bound to the community where composition began.
///
/// Attachment uploads can outlive their route. Capturing the relay identity,
/// signing key, and emoji palette prevents a queued draft from being delivered
/// to a different community after the user switches relays.
class ForumEventDelivery {
  final ProviderContainer _container;
  final String _relayUrl;
  final String? _nsec;
  final SignedEventRelay _relay;
  final List<CustomEmoji> _customEmoji;

  ForumEventDelivery._({
    required ProviderContainer container,
    required String relayUrl,
    required String? nsec,
    required SignedEventRelay relay,
    required List<CustomEmoji> customEmoji,
  }) : _container = container,
       _relayUrl = relayUrl,
       _nsec = nsec,
       _relay = relay,
       _customEmoji = customEmoji;

  /// Captures the active community dependencies for a future delivery.
  factory ForumEventDelivery.capture(ProviderContainer container) {
    final config = container.read(relayConfigProvider);
    return ForumEventDelivery._(
      container: container,
      relayUrl: config.baseUrl,
      nsec: config.nsec,
      relay: SignedEventRelay(
        session: container.read(relaySessionProvider.notifier),
        nsec: config.nsec,
      ),
      customEmoji: List<CustomEmoji>.unmodifiable(
        container.read(customEmojiListProvider),
      ),
    );
  }

  /// Creates a new forum post (kind:45001).
  Future<void> createPost({
    required String channelId,
    required String content,
    List<String> mentionPubkeys = const [],
    List<List<String>> mediaTags = const [],
  }) async {
    await _submit(
      kind: EventKind.forumPost,
      channelId: channelId,
      content: content,
      mentionPubkeys: mentionPubkeys,
      mediaTags: mediaTags,
    );
    _container.invalidate(forumPostsProvider(channelId));
  }

  /// Creates a reply to a forum post (kind:45003).
  Future<void> createReply({
    required String channelId,
    required String parentEventId,
    required String content,
    List<String> mentionPubkeys = const [],
    List<List<String>> mediaTags = const [],
  }) async {
    await _submit(
      kind: EventKind.forumComment,
      channelId: channelId,
      parentEventId: parentEventId,
      content: content,
      mentionPubkeys: mentionPubkeys,
      mediaTags: mediaTags,
    );
    _container.invalidate(forumPostsProvider(channelId));
    _container.invalidate(
      forumThreadProvider((channelId: channelId, eventId: parentEventId)),
    );
  }

  Future<void> _submit({
    required int kind,
    required String channelId,
    required String content,
    String? parentEventId,
    required List<String> mentionPubkeys,
    required List<List<String>> mediaTags,
  }) async {
    final currentConfig = _container.read(relayConfigProvider);
    if (currentConfig.baseUrl != _relayUrl || currentConfig.nsec != _nsec) {
      throw StateError(
        'Forum delivery cancelled because the active community changed',
      );
    }

    final selfPubkey = _relay.pubkey?.toLowerCase();
    final seen = <String>{?selfPubkey};
    final normalizedMentions = [
      for (final pk in mentionPubkeys)
        if (seen.add(pk.toLowerCase())) pk,
    ];

    await _relay.submit(
      kind: kind,
      content: content,
      tags: [
        ['h', channelId],
        if (parentEventId != null) ['e', parentEventId, '', 'reply'],
        for (final pk in normalizedMentions) ['p', pk],
        ...mediaTags,
        ...buildCustomEmojiTags(content, _customEmoji),
      ],
    );
  }
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
