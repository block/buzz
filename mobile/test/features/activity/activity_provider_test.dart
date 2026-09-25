import 'dart:async';

import 'package:buzz/features/activity/activity_provider.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

/// Records subscriptions and DM history queries for Activity projection tests.
class _RecordingSessionNotifier extends RelaySessionNotifier {
  final List<List<String>> dmQueries = [];
  final List<int> queryFilterCounts = [];
  final List<NostrEvent> _history = [];
  final List<({NostrFilter filter, void Function(NostrEvent) onEvent})>
  _subscriptions = [];
  Completer<void>? mentionFetchGate;
  bool failNextMentionFetch = false;
  bool failNextQueryRelay = false;
  int mentionFetchCount = 0;
  int activeMentionFetches = 0;
  int maxActiveMentionFetches = 0;
  Completer<void>? hiddenSubscribeGate;
  bool failNextHiddenSubscribe = false;
  int hiddenUnsubscribeCount = 0;
  // Per-batch gates keyed by hidden-subscribe call order (0-based), so a test
  // can park an individual batch's REQ while letting earlier ones settle.
  final Map<int, Completer<void>> hiddenSubscribeGatesByCall = {};
  int hiddenSubscribeCallCount = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final h = filter.tags['#h'];
    if (h != null) dmQueries.add(h);
    final isMentionFetch =
        filter.tags.containsKey('#p') && filter.kinds.contains(40002);
    if (isMentionFetch) {
      mentionFetchCount += 1;
      activeMentionFetches += 1;
      if (activeMentionFetches > maxActiveMentionFetches) {
        maxActiveMentionFetches = activeMentionFetches;
      }
      try {
        final gate = mentionFetchGate;
        if (gate != null) await gate.future;
        if (failNextMentionFetch) {
          failNextMentionFetch = false;
          throw StateError('transient mention history failure');
        }
      } finally {
        activeMentionFetches -= 1;
      }
    }
    return _history.where((event) => _matches(filter, event)).toList();
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    queryFilterCounts.add(filters.length);
    if (failNextQueryRelay) {
      failNextQueryRelay = false;
      throw StateError('transient HTTP query failure');
    }
    for (final filter in filters) {
      final h = filter.tags['#h'];
      if (h != null) dmQueries.add(h);
    }
    final isMentionFetch = filters.any(
      (filter) => filter.tags.containsKey('#p') && filter.kinds.contains(40002),
    );
    if (isMentionFetch) {
      mentionFetchCount += 1;
      activeMentionFetches += 1;
      if (activeMentionFetches > maxActiveMentionFetches) {
        maxActiveMentionFetches = activeMentionFetches;
      }
      try {
        final gate = mentionFetchGate;
        if (gate != null) await gate.future;
        if (failNextMentionFetch) {
          failNextMentionFetch = false;
          throw StateError('transient mention history failure');
        }
      } finally {
        activeMentionFetches -= 1;
      }
    }
    return _history
        .where((event) => filters.any((filter) => _matches(filter, event)))
        .toList();
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    // A hidden-DM resurface subscription carries the full channel-message kind
    // set plus an `#h` tag; the visible-DM sub uses only kind 9. Batching and
    // teardown assertions look only at the hidden-DM subscriptions.
    final isHidden =
        filter.tags.containsKey('#h') &&
        filter.kinds.length == EventKind.channelMessageEventKinds.length;
    if (isHidden) {
      final callIndex = hiddenSubscribeCallCount++;
      final perCallGate = hiddenSubscribeGatesByCall[callIndex];
      if (perCallGate != null) await perCallGate.future;
      final gate = hiddenSubscribeGate;
      if (gate != null) await gate.future;
      if (failNextHiddenSubscribe) {
        failNextHiddenSubscribe = false;
        throw StateError('relay rejected hidden-DM subscription');
      }
    }
    final subscription = (filter: filter, onEvent: onEvent);
    _subscriptions.add(subscription);
    return () {
      if (isHidden) hiddenUnsubscribeCount += 1;
      _subscriptions.remove(subscription);
    };
  }

  /// The `#h` value lists of every registered hidden-DM subscription, in order.
  List<List<String>> get hiddenDmSubscriptionBatches => [
    for (final subscription in _subscriptions)
      if (subscription.filter.tags.containsKey('#h') &&
          subscription.filter.kinds.length ==
              EventKind.channelMessageEventKinds.length)
        subscription.filter.tags['#h']!,
  ];

  /// The `#h` value lists of every registered visible-DM subscription (kind 9
  /// only), in order.
  List<List<String>> get visibleDmSubscriptionBatches => [
    for (final subscription in _subscriptions)
      if (subscription.filter.tags.containsKey('#h') &&
          subscription.filter.kinds.length == 1 &&
          subscription.filter.kinds.single == 9)
        subscription.filter.tags['#h']!,
  ];

  void emit(NostrEvent event) {
    _history.add(event);
    for (final subscription in List.of(_subscriptions)) {
      if (_matches(subscription.filter, event)) {
        subscription.onEvent(event);
      }
    }
  }

  void seed(NostrEvent event) => _history.add(event);

  bool _matches(NostrFilter filter, NostrEvent event) {
    if (!filter.kinds.contains(event.kind)) return false;
    for (final entry in filter.tags.entries) {
      final tagName = entry.key.startsWith('#')
          ? entry.key.substring(1)
          : entry.key;
      final matchesTag = event.tags.any(
        (tag) =>
            tag.length > 1 && tag[0] == tagName && entry.value.contains(tag[1]),
      );
      if (!matchesTag) return false;
    }
    return true;
  }
}

/// Channels provider that starts loading and resolves on demand, modelling a
/// cold start where the channel list arrives after Activity's first fetch.
class _LateChannelsNotifier extends ChannelsNotifier {
  final Completer<List<Channel>> _completer = Completer<List<Channel>>();

  @override
  Future<List<Channel>> build() => _completer.future;

  void resolve(List<Channel> channels) => _completer.complete(channels);
}

class _FixedRelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() =>
      const RelayConfig(baseUrl: 'https://relay.example', nsec: null);
}

Channel _dmChannel(String id) => Channel(
  id: id,
  name: 'dm',
  channelType: 'dm',
  visibility: 'private',
  description: '',
  createdBy: 'x',
  createdAt: DateTime(2025),
  memberCount: 2,
  isMember: true,
);

NostrEvent _mentionEvent(String id, int createdAt) => NostrEvent(
  id: id,
  pubkey: 'other_pk',
  createdAt: createdAt,
  kind: 40002,
  tags: const [
    ['p', 'me_pk'],
    ['h', 'channel-1'],
  ],
  content: 'Hello from the live relay',
  sig: '',
);

Future<void> _waitFor(bool Function() predicate) async {
  for (var attempt = 0; attempt < 100; attempt++) {
    if (predicate()) return;
    await Future<void>.delayed(const Duration(milliseconds: 10));
  }
  fail('Condition was not reached before timeout');
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('refetches and includes DMs when channels resolve after first '
      'fetch (cold start)', () async {
    final session = _RecordingSessionNotifier();
    final channels = _LateChannelsNotifier();
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(_FixedRelayConfigNotifier.new),
        myPubkeyProvider.overrideWithValue('me_pk'),
        relaySessionProvider.overrideWith(() => session),
        channelsProvider.overrideWith(() => channels),
      ],
    );
    addTearDown(container.dispose);

    // Cold start: channels still loading, so the first fetch has no DM ids.
    await container.read(activityProvider.future);
    expect(session.dmQueries, isEmpty);
    expect(session.queryFilterCounts, [3]);

    // Channel list resolves with a DM → Activity must rebuild and query it.
    channels.resolve([_dmChannel('dm1')]);
    await container.read(channelsProvider.future);
    await container.read(activityProvider.future);

    expect(session.dmQueries, hasLength(1));
    expect(session.dmQueries.single, ['dm1']);
    expect(session.queryFilterCounts, [3, 4]);
  });

  test('does not query DMs when the resolved channel list has none', () async {
    final session = _RecordingSessionNotifier();
    final channels = _LateChannelsNotifier();
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(_FixedRelayConfigNotifier.new),
        myPubkeyProvider.overrideWithValue('me_pk'),
        relaySessionProvider.overrideWith(() => session),
        channelsProvider.overrideWith(() => channels),
      ],
    );
    addTearDown(container.dispose);

    await container.read(activityProvider.future);
    channels.resolve(const []);
    await container.read(channelsProvider.future);
    await container.read(activityProvider.future);

    expect(session.dmQueries, isEmpty);
  });

  test('falls back to websocket history when the HTTP batch fails', () async {
    final session = _RecordingSessionNotifier()
      ..seed(_mentionEvent('fallback-mention', 1_700_000_001))
      ..failNextQueryRelay = true;
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(_FixedRelayConfigNotifier.new),
        myPubkeyProvider.overrideWithValue('me_pk'),
        relaySessionProvider.overrideWith(() => session),
        channelsProvider.overrideWith(
          () => _FixedChannelsNotifier(const <Channel>[]),
        ),
      ],
    );
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    final feed = await container.read(activityProvider.future);

    expect(session.queryFilterCounts, [3]);
    expect(session.mentionFetchCount, 1);
    expect(feed.mentions.map((item) => item.id), ['fallback-mention']);
  });

  test(
    'refreshes the inbox projection when addressed activity arrives',
    () async {
      final session = _RecordingSessionNotifier();
      final container = ProviderContainer(
        overrides: [
          relayConfigProvider.overrideWith(_FixedRelayConfigNotifier.new),
          myPubkeyProvider.overrideWithValue('me_pk'),
          relaySessionProvider.overrideWith(() => session),
          channelsProvider.overrideWith(
            () => _FixedChannelsNotifier(const <Channel>[]),
          ),
        ],
      );
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      await container.read(activityProvider.future);
      await Future<void>.delayed(const Duration(milliseconds: 10));
      expect(container.read(inboxItemsProvider), isEmpty);

      session.emit(
        const NostrEvent(
          id: 'live-mention',
          pubkey: 'other_pk',
          createdAt: 1_700_000_000,
          kind: 40002,
          tags: [
            ['p', 'me_pk'],
            ['h', 'channel-1'],
          ],
          content: 'Hello from the live relay',
          sig: '',
        ),
      );
      await Future<void>.delayed(const Duration(milliseconds: 100));

      expect(container.read(inboxItemsProvider).single.id, 'live-mention');
    },
  );

  test(
    'live activity resurfaces a hidden DM through the existing open action',
    () async {
      const self =
          '1111111111111111111111111111111111111111111111111111111111111111';
      const alice =
          '2222222222222222222222222222222222222222222222222222222222222222';
      const bob =
          '3333333333333333333333333333333333333333333333333333333333333333';
      final session = _RecordingSessionNotifier();
      final reopened = <List<String>>[];
      final container = ProviderContainer(
        overrides: [
          relayConfigProvider.overrideWith(_FixedRelayConfigNotifier.new),
          myPubkeyProvider.overrideWithValue(self),
          relaySessionProvider.overrideWith(() => session),
          channelsProvider.overrideWith(
            () => _FixedChannelsNotifier(
              const <Channel>[],
              hiddenDmIds: const {'hidden-dm'},
            ),
          ),
          channelMembersProvider('hidden-dm').overrideWith(
            (ref) async => [
              ChannelMember(
                pubkey: self,
                role: 'member',
                joinedAt: DateTime(2026),
              ),
              ChannelMember(
                pubkey: alice,
                role: 'member',
                joinedAt: DateTime(2026),
              ),
              ChannelMember(
                pubkey: bob,
                role: 'member',
                joinedAt: DateTime(2026),
              ),
            ],
          ),
          dmResurfaceActionProvider.overrideWithValue((pubkeys) async {
            reopened.add(pubkeys);
            return 'hidden-dm';
          }),
        ],
      );
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      await container.read(activityProvider.future);
      await Future<void>.delayed(const Duration(milliseconds: 10));

      session.emit(
        const NostrEvent(
          id: 'hidden-dm-message',
          pubkey: alice,
          createdAt: 1_700_000_000,
          kind: EventKind.streamMessageV2,
          tags: [
            ['p', self],
            ['h', 'hidden-dm'],
          ],
          content: 'Hello again',
          sig: '',
        ),
      );

      await _waitFor(() => reopened.isNotEmpty);
      expect(reopened, [
        [alice, bob],
      ]);
    },
  );

  test(
    'queues a hidden DM message until channel discovery completes',
    () async {
      const self =
          '1111111111111111111111111111111111111111111111111111111111111111';
      const alice =
          '2222222222222222222222222222222222222222222222222222222222222222';
      final session = _RecordingSessionNotifier();
      late _DeferredChannelsNotifier channels;
      final reopened = <List<String>>[];
      final container = ProviderContainer(
        overrides: [
          relayConfigProvider.overrideWith(_FixedRelayConfigNotifier.new),
          myPubkeyProvider.overrideWithValue(self),
          relaySessionProvider.overrideWith(() => session),
          channelsProvider.overrideWith(
            () => channels = _DeferredChannelsNotifier(
              hiddenDmIds: const {'hidden-dm'},
            ),
          ),
          channelMembersProvider('hidden-dm').overrideWith(
            (ref) async => [
              ChannelMember(
                pubkey: self,
                role: 'member',
                joinedAt: DateTime(2026),
              ),
              ChannelMember(
                pubkey: alice,
                role: 'member',
                joinedAt: DateTime(2026),
              ),
            ],
          ),
          dmResurfaceActionProvider.overrideWithValue((pubkeys) async {
            reopened.add(pubkeys);
            return 'hidden-dm';
          }),
        ],
      );
      addTearDown(container.dispose);

      container.read(channelsProvider);
      await container.read(activityProvider.future);
      await Future<void>.delayed(const Duration(milliseconds: 10));
      session.emit(
        const NostrEvent(
          id: 'hidden-dm-during-discovery',
          pubkey: alice,
          createdAt: 1_700_000_000,
          kind: EventKind.streamMessageV2,
          tags: [
            ['p', self],
            ['h', 'hidden-dm'],
          ],
          content: 'Hello during startup',
          sig: '',
        ),
      );
      expect(reopened, isEmpty);

      channels.complete(const <Channel>[]);
      await container.read(channelsProvider.future);
      await _waitFor(() => reopened.isNotEmpty);
      expect(reopened, [
        [alice],
      ]);
    },
  );

  test('a concurrent follower survives a failed in-flight reopen', () async {
    const self =
        '1111111111111111111111111111111111111111111111111111111111111111';
    const alice =
        '2222222222222222222222222222222222222222222222222222222222222222';
    final session = _RecordingSessionNotifier();
    final reopenAttempts = <List<String>>[];
    final firstReopenGate = Completer<void>();
    var reopenCalls = 0;
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(_FixedRelayConfigNotifier.new),
        myPubkeyProvider.overrideWithValue(self),
        relaySessionProvider.overrideWith(() => session),
        channelsProvider.overrideWith(
          () => _FixedChannelsNotifier(
            const <Channel>[],
            hiddenDmIds: const {'hidden-dm'},
          ),
        ),
        channelMembersProvider('hidden-dm').overrideWith(
          (ref) async => [
            ChannelMember(
              pubkey: self,
              role: 'member',
              joinedAt: DateTime(2026),
            ),
            ChannelMember(
              pubkey: alice,
              role: 'member',
              joinedAt: DateTime(2026),
            ),
          ],
        ),
        dmResurfaceActionProvider.overrideWithValue((pubkeys) async {
          reopenCalls += 1;
          reopenAttempts.add(pubkeys);
          if (reopenCalls == 1) {
            // Hold the first attempt open so the follower coalesces into it,
            // then fail it — the follower must not be silently dropped.
            await firstReopenGate.future;
            throw StateError('transient reopen failure');
          }
          return 'hidden-dm';
        }),
      ],
    );
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    await container.read(activityProvider.future);
    await Future<void>.delayed(const Duration(milliseconds: 10));

    NostrEvent hiddenDmEvent(String id) => NostrEvent(
      id: id,
      pubkey: alice,
      createdAt: 1_700_000_000,
      kind: EventKind.streamMessageV2,
      tags: const [
        ['p', self],
        ['h', 'hidden-dm'],
      ],
      content: 'Hello again',
      sig: '',
    );

    session.emit(hiddenDmEvent('message-a'));
    await _waitFor(() => reopenCalls == 1);
    // Follower arrives while attempt A is in flight → coalesced for retry.
    session.emit(hiddenDmEvent('message-b'));
    await Future<void>.delayed(const Duration(milliseconds: 10));
    firstReopenGate.complete();

    await _waitFor(() => reopenCalls >= 2);
    expect(reopenAttempts, [
      [alice],
      [alice],
    ]);
  });

  test(
    'a follower on a rebuilt subscription reopens after the old attempt retires',
    () async {
      const self =
          '1111111111111111111111111111111111111111111111111111111111111111';
      const alice =
          '2222222222222222222222222222222222222222222222222222222222222222';
      final session = _RecordingSessionNotifier();
      final reopenAttempts = <String>[];
      final gateA = Completer<void>();
      final gateB = Completer<void>();
      var reopenCalls = 0;
      final container = ProviderContainer(
        overrides: [
          relayConfigProvider.overrideWith(_FixedRelayConfigNotifier.new),
          myPubkeyProvider.overrideWithValue(self),
          relaySessionProvider.overrideWith(() => session),
          channelsProvider.overrideWith(
            () => _FixedChannelsNotifier(
              const <Channel>[],
              hiddenDmIds: const {'hidden-dm'},
            ),
          ),
          channelMembersProvider('hidden-dm').overrideWith(
            (ref) async => [
              ChannelMember(
                pubkey: self,
                role: 'member',
                joinedAt: DateTime(2026),
              ),
              ChannelMember(
                pubkey: alice,
                role: 'member',
                joinedAt: DateTime(2026),
              ),
            ],
          ),
          dmResurfaceActionProvider.overrideWithValue((pubkeys) async {
            final call = ++reopenCalls;
            reopenAttempts.add(pubkeys.single);
            // Suspend attempt A (call 1, old generation) and attempt B (call 2,
            // new generation) so both are in flight simultaneously: A must
            // retire while B's entry still occupies the pending map. B's first
            // attempt then fails so its retry loop drains the coalesced
            // follower C as call 3.
            if (call == 1) await gateA.future;
            if (call == 2) {
              await gateB.future;
              throw StateError('transient reopen failure');
            }
            return 'hidden-dm';
          }),
        ],
      );
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      await container.read(activityProvider.future);
      await Future<void>.delayed(const Duration(milliseconds: 10));

      NostrEvent hiddenDmEvent(String id) => NostrEvent(
        id: id,
        pubkey: alice,
        createdAt: 1_700_000_000,
        kind: EventKind.streamMessageV2,
        tags: const [
          ['p', self],
          ['h', 'hidden-dm'],
        ],
        content: 'Hello again',
        sig: '',
      );

      // Attempt A starts on generation N and suspends inside the reopen.
      session.emit(hiddenDmEvent('message-a'));
      await _waitFor(() => reopenCalls == 1);

      // Activity rebuilds (generation N+1). Follower B lands on the replacement
      // subscription: A's entry belongs to the old generation, so B does not
      // coalesce — it installs its own entry and starts a second attempt, which
      // suspends. A and B are now both in flight.
      container.invalidate(activityProvider);
      await container.read(activityProvider.future);
      await Future<void>.delayed(const Duration(milliseconds: 10));
      session.emit(hiddenDmEvent('message-b'));
      await _waitFor(() => reopenCalls == 2);

      // A retires while B is still pending. Its instance-checked cleanup must
      // leave B's entry in the map; unconditional removal would evict B's live
      // entry here.
      gateA.complete();
      await Future<void>.delayed(const Duration(milliseconds: 10));

      // Follower C arrives on the current generation. Because B's entry is still
      // present, C coalesces into it (no new attempt). With unconditional
      // cleanup, A would have evicted B's entry and C would wrongly start a
      // third overlapping attempt.
      session.emit(hiddenDmEvent('message-c'));
      await Future<void>.delayed(const Duration(milliseconds: 10));
      expect(reopenCalls, 2);

      // Releasing B fails its first attempt; the coalesced follower C drives one
      // retry, which succeeds as the third reopen.
      gateB.complete();
      await _waitFor(() => reopenCalls == 3);
      await Future<void>.delayed(const Duration(milliseconds: 10));
      expect(reopenAttempts, [alice, alice, alice]);
    },
  );

  test('a suspended membership read cannot mutate a rebuilt scope', () async {
    const self =
        '1111111111111111111111111111111111111111111111111111111111111111';
    const alice =
        '2222222222222222222222222222222222222222222222222222222222222222';
    final session = _RecordingSessionNotifier();
    final membersRequested = Completer<void>();
    final membersGate = Completer<List<ChannelMember>>();
    var reopenCount = 0;
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(_FixedRelayConfigNotifier.new),
        myPubkeyProvider.overrideWithValue(self),
        relaySessionProvider.overrideWith(() => session),
        channelsProvider.overrideWith(
          () => _FixedChannelsNotifier(
            const <Channel>[],
            hiddenDmIds: const {'hidden-dm'},
          ),
        ),
        channelMembersProvider('hidden-dm').overrideWith((ref) {
          if (!membersRequested.isCompleted) membersRequested.complete();
          return membersGate.future;
        }),
        dmResurfaceActionProvider.overrideWithValue((pubkeys) async {
          reopenCount += 1;
          return 'hidden-dm';
        }),
      ],
    );
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    await container.read(activityProvider.future);
    await Future<void>.delayed(const Duration(milliseconds: 10));
    session.emit(
      const NostrEvent(
        id: 'hidden-dm-stale-message',
        pubkey: alice,
        createdAt: 1_700_000_000,
        kind: EventKind.streamMessageV2,
        tags: [
          ['p', self],
          ['h', 'hidden-dm'],
        ],
        content: 'Hello again',
        sig: '',
      ),
    );
    await membersRequested.future;
    container.invalidate(activityProvider);
    await container.read(activityProvider.future);
    membersGate.complete([
      ChannelMember(pubkey: self, role: 'member', joinedAt: DateTime(2026)),
      ChannelMember(pubkey: alice, role: 'member', joinedAt: DateTime(2026)),
    ]);
    await Future<void>.delayed(const Duration(milliseconds: 20));
    expect(reopenCount, 0);
  });

  test(
    'serializes live refreshes and catches up events queued mid-fetch',
    () async {
      final session = _RecordingSessionNotifier();
      final container = ProviderContainer(
        overrides: [
          relayConfigProvider.overrideWith(_FixedRelayConfigNotifier.new),
          myPubkeyProvider.overrideWithValue('me_pk'),
          relaySessionProvider.overrideWith(() => session),
          channelsProvider.overrideWith(
            () => _FixedChannelsNotifier(const <Channel>[]),
          ),
        ],
      );
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      await container.read(activityProvider.future);
      await Future<void>.delayed(const Duration(milliseconds: 10));

      session.mentionFetchGate = Completer<void>();
      session.emit(_mentionEvent('live-one', 1_700_000_001));
      await _waitFor(() => session.activeMentionFetches == 1);

      session.emit(_mentionEvent('live-two', 1_700_000_002));
      await Future<void>.delayed(const Duration(milliseconds: 100));
      expect(session.maxActiveMentionFetches, 1);

      session.mentionFetchGate!.complete();
      await _waitFor(() => session.mentionFetchCount >= 3);
      await _waitFor(() => container.read(inboxItemsProvider).length == 2);

      expect(session.maxActiveMentionFetches, 1);
      expect(
        container.read(inboxItemsProvider).map((item) => item.id),
        containsAll(['live-one', 'live-two']),
      );
    },
  );

  test('serializes manual and live inbox refreshes', () async {
    final session = _RecordingSessionNotifier();
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(_FixedRelayConfigNotifier.new),
        myPubkeyProvider.overrideWithValue('me_pk'),
        relaySessionProvider.overrideWith(() => session),
        channelsProvider.overrideWith(
          () => _FixedChannelsNotifier(const <Channel>[]),
        ),
      ],
    );
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    await container.read(activityProvider.future);
    await Future<void>.delayed(const Duration(milliseconds: 10));

    session.mentionFetchGate = Completer<void>();
    final manualRefresh = container.read(activityProvider.notifier).refresh();
    await _waitFor(() => session.activeMentionFetches == 1);

    session.emit(_mentionEvent('live-during-manual', 1_700_000_003));
    await Future<void>.delayed(const Duration(milliseconds: 100));
    expect(session.maxActiveMentionFetches, 1);

    session.mentionFetchGate!.complete();
    await manualRefresh;
    await _waitFor(() => session.mentionFetchCount >= 3);
    await _waitFor(
      () =>
          container.read(inboxItemsProvider).single.id == 'live-during-manual',
    );

    expect(session.maxActiveMentionFetches, 1);
  });

  test('recovers a live refresh through websocket history fallback', () async {
    final session = _RecordingSessionNotifier()
      ..seed(_mentionEvent('existing', 1_700_000_001));
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(_FixedRelayConfigNotifier.new),
        myPubkeyProvider.overrideWithValue('me_pk'),
        relaySessionProvider.overrideWith(() => session),
        channelsProvider.overrideWith(
          () => _FixedChannelsNotifier(const <Channel>[]),
        ),
      ],
    );
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    await container.read(activityProvider.future);
    await Future<void>.delayed(const Duration(milliseconds: 10));
    expect(container.read(inboxItemsProvider).single.id, 'existing');

    session.failNextMentionFetch = true;
    session.emit(_mentionEvent('newer', 1_700_000_002));
    await _waitFor(() => session.mentionFetchCount >= 2);
    await Future<void>.delayed(const Duration(milliseconds: 10));

    expect(
      container.read(inboxItemsProvider).map((item) => item.id),
      containsAll(['existing', 'newer']),
    );
  });

  group('hidden-DM subscription batching', () {
    const self =
        '1111111111111111111111111111111111111111111111111111111111111111';

    Set<String> hiddenIds(int count) => {
      for (var i = 0; i < count; i++) 'dm-${i.toString().padLeft(4, '0')}',
    };

    ProviderContainer containerFor(
      _RecordingSessionNotifier session,
      Set<String> hidden,
    ) => ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(_FixedRelayConfigNotifier.new),
        myPubkeyProvider.overrideWithValue(self),
        relaySessionProvider.overrideWith(() => session),
        channelsProvider.overrideWith(
          () => _FixedChannelsNotifier(const <Channel>[], hiddenDmIds: hidden),
        ),
      ],
    );

    test(
      '128 hidden DMs register a single subscription within the cap',
      () async {
        final session = _RecordingSessionNotifier();
        final container = containerFor(session, hiddenIds(128));
        addTearDown(container.dispose);

        await container.read(channelsProvider.future);
        await container.read(activityProvider.future);
        await _waitFor(() => session.hiddenDmSubscriptionBatches.isNotEmpty);

        final batches = session.hiddenDmSubscriptionBatches;
        expect(batches, hasLength(1));
        expect(batches.single, hasLength(128));
      },
    );

    test(
      '129 hidden DMs split into two subscriptions, both within the cap',
      () async {
        final session = _RecordingSessionNotifier();
        final container = containerFor(session, hiddenIds(129));
        addTearDown(container.dispose);

        await container.read(channelsProvider.future);
        await container.read(activityProvider.future);
        await _waitFor(() => session.hiddenDmSubscriptionBatches.length == 2);

        final batches = session.hiddenDmSubscriptionBatches;
        expect(batches.map((batch) => batch.length), [128, 1]);
        final all = batches.expand((batch) => batch).toSet();
        expect(all, hasLength(129));
        for (final batch in batches) {
          expect(batch.length, lessThanOrEqualTo(128));
        }
      },
    );

    test('activity on the final batch resurfaces its DM', () async {
      const alice =
          '2222222222222222222222222222222222222222222222222222222222222222';
      final hidden = hiddenIds(129);
      final target = 'dm-0128';
      final session = _RecordingSessionNotifier();
      final reopened = <List<String>>[];
      final container = ProviderContainer(
        overrides: [
          relayConfigProvider.overrideWith(_FixedRelayConfigNotifier.new),
          myPubkeyProvider.overrideWithValue(self),
          relaySessionProvider.overrideWith(() => session),
          channelsProvider.overrideWith(
            () =>
                _FixedChannelsNotifier(const <Channel>[], hiddenDmIds: hidden),
          ),
          channelMembersProvider(target).overrideWith(
            (ref) async => [
              ChannelMember(
                pubkey: self,
                role: 'member',
                joinedAt: DateTime(2026),
              ),
              ChannelMember(
                pubkey: alice,
                role: 'member',
                joinedAt: DateTime(2026),
              ),
            ],
          ),
          dmResurfaceActionProvider.overrideWithValue((pubkeys) async {
            reopened.add(pubkeys);
            return target;
          }),
        ],
      );
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      await container.read(activityProvider.future);
      await _waitFor(() => session.hiddenDmSubscriptionBatches.length == 2);

      // The target lives only in the second batch; delivering it exercises
      // that batch's live subscription.
      session.emit(
        NostrEvent(
          id: 'final-batch-message',
          pubkey: alice,
          createdAt: 1_700_000_000,
          kind: EventKind.streamMessageV2,
          tags: [
            ['p', self],
            ['h', target],
          ],
          content: 'Hello again',
          sig: '',
        ),
      );

      await _waitFor(() => reopened.isNotEmpty);
      expect(reopened.single, [alice]);
    });

    test('a batch subscription failure does not abort later batches', () async {
      final session = _RecordingSessionNotifier()
        ..failNextHiddenSubscribe = true;
      final container = containerFor(session, hiddenIds(129));
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      await container.read(activityProvider.future);
      // The first batch throws; the loop continues and registers the second.
      await _waitFor(() => session.hiddenDmSubscriptionBatches.isNotEmpty);

      final batches = session.hiddenDmSubscriptionBatches;
      expect(batches, hasLength(1));
      expect(batches.single, hasLength(1));
    });

    test('teardown during batch-1 setup self-disposes batch 1 and never starts '
        'batch 2', () async {
      final session = _RecordingSessionNotifier()
        ..hiddenSubscribeGatesByCall[0] = Completer<void>();
      final container = containerFor(session, hiddenIds(129));

      await container.read(channelsProvider.future);
      await container.read(activityProvider.future);
      // Batch 1's REQ is parked in flight; batch 2 has not been requested.
      await _waitFor(() => session.hiddenSubscribeCallCount == 1);

      // Supersede this generation mid-setup, then let batch 1's REQ resolve.
      container.dispose();
      session.hiddenSubscribeGatesByCall[0]!.complete();

      // Batch 1 resolves stale and self-disposes; batch 2 is never requested.
      await _waitFor(() => session.hiddenUnsubscribeCount == 1);
      expect(session.hiddenUnsubscribeCount, 1);
      expect(session.hiddenSubscribeCallCount, 1);
      expect(session.hiddenDmSubscriptionBatches, isEmpty);
    });

    test(
      'teardown after batch 1 settles while batch 2 is pending tears down both',
      () async {
        final session = _RecordingSessionNotifier()
          ..hiddenSubscribeGatesByCall[1] = Completer<void>();
        final container = containerFor(session, hiddenIds(129));

        await container.read(channelsProvider.future);
        await container.read(activityProvider.future);
        // Batch 1 has settled and registered; batch 2's REQ is parked in flight.
        await _waitFor(() => session.hiddenSubscribeCallCount == 2);
        expect(session.hiddenDmSubscriptionBatches, hasLength(1));

        // Supersede this generation with batch 2 pending, then let it resolve.
        container.dispose();
        session.hiddenSubscribeGatesByCall[1]!.complete();

        // Batch 2 resolves stale and disposes itself plus the settled batch 1.
        await _waitFor(() => session.hiddenUnsubscribeCount == 2);
        expect(session.hiddenUnsubscribeCount, 2);
        expect(session.hiddenDmSubscriptionBatches, isEmpty);
      },
    );

    test(
      '129 visible DMs batch under the cap and hidden activity still resurfaces',
      () async {
        // An over-cap visible-DM set previously rejected as one REQ, aborting
        // the whole live setup — so the hidden batches never registered and
        // resurfacing silently died. Batching the visible set keeps every REQ
        // within the cap and leaves the hidden subscription intact.
        const alice =
            '2222222222222222222222222222222222222222222222222222222222222222';
        final visibleChannels = [
          for (var i = 0; i < 129; i++)
            _dmChannel('visible-${i.toString().padLeft(4, '0')}'),
        ];
        const hiddenTarget = 'hidden-dm';
        final session = _RecordingSessionNotifier();
        final reopened = <List<String>>[];
        final container = ProviderContainer(
          overrides: [
            relayConfigProvider.overrideWith(_FixedRelayConfigNotifier.new),
            myPubkeyProvider.overrideWithValue(self),
            relaySessionProvider.overrideWith(() => session),
            channelsProvider.overrideWith(
              () => _FixedChannelsNotifier(
                visibleChannels,
                hiddenDmIds: const {hiddenTarget},
              ),
            ),
            channelMembersProvider(hiddenTarget).overrideWith(
              (ref) async => [
                ChannelMember(
                  pubkey: self,
                  role: 'member',
                  joinedAt: DateTime(2026),
                ),
                ChannelMember(
                  pubkey: alice,
                  role: 'member',
                  joinedAt: DateTime(2026),
                ),
              ],
            ),
            dmResurfaceActionProvider.overrideWithValue((pubkeys) async {
              reopened.add(pubkeys);
              return hiddenTarget;
            }),
          ],
        );
        addTearDown(container.dispose);

        await container.read(channelsProvider.future);
        await container.read(activityProvider.future);
        await _waitFor(() => session.hiddenDmSubscriptionBatches.isNotEmpty);

        // Every visible-DM REQ stays within the cap, split 128 + 1.
        final visibleBatches = session.visibleDmSubscriptionBatches;
        expect(visibleBatches.map((batch) => batch.length), [128, 1]);
        expect(visibleBatches.expand((batch) => batch).toSet(), hasLength(129));
        // The hidden subscription registered despite the over-cap visible set.
        expect(session.hiddenDmSubscriptionBatches, hasLength(1));

        // Hidden activity still resurfaces its DM.
        session.emit(
          NostrEvent(
            id: 'hidden-message',
            pubkey: alice,
            createdAt: 1_700_000_000,
            kind: EventKind.streamMessageV2,
            tags: [
              ['p', self],
              ['h', hiddenTarget],
            ],
            content: 'Hello again',
            sig: '',
          ),
        );

        await _waitFor(() => reopened.isNotEmpty);
        expect(reopened.single, [alice]);
      },
    );
  });
}

class _FixedChannelsNotifier extends ChannelsNotifier {
  final List<Channel> channels;
  final Set<String> _hiddenDmIds;

  _FixedChannelsNotifier(this.channels, {Set<String> hiddenDmIds = const {}})
    : _hiddenDmIds = hiddenDmIds;

  @override
  bool get hasLoaded => true;

  @override
  Set<String> get hiddenDmIds => _hiddenDmIds;

  @override
  Future<List<Channel>> build() async => channels;
}

class _DeferredChannelsNotifier extends ChannelsNotifier {
  _DeferredChannelsNotifier({required Set<String> hiddenDmIds})
    : _hiddenDmIds = hiddenDmIds;

  final Set<String> _hiddenDmIds;
  final _gate = Completer<List<Channel>>();
  bool _hasLoaded = false;

  @override
  bool get hasLoaded => _hasLoaded;

  @override
  Set<String> get hiddenDmIds => _hiddenDmIds;

  @override
  Future<List<Channel>> build() => _gate.future;

  void complete(List<Channel> channels) {
    _hasLoaded = true;
    _gate.complete(channels);
  }
}
