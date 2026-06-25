import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/read_state/read_state_provider.dart';
import 'package:buzz/features/channels/unread_badge/unread_badge_provider.dart';

/// Unit tests for [unreadBadgeProvider].
///
/// Strategy: override [channelsProvider] with a [_StubbedChannelsNotifier] that
/// returns a pre-built channel list and allows seeding [latestHighPriorityByChannel],
/// and override [readStateProvider] with a [_ReadStateNotifier] that holds a
/// fixed [ReadStateState]. This avoids standing up any relay connection and
/// exercises only the badge-computation logic.
///
/// Because [channelsProvider] is an [AsyncNotifierProvider], the provider starts
/// in [AsyncLoading] — tests that check computed badge values must await
/// [channelsProvider.future] to let the notifier resolve before reading the badge.
void main() {
  // Fixed epoch timestamps (Unix seconds).
  const t10 = 10; // older
  const t20 = 20; // newer — any channel with lastMessageAt == t20 has a message
  const t30 = 30; // even newer — used for high-priority events

  Channel makeChannel({
    required String id,
    String channelType = 'stream',
    bool isMember = true,
    bool isArchived = false,
    int? lastMessageAtSeconds,
  }) {
    return Channel(
      id: id,
      name: id,
      channelType: channelType,
      visibility: 'open',
      description: '',
      createdBy: 'creator',
      createdAt: DateTime.utc(2024),
      memberCount: 1,
      isMember: isMember,
      archivedAt: isArchived ? DateTime.utc(2024) : null,
      lastMessageAt: lastMessageAtSeconds != null
          ? DateTime.fromMillisecondsSinceEpoch(
              lastMessageAtSeconds * 1000,
              isUtc: true,
            )
          : null,
    );
  }

  ProviderContainer buildContainer({
    required List<Channel> channels,
    Map<String, int> readContexts = const {},
    Set<String> locallyForcedChannelIds = const {},
    bool readStateReady = true,
    Map<String, int> highPriorityMap = const {},
  }) {
    final notifier = _StubbedChannelsNotifier(
      channels: channels,
      highPriorityMap: highPriorityMap,
    );

    return ProviderContainer(
      overrides: [
        channelsProvider.overrideWith(() => notifier),
        readStateProvider.overrideWith(
          () => _ReadStateNotifier(
            ReadStateState(
              isReady: readStateReady,
              pubkey: 'me',
              contexts: readContexts,
              version: 1,
              locallyForcedChannelIds: locallyForcedChannelIds,
            ),
          ),
        ),
      ],
    );
  }

  test('all channels read returns (0, 0)', () async {
    final container = buildContainer(
      channels: [
        makeChannel(id: 'ch-a', lastMessageAtSeconds: t20),
        makeChannel(id: 'ch-b', lastMessageAtSeconds: t20),
      ],
      // Read at t30, which is after every message.
      readContexts: {'ch-a': t30, 'ch-b': t30},
    );
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    final badge = container.read(unreadBadgeProvider);
    expect(badge.highPriorityCount, 0);
    expect(badge.generalUnreadCount, 0);
  });

  test(
    'one unread non-DM channel with no high-priority event → (0, 1) general only',
    () async {
      final container = buildContainer(
        channels: [makeChannel(id: 'ch-a', lastMessageAtSeconds: t20)],
        readContexts: {},
        highPriorityMap: {},
      );
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      final badge = container.read(unreadBadgeProvider);
      expect(badge.highPriorityCount, 0);
      expect(badge.generalUnreadCount, 1);
    },
  );

  test('one unread DM channel → (1, 0) high priority', () async {
    final container = buildContainer(
      channels: [
        makeChannel(id: 'dm-a', channelType: 'dm', lastMessageAtSeconds: t20),
      ],
      readContexts: {},
    );
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    final badge = container.read(unreadBadgeProvider);
    expect(badge.highPriorityCount, 1);
    expect(badge.generalUnreadCount, 0);
  });

  test(
    'one unread non-DM with unread high-priority @mention → (1, 0) high priority',
    () async {
      const channelId = 'ch-a';
      final container = buildContainer(
        channels: [makeChannel(id: channelId, lastMessageAtSeconds: t20)],
        // Read up through t10; both the message (t20) and @mention (t30) are
        // newer than the read marker.
        readContexts: {channelId: t10},
        highPriorityMap: {channelId: t30},
      );
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      final badge = container.read(unreadBadgeProvider);
      expect(badge.highPriorityCount, 1);
      expect(badge.generalUnreadCount, 0);
    },
  );

  test(
    'mixed: 1 unread DM + 2 unread non-DMs (no high-priority) → (1, 2)',
    () async {
      final container = buildContainer(
        channels: [
          makeChannel(id: 'dm-a', channelType: 'dm', lastMessageAtSeconds: t20),
          makeChannel(id: 'ch-b', lastMessageAtSeconds: t20),
          makeChannel(id: 'ch-c', lastMessageAtSeconds: t20),
        ],
        readContexts: {},
        highPriorityMap: {},
      );
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      final badge = container.read(unreadBadgeProvider);
      expect(badge.highPriorityCount, 1);
      expect(badge.generalUnreadCount, 2);
    },
  );

  test('archived channel is excluded even if unread', () async {
    final container = buildContainer(
      channels: [
        makeChannel(
          id: 'ch-archived',
          isArchived: true,
          lastMessageAtSeconds: t20,
        ),
        makeChannel(id: 'ch-active', lastMessageAtSeconds: t20),
      ],
      readContexts: {},
    );
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    final badge = container.read(unreadBadgeProvider);
    // Archived channel must not count; active one does.
    expect(badge.highPriorityCount, 0);
    expect(badge.generalUnreadCount, 1);
  });

  test('non-member channel is excluded even if unread', () async {
    final container = buildContainer(
      channels: [
        makeChannel(
          id: 'ch-nonmember',
          isMember: false,
          lastMessageAtSeconds: t20,
        ),
        makeChannel(id: 'ch-member', lastMessageAtSeconds: t20),
      ],
      readContexts: {},
    );
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    final badge = container.read(unreadBadgeProvider);
    expect(badge.highPriorityCount, 0);
    expect(badge.generalUnreadCount, 1);
  });

  test('channel with lastMessageAt == null is excluded', () async {
    final container = buildContainer(
      channels: [
        // No lastMessageAt — provider treats this as having no messages.
        makeChannel(id: 'ch-nomsg'),
        makeChannel(id: 'ch-withmsg', lastMessageAtSeconds: t20),
      ],
      readContexts: {},
    );
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    final badge = container.read(unreadBadgeProvider);
    expect(badge.highPriorityCount, 0);
    expect(badge.generalUnreadCount, 1);
  });

  test(
    'read state not ready (isReady: false) does not suppress unread counts',
    () async {
      // The provider does not gate on isReady — it computes from whatever
      // timestamps are in contexts. With an empty context map and readStateReady=false,
      // readAt is null → channel is unread → (0, 1).
      final container = buildContainer(
        channels: [makeChannel(id: 'ch-a', lastMessageAtSeconds: t20)],
        readContexts: {},
        readStateReady: false,
      );
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      final badge = container.read(unreadBadgeProvider);
      expect(badge.highPriorityCount, 0);
      expect(badge.generalUnreadCount, 1);
    },
  );

  test(
    'locally forced channel counts unread without publishing rollback',
    () async {
      final container = buildContainer(
        channels: [makeChannel(id: 'ch-a', lastMessageAtSeconds: t20)],
        readContexts: {'ch-a': t30},
        locallyForcedChannelIds: {'ch-a'},
      );
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      final badge = container.read(unreadBadgeProvider);
      expect(badge.highPriorityCount, 0);
      expect(badge.generalUnreadCount, 1);
    },
  );

  test('channelsProvider in loading state returns (0, 0)', () {
    // The provider returns const UnreadBadgeState() while channels are loading.
    // We intentionally do NOT await the future here — the channels notifier
    // never resolves (Completer never completes) so the state stays AsyncLoading.
    final container = ProviderContainer(
      overrides: [
        channelsProvider.overrideWith(() => _LoadingChannelsNotifier()),
        readStateProvider.overrideWith(
          () => _ReadStateNotifier(
            const ReadStateState(
              isReady: true,
              pubkey: 'me',
              contexts: {},
              version: 1,
            ),
          ),
        ),
      ],
    );
    addTearDown(container.dispose);

    final badge = container.read(unreadBadgeProvider);
    expect(badge.highPriorityCount, 0);
    expect(badge.generalUnreadCount, 0);
  });

  test(
    'high-priority event older than read marker falls through to general bucket',
    () async {
      // @mention arrived at t10, user read at t20 (after the mention),
      // then a new general message arrived at t30. Channel is unread but
      // the mention is already read → falls through to general.
      const channelId = 'ch-a';
      final container = buildContainer(
        channels: [makeChannel(id: channelId, lastMessageAtSeconds: t30)],
        readContexts: {channelId: t20},
        highPriorityMap: {channelId: t10}, // mention is older than read marker
      );
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      final badge = container.read(unreadBadgeProvider);
      expect(badge.highPriorityCount, 0);
      expect(badge.generalUnreadCount, 1);
    },
  );
}

/// A [ChannelsNotifier] that immediately resolves to a canned [channels] list
/// and exposes a pre-seeded [latestHighPriorityByChannel] map.
///
/// Extends [ChannelsNotifier] so [ref.read(channelsProvider.notifier)] returns
/// an instance whose [latestHighPriorityByChannel] getter works correctly.
class _StubbedChannelsNotifier extends ChannelsNotifier {
  _StubbedChannelsNotifier({
    required List<Channel> channels,
    Map<String, int> highPriorityMap = const {},
  }) : _channels = channels,
       _highPriorityMap = Map<String, int>.unmodifiable(highPriorityMap);

  final List<Channel> _channels;
  final Map<String, int> _highPriorityMap;

  @override
  Future<List<Channel>> build() async => _channels;

  @override
  Map<String, int> get latestHighPriorityByChannel => _highPriorityMap;
}

/// A [ChannelsNotifier] that stays in the loading state indefinitely.
class _LoadingChannelsNotifier extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() => Completer<List<Channel>>().future;

  @override
  Map<String, int> get latestHighPriorityByChannel => const {};
}

/// A [ReadStateNotifier] that returns a fixed [ReadStateState].
class _ReadStateNotifier extends ReadStateNotifier {
  _ReadStateNotifier(this._fixedState);

  final ReadStateState _fixedState;

  @override
  ReadStateState build() => _fixedState;
}
