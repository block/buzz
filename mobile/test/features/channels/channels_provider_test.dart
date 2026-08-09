import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/relay/relay.dart';

/// Tests for [ChannelsNotifier] in the pure-Nostr world.
///
/// The provider performs a two-step WS query:
///   1. kind:39002 memberships tagged `#p:<my-pubkey>`
///   2. kind:39000 metadata for those channel ids
/// then layers per-channel live subscriptions on the `#h` tag.
///
/// Tests stub out the relay session by overriding [relaySessionProvider] with
/// a [_FakeRelaySession] that returns canned events from [fetchHistory] and
/// records [subscribe] calls so we can assert filter shapes and emit live
/// events on demand.
void main() {
  const myPk = 'me';

  test(
    'subscribes per-channel with #h tags (only joined, non-archived)',
    () async {
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, myPk),
          _membership(_channelB, myPk),
          _membership(_channelD, myPk),
        ],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(id: _channelB, name: 'random'),
          // channelD metadata missing -> won't appear in channel list
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);

      // One subscription per joined, non-archived channel.
      expect(session.subscribeFilters, hasLength(2));
      expect(
        session.subscribeFilters.map((f) => f.tags['#h']?.single).toSet(),
        {_channelA, _channelB},
      );
      for (final filter in session.subscribeFilters) {
        expect(filter.kinds, EventKind.channelEventKinds);
        expect(filter.limit, 0);
      }
    },
  );

  test(
    'refreshing an unchanged channel set issues zero new live REQs',
    () async {
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, myPk),
          _membership(_channelB, myPk),
        ],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(id: _channelB, name: 'random'),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      final initialSubscribeCount = session.totalSubscribeCount;

      await container.read(channelsProvider.notifier).refresh();

      expect(session.totalSubscribeCount, initialSubscribeCount);
      expect(session.unsubscribeCount, 0);
      expect(session.subscribeFilters, hasLength(2));
    },
  );

  test(
    'live subscription diff only removes and adds changed channels',
    () async {
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, myPk),
          _membership(_channelB, myPk),
        ],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(id: _channelB, name: 'random'),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      session.memberships = [
        _membership(_channelB, myPk),
        _membership(_channelD, myPk),
      ];
      session.metadata = [
        _meta(id: _channelB, name: 'random'),
        _meta(id: _channelD, name: 'support'),
      ];

      await container.read(channelsProvider.notifier).refresh();

      expect(session.totalSubscribeCount, 3);
      expect(session.unsubscribeCount, 1);
      expect(
        session.subscribeFilters
            .map((filter) => filter.tags['#h']!.single)
            .toSet(),
        {_channelB, _channelD},
      );
    },
  );

  test(
    'empty channel refresh removes every retained live subscription',
    () async {
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, myPk),
          _membership(_channelB, myPk),
        ],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(id: _channelB, name: 'random'),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      session.memberships = [];
      session.metadata = [];

      await container.read(channelsProvider.notifier).refresh();

      expect(session.activeChannels, isEmpty);
      expect(session.activeSubscriptionCount, 0);
      expect(session.unsubscribeCount, 2);
    },
  );

  test(
    'overlapping refreshes retain one live subscription per desired channel',
    () async {
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, myPk),
          _membership(_channelB, myPk),
        ],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(id: _channelB, name: 'random'),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      session.pauseNextSubscribe();
      session.memberships = [
        _membership(_channelA, myPk),
        _membership(_channelB, myPk),
        _membership(_channelD, myPk),
      ];
      session.metadata = [
        _meta(id: _channelA, name: 'general'),
        _meta(id: _channelB, name: 'random'),
        _meta(id: _channelD, name: 'support'),
      ];

      final firstRefresh = container.read(channelsProvider.notifier).refresh();
      await session.nextSubscribeStarted;
      final secondRefresh = container.read(channelsProvider.notifier).refresh();
      session.resumePausedSubscribe();
      await Future.wait([firstRefresh, secondRefresh]);

      expect(session.activeChannels, {_channelA, _channelB, _channelD});
      expect(session.activeSubscriptionCount, 3);
    },
  );

  test(
    'community switch replaces retained live subscriptions on the new relay',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      expect(session.activeChannels, {_channelA});

      session.setStatus(SessionStatus.disconnected);
      session.memberships = [_membership(_channelB, myPk)];
      session.metadata = [_meta(id: _channelB, name: 'random')];
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://new-community.example');
      await Future<void>.delayed(Duration.zero);
      session.setStatus(SessionStatus.connected);
      await container.read(channelsProvider.future);
      await _waitUntil(
        () =>
            session.activeChannels.length == 1 &&
            session.activeChannels.contains(_channelB),
      );

      expect(session.activeChannels, {_channelB});
      expect(session.activeSubscriptionCount, 1);
      expect(session.unsubscribeCount, 1);
    },
  );

  test('live channel events update channel lastMessageAt', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'general', createdAt: 10)],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);

    // Emit a live message event on channelA.
    session.emit(
      NostrEvent(
        id: 'event-1',
        pubkey: 'alice',
        createdAt: 20,
        kind: EventKind.streamMessageV2,
        tags: const [
          ['h', _channelA],
        ],
        content: 'new message',
        sig: 'sig',
      ),
    );

    final channels = container.read(channelsProvider).value!;
    expect(channels.single.lastMessageAt?.millisecondsSinceEpoch, 20 * 1000);
  });

  test('ephemeral (TTL) channels appear in the list', () async {
    // Regression: previously the provider unconditionally dropped any channel
    // with a `ttl` tag, which made TTL channels invisible on iOS even when the
    // user was a member. They should be included so the existing
    // `_EphemeralBadge` UI in `channels_page.dart` can render them.
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk), _membership(_channelB, myPk)],
      metadata: [
        _meta(id: _channelA, name: 'general'),
        _meta(
          id: _channelB,
          name: 'agent-creation-deep-dive',
          ttlSeconds: 86400,
        ),
      ],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    final channels = await container.read(channelsProvider.future);

    expect(
      channels.map((c) => c.name),
      containsAll(['general', 'agent-creation-deep-dive']),
    );
    final ephemeral = channels.firstWhere(
      (c) => c.name == 'agent-creation-deep-dive',
    );
    expect(ephemeral.isEphemeral, isTrue);
    expect(ephemeral.ttlSeconds, 86400);
  });

  test('hidden DMs are filtered from the channel list', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk), _membership(_channelB, myPk)],
      metadata: [
        _meta(id: _channelA, name: 'Alice', channelType: 'dm'),
        _meta(id: _channelB, name: 'Bob', channelType: 'dm'),
      ],
      hiddenDmEvents: [
        _hiddenDms([_channelA], pubkey: myPk),
      ],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    final channels = await container.read(channelsProvider.future);

    expect(channels.map((c) => c.id), [_channelB]);
    expect(
      session.historyFilters.any(
        (filter) =>
            filter.kinds.contains(EventKind.dmVisibility) &&
            filter.tags['#p']?.single == myPk,
      ),
      isTrue,
    );
  });

  test(
    'archived kind:39000 metadata sets Channel.isArchived (covers TTL auto-archive)',
    () async {
      // The relay's TTL reaper auto-archives expired ephemeral channels and
      // republishes kind:39000 with `["archived", "true"]`. The Channel needs
      // `archivedAt != null` so the `_SliverChannelsList` filter
      // (`!channel.isArchived`) hides it from the sidebar after expiry.
      // Previously the mobile parser ignored the `archived` tag, so expired
      // TTL channels would have stayed visible after the `!isEphemeral` guard
      // was removed.
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, myPk),
          _membership(_channelB, myPk),
        ],
        metadata: [
          _meta(id: _channelA, name: 'active'),
          _meta(
            id: _channelB,
            name: 'expired-ttl',
            ttlSeconds: 86400,
            archived: true,
          ),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      final channels = await container.read(channelsProvider.future);
      final expired = channels.firstWhere((c) => c.name == 'expired-ttl');
      expect(expired.isArchived, isTrue);
      expect(expired.isEphemeral, isTrue);
      // The active channel must not be flagged archived.
      final active = channels.firstWhere((c) => c.name == 'active');
      expect(active.isArchived, isFalse);
    },
  );

  test(
    'archive transition invalidates cached channelDetailsProvider',
    () async {
      // Codex review v2 caught: if a TTL channel is opened (caching its
      // ChannelDetails) and then the reaper archives it, the cached details
      // — built from the pre-archive kind:39000 — would clobber the newer
      // archivedAt set on the base Channel during `mergeDetails`. We invalidate
      // the details provider when the archived state flips so the next
      // mergeDetails sees fresh data.
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'active')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      // Initial load.
      final initial = await container.read(channelsProvider.future);
      expect(initial.single.isArchived, isFalse);

      // Prime the detail cache.
      final detailsFiltersBefore = session.historyFilters
          .where((f) => f.kinds.contains(39000) && f.tags['#d'] != null)
          .length;
      await container.read(channelDetailsProvider(_channelA).future);
      final detailsFetchesAfterPrime =
          session.historyFilters
              .where((f) => f.kinds.contains(39000) && f.tags['#d'] != null)
              .length -
          detailsFiltersBefore;
      expect(detailsFetchesAfterPrime, 1);

      // Simulate the reaper auto-archiving the channel by swapping the
      // metadata the fake returns, then refreshing the channels provider.
      session.metadata
        ..clear()
        ..add(_meta(id: _channelA, name: 'active', archived: true));
      await container.read(channelsProvider.notifier).refresh();
      final refreshed = container.read(channelsProvider).value!;
      expect(refreshed.single.isArchived, isTrue);

      // Take a fresh baseline AFTER the refresh — the refresh itself issues a
      // `kinds:[39000], #d:[id]` query as part of channel metadata refetch and
      // we must not count that toward our invalidation assertion. Only the
      // fetch triggered by the second `channelDetailsProvider` read should be
      // attributed to invalidation.
      final detailsFiltersAfterRefresh = session.historyFilters
          .where((f) => f.kinds.contains(39000) && f.tags['#d'] != null)
          .length;

      // Reading the details provider again must trigger a fresh fetch — proving
      // the prior cache was invalidated by the archive transition. Without
      // invalidation, Riverpod would return the cached pre-archive details and
      // no new `kinds:[39000], #d:[id]` filter would be sent.
      await container.read(channelDetailsProvider(_channelA).future);
      final detailsFetchesFromInvalidation =
          session.historyFilters
              .where((f) => f.kinds.contains(39000) && f.tags['#d'] != null)
              .length -
          detailsFiltersAfterRefresh;
      expect(detailsFetchesFromInvalidation, greaterThan(0));
    },
  );

  test(
    'keeps cached channels and live subscriptions during reconnect',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      final initial = await container.read(channelsProvider.future);
      expect(initial.single.name, 'general');
      expect(session.subscribeFilters, hasLength(1));

      session.setStatus(SessionStatus.reconnecting);
      final reconnecting = await container.read(channelsProvider.future);

      expect(reconnecting.single.name, 'general');
      expect(session.subscribeFilters, hasLength(1));
      expect(session.unsubscribeCount, 0);
    },
  );

  test(
    'refreshes cached channels after a disconnected community switch',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(
        (await container.read(channelsProvider.future)).single.name,
        'general',
      );

      session.setStatus(SessionStatus.disconnected);
      session.memberships = [_membership(_channelB, myPk)];
      session.metadata = [_meta(id: _channelB, name: 'random')];
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://new-community.example');
      await Future<void>.delayed(Duration.zero);
      expect(container.read(channelsProvider).value?.single.name, 'general');

      session.setStatus(SessionStatus.connected);
      await Future<void>.delayed(Duration.zero);

      expect(container.read(channelsProvider).value?.single.name, 'random');
    },
  );

  test('recovers an initial fetch failure after reconnecting', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'general')],
      membershipFailures: 1,
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await expectLater(container.read(channelsProvider.future), throwsException);

    session.setStatus(SessionStatus.reconnecting);
    session.setStatus(SessionStatus.connected);
    await Future<void>.delayed(Duration.zero);

    final recovered = await container.read(channelsProvider.future);
    expect(recovered.single.name, 'general');
  });

  test(
    'preserves a successfully loaded empty list while disconnected',
    () async {
      final session = _FakeRelaySession(memberships: [], metadata: []);
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(await container.read(channelsProvider.future), isEmpty);
      final fetchCount = session.historyFilters.length;

      session.setStatus(SessionStatus.reconnecting);
      expect(await container.read(channelsProvider.future), isEmpty);
      expect(session.historyFilters, hasLength(fetchCount));
    },
  );

  test('initial fetch issues membership + metadata queries', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'general')],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);

    // Two history fetches for channel loading, plus one per non-DM channel
    // for high-priority event backfill.
    expect(session.historyFilters.length, greaterThanOrEqualTo(2));
    expect(session.historyFilters[0].kinds, [39002]);
    expect(session.historyFilters[0].tags['#p'], [myPk]);
    expect(session.historyFilters[1].kinds, [39000]);
    expect(session.historyFilters[1].tags['#d'], [_channelA]);

    // And one live subscription on the resulting channel.
    expect(session.subscribeFilters, hasLength(1));
  });
}

const _channelA = '11111111-1111-4111-8111-111111111111';
const _channelB = '22222222-2222-4222-8222-222222222222';
const _channelD = '44444444-4444-4444-8444-444444444444';

/// Build a kind:39002 membership event tagged with the channel id and member.
NostrEvent _membership(String channelId, String pubkey) => NostrEvent(
  id: 'mem-$channelId',
  pubkey: 'creator',
  createdAt: 1,
  kind: 39002,
  tags: [
    ['d', channelId],
    ['p', pubkey],
  ],
  content: '',
  sig: 'sig',
);

NostrEvent _hiddenDms(List<String> channelIds, {required String pubkey}) =>
    NostrEvent(
      id: 'hidden-${channelIds.join('-')}',
      pubkey: 'relay',
      createdAt: 2,
      kind: EventKind.dmVisibility,
      tags: [
        ['d', pubkey],
        ['p', pubkey],
        for (final channelId in channelIds) ['h', channelId],
      ],
      content: '',
      sig: 'sig',
    );

/// Build a kind:39000 channel metadata event.
NostrEvent _meta({
  required String id,
  required String name,
  String channelType = 'stream',
  int createdAt = 1,
  int? ttlSeconds,
  bool archived = false,
}) => NostrEvent(
  id: 'meta-$id',
  pubkey: 'creator',
  createdAt: createdAt,
  kind: 39000,
  tags: [
    ['d', id],
    ['name', name],
    ['t', channelType],
    ['public'],
    if (ttlSeconds != null) ['ttl', '$ttlSeconds'],
    if (archived) ['archived', 'true'],
  ],
  content: '',
  sig: 'sig',
);

ProviderContainer _buildContainer({required _FakeRelaySession session}) {
  return ProviderContainer(
    retry: (_, _) => null,
    overrides: [
      appLifecycleProvider.overrideWith(() => _FakeAppLifecycleNotifier()),
      relaySessionProvider.overrideWith(() => session),
      myPubkeyProvider.overrideWithValue('me'),
    ],
  );
}

Future<void> _waitUntil(bool Function() predicate) async {
  for (var i = 0; i < 100; i++) {
    if (predicate()) return;
    await Future<void>.delayed(Duration.zero);
  }
  fail('Timed out waiting for asynchronous provider work');
}

/// Fake [RelaySessionNotifier] that returns canned events from [fetchHistory]
/// and records subscribe calls.
class _FakeRelaySession extends RelaySessionNotifier {
  _FakeRelaySession({
    required this.memberships,
    required this.metadata,
    this.hiddenDmEvents = const [],
    this.membershipFailures = 0,
  });

  List<NostrEvent> memberships;
  List<NostrEvent> metadata;
  final List<NostrEvent> hiddenDmEvents;
  int membershipFailures;

  final List<NostrFilter> historyFilters = [];
  final List<NostrFilter> subscribeFilters = [];
  final Map<int, (NostrFilter, void Function(NostrEvent))> _subscriptions = {};
  int _nextSubscriptionKey = 0;
  Completer<void>? _pausedSubscribe;
  Completer<void>? _subscribeStarted;
  int unsubscribeCount = 0;
  int totalSubscribeCount = 0;

  Set<String> get activeChannels => {
    for (final (filter, _) in _subscriptions.values) ?filter.tags['#h']?.single,
  };

  int get activeSubscriptionCount => _subscriptions.length;

  Future<void> get nextSubscribeStarted async {
    final started = _subscribeStarted;
    if (started == null) {
      throw StateError('No paused subscription is pending');
    }
    await started.future;
  }

  void pauseNextSubscribe() {
    if (_pausedSubscribe != null) {
      throw StateError('A subscription is already paused');
    }
    _pausedSubscribe = Completer<void>();
    _subscribeStarted = Completer<void>();
  }

  void resumePausedSubscribe() {
    final paused = _pausedSubscribe;
    if (paused == null) throw StateError('No subscription is paused');
    paused.complete();
  }

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    historyFilters.add(filter);
    if (filter.kinds.contains(39002) && filter.tags['#p'] != null) {
      if (membershipFailures > 0) {
        membershipFailures--;
        throw Exception('membership fetch failed');
      }
      // Membership query — return all memberships we have for this pubkey.
      final myPk = filter.tags['#p']?.single;
      return memberships
          .where(
            (e) =>
                e.tags.any((t) => t.length >= 2 && t[0] == 'p' && t[1] == myPk),
          )
          .toList();
    }
    if (filter.kinds.contains(EventKind.dmVisibility)) {
      return hiddenDmEvents;
    }
    if (filter.kinds.contains(39000)) {
      // Metadata query — return all metadata events whose `d` tag matches.
      final ids = (filter.tags['#d'] ?? const <String>[]).toSet();
      return metadata.where((e) => ids.contains(e.getTagValue('d'))).toList();
    }
    return const [];
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    totalSubscribeCount++;
    subscribeFilters.add(filter);
    final paused = _pausedSubscribe;
    if (paused != null) {
      _subscribeStarted!.complete();
      await paused.future;
      _pausedSubscribe = null;
      _subscribeStarted = null;
    }
    final subscriptionKey = ++_nextSubscriptionKey;
    _subscriptions[subscriptionKey] = (filter, onEvent);
    return () {
      final subscription = _subscriptions.remove(subscriptionKey);
      if (subscription == null) return;
      unsubscribeCount++;
      subscribeFilters.remove(subscription.$1);
    };
  }

  void setStatus(SessionStatus status) {
    state = SessionState(status: status);
  }

  /// Emit a live event to all subscribers.
  void emit(NostrEvent event) {
    for (final (_, listener) in List.of(_subscriptions.values)) {
      listener(event);
    }
  }
}

class _FakeAppLifecycleNotifier extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}
