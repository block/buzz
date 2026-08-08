import 'dart:async';

import 'package:buzz/features/activity/activity_provider.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

/// Records subscriptions and DM history queries for Activity projection tests.
class _RecordingSessionNotifier extends RelaySessionNotifier {
  final List<List<String>> dmQueries = [];
  final List<NostrEvent> _history = [];
  final List<({NostrFilter filter, void Function(NostrEvent) onEvent})>
  _subscriptions = [];
  Completer<void>? mentionFetchGate;
  bool failNextMentionFetch = false;
  int mentionFetchCount = 0;
  int activeMentionFetches = 0;
  int maxActiveMentionFetches = 0;

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
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    final subscription = (filter: filter, onEvent: onEvent);
    _subscriptions.add(subscription);
    return () => _subscriptions.remove(subscription);
  }

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

    // Channel list resolves with a DM → Activity must rebuild and query it.
    channels.resolve([_dmChannel('dm1')]);
    await container.read(channelsProvider.future);
    await container.read(activityProvider.future);

    expect(session.dmQueries, hasLength(1));
    expect(session.dmQueries.single, ['dm1']);
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

  test('retains the loaded inbox when a live refresh fails', () async {
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

    expect(container.read(inboxItemsProvider).single.id, 'existing');
  });
}

class _FixedChannelsNotifier extends ChannelsNotifier {
  final List<Channel> channels;

  _FixedChannelsNotifier(this.channels);

  @override
  Future<List<Channel>> build() async => channels;
}
