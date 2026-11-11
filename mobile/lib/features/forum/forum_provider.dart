import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../../shared/relay/relay_author_provider.dart';
import '../channels/channel_window.dart';
import '../channels/thread_window.dart';
import '../channels/channel_management_provider.dart';
import '../../shared/custom_emoji/custom_emoji.dart';
import '../../shared/custom_emoji/custom_emoji_provider.dart';
import 'forum_models.dart';
import 'forum_ancestry.dart';
import 'forum_reader.dart';
import 'forum_root_reader.dart';

/// Pages forum roots without letting reply volume consume the row budget.
final forumPostsProvider =
    AsyncNotifierProvider.family<
      ForumPostsNotifier,
      ForumPostsResponse,
      String
    >(ForumPostsNotifier.new);

/// In-memory forum roots. Polls merge only the head; invalidation resets scope.
class ForumPostsNotifier extends ForumReader<ForumPostsResponse> {
  final String channelId;
  ForumPostsNotifier(this.channelId);

  @override
  Future<ForumSource<ForumPostsResponse>> open(
    void Function() checkActive,
  ) async {
    final config = ref.watch(relayConfigProvider);
    final session = ref.watch(relaySessionProvider.notifier);
    void checkScope() {
      checkActive();
      if (!identical(config, ref.read(relayConfigProvider))) {
        throw StateError('Forum community or identity changed');
      }
    }

    return _PostsSource(
      ForumRootReader(session, channelId, config.baseUrl, checkScope),
    );
  }
}

/// The channel-scoped identity of a forum root.
typedef ForumThreadArgs = ({String channelId, String eventId});

/// Pages mixed-kind replies, admitting only resolved ancestry.
final forumThreadProvider =
    AsyncNotifierProvider.family<
      ForumThreadNotifier,
      ForumThreadResponse,
      ForumThreadArgs
    >(ForumThreadNotifier.new);

/// In-memory thread history with explicit unresolved and continuation state.
class ForumThreadNotifier extends ForumReader<ForumThreadResponse> {
  final ForumThreadArgs args;
  ForumThreadNotifier(this.args);

  @override
  Future<ForumSource<ForumThreadResponse>> open(
    void Function() checkActive,
  ) async {
    final config = ref.watch(relayConfigProvider);
    final session = ref.watch(relaySessionProvider.notifier);
    final reader = ref.watch(myPubkeyProvider);
    void checkScope() {
      checkActive();
      if (!identical(config, ref.read(relayConfigProvider))) {
        throw StateError('Forum community or identity changed');
      }
    }

    checkScope();
    final roots = await session.queryRelay([
      NostrFilter(
        kinds: NostrFilters.forumRootKinds,
        ids: [args.eventId],
        tags: {
          '#h': [args.channelId],
        },
        limit: 1,
      ),
    ]);
    checkScope();
    if (roots.length != 1 ||
        roots.single.id != args.eventId ||
        roots.single.channelId != args.channelId ||
        !NostrFilters.forumRootKinds.contains(roots.single.kind) ||
        roots.single.threadReference.parentId != null) {
      throw StateError('Forum thread not found in this channel');
    }
    await verifyForumEvents(roots);
    checkScope();
    final relayAuthor = await ref.watch(
      relayAuthorProvider(config.baseUrl).future,
    );
    checkScope();
    if (relayAuthor == null || reader == null) {
      throw StateError('Cannot authenticate forum thread bounds');
    }
    return _ThreadSource(
      session,
      roots.single,
      config.baseUrl,
      reader,
      relayAuthor,
      checkScope,
    );
  }
}

class _PostsSource implements ForumSource<ForumPostsResponse> {
  final ForumRootReader reader;
  _PostsSource(this.reader);

  @override
  Future<ChannelWindowPage> fetch(ChannelPageCursor? cursor) =>
      reader.fetch(cursor);

  @override
  ForumPostsResponse render(
    ChannelWindowStore store, {
    bool loadingMore = false,
    Object? loadMoreError,
    Object? refreshError,
  }) => ForumPostsResponse.fromWindow(
    store,
    isLoadingMore: loadingMore,
    loadMoreError: loadMoreError,
    refreshError: refreshError,
  );
}

class _ThreadSource implements ForumSource<ForumThreadResponse> {
  final RelaySessionNotifier session;
  final NostrEvent root;
  final String relayUrl;
  final String reader;
  final String relayAuthor;
  final void Function() checkActive;
  ForumAncestry ancestry;

  _ThreadSource(
    this.session,
    this.root,
    this.relayUrl,
    this.reader,
    this.relayAuthor,
    this.checkActive,
  ) : ancestry = ForumAncestry(root);

  @override
  Future<ChannelWindowPage> fetch(ChannelPageCursor? cursor) async {
    final filter = NostrFilters.forumThread(
      root.id,
      root.channelId!,
      until: cursor?.createdAt,
      beforeId: cursor?.eventId,
    );
    checkActive();
    final events = await session.queryRelay([filter]);
    checkActive();
    final page = await decodeThreadWindow(
      events,
      ThreadWindowRequest(
        relayUrl: relayUrl,
        reader: reader,
        relayAuthor: relayAuthor,
        filter: filter,
      ),
      cursor,
    );
    checkActive();
    await verifyForumEvents(page.aux);
    checkActive();
    final next = ancestry.copy();
    await next.resolve(
      session,
      page.rows.map((row) => row.event).toList(),
      checkActive,
    );
    checkActive();
    ancestry = next;
    return page;
  }

  @override
  ForumThreadResponse render(
    ChannelWindowStore store, {
    bool loadingMore = false,
    Object? loadMoreError,
    Object? refreshError,
  }) => ForumThreadResponse.fromWindow(
    root: root,
    store: store,
    verifiedReplyIds: ancestry.verified,
    isLoadingMore: loadingMore,
    loadMoreError: loadMoreError,
    refreshError: refreshError,
  );
}

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
