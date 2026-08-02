import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../channels/channel.dart';
import '../channels/channels_provider.dart';
import 'feed_item.dart';
import 'inbox_item.dart';

/// Builds the Activity inbox feed over the relay websocket.
///
/// Sources mirror desktop's Home inbox (`useHomeFeedQuery` + `get_feed`):
/// - mentions of me on user-visible channel kinds (also yields thread
///   replies, which the thread filter classifies from NIP-10 tags)
/// - workflow approvals / needs-action events addressed to me
/// - agent job lifecycle events addressed to me (kinds 43001-43006)
/// - recent DM messages from others (desktop surfaces DMs through p-tags;
///   mobile queries DM channels directly so untagged DM sends still appear)
class ActivityNotifier extends AsyncNotifier<HomeFeedResponse> {
  @override
  Future<HomeFeedResponse> build() {
    ref.watch(relayConfigProvider);
    ref.watch(relaySessionProvider);
    // React to the DM channel set (loading → data, membership changes) so a
    // cold start where channels resolve after the first fetch still surfaces
    // DMs without a manual refresh.
    ref.watch(channelsProvider.select(_dmChannelKey));
    return _fetch();
  }

  /// Stable identity for the joined DM channel set: null while channels are
  /// loading, otherwise the sorted member-DM ids. Keeps unrelated channel
  /// updates from refetching the feed.
  static String? _dmChannelKey(AsyncValue<List<Channel>> channels) {
    final value = channels.asData?.value;
    if (value == null) return null;
    final ids = [
      for (final channel in value)
        if (channel.isDm && channel.isMember) channel.id,
    ]..sort();
    return ids.join(',');
  }

  Future<HomeFeedResponse> _fetch() async {
    final myPk = ref.read(myPubkeyProvider);
    if (myPk == null) {
      return HomeFeedResponse(
        mentions: const [],
        needsAction: const [],
        activity: const [],
        agentActivity: const [],
      );
    }

    final session = ref.read(relaySessionProvider.notifier);

    // DM channels come from the channel list; while it is still loading the
    // DM source is skipped, and build() rebuilds when it resolves (see the
    // channelsProvider watch above).
    final dmChannelIds = [
      for (final channel
          in ref.read(channelsProvider).asData?.value ?? const <Channel>[])
        if (channel.isDm && channel.isMember) channel.id,
    ];

    final results = await Future.wait([
      // Mentions of me on user-visible channel content.
      session.fetchHistory(
        NostrFilter(
          kinds: const [9, 40002, 1, 45001, 45003],
          tags: {
            '#p': [myPk],
          },
          limit: 50,
        ),
      ),
      // Workflow approvals addressed to me.
      session.fetchHistory(
        NostrFilter(
          kinds: const [46010, 46011, 46012],
          tags: {
            '#p': [myPk],
          },
          limit: 20,
        ),
      ),
      // Agent job lifecycle events addressed to me.
      session.fetchHistory(
        NostrFilter(
          kinds: const [43001, 43002, 43003, 43004, 43005, 43006],
          tags: {
            '#p': [myPk],
          },
          limit: 20,
        ),
      ),
      // Recent DM traffic (filtered to other senders below).
      if (dmChannelIds.isEmpty)
        Future.value(const <NostrEvent>[])
      else
        session.fetchHistory(
          NostrFilter(kinds: const [9], tags: {'#h': dmChannelIds}, limit: 30),
        ),
    ]);

    bool isFromOther(NostrEvent e) =>
        e.pubkey.toLowerCase() != myPk.toLowerCase();

    // Dedupe across sources by event id, keeping the higher-priority
    // category (needs_action > mention > agent_activity > activity).
    final byId = <String, FeedItem>{};
    void add(Iterable<NostrEvent> events, String category) {
      for (final event in events) {
        final existing = byId[event.id];
        if (existing != null &&
            categoryPriority(existing.category) <= categoryPriority(category)) {
          continue;
        }
        byId[event.id] = _feedItem(event, category: category);
      }
    }

    add(results[1], 'needs_action');
    add(results[0].where(isFromOther), 'mention');
    add(results[2], 'agent_activity');
    add(results[3].where(isFromOther), 'activity');

    final items = byId.values.toList()
      ..sort((a, b) => b.createdAt.compareTo(a.createdAt));

    return HomeFeedResponse(
      mentions: [
        for (final i in items)
          if (i.category == 'mention') i,
      ],
      needsAction: [
        for (final i in items)
          if (i.category == 'needs_action') i,
      ],
      activity: [
        for (final i in items)
          if (i.category == 'activity') i,
      ],
      agentActivity: [
        for (final i in items)
          if (i.category == 'agent_activity') i,
      ],
    );
  }

  FeedItem _feedItem(NostrEvent event, {required String category}) {
    return FeedItem(
      id: event.id,
      kind: event.kind,
      pubkey: event.pubkey,
      content: event.content,
      createdAt: event.createdAt,
      channelId: event.channelId,
      channelName: '',
      tags: event.tags,
      category: category,
    );
  }

  Future<void> refresh() async {
    state = await AsyncValue.guard(_fetch);
  }
}

final activityProvider =
    AsyncNotifierProvider<ActivityNotifier, HomeFeedResponse>(
      ActivityNotifier.new,
    );

/// Conversation-grouped inbox rows derived from the raw feed. DM messages
/// group by DM channel so one conversation renders as one row.
final inboxItemsProvider = Provider<List<InboxItem>>((ref) {
  final feed = ref.watch(activityProvider).value;
  if (feed == null) return const [];
  final dmChannelIds = {
    for (final channel
        in ref.watch(channelsProvider).asData?.value ?? const <Channel>[])
      if (channel.isDm) channel.id,
  };
  return buildInboxItems(feed.all, isDmChannel: dmChannelIds.contains);
});
